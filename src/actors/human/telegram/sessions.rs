//! A bounded, state-driven Telegram view of the native sessions list.
use super::{Pending, Session, nonce, short};
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
}
impl Browser {
    pub fn new() -> Result<Self> {
        Ok(Self {
            message: None,
            rendered: Value::Null,
            nonce: nonce()?,
            page: 0,
            selected: None,
            keys: vec![],
            active_request: None,
            staged_request: None,
            previous_requested: false,
            earlier: None,
            history_waiting: None,
            history_note: None,
            history_exhausted: false,
            outgoing: None,
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
    pub fn commit(&mut self, content: Value) {
        self.rendered = content;
        self.active_request = self.staged_request.clone();
    }
    pub fn permits_answer(&self, message: i64, pending: &Pending) -> bool {
        self.selected(message)
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
        } else if let Some(page) = command.strip_prefix("p:") {
            self.page = page.parse::<usize>()?;
            self.selected = None;
        } else if command == "back" {
            self.selected = None;
            self.reset_detail();
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
        self.staged_request = self.selected.as_ref().and_then(|selected| {
            sessions
                .iter()
                .find(|s| s.key() == *selected)?
                .approval_key()
                .and_then(|key| pending.get(&key).map(|p| (key, p.nonce.clone())))
        });
        let mut rows = Vec::new();
        let text;
        if let Some(key) = &self.selected {
            if let Some(session) = sessions.iter().find(|s| s.key() == *key) {
                text = self.detail(session, state, pending, &mut rows);
            } else {
                text = "This session is closed or hidden in Applications settings.".to_owned();
            }
            if sessions.iter().any(|session| session.key() == *key) {
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
                self.button("↻ Refresh", "refresh")
            ]));
        } else {
            let pages = sessions.len().div_ceil(PAGE_SIZE).max(1);
            self.page = self.page.min(pages - 1);
            text = if sessions.is_empty() {
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
            navigation.push(self.button("↻ Refresh", "refresh"));
            if self.page + 1 < pages {
                navigation.push(self.button("Next ›", &format!("p:{}", self.page + 1)));
            }
            rows.push(json!(navigation));
        }
        json!({"text":short(&text,3899), "reply_markup":{"inline_keyboard":rows}})
    }
    fn detail(
        &self,
        session: &Session,
        state: &monitor::State,
        pending: &HashMap<String, Pending>,
        rows: &mut Vec<Value>,
    ) -> String {
        let mut text = format!(
            "{}\n{}{}",
            session.client.label(),
            status_symbol(session),
            session.title
        );
        text.push_str("\nType a message to send it to this session.");
        let message = self
            .earlier
            .as_ref()
            .map_or(session.last_message.as_str(), |message| {
                message.text.as_str()
            });
        if self.earlier.is_some() {
            text.push_str("\n\nEarlier message");
        }
        if !message.trim().is_empty() {
            text.push_str(&format!("\n\n{}", short(message.trim(), 1700)));
        }
        if let Some(note) = &self.history_note {
            text.push_str(&format!("\n\n{note}"));
        }
        if let Some(send) = self
            .outgoing
            .as_ref()
            .and_then(|id| state.message_sends.get(id))
        {
            text.push_str(&match &send.status {
                monitor::MessageStatus::Sending => "\n\nSending message…".into(),
                monitor::MessageStatus::Sent => "\n\nMessage sent.".into(),
                monitor::MessageStatus::Failed(error) => format!("\n\n{}", short(error, 300)),
            });
        }
        if let Some(request) = session.approval_key().and_then(|key| pending.get(&key)) {
            let (body, keyboard) = request.content();
            let body = body
                .strip_prefix(&format!("{}\n", session.title))
                .unwrap_or(&body);
            text.push_str(&format!("\n\n{}", short(body, 1500)));
            rows.extend(
                keyboard["inline_keyboard"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
        } else if let Some(approval) = &session.approval {
            text.push_str(&format!("\n\n{}", short(&approval.body, 1500)));
        }
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
        text.push_str(&format!("\n\n📁 {}", short(&path, 240)));
        if let Some(branch) = &session.git_branch {
            text.push_str(&format!("\n⑂ {}", short(branch, 160)));
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
        assert!(detail["text"].as_str().unwrap().contains("/work/project"));
        assert!(detail["text"].as_str().unwrap().contains("main"));
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
