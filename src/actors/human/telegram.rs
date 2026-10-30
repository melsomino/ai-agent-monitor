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

pub use crate::domain::telegram::Status;
pub fn diagnostics(status: &Status) -> Value {
    json!({"connected":status.connected,"enabled":status.enabled,"paired":status.paired,"status":status.text,"queued":status.queued,"delivered":status.delivered,"snapshotPending":status.snapshot_pending,"suppressed":status.suppressed,"commandMenuReady":status.command_menu_ready})
}
pub enum Event {
    Connect(String),
    Disconnect,
    Enable(bool),
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
        let mut previous = Vec::new();
        loop {
            if snapshots.has_changed().is_err() {
                break;
            }
            let snapshot = snapshots.borrow_and_update().clone();
            if snapshot.stopping {
                worker.retire_all("Monitor stopped. Answer in the agent app.");
                break;
            }
            if snapshot.sessions != previous {
                previous = snapshot.sessions.clone();
                worker.event(Event::Sync(previous.clone()));
            }
            for event in rx
                .try_recv()
                .into_iter()
                .chain(std::iter::from_fn(|| rx.try_recv().ok()))
                .take(64)
            {
                worker.event(event);
            }
            worker.reconcile_phases(&snapshot);
            worker.flush();
            worker.update_counts();
            worker.poll();
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
}
fn config_path() -> std::path::PathBuf {
    core::home().join("Library/Application Support/AI Agent Monitor/telegram.json")
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
#[derive(Clone)]
struct Pending {
    session: Session,
    interaction: Interaction,
    nonce: String,
    message: Option<i64>,
    question: usize,
    answers: BTreeMap<String, Vec<String>>,
    phase: Option<monitor::Submission>,
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
        })
    }
    fn content(&self) -> (String, Value) {
        let body = self
            .session
            .approval
            .as_ref()
            .map(|a| a.body.trim())
            .unwrap_or_default();
        let mut text = format!("{}\n{body}", self.session.title);
        let mut rows = Vec::new();
        if let Some(question) = self.interaction.questions.get(self.question) {
            text.push_str(&format!(
                "\n\nQuestion {} of {}: {}",
                self.question + 1,
                self.interaction.questions.len(),
                question.title
            ));
            if question.secret {
                text = format!(
                    "{}\nThis request needs a private answer. Open the agent app to respond.",
                    self.session.title
                );
            } else {
                for (index, choice) in question.choices.iter().enumerate().take(90) {
                    rows.push(json!([{"text":short(&choice.label, 80),"callback_data":format!("{}:{index}",self.nonce)}]));
                    if !choice.description.is_empty() {
                        text.push_str(&format!("\n{}: {}", choice.label, choice.description));
                    }
                }
                if question.freeform {
                    text.push_str("\n\nReply to this message to write your own answer.");
                }
            }
        } else {
            for (index, action) in self.interaction.actions.iter().enumerate().take(90) {
                rows.push(json!([{"text":short(&action.label,80),"callback_data":format!("{}:{index}",self.nonce)}]));
                if !action.detail.is_empty() {
                    text.push_str(&format!("\n\n{}: {}", action.label, action.detail));
                }
            }
            if rows.is_empty() {
                text.push_str("\n\nOpen the agent app to respond.");
            }
        }
        match &self.phase {
            Some(monitor::Submission::Sending) => {
                text.push_str("\n\nSending answer…");
                rows.clear();
            }
            Some(monitor::Submission::Reconciling) => {
                text.push_str("\n\nChecking whether the agent received the answer…");
                rows.clear();
            }
            Some(monitor::Submission::Failed(error)) => text.push_str(&format!("\n\n{error}")),
            None => {}
        }
        (short(&text, 3900), json!({"inline_keyboard":rows}))
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
struct Worker {
    monitor: monitor::Handle,
    tx: tokio::sync::mpsc::Sender<Event>,
    status: RefCell<Status>,
    last_published: RefCell<Status>,
    token: String,
    config: Config,
    pending: HashMap<String, Pending>,
    pair: Option<(String, Instant)>,
    retry: Instant,
    generation: u64,
    polling: bool,
    poll_retry: Instant,
    commands_ready: bool,
    commands_retry: Instant,
    outbox: VecDeque<String>,
}
impl Worker {
    fn load(monitor: monitor::Handle, tx: tokio::sync::mpsc::Sender<Event>) -> Self {
        let config = fs::read(config_path())
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        let credential = crate::macos::keychain::load();
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
            pair: None,
            retry: Instant::now(),
            generation: 0,
            polling: false,
            poll_retry: Instant::now(),
            commands_ready: false,
            commands_retry: Instant::now(),
            outbox: VecDeque::new(),
        };
        worker.publish();
        if let Err(error) = credential {
            worker.note(&error.to_string());
        }
        // Old inline buttons never regain authority after a resident restart.
        if !worker.token.is_empty() {
            worker.retire_all("Monitor restarted. Use the latest request message.");
        }
        worker
    }
    fn api(&self, method: &str, payload: Value) -> Result<Value> {
        api(&self.token, method, payload)
    }
    fn publish(&self) {
        *self.status.borrow_mut() = Status {
            connected: !self.token.is_empty(),
            enabled: self.config.enabled,
            paired: self.config.user.is_some(),
            text: if self.token.is_empty() {
                "Not connected".into()
            } else if self.pair.is_some() {
                "Open the bot using the pairing link below, then press Start.".into()
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
                "Reconnect to pair your Telegram account.".into()
            },
            command_menu_ready: self.commands_ready,
            snapshot_pending: 0,
            suppressed: 0,
            queued: self
                .pending
                .values()
                .filter(|p| p.message.is_none())
                .count(),
            delivered: self
                .pending
                .values()
                .filter(|p| p.message.is_some())
                .count(),
            link: self
                .pair
                .as_ref()
                .map(|(code, _)| format!("https://t.me/{}?start={code}", self.config.username)),
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
    fn update_counts(&self) {
        let mut status = self.status.borrow_mut();
        status.command_menu_ready = self.commands_ready;
        status.queued = self
            .pending
            .values()
            .filter(|p| p.message.is_none())
            .count();
        status.delivered = self
            .pending
            .values()
            .filter(|p| p.message.is_some())
            .count();
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
    fn edit(&self, pending: &Pending) -> Result<()> {
        let (text, keyboard) = pending.content();
        self.api("editMessageText", json!({"chat_id":self.config.chat,"message_id":pending.message,"text":text,"reply_markup":keyboard}))?;
        Ok(())
    }
    fn forget(&mut self, key: &str, text: &str) {
        if let Some(pending) = self.pending.remove(key)
            && let Some(message) = pending.message
        {
            self.finish(message, text);
        }
    }
    fn finish(&mut self, message: i64, text: &str) {
        if self.api("editMessageText",json!({"chat_id":self.config.chat,"message_id":message,"text":text,"reply_markup":{"inline_keyboard":[]}})).is_ok() {
            self.config.messages.retain(|id| *id != message); self.save();
        }
    }
    fn retire_all(&mut self, text: &str) {
        self.pending.clear();
        for message in self.config.messages.clone() {
            self.finish(message, text);
        }
    }
    fn event(&mut self, event: Event) {
        match event {
            Event::Updates(generation, updates) => {
                self.polling = false;
                if generation != self.generation {
                    return;
                }
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
                self.note("Connecting…");
                let result = self.connect(token.trim());
                if let Err(error) = result {
                    self.note(&error.to_string());
                }
            }
            Event::Disconnect => {
                self.generation += 1;
                self.retire_all(
                    "Telegram disconnected. This request can no longer be answered here.",
                );
                match crate::macos::keychain::delete() {
                    Ok(()) => {
                        self.outbox.clear();
                        self.token.clear();
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
            Event::Enable(enabled) => {
                self.generation += 1;
                self.config.enabled = enabled && self.config.user.is_some();
                if !self.config.enabled {
                    self.retire_all("Telegram notifications paused. Respond in the agent app.");
                }
                self.save();
                self.publish();
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
            Event::Sync(sessions) => {
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
                    // A discovery gap is not an authoritative resolution.
                    self.forget(
                        &key,
                        "No longer pending. If it returns, use the latest request message.",
                    );
                }
                for session in sessions {
                    if session.approval.is_some() {
                        self.event(Event::Observe(Box::new(session)));
                    }
                }
            }
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
        crate::macos::keychain::save(token)?;
        self.retire_all("Bot reconnected. Use the latest request message.");
        self.generation += 1;
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
        for key in changed {
            if self.pending[&key].message.is_some() {
                let _ = self.edit(&self.pending[&key]);
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
        // One request per pass avoids bursts and leaves time for local changes.
        if let Some(key) = self
            .pending
            .iter()
            .find(|(_, p)| self.config.enabled && p.message.is_none())
            .map(|(k, _)| k.clone())
        {
            let (text, keyboard) = self.pending[&key].content();
            match self.api(
                "sendMessage",
                json!({"chat_id":self.config.chat,"text":text,"reply_markup":keyboard}),
            ) {
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
                    {"command":"status","description":"Show monitor and notification status"},
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
                session_messages(&self.monitor.snapshot().sessions)
            }
            BotCommand::Status => vec![format!("AI Agent Monitor {}\nNotifications: {}\nPending questions: {}",
                core::VERSION, if self.config.enabled {"enabled"} else {"paused"}, self.pending.len())],
            BotCommand::Help => vec!["/sessions — List all open sessions, including idle sessions\n/status — Monitor and notification status\n/help — Show these commands\n\nUse answer buttons on question messages, or reply to one to enter a custom answer.\nCommands require your Mac and monitor to be running.".into()],
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
                let _=self.api("sendMessage",json!({"chat_id":chat,"text":"Paired with AI Agent Monitor. Use /sessions to list open sessions, or the command buttons below.","reply_markup":command_keyboard()}));
                return;
            }
            if !self.paired(user, chat, kind) {
                return;
            }
            if let Some(command) = bot_command(text, &self.config.username) {
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
            {
                let result = self.answer(&key, None, Some(text));
                if let Err(error) = result {
                    let _ = self.api(
                        "sendMessage",
                        json!({"chat_id":chat,"text":error.to_string()}),
                    );
                }
            }
        }
        if let Some(callback) = update.get("callback_query") {
            let user = callback["from"]["id"].as_i64().unwrap_or(0);
            let message = &callback["message"];
            let chat = message["chat"]["id"].as_i64().unwrap_or(0);
            let kind = message["chat"]["type"].as_str().unwrap_or_default();
            let mut feedback = "This request is no longer available.".to_owned();
            if self.authorized(user, chat, kind) {
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
            .find(|(_, p)| p.nonce == nonce && p.message == Some(message_id))
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
            self.edit(&pending)?;
            self.pending.insert(key.to_owned(), pending);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BotCommand {
    Sessions,
    Status,
    Help,
}
fn bot_command(text: &str, username: &str) -> Option<BotCommand> {
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
        "/help" | "/start" => Some(BotCommand::Help),
        _ => None,
    }
}
fn command_keyboard() -> Value {
    json!({"keyboard":[[{"text":"/sessions"},{"text":"/status"}],[{"text":"/help"}]],
        "resize_keyboard":true,"is_persistent":true,"input_field_placeholder":"Choose a command or reply to a question"})
}
fn session_messages(sessions: &[Session]) -> Vec<String> {
    if sessions.is_empty() {
        return vec!["No open agent sessions.".into()];
    }
    let mut sessions: Vec<_> = sessions.iter().collect();
    sessions.sort_by_key(|session| {
        (
            session.client.label().to_lowercase(),
            session.title.to_lowercase(),
            session.agent,
            session.id.as_str(),
        )
    });
    let mut messages = Vec::new();
    let mut page = format!("Open sessions ({})", sessions.len());
    let mut group = "";
    for session in sessions {
        let app = session.client.label();
        let title = short(
            &session
                .title
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            180,
        );
        let status = if session.approval.is_some() {
            "Needs response"
        } else {
            session.activity.label()
        };
        let heading = if group != app {
            format!("\n\n{app}")
        } else {
            String::new()
        };
        let line = format!("\n• {title} · {} · {status}", session.agent_label());
        if page.encode_utf16().count()
            + heading.encode_utf16().count()
            + line.encode_utf16().count()
            > 3900
        {
            messages.push(page);
            page = format!("Open sessions (continued)\n\n{app}{line}");
        } else {
            page.push_str(&heading);
            page.push_str(&line);
        }
        group = app;
    }
    messages.push(page);
    messages
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
            pair: None,
            retry: Instant::now(),
            generation: 0,
            polling: false,
            poll_retry: Instant::now(),
            commands_ready: false,
            commands_retry: Instant::now(),
            outbox: VecDeque::new(),
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
    fn structured_questions_collect_all_answers_and_invalidate_previous_buttons() {
        let mut request = pending(Interaction {
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
    fn button_captions_and_action_values_stay_separate() {
        let mut request = pending(Interaction {
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
        // Results of a poll from an old pairing/enable generation have no authority.
        worker.event(Event::Updates(99, Ok(json!([{"update_id":1000}]))));
        assert_eq!(worker.config.offset, 0);
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
        assert_eq!(bot_command("/help", "my_bot"), Some(BotCommand::Help));
        assert_eq!(bot_command("/start", "my_bot"), Some(BotCommand::Help));
        assert_eq!(bot_command("/sessions@other_bot", "my_bot"), None);
        assert_eq!(bot_command("ordinary answer", "my_bot"), None);
        assert_eq!(command_keyboard()["keyboard"][0][0]["text"], "/sessions");
    }
    #[test]
    fn session_list_includes_idle_sessions_groups_sorts_and_paginates() {
        let mut a = session();
        a.title = "Zulu".into();
        a.approval = None;
        a.activity = Activity::Idle;
        let mut b = a.clone();
        b.title = "Alpha".into();
        b.activity = Activity::Working;
        let text = session_messages(&[a.clone(), b]).join("\n");
        assert!(text.contains("Terminal"));
        assert!(text.contains("Idle"));
        assert!(text.contains("Working"));
        assert!(text.find("Alpha").unwrap() < text.find("Zulu").unwrap());
        assert_eq!(session_messages(&[]), vec!["No open agent sessions."]);
        a.title = "🤖".repeat(200);
        let pages = session_messages(&vec![a; 100]);
        assert!(pages.len() > 1);
        assert!(pages.iter().all(|page| page.encode_utf16().count() <= 3900));
        assert_eq!(
            pages
                .iter()
                .map(|page| page.matches("• ").count())
                .sum::<usize>(),
            100
        );
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
