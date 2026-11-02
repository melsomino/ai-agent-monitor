//! Personal Telegram bots: the resident owns delivery, pairing and replies.
use crate::{
    actors::monitor,
    agents::{
        interaction::{Interaction, Response},
        sessions::Session,
    },
    app::core,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

mod sessions;
mod text;

pub use crate::domain::telegram::Status;
pub fn diagnostics(status: &Status) -> Value {
    json!({"connected":status.connected,"enabled":status.enabled,"paired":status.paired,"status":status.text,"queued":status.queued,"delivered":status.delivered,"snapshotPending":status.snapshot_pending,"suppressed":status.suppressed,"commandMenuReady":status.command_menu_ready})
}
pub enum Event {
    Connect(String),
    Disconnect,
    BeginSetup,
    CancelSetup,
    CandidateUpdates(u64, Result<Value>),
    #[cfg(test)]
    Sync(Vec<Session>),
    Observe(Box<Session>),
    Test,
    Updates(u64, Result<Value>),
}
pub async fn run(
    mut rx: tokio::sync::mpsc::Receiver<Event>,
    monitor: monitor::Handle,
    tx: tokio::sync::mpsc::Sender<Event>,
) {
    let _ = tokio::task::spawn_blocking(move || {
        let mut snapshots = monitor.subscribe();
        let mut worker = Worker::load(monitor, tx);
        loop {
            if snapshots.has_changed().is_err() {
                break;
            }
            let snapshot = snapshots.borrow_and_update().clone();
            if snapshot.stopping {
                worker.retire_all("Monitor stopped. Answer in the agent app.");
                worker.cleanup_before_disconnect();
                break;
            }
            for event in rx
                .try_recv()
                .into_iter()
                .chain(std::iter::from_fn(|| rx.try_recv().ok()))
                .take(64)
            {
                worker.event(event);
            }
            worker.sync_sessions(&snapshot.sessions);
            worker.reconcile_phases(&snapshot);
            worker.flush();
            worker.update_counts();
            worker.poll();
            worker.poll_candidate();
            std::thread::sleep(Duration::from_millis(50));
            if rx.is_closed() {
                break;
            }
        }
    })
    .await;
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Config {
    enabled: bool,
    user: Option<i64>,
    chat: Option<i64>,
    username: String,
    offset: i64,
    #[serde(default)]
    messages: Vec<i64>,
    #[serde(default)]
    selected_session: Option<String>,
}
fn config_path() -> std::path::PathBuf {
    core::state_home().join("telegram.json")
}
fn persist(config: &Config) -> Result<()> {
    let path = config_path();
    let directory = path.parent().unwrap();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut file, config)?;
    file.persist(path)?;
    Ok(())
}
fn nonce() -> Result<String> {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn valid_token(token: &str) -> bool {
    token.len() <= 256
        && token.split_once(':').is_some_and(|(id, secret)| {
            !id.is_empty()
                && id.bytes().all(|b| b.is_ascii_digit())
                && secret.len() >= 20
                && secret
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}
fn short(text: &str, limit: usize) -> String {
    let mut result = String::new();
    let mut length = 0;
    for ch in text.chars() {
        length += ch.len_utf16();
        if length > limit {
            result.push('…');
            break;
        }
        result.push(ch);
    }
    result
}
fn config_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
fn api(token: &str, method: &str, payload: Value) -> Result<Value> {
    anyhow::ensure!(valid_token(token), "Invalid bot token");
    // Both the token URL and request body go through stdin, never argv or logs.
    let config = format!(
        "url = {}\nrequest = \"POST\"\nheader = \"Content-Type: application/json\"\ndata-binary = {}\n",
        config_quote(&format!("https://api.telegram.org/bot{token}/{method}")),
        config_quote(&payload.to_string())
    );
    let mut child = Command::new("/usr/bin/curl")
        .args([
            "--silent",
            "--show-error",
            "--connect-timeout",
            "5",
            "--max-time",
            "12",
            "--max-filesize",
            "2097152",
            "--config",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not start Telegram connection")?;
    child
        .stdin
        .take()
        .context("Could not open Telegram connection")?
        .write_all(config.as_bytes())?;
    let output = child
        .wait_with_output()
        .context("Telegram connection failed")?;
    anyhow::ensure!(
        output.status.success(),
        "Cannot reach Telegram. Check your connection."
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).context("Invalid response from Telegram")?;
    api_result(method, value)
}
fn api_result(method: &str, value: Value) -> Result<Value> {
    if value["ok"] != true && value["error_code"] == 400 {
        let description = value["description"].as_str().unwrap_or_default();
        if matches!(method, "editMessageText" | "editMessageReplyMarkup")
            && description.starts_with("Bad Request: message is not modified")
        {
            return Ok(json!({"unchanged":true}));
        }
        if (method == "deleteMessage" && description == "Bad Request: message to delete not found")
            || (matches!(method, "editMessageText" | "editMessageReplyMarkup")
                && description == "Bad Request: message to edit not found")
        {
            return Ok(json!(true));
        }
    }
    if value["ok"] != true {
        bail!(
            "{}",
            match value["error_code"].as_i64() {
                Some(401) => "Telegram rejected the token. Reconnect with a valid bot token.",
                Some(409) =>
                    "Another client or webhook is receiving this bot's updates. Use a dedicated bot.",
                Some(429) => "Telegram rate limit reached. Retrying shortly.",
                Some(403) => "Telegram access denied. Open the bot and unblock it.",
                _ => "Telegram could not complete the request.",
            }
        );
    }
    Ok(value["result"].clone())
}
// Returns successful cleanup only after deletion or removal of answer controls.
fn retire_message(
    chat: Option<i64>,
    message: i64,
    text: &str,
    mut send: impl FnMut(&str, Value) -> Result<Value>,
) -> Result<()> {
    send("deleteMessage", json!({"chat_id":chat,"message_id":message}))
        .or_else(|_| send("editMessageText", json!({"chat_id":chat,"message_id":message,"text":text,"reply_markup":{"inline_keyboard":[]}})))?;
    Ok(())
}
fn cleanup_once(
    queue: &mut super::retry::Queue<(i64, String)>,
    now: Instant,
    mut send: impl FnMut(i64, &str) -> Result<()>,
) -> Option<(i64, bool)> {
    let (message, text) = queue.pop_ready(now)?;
    let success = send(message, &text).is_ok();
    if !success {
        queue.push((message, text), now + Duration::from_secs(15));
    }
    Some((message, success))
}

#[derive(Clone)]
struct Pending {
    session: Session,
    interaction: Interaction,
    nonce: String,
    message: Option<i64>,
    question: usize,
    answers: BTreeMap<String, Vec<String>>,
    phase: Option<monitor::Submission>,
    browser_delivered: bool,
}
impl Pending {
    fn new(session: Session) -> Result<Self> {
        let interaction = session.interaction();
        Ok(Self {
            session,
            interaction,
            nonce: nonce()?,
            message: None,
            question: 0,
            answers: BTreeMap::new(),
            phase: None,
            browser_delivered: false,
        })
    }
    fn delivered(&self) -> bool {
        self.message.is_some() || self.browser_delivered
    }
    fn body_content(&self) -> (text::Text, Value) {
        let body = self
            .session
            .approval
            .as_ref()
            .map(|a| a.body.trim())
            .unwrap_or_default();
        let mut text = text::Text::default();
        let mut rows = Vec::new();
        let question = self.interaction.questions.get(self.question);
        if question.is_some_and(|q| q.secret) {
            text.plain("This request needs a private answer. Open the agent app to respond.");
        } else {
            text.markdown(body, 2200);
            if let Some(command) = &self.interaction.command {
                text.plain("\n\n");
                text.code_block(&short(command, 1600), Some("bash"));
            }
            if let Some(question) = question {
                text.plain(&format!(
                    "\n\nQuestion {} of {}: ",
                    self.question + 1,
                    self.interaction.questions.len()
                ));
                text.bold(&question.title);
                for (index, choice) in question.choices.iter().enumerate().take(90) {
                    rows.push(json!([{"text":short(&choice.label,80),"callback_data":format!("{}:{index}",self.nonce)}]));
                    if !choice.description.is_empty() {
                        text.plain(&format!("\n{}: ", choice.label));
                        text.markdown(&choice.description, 300);
                    }
                }
                if question.freeform {
                    text.plain("\n\nReply to this message to write your own answer.");
                }
            } else {
                for (index, action) in self.interaction.actions.iter().enumerate().take(90) {
                    rows.push(json!([{"text":short(&action.label,80),"callback_data":format!("{}:{index}",self.nonce)}]));
                    if !action.detail.is_empty() {
                        text.plain(&format!("\n\n{}:\n", action.label));
                        text.code_block(
                            &short(&action.detail, 300),
                            (action.label == "Allow similar commands").then_some("bash"),
                        );
                    }
                }
                if rows.is_empty() {
                    text.plain("\n\nOpen the agent app to respond.");
                }
            }
        }
        match &self.phase {
            Some(monitor::Submission::Sending) => {
                text.plain("\n\nSending answer…");
                rows.clear();
            }
            Some(monitor::Submission::Reconciling) => {
                text.plain("\n\nChecking whether the agent received the answer…");
                rows.clear();
            }
            Some(monitor::Submission::Failed(error)) => text.plain(&format!("\n\n{error}")),
            None => {}
        }
        (text, json!({"inline_keyboard":rows}))
    }
    fn payload(&self) -> Value {
        let (body, keyboard) = self.body_content();
        let mut text = text::Text::default();
        text.bold(&self.session.title);
        text.plain("\n");
        text.append(body);
        text.payload(keyboard)
    }
    #[cfg(test)]
    fn content(&self) -> (String, Value) {
        let payload = self.payload();
        (
            payload["text"].as_str().unwrap().into(),
            payload["reply_markup"].clone(),
        )
    }
    fn choose(&mut self, index: Option<usize>, text: Option<&str>) -> Result<Option<Response>> {
        if let Some(question) = self.interaction.questions.get(self.question) {
            anyhow::ensure!(
                !question.secret,
                "Answer this private question in the agent app."
            );
            let value = if let Some(index) = index {
                question
                    .choices
                    .get(index)
                    .context("This answer is unavailable.")?
                    .label
                    .clone()
            } else {
                anyhow::ensure!(question.freeform, "Choose one of the offered answers.");
                let value = text.unwrap_or_default().trim();
                anyhow::ensure!(
                    !value.is_empty() && value.chars().count() <= 4096,
                    "Enter a nonempty answer, up to 4096 characters."
                );
                value.to_owned()
            };
            self.answers.insert(question.id.clone(), vec![value]);
            if self.question + 1 < self.interaction.questions.len() {
                self.question += 1;
                self.nonce = nonce()?;
                return Ok(None);
            }
            anyhow::ensure!(
                self.interaction.valid_answers(&self.answers),
                "Please answer every question."
            );
            Ok(Some(Response::Answers(self.answers.clone())))
        } else {
            let action = index
                .and_then(|i| self.interaction.actions.get(i))
                .context("Choose an answer button.")?;
            Ok(Some(Response::Action(action.id.clone())))
        }
    }
}
fn edit_pending(token: &str, chat: Option<i64>, pending: &Pending) -> Result<()> {
    let mut payload = pending.payload();
    payload["chat_id"] = json!(chat);
    payload["message_id"] = json!(pending.message);
    api(token, "editMessageText", payload)?;
    Ok(())
}
struct Candidate {
    token: String,
    username: String,
    code: String,
    deadline: Instant,
    offset: i64,
    generation: u64,
    polling: bool,
    retry: Instant,
}
impl Candidate {
    fn paired_account(&self, update: &Value, owner: Option<i64>) -> Option<(i64, i64)> {
        let message = &update["message"];
        let user = message["from"]["id"].as_i64()?;
        let chat = message["chat"]["id"].as_i64()?;
        (owner == Some(user)
            && pairing_matches(
                user,
                chat,
                message["chat"]["type"].as_str()?,
                message["text"].as_str()?,
                &self.code,
                Instant::now() < self.deadline,
            ))
        .then_some((user, chat))
    }
}

// Save both pieces before retiring the running connection. On preference-write
// failure, restore the previous credential and keep the in-memory bot untouched.
fn store_replacement(token: &str, config: &Config, old_token: &str) -> Result<()> {
    crate::platform::credentials::save(token)?;
    if let Err(error) = persist(config) {
        crate::platform::credentials::save(old_token)
            .context("Could not restore the previous bot credential")?;
        return Err(
            error.context("Could not save the new bot; the previous connection is retained")
        );
    }
    Ok(())
}

struct Worker {
    monitor: monitor::Handle,
    tx: tokio::sync::mpsc::Sender<Event>,
    status: RefCell<Status>,
    last_published: RefCell<Status>,
    token: String,
    config: Config,
    pending: HashMap<String, Pending>,
    cleanup: super::retry::Queue<(i64, String)>,
    edits: super::retry::Queue<String>,
    pair: Option<(String, Instant)>,
    retry: Instant,
    generation: u64,
    polling: bool,
    poll_retry: Instant,
    commands_ready: bool,
    commands_retry: Instant,
    outbox: VecDeque<String>,
    browser: Option<sessions::Browser>,
    changing: bool,
    candidate: Option<Candidate>,
    candidate_generation: u64,
}
impl Worker {
    fn load(monitor: monitor::Handle, tx: tokio::sync::mpsc::Sender<Event>) -> Self {
        let config = fs::read(config_path())
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        let credential = crate::platform::credentials::load();
        let mut worker = Self {
            monitor,
            tx,
            status: RefCell::new(Status::default()),
            last_published: RefCell::new(Status::default()),
            token: credential
                .as_ref()
                .ok()
                .and_then(|v| v.clone())
                .unwrap_or_default(),
            config,
            pending: HashMap::new(),
            cleanup: Default::default(),
            edits: Default::default(),
            pair: None,
            retry: Instant::now(),
            generation: 0,
            polling: false,
            poll_retry: Instant::now(),
            commands_ready: false,
            commands_retry: Instant::now(),
            outbox: VecDeque::new(),
            browser: None,
            changing: false,
            candidate: None,
            candidate_generation: 0,
        };
        worker.publish();
        if let Err(error) = credential {
            worker.note(&error.to_string());
        }
        // Old inline buttons never regain authority after a resident restart.
        if !worker.token.is_empty() {
            worker.retire_all("Monitor restarted. Use the latest request message.");
            if let Err(error) = worker.restore_browser() {
                worker.note(&error.to_string());
            }
        }
        worker
    }
    fn api(&self, method: &str, payload: Value) -> Result<Value> {
        api(&self.token, method, payload)
    }
    fn publish(&self) {
        *self.status.borrow_mut() = Status {
            connected: !self.token.is_empty(),
            bot_username: self.config.username.clone(),
            setting_up: self.changing,
            enabled: self.config.enabled,
            paired: self.config.user.is_some(),
            text: if let Some(candidate) = &self.candidate {
                format!(
                    "Press Start in @{} to finish connecting. Your current bot stays connected.",
                    candidate.username
                )
            } else if self.changing && !self.token.is_empty() && self.config.user.is_some() {
                "Your current bot stays connected until you start the new bot.".into()
            } else if self.token.is_empty() {
                String::new()
            } else if self.pair.is_some() {
                "Press Start in Telegram to finish connecting. Use Open bot if needed.".into()
            } else if self.config.user.is_some() {
                format!(
                    "@{} · {}",
                    self.config.username,
                    if self.config.enabled {
                        "Connected"
                    } else {
                        "Paused"
                    }
                )
            } else {
                "Click Connect, then press Start in Telegram.".into()
            },
            command_menu_ready: self.commands_ready,
            snapshot_pending: 0,
            suppressed: 0,
            queued: self.pending.values().filter(|p| !p.delivered()).count(),
            delivered: self.pending.values().filter(|p| p.delivered()).count(),
            link: self
                .candidate
                .as_ref()
                .map(|candidate| {
                    format!(
                        "https://t.me/{}?start={}",
                        candidate.username, candidate.code
                    )
                })
                .or_else(|| {
                    self.pair.as_ref().map(|(code, _)| {
                        format!("https://t.me/{}?start={code}", self.config.username)
                    })
                }),
        };
        self.emit();
    }
    fn poll(&mut self) {
        if self.polling
            || self.token.is_empty()
            || (self.config.user.is_none() && self.pair.is_none())
            || Instant::now() < self.poll_retry
        {
            return;
        }
        let tx = self.tx.clone();
        let token = self.token.clone();
        let offset = self.config.offset;
        let generation = self.generation;
        self.polling = true;
        // Only one inbound poll is in flight. Outgoing delivery and local
        // resolutions keep running while Telegram waits for incoming updates.
        std::thread::spawn(move || {
            let updates = api(
                &token,
                "getUpdates",
                json!({"offset":offset,"timeout":5,"allowed_updates":["message","callback_query"]}),
            );
            let _ = tx.blocking_send(Event::Updates(generation, updates));
        });
    }
    fn poll_candidate(&mut self) {
        if self
            .candidate
            .as_ref()
            .is_some_and(|candidate| Instant::now() >= candidate.deadline)
        {
            self.candidate = None;
            self.publish();
            self.note(
                "New bot pairing expired. Reconnect to try again; your current bot is unchanged.",
            );
        }
        let Some(candidate) = self.candidate.as_mut() else {
            return;
        };
        if candidate.polling || Instant::now() < candidate.retry {
            return;
        }
        candidate.polling = true;
        let (token, offset, generation) = (
            candidate.token.clone(),
            candidate.offset,
            candidate.generation,
        );
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = api(
                &token,
                "getUpdates",
                json!({"offset":offset,"timeout":5,"allowed_updates":["message"]}),
            );
            let _ = tx.blocking_send(Event::CandidateUpdates(generation, result));
        });
    }
    fn stage_candidate(&mut self, token: &str, username: String, code: String) {
        self.candidate_generation = self.candidate_generation.wrapping_add(1);
        self.candidate = Some(Candidate {
            token: token.into(),
            username,
            code,
            deadline: Instant::now() + Duration::from_secs(600),
            offset: 0,
            generation: self.candidate_generation,
            polling: false,
            retry: Instant::now(),
        });
        self.changing = true;
        self.publish();
    }
    fn install_candidate_with(
        &mut self,
        generation: u64,
        user: i64,
        chat: i64,
        store: impl FnOnce(&str, &Config, &str) -> Result<()>,
    ) -> Result<()> {
        let candidate = self
            .candidate
            .as_ref()
            .filter(|candidate| candidate.generation == generation)
            .context("The new bot setup was cancelled")?;
        let token = candidate.token.clone();
        let config = Config {
            enabled: true,
            user: Some(user),
            chat: Some(chat),
            username: candidate.username.clone(),
            offset: candidate.offset,
            selected_session: self.config.selected_session.clone(),
            ..Config::default()
        };
        store(&token, &config, &self.token)?;
        // Old bot controls become inert only after the new pairing and storage succeeded.
        self.retire_all("Bot changed. Use the newly paired bot for requests.");
        for message in self.config.messages.clone() {
            let _ = retire_message(
                self.config.chat,
                message,
                "Bot changed.",
                |method, payload| self.api(method, payload),
            );
        }
        self.token = token;
        self.config = config;
        self.candidate = None;
        self.changing = false;
        self.pair = None;
        self.generation = self.generation.wrapping_add(1);
        self.polling = false;
        self.poll_retry = Instant::now();
        self.retry = Instant::now();
        self.commands_ready = false;
        self.commands_retry = Instant::now();
        self.cleanup = Default::default();
        self.outbox.clear();
        self.outbox
            .push_back("Connected to AI Agent Monitor. Use Sessions to open your agents.".into());
        if let Err(error) = self.restore_browser() {
            self.note(&error.to_string());
        }
        self.publish();
        Ok(())
    }
    fn candidate_updates(&mut self, generation: u64, result: Result<Value>) {
        let Some(candidate) = self
            .candidate
            .as_mut()
            .filter(|candidate| candidate.generation == generation)
        else {
            return;
        };
        candidate.polling = false;
        let updates = match result {
            Ok(updates) => updates,
            Err(error) => {
                candidate.retry = Instant::now() + Duration::from_secs(15);
                self.note(&error.to_string());
                return;
            }
        };
        for update in updates.as_array().into_iter().flatten() {
            let Some(candidate) = self.candidate.as_mut() else {
                break;
            };
            let Some(id) = update["update_id"]
                .as_i64()
                .filter(|id| *id >= candidate.offset)
            else {
                continue;
            };
            candidate.offset = id + 1;
            if let Some((user, chat)) = candidate.paired_account(update, self.config.user) {
                if let Err(error) =
                    self.install_candidate_with(generation, user, chat, store_replacement)
                {
                    self.note(&error.to_string());
                }
                break;
            }
        }
    }

    fn update_counts(&self) {
        let mut status = self.status.borrow_mut();
        status.command_menu_ready = self.commands_ready;
        status.queued = self.pending.values().filter(|p| !p.delivered()).count();
        status.delivered = self.pending.values().filter(|p| p.delivered()).count();
        drop(status);
        self.emit();
    }
    fn emit(&self) {
        let status = self.status.borrow().clone();
        if *self.last_published.borrow() != status {
            *self.last_published.borrow_mut() = status.clone();
            let _ = self
                .monitor
                .send(monitor::Command::TelegramObserved(status));
        }
    }
    fn note(&self, text: &str) {
        self.status.borrow_mut().text = text.into();
        self.emit();
    }
    fn save(&self) {
        if persist(&self.config).is_err() {
            self.note("Could not save Telegram settings.");
        }
    }
    fn remember_selection(&mut self) {
        let selected = self
            .browser
            .as_ref()
            .and_then(|browser| browser.current_session())
            .map(str::to_owned);
        if self.config.selected_session == selected {
            return;
        }
        self.config.selected_session = selected;
        // No bot means no active conversation to persist (also keeps offline
        // transport tests from touching a user's real Telegram configuration).
        if !self.token.is_empty() {
            self.save();
        }
    }
    fn restore_browser(&mut self) -> Result<()> {
        if self.browser.is_none()
            && self.config.user.is_some()
            && self.config.chat.is_some()
            && let Some(session) = &self.config.selected_session
        {
            self.browser = Some(sessions::Browser::resume(session.clone())?);
        }
        Ok(())
    }
    fn edit(&self, pending: &Pending) -> Result<()> {
        edit_pending(&self.token, self.config.chat, pending)
    }
    fn forget(&mut self, key: &str, text: &str) {
        if let Some(pending) = self.pending.remove(key)
            && let Some(message) = pending.message
        {
            self.finish(message, text);
        }
    }
    fn finish(&mut self, message: i64, text: &str) {
        self.cleanup.retain(|(id, _)| *id != message);
        self.cleanup.push((message, text.into()), Instant::now());
    }
    fn flush_cleanup(&mut self) -> bool {
        let token = &self.token;
        let chat = self.config.chat;
        let outcome = cleanup_once(&mut self.cleanup, Instant::now(), |message, text| {
            retire_message(chat, message, text, |method, payload| {
                api(token, method, payload)
            })
        });
        if let Some((message, true)) = outcome {
            self.config.messages.retain(|id| *id != message);
            self.save();
        }
        outcome.is_some()
    }
    fn cleanup_before_disconnect(&mut self) {
        // One best-effort pass before this bot's credentials are discarded.
        // Failed IDs remain persisted on ordinary shutdown for the next launch.
        for _ in 0..self.config.messages.len() {
            if !self.flush_cleanup() {
                break;
            }
        }
    }
    fn sync_sessions(&mut self, sessions: &[Session]) {
        let live: HashSet<_> = sessions.iter().filter_map(Session::approval_key).collect();
        {
            let mut status = self.status.borrow_mut();
            status.snapshot_pending = live.len();
            status.suppressed = 0;
        }
        let stale: Vec<_> = self
            .pending
            .keys()
            .filter(|key| !live.contains(*key))
            .cloned()
            .collect();
        for key in stale {
            self.forget(
                &key,
                "No longer pending. If it returns, use the latest request message.",
            );
        }
        for session in sessions {
            if self.config.enabled
                && session
                    .approval_key()
                    .is_some_and(|key| !self.pending.contains_key(&key))
            {
                self.event(Event::Observe(Box::new(session.clone())));
            }
        }
    }
    fn flush_edits_with(
        &mut self,
        now: Instant,
        mut send: impl FnMut(&Pending) -> Result<()>,
    ) -> bool {
        let Some(key) = self.edits.pop_ready(now) else {
            return false;
        };
        if let Some(pending) = self
            .pending
            .get(&key)
            .filter(|pending| pending.message.is_some())
            && send(pending).is_err()
        {
            self.edits
                .push(key, Instant::now().max(now) + Duration::from_secs(15));
        }
        true
    }
    fn retire_all(&mut self, text: &str) {
        self.browser = None;
        self.pending.clear();
        self.edits = Default::default();
        for message in self.config.messages.clone() {
            self.finish(message, text);
        }
    }
    fn event(&mut self, event: Event) {
        match event {
            Event::BeginSetup => {
                self.changing = true;
                self.publish();
            }
            Event::CancelSetup => {
                self.candidate = None;
                self.changing = false;
                self.publish();
            }
            Event::CandidateUpdates(generation, result) => {
                self.candidate_updates(generation, result)
            }
            Event::Updates(generation, updates) => {
                if generation != self.generation {
                    return;
                }
                self.polling = false;
                match updates {
                    Ok(updates) => {
                        let previous = self.config.offset;
                        for update in updates.as_array().into_iter().flatten() {
                            if let Some(id) = update["update_id"].as_i64()
                                && id >= self.config.offset
                            {
                                self.config.offset = id + 1;
                                self.update(update);
                            }
                        }
                        if previous != self.config.offset {
                            self.save();
                        }
                    }
                    Err(error) => {
                        self.note(&error.to_string());
                        self.poll_retry = Instant::now() + Duration::from_secs(15);
                    }
                }
            }

            Event::Connect(token) => {
                self.note("Checking token…");
                let result = self.connect(token.trim());
                if let Err(error) = result {
                    self.note(&error.to_string());
                }
            }
            Event::Disconnect => {
                self.candidate = None;
                self.changing = false;
                self.generation += 1;
                self.polling = false;
                self.retire_all(
                    "Telegram disconnected. This request can no longer be answered here.",
                );
                self.cleanup_before_disconnect();
                match crate::platform::credentials::delete() {
                    Ok(()) => {
                        self.outbox.clear();
                        self.token.clear();
                        self.cleanup = Default::default();
                        self.config = Config::default();
                        self.pair = None;
                        self.save();
                        self.publish();
                    }
                    Err(error) => {
                        self.config.enabled = false;
                        self.pair = None;
                        self.save();
                        self.publish();
                        self.note(&error.to_string());
                    }
                }
            }
            Event::Observe(session) => {
                if self.config.enabled
                    && let Some(key) = session.approval_key()
                    && !self.pending.contains_key(&key)
                    && self.pending.len() < 128
                {
                    match Pending::new(*session) {
                        Ok(pending) => {
                            self.pending.insert(key, pending);
                        }
                        Err(_) => self.note(
                            "Could not prepare the question for Telegram. Retrying on refresh.",
                        ),
                    }
                }
            }
            #[cfg(test)]
            Event::Sync(sessions) => self.sync_sessions(&sessions),
            Event::Test => {
                if self.config.enabled && self.config.chat.is_some() {
                    match self.api("sendMessage",json!({"chat_id":self.config.chat,"text":"AI Agent Monitor is connected. Agent questions will appear here with answer buttons."})) {
                        Ok(_) => self.note("Test notification sent."), Err(error)=>self.note(&error.to_string()),
                    }
                }
            }
        }
    }
    fn connect(&mut self, token: &str) -> Result<()> {
        anyhow::ensure!(
            valid_token(token),
            "Enter the bot token supplied by @BotFather."
        );
        let me = api(token, "getMe", json!({}))?;
        let username = me["username"]
            .as_str()
            .filter(|s| s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
            .context("Invalid bot identity")?
            .to_owned();
        let webhook = api(token, "getWebhookInfo", json!({}))?;
        anyhow::ensure!(
            webhook["url"].as_str().unwrap_or_default().is_empty(),
            "This bot uses a webhook. Use a dedicated bot without a webhook."
        );
        let code = nonce()?;
        if !self.token.is_empty() && self.config.user.is_some() {
            anyhow::ensure!(
                token != self.token,
                "This bot is already connected. Cancel to keep using it."
            );
            self.stage_candidate(token, username, code);
            return Ok(());
        }
        crate::platform::credentials::save(token)?;
        self.retire_all("Bot reconnected. Use the latest request message.");
        self.cleanup_before_disconnect();
        self.cleanup = Default::default();
        self.generation += 1;
        self.polling = false;
        self.token = token.to_owned();
        self.outbox.clear();
        self.commands_ready = false;
        self.commands_retry = Instant::now();
        self.config = Config {
            username,
            ..Config::default()
        };
        self.pair = Some((code, Instant::now() + Duration::from_secs(600)));
        self.retry = Instant::now();
        self.poll_retry = Instant::now();
        self.save();
        self.publish();
        Ok(())
    }
    fn reconcile_phases(&mut self, snapshot: &monitor::State) {
        let changed: Vec<_> = self
            .pending
            .iter_mut()
            .filter_map(|(key, pending)| {
                let phase = snapshot.submissions.get(key).cloned();
                if pending.phase != phase {
                    pending.phase = phase;
                    Some(key.clone())
                } else {
                    None
                }
            })
            .collect();
        self.edits.retain(|key| self.pending.contains_key(key));
        for key in changed {
            if self.pending[&key].message.is_some() {
                self.edits.retain(|queued| *queued != key);
                self.edits.push(key, Instant::now());
            }
        }
    }
    fn flush(&mut self) {
        self.ensure_commands();
        if self
            .pair
            .as_ref()
            .is_some_and(|(_, deadline)| Instant::now() >= *deadline)
        {
            self.pair = None;
            self.publish();
            self.note("Pairing expired. Reconnect to get a new link.");
        }
        if self.token.is_empty() || self.config.chat.is_none() || Instant::now() < self.retry {
            return;
        }
        // First present requests in the selected session. Only a successful
        // presentation records delivery; a failed API call keeps the request queued.
        if self.pending.values().any(|pending| {
            !pending.browser_delivered
                && self
                    .browser
                    .as_ref()
                    .is_some_and(|browser| browser.can_present(pending))
        }) {
            self.flush_browser();
            if Instant::now() < self.retry {
                return;
            }
        }
        // One request per pass avoids bursts and leaves time for local changes.
        if let Some(key) = self.next_request_message() {
            let mut payload = self.pending[&key].payload();
            payload["chat_id"] = json!(self.config.chat);
            match self.api("sendMessage", payload) {
                Ok(result) => {
                    if let Some(id) = result["message_id"].as_i64() {
                        self.pending.get_mut(&key).unwrap().message = Some(id);
                        self.config.messages.push(id);
                        self.save();
                        self.retry = Instant::now() + Duration::from_secs(1);
                    }
                }
                Err(error) => {
                    self.note(&error.to_string());
                    self.retry = Instant::now() + Duration::from_secs(15);
                }
            }
            return;
        }
        let token = self.token.clone();
        let chat = self.config.chat;
        if self.flush_edits_with(Instant::now(), |pending| {
            edit_pending(&token, chat, pending)
        }) {
            self.retry = Instant::now() + Duration::from_secs(1);
            return;
        }
        if self.flush_cleanup() {
            self.retry = Instant::now() + Duration::from_secs(1);
            return;
        }
        if let Some(text) = self.outbox.front() {
            match self.api(
                "sendMessage",
                json!({"chat_id":self.config.chat,"text":text,"reply_markup":command_keyboard()}),
            ) {
                Ok(_) => {
                    self.outbox.pop_front();
                    self.retry = Instant::now() + Duration::from_secs(1);
                }
                Err(error) => {
                    self.note(&error.to_string());
                    self.retry = Instant::now() + Duration::from_secs(15);
                }
            }
            return;
        }
        self.flush_browser();
    }
    fn next_request_message(&self) -> Option<String> {
        self.pending
            .iter()
            .find(|(_, p)| {
                self.config.enabled
                    && !p.delivered()
                    && !self
                        .browser
                        .as_ref()
                        .is_some_and(|browser| browser.can_present(p))
            })
            .map(|(key, _)| key.clone())
    }
    fn adopt_browser_requests(&mut self) {
        let Some(browser) = &self.browser else {
            return;
        };
        let Some(message) = browser.message else {
            return;
        };
        let shown: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| browser.permits_answer(message, p))
            .map(|(key, _)| key.clone())
            .collect();
        for key in shown {
            let pending = self.pending.get_mut(&key).unwrap();
            pending.browser_delivered = true;
            let old = pending.message.take();
            self.edits.retain(|queued| *queued != key);
            if let Some(old) = old.filter(|id| *id != message) {
                self.finish(old, "This request is shown in the session message.");
            }
        }
    }
    fn flush_browser(&mut self) {
        let snapshot = self.monitor.snapshot();
        let token = self.token.clone();
        if self.flush_browser_with(&snapshot, |method, payload| api(&token, method, payload)) {
            self.save();
        }
    }
    fn flush_browser_with(
        &mut self,
        snapshot: &monitor::State,
        mut send: impl FnMut(&str, Value) -> Result<Value>,
    ) -> bool {
        let Some(browser) = self.browser.as_mut() else {
            return false;
        };
        let content = browser.content(snapshot, &self.pending);
        // Only new agent text advances the chat. Request and status changes edit
        // the current card, keeping its message ID and answer buttons together.
        if browser.needs_new_message() {
            let old = browser.message.unwrap();
            let cleanup = send(
                "editMessageReplyMarkup",
                json!({"chat_id":self.config.chat,"message_id":old,"reply_markup":{"inline_keyboard":[]}}),
            );
            match cleanup {
                Ok(_) => {
                    self.browser.as_mut().unwrap().detach_message();
                    self.config.messages.retain(|id| *id != old);
                    self.retry = Instant::now() + Duration::from_secs(1);
                    return true;
                }
                Err(error) => {
                    self.note(&error.to_string());
                    self.retry = Instant::now() + Duration::from_secs(15);
                    return false;
                }
            }
        }
        if content == browser.rendered {
            browser.commit(content);
            self.adopt_browser_requests();
            return false;
        }
        let message = browser.message;
        let mut payload = content.clone();
        payload["chat_id"] = json!(self.config.chat);
        if let Some(message) = message {
            payload["message_id"] = json!(message);
        }
        let method = if message.is_some() {
            "editMessageText"
        } else {
            "sendMessage"
        };
        match send(method, payload) {
            Ok(result) => {
                let mut changed = false;
                if let Some(id) = result["message_id"].as_i64() {
                    let browser = self.browser.as_mut().unwrap();
                    browser.message = Some(id);
                    browser.commit(content);
                    if message.is_none() {
                        self.config.messages.push(id);
                        changed = true;
                    }
                } else if message.is_some() && result["unchanged"] == true {
                    self.browser.as_mut().unwrap().commit(content);
                } else if message.is_some() {
                    self.browser.as_mut().unwrap().detach_message();
                }
                self.adopt_browser_requests();
                self.retry = Instant::now() + Duration::from_secs(1);
                changed
            }
            Err(error) => {
                self.note(&error.to_string());
                self.retry = Instant::now() + Duration::from_secs(15);
                false
            }
        }
    }
    fn ensure_commands(&mut self) {
        let Some(chat) = self.config.chat else {
            return;
        };
        if self.token.is_empty() || self.commands_ready || Instant::now() < self.commands_retry {
            return;
        }
        let result = self
            .api(
                "setMyCommands",
                json!({"scope":{"type":"chat","chat_id":chat},"commands":[
                    {"command":"sessions","description":"List all open agent sessions"},
                    {"command":"info","description":"Show the selected session folder and Git branch"},
                    {"command":"help","description":"Show available commands"}
                ]}),
            )
            .and_then(|_| {
                self.api(
                    "setChatMenuButton",
                    json!({"chat_id":chat,"menu_button":{"type":"commands"}}),
                )
            });
        match result {
            Ok(_) => self.commands_ready = true,
            Err(error) => {
                self.note(&error.to_string());
                self.commands_retry = Instant::now() + Duration::from_secs(30);
            }
        }
    }
    fn paired(&self, user: i64, chat: i64, kind: &str) -> bool {
        self.config.user == Some(user) && self.config.chat == Some(chat) && kind == "private"
    }
    fn command(&mut self, command: BotCommand) -> Result<()> {
        let messages = match command {
            BotCommand::Sessions => {
                let browser = sessions::Browser::new()?;
                if let Some(old) = self.browser.take().and_then(|browser| browser.message) {
                    self.finish(old, "Session list refreshed.");
                }
                self.browser = Some(browser);
                self.remember_selection();
                return Ok(());
            }
            BotCommand::Info => {
                if self.browser.as_mut().is_some_and(|browser|browser.open_info()) {return Ok(());}
                vec!["Select a session in Sessions first.".into()]
            }
            BotCommand::Status => vec![format!("AI Agent Monitor {}\nNotifications: {}\nPending questions: {}",
                core::VERSION, if self.config.enabled {"enabled"} else {"paused"}, self.pending.len())],
            BotCommand::Help => vec!["/sessions — List all open sessions, including idle sessions\n/status — Monitor and notification status\n/info — Selected session folder and Git branch\n/help — Show these commands\n\nOpen a session and type in chat to send a message to that agent. Previous message loads an earlier agent message. Use answer buttons on question messages, or reply to one to enter a custom answer.\nCommands require your computer and monitor to be running.".into()],
        };
        anyhow::ensure!(
            self.outbox.len() + messages.len() <= 128,
            "Previous command replies are still sending. Please try again shortly."
        );
        self.outbox.extend(messages);
        Ok(())
    }
    fn authorized(&self, user: i64, chat: i64, kind: &str) -> bool {
        self.config.enabled && self.paired(user, chat, kind)
    }
    fn update(&mut self, update: &Value) {
        if let Some(message) = update.get("message") {
            let user = message["from"]["id"].as_i64().unwrap_or(0);
            let chat = message["chat"]["id"].as_i64().unwrap_or(0);
            let kind = message["chat"]["type"].as_str().unwrap_or_default();
            let text = message["text"].as_str().unwrap_or_default();
            if let Some((code, deadline)) = &self.pair
                && pairing_matches(user, chat, kind, text, code, Instant::now() < *deadline)
            {
                self.config.user = Some(user);
                self.config.chat = Some(chat);
                self.config.enabled = true;
                self.pair = None;
                self.save();
                self.publish();
                let _=self.api("sendMessage",json!({"chat_id":chat,"text":"Connected to AI Agent Monitor. Use /sessions to list open sessions, or the command buttons below.","reply_markup":command_keyboard()}));
                return;
            }
            if !self.paired(user, chat, kind) {
                return;
            }
            // Text button labels are commands only outside an explicit reply.
            // A custom answer such as "Status" must still reach the question.
            if let Some(command) = bot_command(text, &self.config.username)
                && (text.trim_start().starts_with('/') || message["reply_to_message"].is_null())
            {
                if let Err(error) = self.command(command) {
                    let _ = self.api("sendMessage", json!({"chat_id":chat,"text":error.to_string(),"reply_markup":command_keyboard()}));
                }
                return;
            }
            if !self.authorized(user, chat, kind) {
                return;
            }
            if let Some(message_id) = message["reply_to_message"]["message_id"].as_i64()
                && let Some(key) = self
                    .pending
                    .iter()
                    .find(|(_, p)| p.message == Some(message_id))
                    .map(|(k, _)| k.clone())
                    .or_else(|| {
                        let selected = self.browser.as_ref()?.selected(message_id)?;
                        self.pending
                            .iter()
                            .find(|(_, p)| {
                                p.session.key() == selected
                                    && self.browser.as_ref().is_some_and(|browser| {
                                        browser.permits_answer(message_id, p)
                                    })
                            })
                            .map(|(key, _)| key.clone())
                    })
            {
                let result = self.answer(&key, None, Some(text));
                if let Err(error) = result {
                    let _ = self.api(
                        "sendMessage",
                        json!({"chat_id":chat,"text":error.to_string()}),
                    );
                }
            }
            if message["reply_to_message"].is_null() && !text.trim().is_empty() {
                let session = self.config.selected_session.clone().or_else(|| {
                    self.browser
                        .as_ref()
                        .and_then(|browser| browser.current_session())
                        .map(str::to_owned)
                });
                let Some(session) = session else {
                    self.outbox.push_back(
                        "No session selected. Open Sessions and choose where to send your message."
                            .into(),
                    );
                    return;
                };
                if let Err(error) = self.restore_browser() {
                    self.note(&error.to_string());
                }
                let Some(message_id) = message["message_id"].as_i64().filter(|id| *id > 0) else {
                    return;
                };
                let id = format!("telegram:{chat}:{message_id}");
                match self.monitor.send(monitor::Command::SendMessage {
                    id: id.clone(),
                    session,
                    text: text.to_owned(),
                }) {
                    Ok(()) => {
                        if let Some(browser) = self.browser.as_mut() {
                            browser.track_send(id);
                        }
                    }
                    Err(error) => self.outbox.push_back(error),
                }
            }
        }
        if let Some(callback) = update.get("callback_query") {
            let user = callback["from"]["id"].as_i64().unwrap_or(0);
            let message = &callback["message"];
            let chat = message["chat"]["id"].as_i64().unwrap_or(0);
            let kind = message["chat"]["type"].as_str().unwrap_or_default();
            let mut feedback = "This request is no longer available.".to_owned();
            let navigation = if self.paired(user, chat, kind) {
                self.browser
                    .as_mut()
                    .map(|browser| {
                        browser.navigate(
                            message["message_id"].as_i64().unwrap_or(0),
                            callback["data"].as_str().unwrap_or_default(),
                        )
                    })
                    .transpose()
            } else {
                Ok(None)
            };
            if matches!(&navigation, Ok(Some(true))) {
                self.remember_selection();
                let snapshot = self.monitor.snapshot();
                let history = self
                    .browser
                    .as_mut()
                    .and_then(|browser| browser.history_command(&snapshot));
                feedback = "Updated.".into();
                if let Some(command) = history
                    && let Err(error) = self.monitor.send(command)
                {
                    self.browser.as_mut().unwrap().history_error(error.clone());
                    feedback = error;
                }
            } else if let Err(error) = navigation {
                feedback = error.to_string();
            } else if self.authorized(user, chat, kind) {
                if let Some((key, index)) = self.callback_key(callback) {
                    feedback = match self.answer(&key, Some(index), None) {
                        Ok(()) => "Answer queued.".into(),
                        Err(error) => error.to_string(),
                    };
                }
            } else {
                feedback = "This account is not paired with the monitor.".into();
            }
            let _ = self.api(
                "answerCallbackQuery",
                json!({"callback_query_id":callback["id"],"text":short(&feedback,180)}),
            );
        }
    }
    fn callback_key(&self, callback: &Value) -> Option<(String, usize)> {
        let user = callback["from"]["id"].as_i64()?;
        let message = &callback["message"];
        if !self.authorized(
            user,
            message["chat"]["id"].as_i64()?,
            message["chat"]["type"].as_str()?,
        ) {
            return None;
        }
        let (nonce, index) = callback["data"].as_str()?.split_once(':')?;
        let index = index.parse::<usize>().ok()?;
        let message_id = message["message_id"].as_i64()?;
        self.pending
            .iter()
            .find(|(_, p)| {
                p.nonce == nonce
                    && (p.message == Some(message_id)
                        || self
                            .browser
                            .as_ref()
                            .is_some_and(|browser| browser.permits_answer(message_id, p)))
            })
            .map(|(key, _)| (key.clone(), index))
    }
    fn answer(&mut self, key: &str, index: Option<usize>, text: Option<&str>) -> Result<()> {
        let pending = self
            .pending
            .get(key)
            .context("This request is no longer available.")?;
        let snapshot = self.monitor.snapshot();
        let current = snapshot
            .sessions
            .iter()
            .find(|s| s.key() == pending.session.key() && s.approval == pending.session.approval)
            .cloned();
        let Some(current) = current else {
            self.forget(key, "Resolved or no longer pending.");
            bail!("This request is no longer pending.");
        };
        anyhow::ensure!(
            current.interaction() == pending.interaction,
            "The question changed. Answer it in the agent app."
        );
        anyhow::ensure!(
            current.authoritative,
            "The agent connection is being checked. Try again shortly."
        );
        anyhow::ensure!(
            !matches!(
                snapshot.submissions.get(key),
                Some(monitor::Submission::Sending | monitor::Submission::Reconciling)
            ),
            "The answer is already being sent."
        );
        let mut pending = self.pending[key].clone();
        let response = pending.choose(index, text)?;
        if let Some(response) = response {
            self.monitor
                .submit(&current, response)
                .map_err(anyhow::Error::msg)?;
            // Do not resolve locally. Every channel observes the Monitor's next state.
        } else {
            if pending.message.is_some() {
                self.edit(&pending)?;
            }
            self.pending.insert(key.to_owned(), pending);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BotCommand {
    Sessions,
    Status,
    Info,
    Help,
}
fn bot_command(text: &str, username: &str) -> Option<BotCommand> {
    match text.trim() {
        "Sessions" => return Some(BotCommand::Sessions),
        "Status" => return Some(BotCommand::Status),
        "Info" => return Some(BotCommand::Info),
        "Help" => return Some(BotCommand::Help),
        _ => {}
    }
    let command = text.split_whitespace().next()?;
    let command = if let Some((command, target)) = command.split_once('@') {
        if !target.eq_ignore_ascii_case(username) {
            return None;
        }
        command
    } else {
        command
    };
    match command {
        "/sessions" => Some(BotCommand::Sessions),
        "/status" => Some(BotCommand::Status),
        "/info" => Some(BotCommand::Info),
        "/help" | "/start" => Some(BotCommand::Help),
        _ => None,
    }
}
fn command_keyboard() -> Value {
    json!({"keyboard":[[{"text":"Sessions"},{"text":"Help"}]],
        "resize_keyboard":true,"is_persistent":true,"input_field_placeholder":"Choose a command or reply to a question"})
}
fn pairing_matches(user: i64, chat: i64, kind: &str, text: &str, code: &str, valid: bool) -> bool {
    valid
        && user > 0
        && user == chat
        && kind == "private"
        && text.strip_prefix("/start ") == Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{
        interaction::{Action, Choice, Question},
        sessions::{Activity, Approval, Client},
    };
    fn session() -> Session {
        Session {
            authoritative: true,
            interaction_data: Default::default(),
            target: None,
            agent: "codex",
            id: "session".into(),
            title: "A chat".into(),
            cwd: "/work".into(),
            workspace: None,
            git_branch: None,
            updated_at: 0,
            client: Client::Terminal,
            activity: Activity::Confirmation,
            last_message: String::new(),
            approval: Some(Approval {
                id: "request".into(),
                body: "Choose carefully".into(),
                expires: u64::MAX,
                can_allow: true,
            }),
        }
    }
    fn pending(interaction: Interaction) -> Pending {
        Pending {
            session: session(),
            interaction,
            nonce: "random-opaque-value".into(),
            message: Some(50),
            question: 0,
            answers: BTreeMap::new(),
            phase: None,
            browser_delivered: false,
        }
    }
    fn worker() -> Worker {
        Worker {
            monitor: monitor::Handle::testing(),
            tx: tokio::sync::mpsc::channel(128).0,
            status: RefCell::new(Status::default()),
            last_published: RefCell::new(Status::default()),
            token: String::new(),
            config: Config {
                enabled: true,
                user: Some(42),
                chat: Some(42),
                ..Config::default()
            },
            pending: HashMap::new(),
            cleanup: Default::default(),
            edits: Default::default(),
            pair: None,
            retry: Instant::now(),
            generation: 0,
            polling: false,
            poll_retry: Instant::now(),
            commands_ready: false,
            commands_retry: Instant::now(),
            outbox: VecDeque::new(),
            browser: None,
            changing: false,
            candidate: None,
            candidate_generation: 0,
        }
    }
    fn question(id: &str, freeform: bool) -> Question {
        Question {
            id: id.into(),
            title: format!("Question {id}"),
            choices: vec![
                Choice {
                    label: "Yes".into(),
                    description: "Continue".into(),
                },
                Choice {
                    label: "No".into(),
                    description: String::new(),
                },
            ],
            freeform,
            secret: false,
        }
    }
    fn connected_worker() -> Worker {
        let mut worker = worker();
        worker.token = "old-token".into();
        worker.config.username = "old_bot".into();
        worker.config.selected_session = Some(session().key());
        worker
            .pending
            .insert("old-request".into(), pending(Interaction::allow()));
        worker.publish();
        worker
    }

    #[test]
    fn current_bot_hint_is_only_shown_for_an_existing_connection() {
        let mut initial = worker();
        initial.config.user = None;
        initial.config.chat = None;
        initial.event(Event::BeginSetup);
        assert!(!initial.status.borrow().text.contains("current bot"));
        let mut existing = connected_worker();
        existing.event(Event::BeginSetup);
        assert!(existing.status.borrow().text.contains("current bot"));
    }

    #[test]
    fn staging_and_cancelling_a_replacement_keep_the_current_bot_and_requests() {
        let mut worker = connected_worker();
        worker.event(Event::BeginSetup);
        worker.stage_candidate("new-token", "new_bot".into(), "pair-code".into());
        let generation = worker.candidate.as_ref().unwrap().generation;
        assert_eq!(worker.token, "old-token");
        assert_eq!(worker.config.username, "old_bot");
        assert!(worker.config.enabled);
        assert!(worker.pending.contains_key("old-request"));
        assert_eq!(worker.status.borrow().bot_username, "old_bot");
        assert!(worker.status.borrow().setting_up);
        assert!(
            worker
                .status
                .borrow()
                .link
                .as_ref()
                .unwrap()
                .contains("new_bot")
        );
        worker.event(Event::CancelSetup);
        worker.event(Event::CandidateUpdates(generation, Ok(json!([]))));
        assert!(worker.candidate.is_none());
        assert!(!worker.status.borrow().setting_up);
        assert_eq!(worker.token, "old-token");
        assert!(worker.authorized(42, 42, "private"));
        assert!(worker.pending.contains_key("old-request"));
    }

    #[test]
    fn new_pairing_requires_the_current_private_account_and_exact_unexpired_code() {
        let mut worker = connected_worker();
        worker.stage_candidate("new-token", "new_bot".into(), "pair-code".into());
        let message = |user, chat, kind, code| json!({"message":{"from":{"id":user},"chat":{"id":chat,"type":kind},"text":code}});
        let candidate = worker.candidate.as_mut().unwrap();
        assert_eq!(
            candidate.paired_account(&message(42, 42, "private", "/start pair-code"), Some(42)),
            Some((42, 42))
        );
        for update in [
            message(43, 43, "private", "/start pair-code"),
            message(42, -7, "group", "/start pair-code"),
            message(42, 42, "private", "/start wrong"),
        ] {
            assert!(candidate.paired_account(&update, Some(42)).is_none());
        }
        candidate.deadline = Instant::now() - Duration::from_secs(1);
        assert!(
            candidate
                .paired_account(&message(42, 42, "private", "/start pair-code"), Some(42))
                .is_none()
        );
        worker.poll_candidate();
        assert!(worker.candidate.is_none());
        assert_eq!(worker.token, "old-token");
        assert!(worker.pending.contains_key("old-request"));
    }

    #[test]
    fn replacement_storage_or_network_failure_does_not_disconnect_the_old_bot() {
        let mut worker = connected_worker();
        worker.stage_candidate("new-token", "new_bot".into(), "pair-code".into());
        let generation = worker.candidate.as_ref().unwrap().generation;
        worker.event(Event::CandidateUpdates(
            generation,
            Err(anyhow::anyhow!("offline")),
        ));
        assert_eq!(worker.token, "old-token");
        assert!(worker.pending.contains_key("old-request"));
        let result = worker.install_candidate_with(generation, 42, 42, |new, config, old| {
            assert_eq!((new, old), ("new-token", "old-token"));
            assert_eq!(config.username, "new_bot");
            bail!("Keychain unavailable")
        });
        assert!(result.is_err());
        assert_eq!(worker.token, "old-token");
        assert_eq!(worker.config.username, "old_bot");
        assert_eq!(worker.config.selected_session, Some(session().key()));
        assert!(worker.pending.contains_key("old-request"));
        assert!(worker.candidate.is_some());
    }

    #[test]
    fn successful_replacement_commits_once_and_ignores_old_poll_completions() {
        let mut worker = connected_worker();
        let old_generation = worker.generation;
        worker.stage_candidate("new-token", "new_bot".into(), "pair-code".into());
        let generation = worker.candidate.as_ref().unwrap().generation;
        worker
            .install_candidate_with(generation, 42, 42, |_, _, _| Ok(()))
            .unwrap();
        assert_eq!(worker.token, "new-token");
        assert_eq!(worker.config.username, "new_bot");
        assert!(worker.config.enabled);
        assert_eq!(worker.config.user, Some(42));
        assert_eq!(worker.config.selected_session, Some(session().key()));
        assert!(worker.pending.is_empty());
        assert!(worker.candidate.is_none());
        assert!(!worker.status.borrow().setting_up);
        assert_eq!(worker.status.borrow().bot_username, "new_bot");
        worker.polling = true;
        worker.event(Event::Updates(old_generation, Ok(json!([]))));
        assert!(
            worker.polling,
            "the previous bot's completion must not unlock the new poll"
        );
        assert!(
            worker
                .install_candidate_with(generation, 42, 42, |_, _, _| panic!(
                    "must not commit twice"
                ))
                .is_err()
        );
    }

    #[test]
    fn already_removed_messages_and_unchanged_edits_complete_successfully() {
        for (method, description) in [
            ("deleteMessage", "Bad Request: message to delete not found"),
            ("editMessageText", "Bad Request: message to edit not found"),
            (
                "editMessageText",
                "Bad Request: message is not modified: same contents",
            ),
        ] {
            assert!(
                api_result(
                    method,
                    json!({"ok":false,"error_code":400,"description":description})
                )
                .is_ok()
            );
        }
        assert!(
            api_result(
                "deleteMessage",
                json!({"ok":false,"error_code":429,"description":"retry later"})
            )
            .is_err()
        );
        assert!(
            api_result(
                "deleteMessage",
                json!({"ok":false,"error_code":400,"description":"Bad Request: chat not found"})
            )
            .is_err()
        );
        assert!(api_result("sendMessage", json!({"ok":false,"error_code":400,"description":"Bad Request: message to delete not found"})).is_err());
    }
    #[test]
    fn failed_phase_edits_retry_the_latest_state_without_new_monitor_changes() {
        let mut worker = worker();
        let pending = pending(Interaction::allow());
        let key = pending.session.approval_key().unwrap();
        worker.pending.insert(key.clone(), pending);
        let mut snapshot = monitor::State::default();
        snapshot
            .submissions
            .insert(key.clone(), monitor::Submission::Sending);
        worker.reconcile_phases(&snapshot);
        let now = Instant::now();
        assert!(worker.flush_edits_with(now, |pending| {
            assert_eq!(pending.phase, Some(monitor::Submission::Sending));
            bail!("offline")
        }));
        assert!(!worker.flush_edits_with(now + Duration::from_secs(14), |_| panic!("early retry")));
        assert!(worker.flush_edits_with(now + Duration::from_secs(16), |_| Ok(())));
        assert!(worker.edits.next_due().is_none());
        // A failed answer must restore controls, even when the first edit fails.
        snapshot
            .submissions
            .insert(key, monitor::Submission::Failed("Not sent".into()));
        worker.reconcile_phases(&snapshot);
        assert!(worker.flush_edits_with(Instant::now(), |pending| {
            let (_, keyboard) = pending.content();
            assert!(!keyboard["inline_keyboard"].as_array().unwrap().is_empty());
            Ok(())
        }));
    }
    #[test]
    fn unchanged_questions_are_restored_after_resume_and_pairing() {
        let sessions = vec![session()];
        let key = sessions[0].approval_key().unwrap();
        let mut worker = worker();
        worker.sync_sessions(&sessions);
        let nonce = worker.pending[&key].nonce.clone();
        worker.sync_sessions(&sessions);
        assert_eq!(worker.pending[&key].nonce, nonce);
        worker.config.enabled = false;
        worker.retire_all("Paused");
        worker.sync_sessions(&sessions);
        assert!(worker.pending.is_empty());
        worker.config.enabled = true;
        worker.sync_sessions(&sessions);
        assert!(worker.pending.contains_key(&key));
        // First pairing enables delivery without changing the Monitor snapshot.
        let mut unpaired = self::worker();
        unpaired.config.enabled = false;
        unpaired.config.chat = None;
        unpaired.sync_sessions(&sessions);
        assert!(unpaired.pending.is_empty());
        unpaired.config.chat = Some(42);
        unpaired.config.enabled = true;
        unpaired.sync_sessions(&sessions);
        assert!(unpaired.pending.contains_key(&key));
    }
    #[test]
    fn failed_cleanup_retries_without_restart_and_falls_back_to_editing() {
        let now = Instant::now();
        let mut queue = super::super::retry::Queue::default();
        queue.push((50, "Resolved".to_owned()), now);
        let mut calls = Vec::new();
        let outcome = cleanup_once(&mut queue, now, |message, text| {
            retire_message(Some(42), message, text, |method, _| {
                calls.push(method.to_owned());
                bail!("offline")
            })
        });
        assert_eq!(outcome, Some((50, false)));
        assert_eq!(calls, ["deleteMessage", "editMessageText"]);
        assert_eq!(
            cleanup_once(&mut queue, now + Duration::from_secs(14), |_, _| panic!(
                "too early"
            )),
            None
        );
        calls.clear();
        let outcome = cleanup_once(
            &mut queue,
            now + Duration::from_secs(15),
            |message, text| {
                retire_message(Some(42), message, text, |method, payload| {
                    calls.push(method.to_owned());
                    assert_eq!(payload["message_id"], 50);
                    if method == "deleteMessage" {
                        bail!("message is too old");
                    }
                    assert_eq!(payload["reply_markup"]["inline_keyboard"], json!([]));
                    Ok(json!(true))
                })
            },
        );
        assert_eq!(outcome, Some((50, true)));
        assert_eq!(calls, ["deleteMessage", "editMessageText"]);
        assert!(queue.next_due().is_none());
        retire_message(Some(42), 51, "Resolved", |method, _| {
            assert_eq!(method, "deleteMessage");
            Ok(json!(true))
        })
        .unwrap();
    }
    #[test]
    fn pairing_requires_private_account_exact_unexpired_nonce() {
        assert!(pairing_matches(
            42,
            42,
            "private",
            "/start code",
            "code",
            true
        ));
        assert!(!pairing_matches(
            42,
            42,
            "private",
            "/start other",
            "code",
            true
        ));
        assert!(!pairing_matches(
            42,
            42,
            "private",
            "/start code",
            "code",
            false
        ));
        assert!(!pairing_matches(
            42,
            -12,
            "group",
            "/start code",
            "code",
            true
        ));
        assert!(!pairing_matches(
            0,
            0,
            "private",
            "/start code",
            "code",
            true
        ));
    }
    #[test]
    fn callback_requires_paired_user_chat_message_and_current_nonce() {
        let mut worker = worker();
        worker
            .pending
            .insert("key".into(), pending(Interaction::allow()));
        let callback = json!({"from":{"id":42},"message":{"chat":{"id":42,"type":"private"},"message_id":50},"data":"random-opaque-value:0"});
        assert_eq!(worker.callback_key(&callback), Some(("key".into(), 0)));
        for invalid in [
            json!({"from":{"id":99},"message":callback["message"],"data":callback["data"]}),
            json!({"from":{"id":42},"message":{"chat":{"id":99,"type":"private"},"message_id":50},"data":callback["data"]}),
            json!({"from":{"id":42},"message":{"chat":{"id":42,"type":"group"},"message_id":50},"data":callback["data"]}),
            json!({"from":{"id":42},"message":{"chat":{"id":42,"type":"private"},"message_id":51},"data":callback["data"]}),
            json!({"from":{"id":42},"message":callback["message"],"data":"old-value:0"}),
        ] {
            assert!(worker.callback_key(&invalid).is_none());
        }
        worker.config.enabled = false;
        assert!(worker.callback_key(&callback).is_none());
        worker.config.enabled = true;
        worker.pending.clear();
        assert!(worker.callback_key(&callback).is_none());
    }
    #[test]
    fn browser_answers_require_the_delivered_request_and_its_message() {
        let mut worker = worker();
        let mut request = pending(Interaction::allow());
        request.session.interaction_data = request.interaction.clone();
        let key = request.session.approval_key().unwrap();
        let state = monitor::State {
            sessions: vec![request.session.clone()],
            ..Default::default()
        };
        let data = format!("{}:0", request.nonce);
        worker.pending.insert(key.clone(), request);
        let mut browser = sessions::Browser::new().unwrap();
        browser.message = Some(77);
        let list = browser.content(&state, &worker.pending);
        let select = list["reply_markup"]["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .find_map(|button| {
                let data = button["callback_data"].as_str()?;
                data.contains(":s:").then_some(data)
            })
            .unwrap();
        browser.navigate(77, select).unwrap();
        let detail = browser.content(&state, &worker.pending);
        worker.browser = Some(browser);
        let callback = json!({"from":{"id":42},"message":{"chat":{"id":42,"type":"private"},"message_id":77},"data":data});
        assert!(worker.callback_key(&callback).is_none());
        worker.browser.as_mut().unwrap().commit(detail);
        assert_eq!(worker.callback_key(&callback), Some((key.clone(), 0)));
        let mut wrong_message = callback.clone();
        wrong_message["message"]["message_id"] = json!(78);
        assert!(worker.callback_key(&wrong_message).is_none());
        worker.pending.get_mut(&key).unwrap().nonce = "next-step".into();
        assert!(worker.callback_key(&callback).is_none());
    }
    #[test]
    fn chat_text_targets_selected_session_but_explicit_reply_remains_an_answer() {
        let mut worker = worker();
        let mut request = pending(Interaction {
            command: None,
            actions: vec![],
            questions: vec![question("one", true)],
        });
        request.session.interaction_data = request.interaction.clone();
        request.message = Some(50);
        let state = monitor::State {
            sessions: vec![request.session.clone()],
            ..Default::default()
        };
        let (tx, mut commands) = tokio::sync::mpsc::channel(8);
        let (_, snapshots) = tokio::sync::watch::channel(std::sync::Arc::new(state.clone()));
        worker.monitor = monitor::Handle {
            tx,
            state: snapshots,
        };
        worker
            .pending
            .insert(request.session.approval_key().unwrap(), request);
        let mut browser = sessions::Browser::new().unwrap();
        browser.message = Some(77);
        let list = browser.content(&state, &worker.pending);
        let select = list["reply_markup"]["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .find_map(|button| {
                button["callback_data"]
                    .as_str()
                    .filter(|data| data.contains(":s:"))
            })
            .unwrap();
        browser.navigate(77, select).unwrap();
        worker.browser = Some(browser);
        worker.update(&json!({"message":{"message_id":100,"from":{"id":42},"chat":{"id":42,"type":"private"},"text":"Continue with tests"}}));
        let monitor::Command::SendMessage { id, session, text } = commands.try_recv().unwrap()
        else {
            panic!("message command expected")
        };
        assert_eq!(id, "telegram:42:100");
        assert_eq!(session, "codex:session");
        assert_eq!(text, "Continue with tests");
        worker.update(&json!({"message":{"message_id":101,"from":{"id":42},"chat":{"id":42,"type":"private"},"reply_to_message":{"message_id":50},"text":"My custom answer"}}));
        let monitor::Command::Submit { response, .. } = commands.try_recv().unwrap() else {
            panic!("answer command expected")
        };
        assert_eq!(
            response,
            Response::Answers(BTreeMap::from([(
                "one".into(),
                vec!["My custom answer".into()]
            )]))
        );
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn another_sessions_request_and_answer_keep_chat_text_targeting_the_selected_session() {
        let mut worker = worker();
        let mut current = session();
        current.title = "Current session".into();
        current.approval = None;
        current.interaction_data = Interaction::default();
        let current_key = current.key();
        let mut other = pending(Interaction::allow());
        other.session.id = "other".into();
        other.session.title = "Other session".into();
        other.session.approval.as_mut().unwrap().id = "other-request".into();
        other.session.interaction_data = other.interaction.clone();
        let other_key = other.session.approval_key().unwrap();
        let state = monitor::State {
            sessions: vec![current, other.session.clone()],
            ..Default::default()
        };
        let (tx, mut commands) = tokio::sync::mpsc::channel(8);
        let (_, snapshots) = tokio::sync::watch::channel(std::sync::Arc::new(state.clone()));
        worker.monitor = monitor::Handle {
            tx,
            state: snapshots,
        };
        let mut browser = sessions::Browser::new().unwrap();
        browser.message = Some(77);
        let list = browser.content(&state, &worker.pending);
        let select = list["reply_markup"]["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .find(|button| button["text"].as_str().unwrap().contains("Current session"))
            .unwrap()["callback_data"]
            .as_str()
            .unwrap();
        browser.navigate(77, select).unwrap();
        let detail = browser.content(&state, &worker.pending);
        browser.commit(detail);
        worker.browser = Some(browser);
        worker.event(Event::Observe(Box::new(other.session)));
        assert_eq!(worker.next_request_message(), Some(other_key.clone()));
        worker.pending.get_mut(&other_key).unwrap().message = Some(99);
        worker.flush_browser_with(&state, |_, _| Ok(json!({"message_id":77})));
        assert_eq!(
            worker.browser.as_ref().unwrap().current_session(),
            Some(current_key.as_str())
        );
        worker.answer(&other_key, Some(0), None).unwrap();
        assert!(matches!(
            commands.try_recv().unwrap(),
            monitor::Command::Submit { .. }
        ));
        worker.update(&json!({"message":{"message_id":100,"from":{"id":42},"chat":{"id":42,"type":"private"},"text":"Keep working here"}}));
        let monitor::Command::SendMessage { session, text, .. } = commands.try_recv().unwrap()
        else {
            panic!("message command expected")
        };
        assert_eq!(session, current_key);
        assert_eq!(text, "Keep working here");
    }

    #[test]
    fn restored_selection_survives_presentation_loss_and_routes_plain_text() {
        let mut worker = worker();
        let key = session().key();
        worker.config.selected_session = Some(key.clone());
        // A process restart deserializes the selected key, not old callback IDs.
        worker.config =
            serde_json::from_value(serde_json::to_value(&worker.config).unwrap()).unwrap();
        worker.restore_browser().unwrap();
        assert_eq!(
            worker.browser.as_ref().unwrap().current_session(),
            Some(key.as_str())
        );
        assert!(worker.browser.as_ref().unwrap().message.is_none());
        worker.browser = None;
        let (tx, mut commands) = tokio::sync::mpsc::channel(8);
        worker.monitor.tx = tx;
        worker.update(&json!({"message":{"message_id":101,"from":{"id":42},"chat":{"id":42,"type":"private"},"text":"Continue in my session"}}));
        let monitor::Command::SendMessage { session, text, .. } = commands.try_recv().unwrap()
        else {
            panic!("message expected")
        };
        assert_eq!(session, key);
        assert_eq!(text, "Continue in my session");
        assert!(worker.browser.is_some());
        // Explicitly opening Sessions clears the target; arbitrary text is never lost silently.
        worker.command(BotCommand::Sessions).unwrap();
        assert!(worker.config.selected_session.is_none());
        worker.update(&json!({"message":{"message_id":102,"from":{"id":42},"chat":{"id":42,"type":"private"},"text":"No target"}}));
        assert!(commands.try_recv().is_err());
        assert!(
            worker
                .outbox
                .back()
                .unwrap()
                .contains("No session selected")
        );
    }

    #[test]
    fn old_telegram_configuration_without_selection_still_loads() {
        let config: Config = serde_json::from_value(
            json!({"enabled":true,"user":42,"chat":42,"username":"bot","offset":0,"messages":[]}),
        )
        .unwrap();
        assert!(config.selected_session.is_none());
    }

    #[test]
    fn question_payload_formats_body_and_preserves_exact_button_captions() {
        let mut request = pending(Interaction::allow());
        request.session.title = "Chat 🤖".into();
        request.session.approval.as_mut().unwrap().body = "Run **tests** in `some_dir`?".into();
        let payload = request.payload();
        assert_eq!(payload["text"], "Chat 🤖\nRun tests in some_dir?");
        assert_eq!(
            payload["reply_markup"]["inline_keyboard"][0][0]["text"],
            "Allow"
        );
        assert!(
            payload["entities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entity| entity["type"] == "code")
        );
        assert!(payload.get("parse_mode").is_none());
        request.interaction.questions = vec![question("private", true)];
        request.interaction.questions[0].secret = true;
        assert!(
            !request.payload()["text"]
                .as_str()
                .unwrap()
                .contains("some_dir")
        );
    }
    fn browsing_request(request: Pending) -> (Worker, monitor::State, String, String) {
        let mut worker = worker();
        let mut chat = request.session.clone();
        chat.interaction_data = request.interaction.clone();
        let key = chat.approval_key().unwrap();
        let state = monitor::State {
            sessions: vec![chat],
            ..Default::default()
        };
        worker.pending.insert(key.clone(), request);
        let mut browser = sessions::Browser::new().unwrap();
        browser.message = Some(77);
        let list = browser.content(&state, &worker.pending);
        browser.commit(list.clone());
        let data = list["reply_markup"]["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .find_map(|button| {
                button["callback_data"]
                    .as_str()
                    .filter(|data| data.contains(":s:"))
            })
            .unwrap()
            .to_owned();
        browser.navigate(77, &data).unwrap();
        worker.browser = Some(browser);
        (worker, state, key, data)
    }
    #[test]
    fn browser_delivery_prevents_a_duplicate_notification_and_failed_delivery_stays_queued() {
        let mut request = pending(Interaction::allow());
        request.message = None;
        let (mut worker, state, key, select) = browsing_request(request);
        assert!(worker.next_request_message().is_none());
        assert!(!worker.pending[&key].delivered());
        worker.flush_browser_with(&state, |_, _| bail!("offline"));
        assert!(!worker.pending[&key].browser_delivered);
        let detail = worker
            .browser
            .as_mut()
            .unwrap()
            .content(&state, &worker.pending);
        let back = detail["reply_markup"]["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .find_map(|button| {
                button["callback_data"]
                    .as_str()
                    .filter(|data| data.ends_with(":back"))
            })
            .unwrap()
            .to_owned();
        worker
            .browser
            .as_mut()
            .unwrap()
            .navigate(77, &back)
            .unwrap();
        assert_eq!(worker.next_request_message(), Some(key.clone()));
        worker
            .browser
            .as_mut()
            .unwrap()
            .navigate(77, &select)
            .unwrap();
        worker.flush_browser_with(&state, |method, _| {
            assert_eq!(method, "editMessageText");
            Ok(json!({"message_id":77}))
        });
        assert!(worker.pending[&key].browser_delivered);
        assert!(worker.pending[&key].message.is_none());
        worker.update_counts();
        assert_eq!(worker.status.borrow().queued, 0);
        assert_eq!(worker.status.borrow().delivered, 1);
        worker
            .browser
            .as_mut()
            .unwrap()
            .navigate(77, &back)
            .unwrap();
        assert!(
            worker.next_request_message().is_none(),
            "already shown requests must not notify again after Back"
        );
    }
    #[test]
    fn consecutive_approvals_do_not_archive_copies_of_the_same_agent_reply() {
        fn flush(
            worker: &mut Worker,
            state: &monitor::State,
            messages: &mut BTreeMap<i64, Value>,
            next: &mut i64,
        ) {
            worker.flush_browser_with(state, |method, payload| {
                let id = payload["message_id"].as_i64().unwrap_or(*next);
                match method {
                    "editMessageReplyMarkup" => {
                        messages.get_mut(&id).unwrap()["reply_markup"] =
                            payload["reply_markup"].clone();
                        Ok(json!(true))
                    }
                    "editMessageText" => {
                        messages.insert(id, payload);
                        Ok(json!({"message_id":id}))
                    }
                    "sendMessage" => {
                        *next += 1;
                        messages.insert(id, payload);
                        Ok(json!({"message_id":id}))
                    }
                    _ => panic!("unexpected Telegram method: {method}"),
                }
            });
        }
        let (mut worker, mut state, _, _) = browsing_request(pending(Interaction::allow()));
        state.sessions[0].approval = None;
        state.sessions[0].activity = Activity::Working;
        state.sessions[0].last_message = "I will commit and push the changes.".into();
        worker.pending.clear();
        let mut messages = BTreeMap::new();
        let mut next = 78;
        flush(&mut worker, &state, &mut messages, &mut next);
        assert_eq!(messages.len(), 1);
        for question in ["Commit?", "Push?"] {
            state.sessions[0].approval = Some(Approval {
                id: question.into(),
                body: question.into(),
                expires: u64::MAX,
                can_allow: true,
            });
            state.sessions[0].activity = Activity::Confirmation;
            let request = Pending::new(state.sessions[0].clone()).unwrap();
            worker
                .pending
                .insert(state.sessions[0].approval_key().unwrap(), request);
            let old = worker.browser.as_ref().unwrap().message.unwrap();
            flush(&mut worker, &state, &mut messages, &mut next);
            assert!(messages.contains_key(&old));
            assert_eq!(worker.browser.as_ref().unwrap().message, Some(old));
            assert_eq!(next, 78, "requests must edit, never send a new message");
            flush(&mut worker, &state, &mut messages, &mut next);
            assert_eq!(messages.len(), 1);
            assert!(
                messages.values().next().unwrap()["text"]
                    .as_str()
                    .unwrap()
                    .contains(question)
            );
            state.sessions[0].approval = None;
            state.sessions[0].activity = Activity::Working;
            worker.pending.clear();
            flush(&mut worker, &state, &mut messages, &mut next);
            assert_eq!(messages.len(), 1);
            assert!(
                !messages.values().next().unwrap()["text"]
                    .as_str()
                    .unwrap()
                    .contains(question)
            );
        }
        state.sessions[0].last_message = "Pushed to main.".into();
        state.sessions[0].activity = Activity::Idle;
        flush(&mut worker, &state, &mut messages, &mut next);
        flush(&mut worker, &state, &mut messages, &mut next);
        assert_eq!(
            messages.len(),
            2,
            "only the two genuine replies belong in history"
        );
        assert_eq!(
            messages
                .values()
                .filter(|message| message["text"].as_str().unwrap().contains("I will commit"))
                .count(),
            1
        );
        assert_eq!(
            worker.browser.as_ref().unwrap().current_session(),
            Some(state.sessions[0].key().as_str())
        );
    }

    #[test]
    fn failed_request_card_edit_keeps_ownership_until_retry() {
        let (mut worker, mut state, _, _) = browsing_request(pending(Interaction::allow()));
        state.sessions[0].approval = None;
        worker.pending.clear();
        worker.flush_browser_with(&state, |_, _| Ok(json!({"message_id":77})));
        state.sessions[0].approval = Some(Approval {
            id: "next".into(),
            body: "Next?".into(),
            expires: u64::MAX,
            can_allow: true,
        });
        let key = state.sessions[0].approval_key().unwrap();
        worker.pending.insert(
            key.clone(),
            Pending::new(state.sessions[0].clone()).unwrap(),
        );
        worker.flush_browser_with(&state, |method, _| {
            assert_eq!(method, "editMessageText");
            bail!("offline")
        });
        assert_eq!(worker.browser.as_ref().unwrap().message, Some(77));
        assert!(!worker.pending[&key].delivered());
        worker.flush_browser_with(&state, |method, payload| {
            assert_eq!(method, "editMessageText");
            assert_eq!(payload["message_id"], 77);
            Ok(json!({"message_id":77}))
        });
        assert_eq!(worker.browser.as_ref().unwrap().message, Some(77));
        assert!(worker.pending[&key].browser_delivered);
    }

    #[test]
    fn showing_a_request_in_the_session_retires_its_old_standalone_notification() {
        let request = pending(Interaction::allow());
        let (mut worker, state, key, _) = browsing_request(request);
        worker.flush_browser_with(&state, |_, _| Ok(json!({"message_id":77})));
        assert!(worker.pending[&key].browser_delivered);
        assert!(worker.pending[&key].message.is_none());
        let (id, _) = worker
            .cleanup
            .pop_ready(Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert_eq!(id, 50);
        assert!(worker.next_request_message().is_none());
        let old = json!({"from":{"id":42},"message":{"message_id":50,"chat":{"id":42,"type":"private"}},"data":"random-opaque-value:0"});
        assert!(worker.callback_key(&old).is_none());
        let mut current = old;
        current["message"]["message_id"] = json!(77);
        assert_eq!(worker.callback_key(&current), Some((key, 0)));
    }
    #[test]
    fn hidden_session_does_not_suppress_an_undelivered_question() {
        let mut request = pending(Interaction::allow());
        request.message = None;
        let (mut worker, mut state, key, _) = browsing_request(request);
        state
            .preferences
            .applications
            .observe([crate::agents::sessions::Client::Terminal]);
        state.preferences.applications.set_enabled(
            &crate::agents::sessions::Client::Terminal.application_id(),
            false,
        );
        worker
            .browser
            .as_mut()
            .unwrap()
            .content(&state, &worker.pending);
        assert!(!worker.pending[&key].browser_delivered);
        assert_eq!(worker.next_request_message(), Some(key));
    }
    #[test]
    fn multi_question_browser_answers_advance_without_editing_a_missing_notification() {
        let mut request = pending(Interaction {
            command: None,
            actions: vec![],
            questions: vec![question("one", false), question("two", false)],
        });
        request.message = None;
        let (mut worker, state, key, _) = browsing_request(request);
        let (_, snapshots) = tokio::sync::watch::channel(std::sync::Arc::new(state.clone()));
        worker.monitor = monitor::Handle {
            tx: tokio::sync::mpsc::channel(8).0,
            state: snapshots,
        };
        worker.flush_browser_with(&state, |_, _| Ok(json!({"message_id":77})));
        assert!(worker.answer(&key, Some(0), None).is_ok());
        assert_eq!(worker.pending[&key].question, 1);
        assert!(worker.next_request_message().is_none());
        worker.flush_browser_with(&state, |method, payload| {
            assert_eq!(method, "editMessageText");
            assert!(payload["text"].as_str().unwrap().contains("Question two"));
            Ok(json!({"message_id":77}))
        });
        assert!(worker.pending[&key].browser_delivered);
    }
    #[test]
    fn new_agent_message_removes_old_controls_before_posting_the_new_message() {
        let mut worker = worker();
        let mut chat = session();
        chat.approval = None;
        chat.last_message = "First message".into();
        chat.activity = crate::agents::sessions::Activity::Idle;
        let mut state = monitor::State {
            sessions: vec![chat],
            ..Default::default()
        };
        let mut browser = sessions::Browser::new().unwrap();
        browser.message = Some(77);
        let list = browser.content(&state, &worker.pending);
        let select = list["reply_markup"]["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .find_map(|button| {
                button["callback_data"]
                    .as_str()
                    .filter(|data| data.contains(":s:"))
            })
            .unwrap();
        browser.navigate(77, select).unwrap();
        let detail = browser.content(&state, &worker.pending);
        browser.commit(detail);
        worker.browser = Some(browser);
        worker.config.messages = vec![50, 77];
        state.sessions[0].last_message = "Second **message**".into();
        assert!(!worker.flush_browser_with(&state, |method, payload| {
            assert_eq!(method, "editMessageReplyMarkup");
            assert_eq!(payload["message_id"], 77);
            bail!("offline")
        }));
        assert_eq!(worker.browser.as_ref().unwrap().message, Some(77));
        assert!(worker.flush_browser_with(&state, |method, payload| {
            assert_eq!(method, "editMessageReplyMarkup");
            assert_eq!(payload["message_id"], 77);
            assert_eq!(payload["reply_markup"]["inline_keyboard"], json!([]));
            Ok(json!(true))
        }));
        assert_eq!(worker.config.messages, vec![50]);
        assert!(worker.browser.as_ref().unwrap().selected(77).is_none());
        assert!(worker.flush_browser_with(&state, |method, payload| {
            assert_eq!(method, "sendMessage");
            assert!(payload.get("message_id").is_none());
            assert!(payload["text"].as_str().unwrap().contains("Second message"));
            assert!(
                !payload["reply_markup"]["inline_keyboard"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            Ok(json!({"message_id":78}))
        }));
        assert_eq!(worker.config.messages, vec![50, 78]);
        assert_eq!(worker.browser.as_ref().unwrap().message, Some(78));
        assert!(!worker.flush_browser_with(&state, |_, _| panic!(
            "unchanged message must not be resent"
        )));
        state.sessions[0].activity = crate::agents::sessions::Activity::Working;
        assert!(!worker.flush_browser_with(&state, |method, payload| {
            assert_eq!(method, "editMessageText");
            assert_eq!(payload["message_id"], 78);
            Ok(json!({"message_id":78}))
        }));
    }
    #[test]
    fn structured_questions_collect_all_answers_and_invalidate_previous_buttons() {
        let mut request = pending(Interaction {
            command: None,
            actions: vec![],
            questions: vec![question("one", false), question("two", true)],
        });
        let original = request.nonce.clone();
        assert!(request.choose(Some(0), None).unwrap().is_none());
        assert_ne!(request.nonce, original);
        assert_eq!(request.question, 1);
        assert_eq!(
            request.choose(None, Some("My own answer")).unwrap(),
            Some(Response::Answers(BTreeMap::from([
                ("one".into(), vec!["Yes".into()]),
                ("two".into(), vec!["My own answer".into()])
            ])))
        );
    }
    #[test]
    fn only_offered_choices_and_explicitly_freeform_questions_are_accepted() {
        let mut request = pending(Interaction {
            command: None,
            actions: vec![],
            questions: vec![question("one", false)],
        });
        assert!(request.choose(Some(5), None).is_err());
        assert!(request.choose(None, Some("Injected")).is_err());
        assert!(request.answers.is_empty());
        request.interaction.questions[0].freeform = true;
        assert!(request.choose(None, Some("  ")).is_err());
        request.interaction.questions[0].secret = true;
        assert!(request.choose(Some(0), None).is_err());
        let (text, keyboard) = request.content();
        assert!(!text.contains("Choose carefully"));
        assert!(keyboard["inline_keyboard"].as_array().unwrap().is_empty());
    }
    #[test]
    fn approval_command_is_literal_code_below_question() {
        let command = "printf '%s' '<hello> **literal**'";
        let mut interaction = Interaction::allow();
        interaction.command = Some(command.into());
        let request = pending(interaction);
        let payload = request.payload();
        let text = payload["text"].as_str().unwrap();
        let question = request.session.approval.as_ref().unwrap().body.as_str();
        assert!(text.find(question).unwrap() < text.find(command).unwrap());
        assert!(
            payload["entities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entity| {
                    entity["type"] == "pre"
                        && entity["language"] == "bash"
                        && entity["length"] == command.encode_utf16().count()
                })
        );
    }

    #[test]
    fn button_captions_and_action_values_stay_separate() {
        let mut request = pending(Interaction {
            command: None,
            questions: vec![],
            actions: vec![
                Action {
                    id: "accept".into(),
                    label: "Allow once".into(),
                    detail: String::new(),
                },
                Action {
                    id: "decline".into(),
                    label: "Deny".into(),
                    detail: String::new(),
                },
            ],
        });
        let (_, keyboard) = request.content();
        assert_eq!(keyboard["inline_keyboard"][0][0]["text"], "Allow once");
        assert_eq!(keyboard["inline_keyboard"][1][0]["text"], "Deny");
        assert_eq!(
            request.choose(Some(1), None).unwrap(),
            Some(Response::Action("decline".into()))
        );
        assert!(request.choose(Some(2), None).is_err());
    }
    #[test]
    fn unicode_lengths_and_curl_stdin_escaping_are_bounded() {
        assert_eq!(short("a🤖b", 3), "a🤖…");
        assert_eq!(short("a🤖b", 4), "a🤖b");
        assert_eq!(config_quote("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert!(valid_token("123456:abcdefghijklmnopqrstuvwxyz_123456789"));
        assert!(!valid_token("123456:bad\nurl=evil"));
        assert!(!valid_token("123456:short"));
        assert!(!valid_token("https://example.com"));
    }
    #[test]
    fn persisted_settings_never_contain_token_or_pairing_secret() {
        let config = Config {
            enabled: true,
            user: Some(42),
            chat: Some(42),
            username: "my_bot".into(),
            offset: 100,
            messages: vec![1, 2],
            selected_session: Some("codex:current".into()),
        };
        let value = serde_json::to_value(&config).unwrap();
        assert!(value.get("token").is_none());
        assert!(value.get("pair").is_none());
        assert_eq!(
            serde_json::from_value::<Config>(value).unwrap().chat,
            Some(42)
        );
    }
    #[test]
    fn temporary_discovery_gaps_do_not_permanently_suppress_a_question() {
        let mut worker = worker();
        let session = session();
        let key = session.approval_key().unwrap();
        worker.event(Event::Sync(vec![session.clone()]));
        assert!(worker.pending.contains_key(&key));
        worker.event(Event::Sync(vec![]));
        assert!(!worker.pending.contains_key(&key));
        worker.event(Event::Sync(vec![session]));
        assert!(worker.pending.contains_key(&key));
    }
    #[test]
    fn pending_delivery_can_be_queued_while_a_long_poll_is_in_flight() {
        let mut worker = worker();
        worker.polling = true;
        let session = session();
        let key = session.approval_key().unwrap();
        worker.event(Event::Observe(Box::new(session)));
        assert!(worker.pending.contains_key(&key));
        worker.event(Event::Sync(vec![]));
        assert!(!worker.pending.contains_key(&key));
        assert!(worker.polling);
        // Results from an old bot have no authority over the current poll.
        worker.event(Event::Updates(99, Ok(json!([{"update_id":1000}]))));
        assert_eq!(worker.config.offset, 0);
        assert!(worker.polling);
        worker.event(Event::Updates(worker.generation, Ok(json!([]))));
        assert!(!worker.polling);
    }
    #[test]
    fn commands_match_text_buttons_and_bot_addressing() {
        assert_eq!(
            bot_command("/sessions", "my_bot"),
            Some(BotCommand::Sessions)
        );
        assert_eq!(
            bot_command("/sessions@MY_BOT", "my_bot"),
            Some(BotCommand::Sessions)
        );
        assert_eq!(bot_command("/status", "my_bot"), Some(BotCommand::Status));
        assert_eq!(bot_command("/info", "my_bot"), Some(BotCommand::Info));
        assert_eq!(bot_command("Info", "my_bot"), Some(BotCommand::Info));
        assert_eq!(bot_command("/help", "my_bot"), Some(BotCommand::Help));
        assert_eq!(bot_command("/start", "my_bot"), Some(BotCommand::Help));
        assert_eq!(bot_command("/sessions@other_bot", "my_bot"), None);
        assert_eq!(bot_command("ordinary answer", "my_bot"), None);
        assert_eq!(
            bot_command("Sessions", "my_bot"),
            Some(BotCommand::Sessions)
        );
        assert_eq!(bot_command("Status", "my_bot"), Some(BotCommand::Status));
        assert_eq!(bot_command("Help", "my_bot"), Some(BotCommand::Help));
        assert_eq!(bot_command("Sessions in this project", "my_bot"), None);
        assert_eq!(command_keyboard()["keyboard"][0][0]["text"], "Sessions");
        assert_eq!(command_keyboard()["keyboard"][0][1]["text"], "Help");
    }
    #[test]
    fn paused_notifications_allow_paired_commands_but_not_answers() {
        let mut worker = worker();
        worker.config.enabled = false;
        assert!(worker.paired(42, 42, "private"));
        assert!(!worker.authorized(42, 42, "private"));
        assert!(!worker.paired(99, 42, "private"));
        assert!(!worker.paired(42, 42, "group"));
    }
    #[test]
    fn command_dispatch_rejects_unpaired_users_and_works_while_paused() {
        let mut worker = worker();
        worker.config.username = "my_bot".into();
        worker.config.enabled = false;
        worker.update(
            &json!({"message":{"from":{"id":99},"chat":{"id":42,"type":"private"},"text":"/help"}}),
        );
        assert!(worker.outbox.is_empty());
        worker.update(&json!({"message":{"from":{"id":42},"chat":{"id":42,"type":"private"},"text":"/help@my_bot"}}));
        assert_eq!(worker.outbox.len(), 1);
        assert!(worker.outbox.front().unwrap().contains("/sessions"));
    }
}
