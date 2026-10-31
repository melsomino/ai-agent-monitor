//! A bounded, state-driven Telegram view of the native sessions list.
use super::{Pending, Session, nonce, short, text::Text};
use crate::{
    actors::monitor,
    agents::sessions::{Activity, Client},
};
use anyhow::{Result, bail};
use objc2_foundation::{NSComparisonResult, NSString};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const PAGE_SIZE: usize = 30;

pub(super) struct Browser {
    pub message: Option<i64>,
    pub rendered: Value,
    nonce: String,
    page: usize,
    selected: Option<String>,
    selected_available: bool,
    // IDs never change when sessions are reordered or removed.
    keys: Vec<String>,
    active_request: Option<(String, String)>,
    staged_request: Option<(String, String)>,
    previous_requested: bool,
    earlier: Option<crate::domain::history::Message>,
    history_waiting: Option<(u64, Instant)>,
    history_note: Option<String>,
    history_exhausted: bool,
    outgoing: Option<String>,
    show_info: bool,
    delivered_agent: Option<(String, String)>,
    delivered_session: Option<String>,
    delivered_interaction: Option<String>,
    staged_interaction: Option<String>,
    staged_agent: Option<(String, String)>,
}
impl Browser {
    pub fn new() -> Result<Self> {
        Ok(Self {
            message: None,
            rendered: Value::Null,
            nonce: nonce()?,
            page: 0,
            selected: None,
            selected_available: false,
            keys: vec![],
            active_request: None,
            staged_request: None,
            previous_requested: false,
            earlier: None,
            history_waiting: None,
            history_note: None,
            history_exhausted: false,
            outgoing: None,
            show_info: false,
            delivered_agent: None,
            delivered_session: None,
            delivered_interaction: None,
            staged_interaction: None,
            staged_agent: None,
        })
    }
    pub fn selected(&self, message: i64) -> Option<&str> {
        (self.message == Some(message))
            .then_some(self.selected.as_deref())
            .flatten()
    }
    pub fn current_session(&self) -> Option<&str> {
        self.selected.as_deref()
    }
    fn reset_detail(&mut self) {
        self.earlier = None;
        self.history_waiting = None;
        self.history_note = None;
        self.history_exhausted = false;
        self.previous_requested = false;
        self.outgoing = None;
        self.show_info = false;
    }
    pub fn track_send(&mut self, id: String) {
        self.reset_detail();
        self.outgoing = Some(id);
    }
    pub fn history_error(&mut self, error: String) {
        self.history_waiting = None;
        self.history_note = Some(error);
    }
    pub fn history_command(&mut self, state: &monitor::State) -> Option<monitor::Command> {
        if !std::mem::take(&mut self.previous_requested)
            || self.history_waiting.is_some()
            || self.history_exhausted
        {
            return None;
        }
        let session = self.selected.clone()?;
        if !state
            .sessions
            .iter()
            .any(|s| s.key() == session && state.preferences.applications.enabled(s.client))
        {
            return None;
        }
        let history = state.history.get(&session);
        if history.is_some_and(|history| history.loading) {
            self.history_note =
                Some("History is loading. Try Previous message again shortly.".into());
            return None;
        }
        let generation = history.map_or(1, |history| history.generation.wrapping_add(1));
        self.history_waiting = Some((generation, Instant::now()));
        self.history_note = None;
        Some(monitor::Command::History {
            session,
            cursor: self.earlier.as_ref().map(|message| message.cursor.clone()),
            generation,
        })
    }
    fn reconcile_history(&mut self, state: &monitor::State) {
        let Some((generation, started)) = self.history_waiting else {
            return;
        };
        if let Some(history) = self
            .selected
            .as_ref()
            .and_then(|key| state.history.get(key))
            && history.generation == generation
            && !history.loading
        {
            self.history_waiting = None;
            self.history_exhausted = history.exhausted;
            self.history_note = history.error.clone();
            if !history.exhausted && history.error.is_none() {
                if let Some(message) = history
                    .messages
                    .iter()
                    .find(|message| Some(&message.cursor) == history.cursor.as_ref())
                {
                    self.earlier = Some(message.clone());
                }
            } else if history.exhausted {
                self.history_note = Some("No earlier messages.".into());
            }
        } else if started.elapsed() > Duration::from_secs(10) {
            self.history_error("History request interrupted. Try Previous message again.".into());
        }
    }
    pub fn open_info(&mut self) -> bool {
        if self.selected.is_none() {
            return false;
        }
        self.show_info = true;
        true
    }
    pub fn needs_new_message(&self) -> bool {
        let new_text = self
            .delivered_agent
            .as_ref()
            .zip(self.staged_agent.as_ref())
            .is_some_and(|((old_key, old), (key, text))| {
                old_key == key && old != text && !text.is_empty()
            });
        let new_request = self.selected.is_some()
            && self.delivered_session == self.selected
            && self
                .staged_request
                .as_ref()
                .is_some_and(|(key, _)| self.delivered_interaction.as_ref() != Some(key));
        self.message.is_some() && (new_text || new_request)
    }
    pub fn detach_message(&mut self) {
        self.message = None;
        self.rendered = Value::Null;
        self.active_request = None;
    }
    pub fn commit(&mut self, content: Value) {
        self.rendered = content;
        self.active_request = self.staged_request.clone();
        self.delivered_agent = self.staged_agent.clone();
        self.delivered_session = self.selected.clone();
        self.delivered_interaction = self.staged_interaction.clone();
    }
    pub fn can_present(&self, pending: &Pending) -> bool {
        self.selected_available
            && !self.show_info
            && self
                .current_session()
                .is_some_and(|key| key == pending.session.key())
    }
    pub fn permits_answer(&self, message: i64, pending: &Pending) -> bool {
        !self.show_info
            && self
                .selected(message)
                .is_some_and(|key| key == pending.session.key())
            && self.active_request.as_ref().is_some_and(|(key, nonce)| {
                pending.session.approval_key().as_ref() == Some(key) && *nonce == pending.nonce
            })
    }
    pub fn navigate(&mut self, message: i64, data: &str) -> Result<bool> {
        let Some(command) = data.strip_prefix(&format!("{}:", self.nonce)) else {
            return Ok(false);
        };
        if self.message != Some(message) {
            bail!("This session list is no longer active. Use Sessions.");
        }
        if let Some(index) = command.strip_prefix("s:") {
            self.reset_detail();
            self.selected = Some(
                self.keys
                    .get(index.parse::<usize>()?)
                    .ok_or_else(|| anyhow::anyhow!("Session unavailable."))?
                    .clone(),
            );

            self.selected_available = true;
        } else if let Some(page) = command.strip_prefix("p:") {
            self.page = page.parse::<usize>()?;
            self.selected = None;
        } else if command == "back" {
            self.selected = None;
            self.reset_detail();
        } else if command == "info" {
            self.show_info = true;
        } else if command == "message" {
            self.show_info = false;
        } else if command == "previous" {
            self.previous_requested = true;
        } else if command == "latest" {
            self.reset_detail();
        } else if command != "refresh" {
            bail!("Unknown session action.");
        }
        Ok(true)
    }
    fn button(&self, text: &str, command: &str) -> Value {
        json!({"text":short(text,79),"callback_data":format!("{}:{command}",self.nonce)})
    }
    pub fn content(&mut self, state: &monitor::State, pending: &HashMap<String, Pending>) -> Value {
        self.reconcile_history(state);
        let mut sessions: Vec<_> = state
            .sessions
            .iter()
            .filter(|s| state.preferences.applications.enabled(s.client))
            .collect();
        sessions.sort_by(|a, b| {
            client_order(a.client)
                .cmp(&client_order(b.client))
                .then_with(|| a.client.application_id().cmp(&b.client.application_id()))
                .then_with(|| {
                    match NSString::from_str(&a.title)
                        .localizedStandardCompare(&NSString::from_str(&b.title))
                    {
                        NSComparisonResult::Ascending => std::cmp::Ordering::Less,
                        NSComparisonResult::Descending => std::cmp::Ordering::Greater,
                        _ => a.key().cmp(&b.key()),
                    }
                })
        });
        self.selected_available = self
            .selected
            .as_ref()
            .is_some_and(|key| sessions.iter().any(|session| session.key() == *key));
        self.staged_agent = self.selected.as_ref().and_then(|selected| {
            let session = sessions.iter().find(|s| s.key() == *selected)?;
            if session.last_message.is_empty() {
                self.delivered_agent
                    .clone()
                    .filter(|(key, _)| key == selected)
            } else {
                Some((selected.clone(), session.last_message.clone()))
            }
        });
        let pending_request = self.selected.as_ref().and_then(|selected| {
            sessions
                .iter()
                .find(|session| session.key() == *selected)?
                .approval_key()
                .and_then(|key| pending.get(&key).map(|p| (key, p.nonce.clone())))
        });
        self.staged_interaction = pending_request.as_ref().map(|(key, _)| key.clone());
        self.staged_request = if self.show_info {
            None
        } else {
            pending_request.clone()
        };
        if self.needs_new_message() {
            // New output or a new request must notify, not just silently edit
            // the previous message while Telegram may be in the background.
            let outgoing = self.outgoing.take();
            self.reset_detail();
            self.outgoing = outgoing;
            self.staged_request = pending_request;
        }
        let mut rows = Vec::new();
        let text;
        if let Some(key) = &self.selected {
            if let Some(session) = sessions.iter().find(|s| s.key() == *key) {
                text = if self.show_info {
                    self.info(session)
                } else {
                    self.detail(session, state, pending, &mut rows)
                };
            } else {
                let mut value = Text::default();
                value.plain("This session is closed or hidden in Applications settings.");
                text = value;
            }
            if !self.show_info && sessions.iter().any(|session| session.key() == *key) {
                let mut history = vec![self.button(
                    if self.history_waiting.is_some() {
                        "Loading…"
                    } else {
                        "↑ Previous message"
                    },
                    "previous",
                )];
                if self.earlier.is_some() {
                    history.push(self.button("Latest message", "latest"));
                }
                rows.push(json!(history));
            }
            rows.push(json!([
                self.button("‹ Sessions", "back"),
                if self.show_info {
                    self.button("‹ Message", "message")
                } else {
                    self.button("Info", "info")
                },
            ]));
        } else {
            let pages = sessions.len().div_ceil(PAGE_SIZE).max(1);
            self.page = self.page.min(pages - 1);
            let caption = if sessions.is_empty() {
                "No open agent sessions.".to_owned()
            } else if pages > 1 {
                format!(
                    "Sessions ({}) · {} / {pages}\nSelect a session to expand it.",
                    sessions.len(),
                    self.page + 1
                )
            } else {
                format!(
                    "Sessions ({})\nSelect a session to expand it.",
                    sessions.len()
                )
            };
            let mut value = Text::default();
            value.plain(&caption);
            text = value;
            for session in sessions.iter().skip(self.page * PAGE_SIZE).take(PAGE_SIZE) {
                let key = session.key();
                let index = if let Some(index) = self.keys.iter().position(|k| *k == key) {
                    index
                } else {
                    self.keys.push(key);
                    self.keys.len() - 1
                };
                let title = session
                    .title
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let status = status_symbol(session);
                rows.push(json!([self.button(
                    &format!(
                        "{} {}  {status}›",
                        app_symbol(session.client),
                        short(&title, 64)
                    ),
                    &format!("s:{index}")
                )]));
            }
            let mut navigation = Vec::new();
            if self.page > 0 {
                navigation.push(self.button("‹ Previous", &format!("p:{}", self.page - 1)));
            }
            if self.page + 1 < pages {
                navigation.push(self.button("Next ›", &format!("p:{}", self.page + 1)));
            }
            if !navigation.is_empty() {
                rows.push(json!(navigation));
            }
        }
        text.payload(json!({"inline_keyboard":rows}))
    }
    fn detail(
        &self,
        session: &Session,
        state: &monitor::State,
        pending: &HashMap<String, Pending>,
        rows: &mut Vec<Value>,
    ) -> Text {
        let mut text = self.header(session);
        let message = self
            .earlier
            .as_ref()
            .map_or(session.last_message.as_str(), |message| {
                message.text.as_str()
            });
        if self.earlier.is_some() {
            text.plain("\n\nEarlier message");
        }
        if !message.trim().is_empty() {
            text.plain("\n\n");
            text.markdown(message.trim(), 1700);
        }
        if let Some(note) = &self.history_note {
            text.plain(&format!("\n\n{note}"));
        }
        if let Some(send) = self
            .outgoing
            .as_ref()
            .and_then(|id| state.message_sends.get(id))
        {
            match &send.status {
                monitor::MessageStatus::Sending => text.plain("\n\nSending message…"),
                monitor::MessageStatus::Sent => {}
                monitor::MessageStatus::Failed(error) => {
                    text.plain(&format!("\n\n{}", short(error, 300)))
                }
            }
        }
        if let Some(request) = session.approval_key().and_then(|key| pending.get(&key)) {
            let (body, keyboard) = request.body_content();
            text.plain("\n\n");
            text.append_limited(body, 1500);
            rows.extend(
                keyboard["inline_keyboard"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
        } else if let Some(approval) = &session.approval {
            text.plain("\n\n");
            text.markdown(&approval.body, 1500);
        }
        text
    }
    fn header(&self, session: &Session) -> Text {
        let mut text = Text::default();
        text.plain(&format!("{} ", app_symbol(session.client)));
        text.bold(&session.title);
        let status = status_symbol(session).trim();
        if !status.is_empty() {
            text.plain(&format!("  {status}"));
        }
        text
    }
    fn info(&self, session: &Session) -> Text {
        let mut text = self.header(session);
        let directory = session.working_directory();
        let home = std::env::var_os("HOME");
        let path = home
            .as_ref()
            .and_then(|home| directory.strip_prefix(home).ok())
            .map(|relative| {
                if relative.as_os_str().is_empty() {
                    "~".into()
                } else {
                    format!("~/{}", relative.display())
                }
            })
            .unwrap_or_else(|| directory.display().to_string());
        text.plain("\n\n📁 ");
        text.code(&short(&path, 240));
        if let Some(branch) = &session.git_branch {
            text.plain("\n⑂ ");
            text.code(&short(branch, 160));
        }
        text
    }
}
fn app_symbol(client: Client) -> &'static str {
    match client {
        Client::VSCode => "💻",
        Client::Terminal => "⌨️",
        Client::Desktop { name: "Codex", .. } => "🤖",
        Client::Desktop { .. } => "🖥",
        Client::Unknown => "◻️",
    }
}
fn client_order(client: Client) -> u8 {
    match client {
        Client::VSCode => 0,
        Client::Terminal => 1,
        Client::Desktop { .. } => 2,
        Client::Unknown => 3,
    }
}
fn status_symbol(session: &Session) -> &'static str {
    if session.approval.is_some() || session.activity == Activity::Confirmation {
        "🟠 "
    } else if session.activity == Activity::Working {
        "🟢 "
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::sessions::Approval;
    fn session(id: &str, title: &str, client: Client) -> Session {
        Session {
            authoritative: true,
            agent: "codex",
            id: id.into(),
            title: title.into(),
            cwd: "/work/project".into(),
            workspace: None,
            git_branch: Some("main".into()),
            updated_at: 0,
            client,
            activity: Activity::Idle,
            last_message: "Latest message".into(),
            interaction_data: Default::default(),
            target: None,
            approval: None,
        }
    }
    fn callbacks(content: &Value) -> Vec<(String, String)> {
        content["reply_markup"]["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .map(|button| {
                (
                    button["text"].as_str().unwrap().into(),
                    button["callback_data"].as_str().unwrap().into(),
                )
            })
            .collect()
    }
    #[test]
    fn list_matches_native_groups_order_visibility_and_idle_status() {
        let mut state = monitor::State::default();
        let mut working = session("1", "Session 10", Client::VSCode);
        working.activity = Activity::Working;
        let mut asking = session("2", "Session 2", Client::VSCode);
        asking.activity = Activity::Confirmation;
        state.sessions = vec![
            session("3", "Terminal chat", Client::Terminal),
            working,
            asking,
        ];
        let mut browser = Browser::new().unwrap();
        let content = browser.content(&state, &HashMap::new());
        assert!(
            !callbacks(&content)
                .iter()
                .any(|(label, _)| label.contains("Refresh"))
        );
        let labels: Vec<_> = callbacks(&content)
            .into_iter()
            .map(|(title, _)| title)
            .collect();
        assert_eq!(
            &labels[..3],
            &[
                "💻 Session 2  🟠 ›",
                "💻 Session 10  🟢 ›",
                "⌨️ Terminal chat  ›"
            ]
        );
        assert!(
            !labels
                .iter()
                .any(|label| label == "VS Code" || label == "Terminal")
        );
        state.preferences.applications.observe([Client::Terminal]);
        state
            .preferences
            .applications
            .set_enabled(&Client::Terminal.application_id(), false);
        assert!(
            !browser
                .content(&state, &HashMap::new())
                .to_string()
                .contains("Terminal chat")
        );
    }
    #[test]
    fn callbacks_keep_session_identity_after_sort_changes_and_reject_other_messages() {
        let mut browser = Browser::new().unwrap();
        browser.message = Some(7);
        let mut state = monitor::State {
            sessions: vec![
                session("1", "Alpha", Client::VSCode),
                session("2", "Beta", Client::VSCode),
            ],
            ..Default::default()
        };
        let content = browser.content(&state, &HashMap::new());
        let data = callbacks(&content)
            .into_iter()
            .find(|(text, _)| text.contains("Alpha"))
            .unwrap()
            .1;
        state.sessions[0].title = "Zulu".into();
        browser.content(&state, &HashMap::new());
        assert!(browser.navigate(8, &data).is_err());
        assert!(browser.navigate(7, &data).unwrap());
        assert_eq!(browser.selected(7), Some("codex:1"));
        assert_eq!(browser.selected(8), None);
        let detail = browser.content(&state, &HashMap::new());
        assert!(detail["text"].as_str().unwrap().contains("Zulu"));
        assert!(detail["text"].as_str().unwrap().contains("Latest message"));
        assert!(!detail["text"].as_str().unwrap().contains("/work/project"));
        browser
            .navigate(7, &format!("{}:info", browser.nonce))
            .unwrap();
        let info = browser.content(&state, &HashMap::new());
        assert!(info["text"].as_str().unwrap().contains("/work/project"));
        assert!(info["text"].as_str().unwrap().contains("main"));
        state.sessions.clear();
        assert!(
            browser.content(&state, &HashMap::new())["text"]
                .as_str()
                .unwrap()
                .contains("closed")
        );
    }
    #[test]
    fn request_buttons_are_bound_to_delivered_request_and_disappear_on_resolution() {
        let mut s = session("1", "Chat", Client::VSCode);
        s.approval = Some(Approval {
            id: "request".into(),
            body: "Allow test?".into(),
            expires: u64::MAX,
            can_allow: true,
        });
        s.interaction_data = crate::domain::interaction::Interaction::allow();
        let p = Pending::new(s.clone()).unwrap();
        let key = s.approval_key().unwrap();
        let mut pending = HashMap::from([(key.clone(), p)]);
        let mut state = monitor::State {
            sessions: vec![s],
            ..Default::default()
        };
        let mut browser = Browser::new().unwrap();
        browser.message = Some(7);
        let list = browser.content(&state, &pending);
        let data = callbacks(&list)
            .into_iter()
            .find(|(_, data)| data.contains(":s:"))
            .unwrap()
            .1;
        browser.navigate(7, &data).unwrap();
        let detail = browser.content(&state, &pending);
        assert!(detail["text"].as_str().unwrap().contains("Allow test?"));
        assert!(
            callbacks(&detail)
                .iter()
                .any(|(_, data)| data.starts_with(&pending[&key].nonce))
        );
        assert!(!browser.permits_answer(7, &pending[&key]));
        browser.commit(detail);
        assert!(browser.permits_answer(7, &pending[&key]));
        assert!(!browser.permits_answer(8, &pending[&key]));
        pending.get_mut(&key).unwrap().nonce = "new-question-step".into();
        assert!(!browser.permits_answer(7, &pending[&key]));
        pending.clear();
        state.sessions[0].approval = None;
        let resolved = browser.content(&state, &pending);
        assert!(!resolved["text"].as_str().unwrap().contains("Allow test?"));
        assert_eq!(callbacks(&resolved).len(), 3);
    }
    #[test]
    fn previous_message_uses_monitor_history_and_latest_restores_live_message() {
        let mut state = monitor::State {
            sessions: vec![session("1", "Chat", Client::VSCode)],
            ..Default::default()
        };
        let mut browser = Browser::new().unwrap();
        browser.message = Some(7);
        let list = browser.content(&state, &HashMap::new());
        let data = callbacks(&list)
            .into_iter()
            .find(|(_, data)| data.contains(":s:"))
            .unwrap()
            .1;
        browser.navigate(7, &data).unwrap();
        browser
            .navigate(7, &format!("{}:previous", browser.nonce))
            .unwrap();
        let monitor::Command::History {
            session: key,
            cursor,
            generation,
        } = browser.history_command(&state).unwrap()
        else {
            panic!("history command expected")
        };
        assert_eq!(key, "codex:1");
        assert!(cursor.is_none());
        let previous = crate::domain::history::Message {
            cursor: "older".into(),
            text: "Previous agent message".into(),
        };
        state.history.insert(
            key.clone(),
            std::sync::Arc::new(monitor::History {
                messages: vec![previous],
                cursor: Some("older".into()),
                generation,
                ..Default::default()
            }),
        );
        let detail = browser.content(&state, &HashMap::new());
        assert!(
            detail["text"]
                .as_str()
                .unwrap()
                .contains("Previous agent message")
        );
        assert!(
            !detail["text"]
                .as_str()
                .unwrap()
                .contains("Latest message\n")
        );
        browser
            .navigate(7, &format!("{}:previous", browser.nonce))
            .unwrap();
        let monitor::Command::History { cursor, .. } = browser.history_command(&state).unwrap()
        else {
            panic!("history command expected")
        };
        assert_eq!(cursor.as_deref(), Some("older"));
        browser
            .navigate(7, &format!("{}:latest", browser.nonce))
            .unwrap();
        let latest = browser.content(&state, &HashMap::new());
        assert!(latest["text"].as_str().unwrap().contains("Latest message"));
        assert!(
            !latest["text"]
                .as_str()
                .unwrap()
                .contains("Previous agent message")
        );
    }
    #[test]
    fn rendered_message_entities_are_shifted_past_literal_header_and_clipped_safely() {
        let mut chat = session("1", "Chat **literal** 🤖", Client::VSCode);
        chat.last_message =
            "## Итог\n\n**Готово 🤖**: [ссылка](https://example.com) и `file_name.rs`".into();
        let mut state = monitor::State {
            sessions: vec![chat],
            ..Default::default()
        };
        let mut browser = Browser::new().unwrap();
        browser.message = Some(7);
        let list = browser.content(&state, &HashMap::new());
        let data = callbacks(&list)
            .into_iter()
            .find(|(_, data)| data.contains(":s:"))
            .unwrap()
            .1;
        browser.navigate(7, &data).unwrap();
        let payload = browser.content(&state, &HashMap::new());
        let text = payload["text"].as_str().unwrap();
        let utf16: Vec<_> = text.encode_utf16().collect();
        assert!(text.contains("Chat **literal** 🤖"));
        assert!(text.contains("Готово 🤖: ссылка и file_name.rs"));
        let spans: Vec<_> = payload["entities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entity| {
                let start = entity["offset"].as_u64().unwrap() as usize;
                let len = entity["length"].as_u64().unwrap() as usize;
                (
                    entity["type"].as_str().unwrap(),
                    String::from_utf16(&utf16[start..start + len]).unwrap(),
                )
            })
            .collect();
        assert!(
            spans
                .iter()
                .any(|(kind, span)| *kind == "bold" && span.contains("Готово 🤖"))
        );
        assert!(
            spans
                .iter()
                .any(|(kind, span)| *kind == "text_link" && span == "ссылка")
        );
        assert!(
            spans
                .iter()
                .any(|(kind, span)| *kind == "code" && span == "file_name.rs")
        );
        state.sessions[0].last_message = format!("**{}**", "🤖".repeat(3000));
        let clipped = browser.content(&state, &HashMap::new());
        let size = clipped["text"].as_str().unwrap().encode_utf16().count();
        assert!(size <= 3900);
        for entity in clipped["entities"].as_array().unwrap() {
            assert!(
                entity["offset"].as_u64().unwrap() + entity["length"].as_u64().unwrap()
                    <= size as u64
            );
        }
    }
    #[test]
    fn compact_header_hides_service_text_and_metadata_until_info() {
        let mut chat = session("1", "Chat", Client::VSCode);
        chat.activity = Activity::Working;
        let mut state = monitor::State {
            sessions: vec![chat],
            ..Default::default()
        };
        let mut browser = Browser::new().unwrap();
        browser.message = Some(7);
        let list = browser.content(&state, &HashMap::new());
        let data = callbacks(&list)
            .into_iter()
            .find(|(_, data)| data.contains(":s:"))
            .unwrap()
            .1;
        browser.navigate(7, &data).unwrap();
        browser.track_send("send".into());
        state.message_sends.insert(
            "send".into(),
            monitor::MessageSend {
                session: "codex:1".into(),
                status: monitor::MessageStatus::Sent,
            },
        );
        let detail = browser.content(&state, &HashMap::new());
        let text = detail["text"].as_str().unwrap();
        assert!(text.starts_with("💻 Chat  🟢\n\nLatest message"));
        for unwanted in [
            "VS Code",
            "Type a message",
            "Message sent",
            "/work/project",
            "main",
        ] {
            assert!(!text.contains(unwanted));
        }
        assert!(
            !callbacks(&detail)
                .iter()
                .any(|(label, _)| label.contains("Refresh"))
        );
        assert!(browser.open_info());
        let info = browser.content(&state, &HashMap::new());
        assert!(
            !callbacks(&info)
                .iter()
                .any(|(label, _)| label.contains("Refresh"))
        );
        assert!(info["text"].as_str().unwrap().contains("/work/project"));
        assert!(info["text"].as_str().unwrap().contains("main"));
        assert!(!info["text"].as_str().unwrap().contains("Latest message"));
        browser
            .navigate(7, &format!("{}:message", browser.nonce))
            .unwrap();
        assert!(
            browser.content(&state, &HashMap::new())["text"]
                .as_str()
                .unwrap()
                .contains("Latest message")
        );
    }
    #[test]
    fn only_new_agent_text_moves_controls_to_a_new_message() {
        let mut state = monitor::State {
            sessions: vec![session("1", "Chat", Client::VSCode)],
            ..Default::default()
        };
        let mut browser = Browser::new().unwrap();
        browser.message = Some(7);
        let list = browser.content(&state, &HashMap::new());
        let data = callbacks(&list)
            .into_iter()
            .find(|(_, data)| data.contains(":s:"))
            .unwrap()
            .1;
        browser.navigate(7, &data).unwrap();
        let detail = browser.content(&state, &HashMap::new());
        browser.commit(detail);
        state.sessions[0].activity = Activity::Working;
        let status = browser.content(&state, &HashMap::new());
        assert!(!browser.needs_new_message());
        browser.commit(status);
        browser.open_info();
        let info = browser.content(&state, &HashMap::new());
        assert!(!browser.needs_new_message());
        browser.commit(info);
        state.sessions[0].last_message = "A new agent reply".into();
        let fresh = browser.content(&state, &HashMap::new());
        assert!(browser.needs_new_message());
        assert!(
            fresh["text"]
                .as_str()
                .unwrap()
                .contains("A new agent reply")
        );
        assert!(!fresh["text"].as_str().unwrap().contains("/work/project"));
        browser.detach_message();
        assert_eq!(browser.selected(7), None);
        browser.message = Some(8);
        browser.commit(fresh);
        assert!(!browser.needs_new_message());
        assert_eq!(browser.selected(8), Some("codex:1"));
    }
    #[test]
    fn a_new_request_posts_a_new_session_message_even_without_new_agent_text() {
        let mut state = monitor::State {
            sessions: vec![session("1", "Chat", Client::VSCode)],
            ..Default::default()
        };
        let mut browser = Browser::new().unwrap();
        browser.message = Some(7);
        let list = browser.content(&state, &HashMap::new());
        browser.commit(list.clone());
        let data = callbacks(&list)
            .into_iter()
            .find(|(_, data)| data.contains(":s:"))
            .unwrap()
            .1;
        browser.navigate(7, &data).unwrap();
        let detail = browser.content(&state, &HashMap::new());
        browser.commit(detail);
        state.sessions[0].approval = Some(Approval {
            id: "new-request".into(),
            body: "Allow?".into(),
            expires: u64::MAX,
            can_allow: true,
        });
        state.sessions[0].interaction_data = crate::domain::interaction::Interaction::allow();
        let key = state.sessions[0].approval_key().unwrap();
        let mut pending = HashMap::from([(
            key.clone(),
            Pending::new(state.sessions[0].clone()).unwrap(),
        )]);
        let request = browser.content(&state, &pending);
        assert!(browser.needs_new_message());
        browser.detach_message();
        browser.message = Some(8);
        browser.commit(request);
        assert!(browser.permits_answer(8, &pending[&key]));
        browser.open_info();
        let info = browser.content(&state, &pending);
        browser.commit(info);
        browser
            .navigate(8, &format!("{}:message", browser.nonce))
            .unwrap();
        browser.content(&state, &pending);
        assert!(
            !browser.needs_new_message(),
            "returning from Info is not a new request"
        );
        pending.get_mut(&key).unwrap().nonce = "next-question-step".into();
        browser.content(&state, &pending);
        assert!(
            !browser.needs_new_message(),
            "draft steps of one request edit the current message"
        );
    }
    #[test]
    fn large_lists_paginate_and_keep_telegram_limits() {
        let state = monitor::State {
            sessions: (0..100)
                .map(|i| session(&i.to_string(), &"🤖".repeat(200), Client::VSCode))
                .collect(),
            ..Default::default()
        };
        let mut browser = Browser::new().unwrap();
        browser.message = Some(7);
        for page in 0..4 {
            browser
                .navigate(7, &format!("{}:p:{page}", browser.nonce))
                .unwrap();
            let content = browser.content(&state, &HashMap::new());
            assert!(content["text"].as_str().unwrap().encode_utf16().count() <= 3900);
            let buttons = callbacks(&content);
            assert!(buttons.len() <= 35);
            assert!(buttons.iter().all(|(_, data)| data.len() <= 64));
            assert!(
                buttons
                    .iter()
                    .all(|(text, _)| text.encode_utf16().count() <= 80)
            );
            assert!(buttons.iter().any(|(_, data)| data.contains(":s:")));
        }
    }
}
