//! Observe terminal Codex through the existing shared App Server. Start a turn
//! only for an explicit user message to an already loaded, idle session.
use super::{requests, runtime};
use crate::agents::{
    interaction::{Delivery, Interaction, Response},
    sessions::{Activity, Approval, Session},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, client::client_with_config, protocol::WebSocketConfig};

const WAIT: Duration = Duration::from_secs(5);
const MAX_MESSAGE: usize = 8 * 1024 * 1024;
#[derive(Clone, PartialEq, Eq)]
struct Pending {
    token: String,
    request: Value,
}
#[derive(Clone, PartialEq, Eq)]
struct View {
    activity: Activity,
    pending: Vec<Pending>,
    progress: Option<String>,
    item: Option<String>,
    summary: String,
    activities: std::collections::VecDeque<crate::domain::session_activity::SessionActivity>,
}
struct Submit {
    session: String,
    approval: Approval,
    response: Response,
    result: mpsc::SyncSender<Delivery>,
    deadline: Instant,
}
struct SendMessage {
    session: String,
    text: String,
    result: mpsc::SyncSender<Result<(), String>>,
    deadline: Instant,
}
enum Command {
    Answer(Submit),
    Message(SendMessage),
}
impl Command {
    fn reject(self) {
        match self {
            Self::Answer(c) => {
                let _ = c.result.send(Delivery::NotSent);
            }
            Self::Message(c) => {
                let _ = c
                    .result
                    .send(Err("Message not sent: no live terminal connection.".into()));
            }
        }
    }
}
pub struct Runtime {
    wanted: tokio::sync::watch::Sender<HashSet<String>>,
    observed: tokio::sync::watch::Receiver<Arc<HashMap<String, View>>>,
    submit: mpsc::SyncSender<Command>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self::new(crate::app::core::codex_home().join("app-server-control/app-server-control.sock"))
    }
}
impl Runtime {
    fn new(path: PathBuf) -> Self {
        Self::with_updates(path, Arc::new(tokio::sync::Notify::new()))
    }
    pub(super) fn with_updates(path: PathBuf, updates: Arc<tokio::sync::Notify>) -> Self {
        let (wanted, mut wanted_rx) = tokio::sync::watch::channel(HashSet::new());
        let (published, observed) = tokio::sync::watch::channel(Arc::new(HashMap::new()));
        let (submit, commands) = mpsc::sync_channel(16);
        thread::spawn(move || {
            let mut generation = 0u64;
            while wanted_rx.has_changed().is_ok() {
                if !wanted_rx.borrow().is_empty() && path.exists() {
                    generation = generation.wrapping_add(1);
                    if let Err(error) = watch(
                        &path,
                        generation,
                        &mut wanted_rx,
                        &published,
                        &commands,
                        &updates,
                    ) {
                        eprintln!("Codex App Server observer disconnected: {error:#}");
                    }
                    if !published.borrow().is_empty() {
                        published.send_replace(Arc::new(HashMap::new()));
                        updates.notify_one();
                    }
                }
                // No server launch, preference changes or API calls on discovery failure.
                while let Ok(command) = commands.try_recv() {
                    command.reject();
                }
                thread::sleep(Duration::from_millis(500));
            }
        });
        Self {
            wanted,
            observed,
            submit,
        }
    }
    pub fn observe(&self, sessions: &mut [Session]) {
        let ids = sessions
            .iter()
            .filter(|s| s.client == crate::agents::sessions::Client::Terminal)
            .map(|s| s.id.clone())
            .collect();
        if *self.wanted.borrow() != ids {
            self.wanted.send_replace(ids);
        }
        let views = self.observed.borrow();
        for session in sessions {
            if !self.wanted.borrow().contains(&session.id) {
                continue;
            }
            if let Some(view) = views.get(&session.id) {
                session.connection = crate::domain::session::ConnectionState::live(true);
                session.activity = view.activity;
                for activity in &view.activities {
                    session.upsert_activity(activity.clone());
                }
                if !view.pending.is_empty() {
                    session.activity = Activity::Confirmation;
                }
                let pending: Vec<_> = view.pending.iter().map(|p| approval(p).id).collect();
                for a in &mut session.activities {
                    if let crate::domain::session_activity::SessionActivity::Request(r) = a
                        && r.status == crate::domain::session_activity::RequestStatus::Pending
                        && !pending.contains(&r.approval.id)
                    {
                        r.status = crate::domain::session_activity::RequestStatus::Resolved;
                    }
                }
                for pending in &view.pending {
                    use crate::domain::session_activity::*;
                    let time = now();
                    let id = format!(
                        "appserver-request:{}:{}:{}",
                        pending.request["params"]["turnId"],
                        pending.request["method"],
                        pending.request["id"]
                    );
                    let old = session.activities.iter().find(|a| a.id() == id);
                    let started = old.map_or(time, SessionActivity::started_at);
                    let updated = old.map_or(time, SessionActivity::updated_at);
                    session.upsert_activity(SessionActivity::Request(SessionRequest {
                        id,
                        started_at: started,
                        updated_at: updated,
                        approval: approval(pending),
                        interaction: requests::interaction(&pending.request),
                        status: RequestStatus::Pending,
                    }));
                }
            }
        }
    }
    pub fn interaction(&self, session: &Session) -> Interaction {
        let views = self.observed.borrow();
        pending_for(&views, session)
            .map(|p| requests::interaction(&p.request))
            .unwrap_or_default()
    }
    pub fn owns_approval(session: &Session) -> bool {
        session
            .approval()
            .as_ref()
            .is_some_and(|a| a.id.starts_with("appserver:"))
    }
    pub fn deliver(&self, session: &Session, response: &Response) -> Delivery {
        if pending_for(&self.observed.borrow(), session).is_none() {
            return Delivery::NotSent;
        }
        let (result, receiver) = mpsc::sync_channel(1);
        let command = Submit {
            session: session.id.clone(),
            approval: session.approval().cloned().unwrap(),
            response: response.clone(),
            result,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        if self.submit.try_send(Command::Answer(command)).is_err() {
            return Delivery::NotSent;
        }
        // A missing result after admission is uncertain, never a reason to retry automatically.
        receiver
            .recv_timeout(Duration::from_secs(15))
            .unwrap_or(Delivery::Uncertain)
    }
    pub fn send_message(&self, session: &Session, text: &str) -> Result<()> {
        ensure!(
            self.observed
                .borrow()
                .get(&session.id)
                .is_some_and(|view| view.activity == Activity::Idle && view.pending.is_empty()),
            "Message not sent: terminal session is unavailable or busy."
        );
        ensure!(
            !text.trim().is_empty() && text.chars().count() <= 16000,
            "Invalid message length."
        );
        let (result, receiver) = mpsc::sync_channel(1);
        self.submit
            .try_send(Command::Message(SendMessage {
                session: session.id.clone(),
                text: text.into(),
                result,
                deadline: Instant::now() + Duration::from_secs(10),
            }))
            .map_err(|_| anyhow::anyhow!("Message not sent: terminal connection is busy."))?;
        receiver.recv_timeout(Duration::from_secs(20))
            .unwrap_or_else(|_| Err("Could not confirm delivery. Check the agent before sending this message again.".into()))
            .map_err(anyhow::Error::msg)
    }
}
fn approval(p: &Pending) -> Approval {
    Approval {
        id: p.token.clone(),
        body: runtime::body(&p.request),
        expires: u64::MAX,
        can_allow: requests::interaction(&p.request)
            .actions
            .iter()
            .any(|a| a.id == "allow"),
    }
}
fn pending_for<'a>(views: &'a HashMap<String, View>, session: &Session) -> Option<&'a Pending> {
    let expected = session.approval()?;
    views
        .get(&session.id)?
        .pending
        .iter()
        .find(|p| approval(p) == *expected)
}
struct Sent {
    session: String,
    result: mpsc::SyncSender<Delivery>,
    deadline: Instant,
}
struct Connection {
    ws: WebSocket<UnixStream>,
    views: HashMap<String, View>,
    sent: HashMap<String, Sent>,
    next: u64,
    generation: u64,
    next_ticket: u64,
    published: tokio::sync::watch::Sender<Arc<HashMap<String, View>>>,
    updates: Arc<tokio::sync::Notify>,
}
impl Connection {
    fn loaded_threads(&mut self) -> Result<HashSet<String>> {
        let mut loaded = HashSet::new();
        let mut cursor = Value::Null;
        let mut cursors = HashSet::new();
        loop {
            let page = self.rpc("thread/loaded/list", json!({"cursor":cursor,"limit":100}))?;
            loaded.extend(
                page["data"]
                    .as_array()
                    .context("Missing loaded threads")?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            );
            cursor = page["nextCursor"].clone();
            if cursor.is_null() {
                return Ok(loaded);
            }
            ensure!(
                loaded.len() < 10000 && cursors.len() < 100 && cursors.insert(cursor.to_string()),
                "Invalid loaded-thread pagination"
            );
        }
    }
    fn send_message(&mut self, command: SendMessage) {
        let mut attempted = false;
        let result = (|| -> Result<()> {
            ensure!(
                command.deadline > Instant::now(),
                "Message expired before sending."
            );
            ensure!(
                self.views
                    .get(&command.session)
                    .is_some_and(|v| v.activity == Activity::Idle && v.pending.is_empty()),
                "Terminal session is unavailable or busy."
            );
            ensure!(
                self.loaded_threads()?.contains(&command.session),
                "Terminal session is no longer loaded."
            );
            let fresh = self.rpc(
                "thread/read",
                json!({"threadId":command.session,"includeTurns":false}),
            )?;
            ensure!(
                fresh["thread"]["id"] == command.session
                    && runtime::activity(&fresh["thread"]["status"]) == Some(Activity::Idle),
                "Terminal session is no longer idle."
            );
            ensure!(
                command.deadline > Instant::now()
                    && self
                        .views
                        .get(&command.session)
                        .is_some_and(|v| v.activity == Activity::Idle && v.pending.is_empty()),
                "Terminal session changed before sending."
            );
            self.next += 1;
            let id = format!("monitor-message-{}", self.next);
            let client_id = runtime::client_message_id()?;
            attempted = true;
            self.send(json!({"id":id,"method":"turn/start","params":{
                "threadId":command.session,"clientUserMessageId":client_id,
                "input":[{"type":"text","text":command.text,"text_elements":[]}]
            }}))?;
            let deadline = Instant::now() + WAIT;
            while Instant::now() < deadline {
                if let Some(message) = self.read()? {
                    if message["id"] == id && message.get("method").is_none() {
                        if let Some(error) = message.get("error") {
                            attempted = false;
                            anyhow::bail!("Codex rejected the message: {error}");
                        }
                        ensure!(
                            message["result"]["turn"]["id"].is_string(),
                            "Missing turn acknowledgement"
                        );
                        return Ok(());
                    }
                    self.event(message);
                }
            }
            anyhow::bail!("Turn acknowledgement timed out")
        })();
        let result = result.map_err(|error| {
            if attempted {
                "Could not confirm delivery. Check the agent before sending this message again."
                    .into()
            } else {
                format!("Message not sent: {error}")
            }
        });
        let _ = command.result.send(result);
    }
    fn connect(
        path: &Path,
        generation: u64,
        published: &tokio::sync::watch::Sender<Arc<HashMap<String, View>>>,
        updates: &Arc<tokio::sync::Notify>,
    ) -> Result<Self> {
        // Codex exposes its control socket through a same-user symlink to a
        // short path in /tmp (Unix socket paths have a small length limit).
        let entry = fs::symlink_metadata(path)?;
        let uid = unsafe { libc::geteuid() };
        ensure!(entry.uid() == uid, "Untrusted App Server socket entry");
        let resolved = fs::canonicalize(path)?;
        let meta = fs::symlink_metadata(&resolved)?;
        ensure!(
            meta.file_type().is_socket() && meta.uid() == uid && meta.mode() & 0o022 == 0,
            "Untrusted App Server socket"
        );
        let socket = UnixStream::connect(&resolved)?;
        socket.set_read_timeout(Some(WAIT))?;
        socket.set_write_timeout(Some(Duration::from_secs(1)))?;
        let config = WebSocketConfig::default()
            .read_buffer_size(64 * 1024)
            .write_buffer_size(0)
            .max_write_buffer_size(MAX_MESSAGE + 1024)
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let (ws, _) = client_with_config("ws://localhost/", socket, Some(config))?;
        let mut c = Self {
            ws,
            views: HashMap::new(),
            sent: HashMap::new(),
            next: 0,
            generation,
            next_ticket: 0,
            published: published.clone(),
            updates: updates.clone(),
        };
        c.ws.get_mut()
            .set_read_timeout(Some(Duration::from_millis(100)))?;
        c.rpc("initialize", json!({"clientInfo":{"name":"ai_agent_monitor","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true,"optOutNotificationMethods":["item/agentMessage/delta","item/reasoning/textDelta","item/commandExecution/outputDelta","item/fileChange/outputDelta"]}}))?;
        c.send(json!({"method":"initialized","params":{}}))?;
        Ok(c)
    }
    fn send(&mut self, value: Value) -> Result<()> {
        self.ws.send(Message::Text(value.to_string().into()))?;
        Ok(())
    }
    fn publish(&self) {
        let changed = **self.published.borrow() != self.views;
        if changed {
            self.published.send_replace(Arc::new(self.views.clone()));
            self.updates.notify_one();
        }
    }
    fn read(&mut self) -> Result<Option<Value>> {
        match self.ws.read() {
            Ok(Message::Text(text)) => Ok(Some(serde_json::from_str(&text)?)),
            Ok(Message::Close(_)) => anyhow::bail!("App Server closed the connection"),
            Ok(_) => {
                self.ws.flush()?;
                Ok(None)
            }
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }
    fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let id = format!("monitor-{}", self.next);
        self.send(json!({"id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if let Some(message) = self.read()? {
                if message["id"] == id && message.get("method").is_none() {
                    ensure!(
                        message.get("error").is_none(),
                        "App Server {method}: {}",
                        message["error"]
                    );
                    return Ok(message["result"].clone());
                }
                self.event(message);
            }
        }
        anyhow::bail!("App Server {method} timed out")
    }
    fn event(&mut self, message: Value) {
        let params = &message["params"];
        let Some(id) = params["threadId"].as_str() else {
            return;
        };
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        use crate::domain::session_activity::*;
        let method = message["method"].as_str();
        if matches!(method, Some("item/started" | "item/completed")) {
            let item = &params["item"];
            if let Some(item_id) = item["id"].as_str() {
                let event_id = format!("appserver-item:{}:{item_id}", params["turnId"]);
                let time = now();
                if method == Some("item/started") {
                    for a in &mut view.activities {
                        if let SessionActivity::Action(a) = a {
                            a.current = false;
                        }
                    }
                }
                if let Some(text) = super::progress::caption(item) {
                    let kind = match item["type"].as_str() {
                        Some("commandExecution") => ActionKind::Command,
                        Some("reasoning") => ActionKind::Reasoning,
                        _ => ActionKind::Tool,
                    };
                    upsert(
                        &mut view.activities,
                        SessionActivity::Action(SessionAction {
                            id: event_id,
                            started_at: time,
                            updated_at: time,
                            kind,
                            current: method == Some("item/started")
                                || view.item.as_deref() == Some(item_id),
                            text,
                            status: if method == Some("item/completed") {
                                if item["status"] == "failed" {
                                    ActionStatus::Failed
                                } else {
                                    ActionStatus::Completed
                                }
                            } else {
                                ActionStatus::Running
                            },
                        }),
                    );
                } else if item["type"] == "agentMessage"
                    && let Some(text) = item["text"].as_str().filter(|s| !s.is_empty())
                {
                    upsert(
                        &mut view.activities,
                        SessionActivity::Message(SessionMessage {
                            id: event_id,
                            started_at: time,
                            updated_at: time,
                            role: MessageRole::Agent,
                            text: text.to_owned(),
                            images: vec![],
                        }),
                    );
                }
            }
        }
        match message["method"].as_str() {
            Some("thread/status/changed") => {
                if let Some(activity) = runtime::activity(&params["status"]) {
                    view.activity = activity;
                    if matches!(activity, Activity::Idle | Activity::Unknown) {
                        view.progress = None;
                        view.item = None;
                        view.summary.clear();
                    }
                }
            }
            Some("item/started") => {
                let item = &params["item"];
                view.item = item["id"].as_str().map(str::to_owned);
                view.summary.clear();
                if matches!(item["type"].as_str(), Some("agentMessage" | "userMessage")) {
                    view.progress = None;
                } else if !(super::progress::empty_summary(item) && view.progress.is_some())
                    && let Some(caption) = super::progress::caption(item)
                {
                    view.progress = Some(caption);
                }
            }
            Some("item/completed") if params["item"]["id"].as_str() == view.item.as_deref() => {
                // Retain the last caption after completion, as Codex does.
                // A newer item/message or terminal thread state supersedes it.
                if params["item"]["type"] == "reasoning"
                    && !super::progress::empty_summary(&params["item"])
                {
                    view.progress = super::progress::caption(&params["item"]);
                }
            }
            Some("item/reasoning/summaryTextDelta")
                if params["itemId"].as_str().is_some()
                    && params["itemId"].as_str() == view.item.as_deref() =>
            {
                if let Some(delta) = params["delta"].as_str() {
                    view.summary.extend(
                        delta
                            .chars()
                            .take(512usize.saturating_sub(view.summary.chars().count())),
                    );
                    if !view.summary.trim().is_empty() {
                        view.progress = Some(super::progress::bounded(&format!(
                            "Thinking: {}",
                            view.summary
                        )));
                        if let Some(SessionActivity::Action(action))=view.activities.iter_mut().rev().find(|a|matches!(a,SessionActivity::Action(v) if v.kind==ActionKind::Reasoning)) {action.text=view.progress.clone().unwrap();action.updated_at=now();}
                    }
                }
            }
            Some("turn/started" | "turn/completed") => {
                for a in &mut view.activities {
                    if let SessionActivity::Action(a) = a {
                        a.current = false;
                    }
                }
                view.progress = None;
                view.item = None;
                view.summary.clear();
            }
            Some("thread/closed" | "thread/archived") => {
                self.views.remove(id);
            }
            Some("serverRequest/resolved") => {
                let key = params["requestId"].to_string();
                view.pending
                    .retain(|p| p.request["id"] != params["requestId"]);
                if self.sent.get(&key).is_some_and(|sent| sent.session == id)
                    && let Some(sent) = self.sent.remove(&key)
                {
                    let _ = sent.result.send(Delivery::Confirmed);
                }
            }
            Some(method)
                if supported(method)
                    && (message["id"].is_string() || message["id"].is_number())
                    && !view.pending.iter().any(|p| p.request == message)
                    && view.pending.len() < 128 =>
            {
                view.pending.retain(|p| p.request["id"] != message["id"]);
                self.next_ticket += 1;
                view.pending.push(Pending {
                    token: format!(
                        "appserver:{}:{}:{}",
                        std::process::id(),
                        self.generation,
                        self.next_ticket
                    ),
                    request: message,
                });
            }
            _ => {}
        }
        self.publish();
    }
    fn submit(&mut self, command: Submit) -> Result<()> {
        if command.deadline <= Instant::now() {
            let _ = command.result.send(Delivery::NotSent);
            return Ok(());
        }
        let pending = self
            .views
            .get(&command.session)
            .and_then(|v| v.pending.iter().find(|p| approval(p) == command.approval));
        let wire = pending.and_then(|p| {
            let (_, field, result) = requests::wire_response(&p.request, &command.response)?;
            let result = if field == "decision" {
                json!({"decision":result})
            } else {
                result
            };
            Some((p.request["id"].clone(), result))
        });
        let Some((id, result)) = wire else {
            let _ = command.result.send(Delivery::NotSent);
            return Ok(());
        };
        let key = id.to_string();
        if self.sent.contains_key(&key) {
            let _ = command.result.send(Delivery::Uncertain);
            return Ok(());
        }
        // Preserve the original JSON request ID and answer on the observing connection.
        self.sent.insert(
            key,
            Sent {
                session: command.session,
                result: command.result,
                deadline: Instant::now() + WAIT,
            },
        );
        self.send(json!({"id":id,"result":result}))
    }
    fn expire(&mut self) {
        self.sent.retain(|_, s| {
            if s.deadline <= Instant::now() {
                let _ = s.result.send(Delivery::Uncertain);
                false
            } else {
                true
            }
        });
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        for (_, s) in self.sent.drain() {
            let _ = s.result.send(Delivery::Uncertain);
        }
    }
}
fn supported(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
            | "item/tool/requestUserInput"
    )
}
fn watch(
    path: &Path,
    generation: u64,
    wanted: &mut tokio::sync::watch::Receiver<HashSet<String>>,
    published: &tokio::sync::watch::Sender<Arc<HashMap<String, View>>>,
    commands: &mpsc::Receiver<Command>,
    updates: &Arc<tokio::sync::Notify>,
) -> Result<()> {
    let mut c = Connection::connect(path, generation, published, updates)?;
    let mut checked = Instant::now() - WAIT;
    loop {
        if wanted.has_changed().is_err() || wanted.borrow().is_empty() {
            return Ok(());
        }
        if wanted.has_changed()? || checked.elapsed() >= WAIT {
            let requested = wanted.borrow_and_update().clone();
            let loaded = c.loaded_threads()?;
            let active: HashSet<_> = requested.intersection(&loaded).cloned().collect();
            for id in c
                .views
                .keys()
                .filter(|id| !active.contains(*id))
                .cloned()
                .collect::<Vec<_>>()
            {
                c.views.remove(&id);
                c.rpc("thread/unsubscribe", json!({"threadId":id}))?;
            }
            for id in active {
                if !c.views.contains_key(&id) {
                    c.views.insert(
                        id.clone(),
                        View {
                            activity: Activity::Unknown,
                            pending: vec![],
                            progress: None,
                            item: None,
                            summary: String::new(),
                            activities: Default::default(),
                        },
                    );
                    // Join only a thread the daemon already reports loaded. No configuration
                    // overrides, history hydration, new turn, server launch, or ownership claim.
                    let result =
                        c.rpc("thread/resume", json!({"threadId":id,"excludeTurns":true}))?;
                    if let Some(view) = c.views.get_mut(&id) {
                        view.activity = runtime::activity(&result["thread"]["status"])
                            .unwrap_or(Activity::Unknown);
                    }
                }
            }
            checked = Instant::now();
            c.publish();
        }
        // Process an incoming resolution before admitting a queued human response.
        let mut drained = false;
        for _ in 0..512 {
            if let Some(message) = c.read()? {
                c.event(message);
            } else {
                drained = true;
                break;
            }
        }
        ensure!(drained, "App Server observer fell behind");
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Answer(command) => c.submit(command)?,
                Command::Message(command) => {
                    if wanted.borrow().contains(&command.session) {
                        c.send_message(command);
                    } else {
                        Command::Message(command).reject();
                    }
                }
            }
        }
        c.expire();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    fn message_probe(
        peer: impl FnOnce(&mut WebSocket<UnixStream>) + Send + 'static,
    ) -> Result<(), String> {
        let (socket, other) = UnixStream::pair().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        other
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let server = thread::spawn(move || {
            let mut ws =
                WebSocket::from_raw_socket(other, tungstenite::protocol::Role::Server, None);
            peer(&mut ws);
        });
        let (published, _observed) = tokio::sync::watch::channel(Arc::new(HashMap::new()));
        let mut c = Connection {
            ws: WebSocket::from_raw_socket(socket, tungstenite::protocol::Role::Client, None),
            views: HashMap::from([(
                "terminal".into(),
                View {
                    activity: Activity::Idle,
                    pending: vec![],
                    progress: None,
                    item: None,
                    summary: String::new(),
                    activities: Default::default(),
                },
            )]),
            sent: HashMap::new(),
            next: 0,
            generation: 1,
            next_ticket: 0,
            published,
            updates: Arc::new(tokio::sync::Notify::new()),
        };
        let (result, receiver) = mpsc::sync_channel(1);
        c.send_message(SendMessage {
            session: "terminal".into(),
            text: "Synthetic message".into(),
            result,
            deadline: Instant::now() + Duration::from_secs(10),
        });
        server.join().unwrap();
        receiver.recv().unwrap()
    }
    fn message_loaded(ws: &mut WebSocket<UnixStream>, loaded: bool) {
        let list = read(ws);
        assert_eq!(list["method"], "thread/loaded/list");
        write(
            ws,
            json!({"id":list["id"],"result":{"data":if loaded {vec!["terminal"]}else{vec![]},"nextCursor":null}}),
        );
    }
    fn message_idle(ws: &mut WebSocket<UnixStream>, idle: bool) {
        let fresh = read(ws);
        assert_eq!(fresh["method"], "thread/read");
        assert_eq!(
            fresh["params"],
            json!({"threadId":"terminal","includeTurns":false})
        );
        write(
            ws,
            json!({"id":fresh["id"],"result":{"thread":{"id":"terminal","status":{"type":if idle {"idle"}else{"active"}}}}}),
        );
    }
    #[test]
    fn sends_to_loaded_idle_terminal_without_resuming_or_overriding_configuration() {
        assert!(message_probe(|ws| {
            message_loaded(ws, true);
            message_idle(ws, true);
            let turn = read(ws);
            assert_eq!(turn["method"], "turn/start");
            assert_eq!(turn["params"]["threadId"], "terminal");
            assert_eq!(turn["params"]["input"][0]["text"], "Synthetic message");
            assert!(turn["params"]["clientUserMessageId"].is_string());
            assert_eq!(turn["params"].as_object().unwrap().len(),3);
            write(ws,json!({"id":turn["id"],"result":{"turn":{"id":"new-turn","status":"inProgress"}}}));
        }).is_ok());
    }
    #[test]
    fn closed_or_newly_busy_terminal_never_receives_a_turn() {
        for loaded in [false, true] {
            let error = message_probe(move |ws| {
                message_loaded(ws, loaded);
                if loaded {
                    message_idle(ws, false);
                }
                assert!(
                    ws.read().is_err(),
                    "unexpected write after failed revalidation"
                );
            })
            .unwrap_err();
            assert!(error.starts_with("Message not sent:"));
        }
    }
    #[test]
    fn terminal_rejection_is_not_confused_with_uncertain_delivery() {
        let error = message_probe(|ws| {
            message_loaded(ws, true);
            message_idle(ws, true);
            let turn = read(ws);
            write(
                ws,
                json!({"id":turn["id"],"error":{"code":-32600,"message":"Synthetic rejection"}}),
            );
        })
        .unwrap_err();
        assert!(error.contains("Codex rejected"));
    }
    #[test]
    fn terminal_disconnect_after_message_is_uncertain_without_retry() {
        let error = message_probe(|ws| {
            message_loaded(ws, true);
            message_idle(ws, true);
            assert_eq!(read(ws)["method"], "turn/start");
            ws.close(None).unwrap();
        })
        .unwrap_err();
        assert!(error.contains("Could not confirm delivery"));
    }
    fn session() -> Session {
        {
            let mut __session = Session {
                agent: "codex",
                id: "terminal".into(),
                title: "Synthetic terminal".into(),
                cwd: PathBuf::from("/tmp"),
                workspace: None,
                git_branch: None,
                model: None,
                model_display_name: None,
                reasoning_effort: None,
                updated_at: 0,
                client: crate::agents::sessions::Client::Terminal,
                activity: Activity::Working,
                target: None,
                connection: if false {
                    crate::domain::session::ConnectionState::live(true)
                } else {
                    Default::default()
                },
                activities: Default::default(),
            };
            __session.seed_snapshot(String::new(), vec![], None, None, Interaction::default());
            __session
        }
    }
    fn read(ws: &mut WebSocket<UnixStream>) -> Value {
        loop {
            if let Message::Text(text) = ws.read().unwrap() {
                return serde_json::from_str(&text).unwrap();
            }
        }
    }
    fn write(ws: &mut WebSocket<UnixStream>, value: Value) {
        ws.send(Message::Text(value.to_string().into())).unwrap();
    }
    fn handshake(listener: &UnixListener, request: Value) -> WebSocket<UnixStream> {
        let (socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        let mut ws = tungstenite::accept(socket).unwrap();
        let init = read(&mut ws);
        assert_eq!(init["method"], "initialize");
        write(&mut ws, json!({"id":init["id"],"result":{}}));
        assert_eq!(read(&mut ws)["method"], "initialized");
        let list = read(&mut ws);
        assert_eq!(list["method"], "thread/loaded/list");
        write(
            &mut ws,
            json!({"id":list["id"],"result":{"data":["terminal"],"nextCursor":null}}),
        );
        let join = read(&mut ws);
        assert_eq!(join["method"], "thread/resume");
        assert_eq!(
            join["params"],
            json!({"threadId":"terminal","excludeTurns":true})
        );
        write(&mut ws, request);
        write(
            &mut ws,
            json!({"id":join["id"],"result":{"thread":{"id":"terminal","status":{"type":"active","activeFlags":["waitingOnApproval"]}}}}),
        );
        ws
    }
    fn request(id: Value) -> Value {
        json!({"id":id,"method":"item/commandExecution/requestApproval","params":{"threadId":"terminal","turnId":"turn","itemId":"item","command":"echo synthetic","reason":"Synthetic confirmation","availableDecisions":["accept","decline"]}})
    }
    #[test]
    fn progress_survives_completion_until_replaced_or_a_message_arrives() {
        let (socket, _peer) = UnixStream::pair().unwrap();
        let (published, _observed) = tokio::sync::watch::channel(Arc::new(HashMap::new()));
        let mut connection = Connection {
            ws: WebSocket::from_raw_socket(socket, tungstenite::protocol::Role::Client, None),
            views: HashMap::from([(
                "terminal".into(),
                View {
                    activity: Activity::Working,
                    pending: vec![],
                    progress: None,
                    item: None,
                    summary: String::new(),
                    activities: Default::default(),
                },
            )]),
            sent: HashMap::new(),
            next: 0,
            generation: 1,
            next_ticket: 0,
            published,
            updates: Arc::new(tokio::sync::Notify::new()),
        };
        connection.event(json!({"method":"item/started","params":{"threadId":"terminal","item":{"type":"commandExecution","id":"cmd","status":"inProgress","command":"cargo test"}}}));
        assert_eq!(
            connection.views["terminal"].progress.as_deref(),
            Some("Running cargo test")
        );
        connection.event(
            json!({"method":"item/completed","params":{"threadId":"terminal","item":{"id":"cmd"}}}),
        );
        assert_eq!(
            connection.views["terminal"].progress.as_deref(),
            Some("Running cargo test")
        );
        connection.event(json!({"method":"item/started","params":{"threadId":"terminal","item":{"id":"thought","type":"reasoning"}}}));
        connection.event(json!({"method":"item/reasoning/textDelta","params":{"threadId":"terminal","itemId":"thought","delta":"RAW REASONING"}}));
        assert_eq!(
            connection.views["terminal"].progress.as_deref(),
            Some("Running cargo test"),
            "empty reasoning and raw text must not replace the last public caption"
        );
        connection.event(json!({"method":"item/reasoning/summaryTextDelta","params":{"threadId":"terminal","itemId":"thought","delta":"   "}}));
        assert_eq!(
            connection.views["terminal"].progress.as_deref(),
            Some("Running cargo test")
        );
        connection.event(json!({"method":"item/reasoning/summaryTextDelta","params":{"threadId":"terminal","itemId":"thought","delta":"Checking "}}));
        connection.event(json!({"method":"item/reasoning/summaryTextDelta","params":{"threadId":"terminal","itemId":"thought","delta":"tests"}}));
        assert_eq!(
            connection.views["terminal"].progress.as_deref(),
            Some("Thinking: Checking tests")
        );
        connection.event(json!({"method":"item/completed","params":{"threadId":"terminal","item":{"id":"thought"}}}));
        assert_eq!(
            connection.views["terminal"].progress.as_deref(),
            Some("Thinking: Checking tests")
        );
        connection.event(json!({"method":"item/started","params":{"threadId":"terminal","item":{"id":"answer","type":"agentMessage"}}}));
        assert!(
            connection.views["terminal"].progress.is_none(),
            "the next agent message replaces the caption"
        );
        connection.event(json!({"method":"thread/status/changed","params":{"threadId":"terminal","status":{"type":"idle"}}}));
        assert!(connection.views["terminal"].progress.is_none());
    }
    fn observe(runtime: &Runtime, session: &mut Session, predicate: impl Fn(&Session) -> bool) {
        let end = Instant::now() + Duration::from_secs(4);
        while Instant::now() < end {
            runtime.observe(std::slice::from_mut(session));
            if predicate(session) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("observation timed out: {session:?}");
    }
    #[test]
    #[ignore = "requires an explicitly supplied isolated App Server with a synthetic pending approval"]
    fn local_server_roundtrip() {
        let path = PathBuf::from(
            std::env::var_os("AAM_TEST_APP_SERVER_SOCKET").expect("isolated test socket"),
        );
        let id = std::env::var("AAM_TEST_APP_SERVER_THREAD").expect("synthetic test thread");
        assert!(
            path.starts_with(std::env::temp_dir()),
            "only temporary test servers are allowed"
        );
        let runtime = Runtime::new(path);
        let mut s = session();
        s.id = id;
        observe(&runtime, &mut s, |s| s.approval().is_some());
        let interaction = runtime.interaction(&s);
        let deny = interaction
            .actions
            .iter()
            .find(|a| a.label == "Deny")
            .expect("deny control");
        assert_eq!(
            runtime.deliver(&s, &Response::Action(deny.id.clone())),
            Delivery::Confirmed
        );
    }
    #[test]
    fn replays_pending_request_and_answers_on_same_connection_with_exact_id() {
        for id in [json!(42), json!("42")] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("server.sock");
            let listener = UnixListener::bind(&path).unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let expected = id.clone();
            let server = thread::spawn(move || {
                let mut ws = handshake(&listener, request(expected.clone()));
                let reply = read(&mut ws);
                assert_eq!(
                    reply,
                    json!({"id":expected,"result":{"decision":"decline"}})
                );
                write(
                    &mut ws,
                    json!({"method":"serverRequest/resolved","params":{"threadId":"terminal","requestId":expected}}),
                );
                write(
                    &mut ws,
                    json!({"method":"thread/status/changed","params":{"threadId":"terminal","status":{"type":"idle"}}}),
                );
                thread::sleep(Duration::from_millis(150));
            });
            let runtime = Runtime::new(path);
            let mut s = session();
            observe(&runtime, &mut s, |s| s.approval().is_some());
            assert_eq!(s.activity, Activity::Confirmation);
            assert!(s.authoritative());
            assert_eq!(
                runtime.interaction(&s).command.as_deref(),
                Some("echo synthetic")
            );
            assert_eq!(
                runtime.deliver(&s, &Response::Action("decision:1".into())),
                Delivery::Confirmed
            );
            observe(&runtime, &mut s, |s| {
                s.approval().is_none() && s.activity == Activity::Idle
            });
            server.join().unwrap();
        }
    }
    #[test]
    fn external_answer_invalidates_the_old_ticket_without_writing_a_reply() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("server.sock");
        let listener = UnixListener::bind(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let (release, wait) = mpsc::sync_channel(1);
        let server = thread::spawn(move || {
            let mut ws = handshake(&listener, request(json!("external")));
            wait.recv().unwrap();
            write(
                &mut ws,
                json!({"method":"serverRequest/resolved","params":{"threadId":"terminal","requestId":"external"}}),
            );
            write(
                &mut ws,
                json!({"method":"thread/status/changed","params":{"threadId":"terminal","status":{"type":"idle"}}}),
            );
            ws.get_mut()
                .set_read_timeout(Some(Duration::from_millis(300)))
                .unwrap();
            assert!(
                matches!(ws.read(),Err(tungstenite::Error::Io(e)) if matches!(e.kind(),io::ErrorKind::WouldBlock|io::ErrorKind::TimedOut))
            );
        });
        let runtime = Runtime::new(path);
        let mut s = session();
        observe(&runtime, &mut s, |s| s.approval().is_some());
        let stale = s.clone();
        release.send(()).unwrap();
        observe(&runtime, &mut s, |s| s.approval().is_none());
        assert_eq!(
            runtime.deliver(&stale, &Response::Action("allow".into())),
            Delivery::NotSent
        );
        server.join().unwrap();
    }
    #[test]
    fn disconnect_after_write_is_uncertain_and_reconnect_cannot_reuse_ticket() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("server.sock");
        let listener = UnixListener::bind(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let (done, wait) = mpsc::sync_channel(1);
        let server = thread::spawn(move || {
            {
                let mut ws = handshake(&listener, request(json!(7)));
                let reply = read(&mut ws);
                assert_eq!(reply["id"], 7);
            }
            let mut ws = handshake(&listener, request(json!(7)));
            wait.recv().unwrap();
            ws.get_mut()
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            assert!(matches!(ws.read(), Err(tungstenite::Error::Io(_))));
        });
        let runtime = Runtime::new(path);
        let mut s = session();
        observe(&runtime, &mut s, |s| s.approval().is_some());
        let stale = s.clone();
        assert_eq!(
            runtime.deliver(&s, &Response::Action("decision:1".into())),
            Delivery::Uncertain
        );
        let old = stale.approval().unwrap().id.clone();
        observe(&runtime, &mut s, |s| {
            s.approval().is_some_and(|a| a.id != old)
        });
        assert_eq!(
            runtime.deliver(&stale, &Response::Action("allow".into())),
            Delivery::NotSent
        );
        done.send(()).unwrap();
        server.join().unwrap();
    }
    #[test]
    fn absent_thread_is_never_resumed_and_invalid_action_is_not_sent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("server.sock");
        let listener = UnixListener::bind(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut ws = tungstenite::accept(socket).unwrap();
            let init = read(&mut ws);
            write(&mut ws, json!({"id":init["id"],"result":{}}));
            read(&mut ws);
            let list = read(&mut ws);
            write(
                &mut ws,
                json!({"id":list["id"],"result":{"data":[],"nextCursor":null}}),
            );
            ws.get_mut()
                .set_read_timeout(Some(Duration::from_millis(300)))
                .unwrap();
            assert!(matches!(ws.read(), Err(tungstenite::Error::Io(_))));
        });
        let runtime = Runtime::new(path);
        let mut s = session();
        runtime.observe(std::slice::from_mut(&mut s));
        assert_eq!(
            runtime.deliver(&s, &Response::Action("allow".into())),
            Delivery::NotSent
        );
        server.join().unwrap();
        assert!(runtime.observed.borrow().is_empty());
    }
}
