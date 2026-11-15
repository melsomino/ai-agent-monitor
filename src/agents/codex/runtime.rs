//! Observe the existing owner without resuming threads. Explicit replies are forwarded
//! to that owner; async question replies use its normal user-message admission.
use super::{async_questions, projection, requests};
use crate::agents::interaction::{Interaction, Response};
use crate::agents::sessions::{Activity, Approval};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{self, BufReader, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
const STREAM_VERSION: u64 = 11;

// Verified against the installed Codex 26.1106 (v11) and 26.1111 (v13)
// implementations: the projected state fields and snapshot/patch envelopes are
// compatible. Do not accept arbitrary future versions on permission paths.
fn supported_stream(message: &Value) -> bool {
    message["method"] == "thread-stream-state-changed"
        && matches!(message["version"].as_u64(), Some(11 | 13))
}
// Owners include embedded tool images in snapshots, even though the observer
// projects them out. Receive bounded large snapshots without widening send limits.
const MAX_FRAME: usize = 256 * 1024 * 1024;
const MAX_REQUEST: usize = 32 * 1024 * 1024;
// Large owner snapshots can take several seconds to transfer and decode.
// This budget applies both before sending and while verifying resolution.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Clone, Debug)]
struct Live {
    owner: String,
    revision: u64,
    state: Arc<Value>,
}
#[derive(Clone)]
struct Ticket {
    session: String,
    owner: String,
    request: Value,
}
#[derive(Default)]
struct State {
    tickets: HashMap<String, Ticket>,
    next_ticket: u64,
    activities: HashMap<String, VecDeque<crate::domain::session_activity::SessionActivity>>,
}
pub struct Runtime {
    socket: std::path::PathBuf,
    wanted: tokio::sync::watch::Sender<HashSet<String>>,
    observed: tokio::sync::watch::Receiver<Arc<HashMap<String, Live>>>,
    state: RefCell<State>,
    epoch: u128,
}
impl Default for Runtime {
    fn default() -> Self {
        Self::with_updates(Arc::new(tokio::sync::Notify::new()))
    }
}
impl Runtime {
    pub(super) fn with_updates(updates: Arc<tokio::sync::Notify>) -> Self {
        let socket = socket_path(&crate::app::core::codex_home());
        let watched_socket = socket.clone();
        let (wanted, mut requests) = tokio::sync::watch::channel(HashSet::new());
        let (published, observed) = tokio::sync::watch::channel(Arc::new(HashMap::new()));
        thread::spawn(move || {
            while requests.has_changed().is_ok() {
                if requests.borrow().is_empty() {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                if let Err(error) = watch(&mut requests, &published, &watched_socket, &updates) {
                    eprintln!("Codex observer disconnected: {error:#}");
                    publish_live(&published, &HashMap::new(), &updates);
                }
                if requests.has_changed().is_err() {
                    break;
                }
                thread::sleep(Duration::from_secs(2));
            }
        });
        Self {
            socket,
            wanted,
            observed,
            state: RefCell::new(State::default()),
            epoch: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        }
    }
}
#[derive(Clone)]
pub struct View {
    pub activity: Activity,
    #[cfg(test)]
    pub approval: Option<Approval>,
    pub activities: VecDeque<crate::domain::session_activity::SessionActivity>,
}

impl Runtime {
    pub fn views(&self, ids: &HashSet<String>) -> HashMap<String, View> {
        if *self.wanted.borrow() != *ids {
            self.wanted.send_replace(ids.clone());
        }
        let current = self.observed.borrow().clone();
        let mut state = self.state.borrow_mut();
        state.tickets.retain(|_, t| {
            // Missing transport state is not evidence that the owner resolved
            // or edited a request. Keep its identity through reconnects, but
            // expose no authoritative view until a fresh snapshot arrives.
            ids.contains(&t.session)
                && current.get(&t.session).is_none_or(|s| {
                    s.owner == t.owner
                        && (s.state["requests"]
                            .as_array()
                            .is_some_and(|requests| requests.contains(&t.request))
                            || async_questions::pending(&s.state).as_ref() == Some(&t.request))
                })
        });
        state.activities.retain(|id, _| ids.contains(id));
        let mut result = HashMap::new();
        for id in ids {
            let Some(live) = current.get(id) else {
                continue;
            };
            let Some(activity) = activity(&live.state["threadRuntimeStatus"]) else {
                continue;
            };
            let async_request = async_questions::pending(&live.state);
            use crate::domain::session_activity::*;
            let addressed: Vec<_> = live.state["requests"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| {
                    matches!(
                        r["method"].as_str(),
                        Some(
                            "item/commandExecution/requestApproval"
                                | "item/fileChange/requestApproval"
                                | "item/permissions/requestApproval"
                                | "item/tool/requestUserInput"
                                | "item/tool/requestOptionPicker"
                                | "mcpServer/elicitation/request"
                        )
                    )
                })
                .chain(async_request.as_ref())
                .take(128)
                .collect();
            let mut active_requests = Vec::new();
            for request in addressed {
                let token = state
                    .tickets
                    .iter()
                    .find(|(_, t)| {
                        t.session == *id && t.owner == live.owner && t.request == *request
                    })
                    .map(|(token, _)| token.clone())
                    .unwrap_or_else(|| {
                        state.next_ticket += 1;
                        let token = format!(
                            "ipc:{}:{}:{}",
                            std::process::id(),
                            self.epoch,
                            state.next_ticket
                        );
                        state.tickets.insert(
                            token.clone(),
                            Ticket {
                                session: id.clone(),
                                owner: live.owner.clone(),
                                request: request.clone(),
                            },
                        );
                        token
                    });
                let approval = Approval {
                    id: token,
                    body: body(request),
                    expires: u64::MAX,
                    can_allow: requests::interaction(request)
                        .actions
                        .iter()
                        .any(|a| a.id == "allow"),
                };
                let time = now();
                active_requests.push(SessionActivity::Request(SessionRequest {
                    id: format!(
                        "codex-request:{}:{}:{}",
                        request["params"]["turnId"], request["method"], request["id"]
                    ),
                    started_at: time,
                    updated_at: time,
                    approval,
                    interaction: requests::interaction(request),
                    status: RequestStatus::Pending,
                }));
            }
            let recent = state.activities.entry(id.clone()).or_default();
            let current_action = if activity == Activity::Working {
                super::progress::current_event(&live.state)
            } else {
                None
            };
            for mut incoming in super::progress::recent_events(&live.state) {
                if current_action
                    .as_ref()
                    .is_some_and(|a| a.id == incoming.id())
                {
                    continue;
                }
                if let Some(old) = recent.iter().find(|a| a.id() == incoming.id()) {
                    let updated =
                        if let (SessionActivity::Action(old), SessionActivity::Action(next)) =
                            (old, &incoming)
                        {
                            if old.text == next.text && old.status == next.status {
                                old.updated_at
                            } else {
                                now()
                            }
                        } else {
                            incoming.updated_at()
                        };
                    incoming.set_times(old.started_at(), updated);
                }
                upsert(recent, incoming);
            }
            for a in recent.iter_mut() {
                if let SessionActivity::Action(a) = a {
                    a.current = current_action
                        .as_ref()
                        .is_some_and(|current| current.id == a.id);
                }
            }
            if let Some(mut action) = current_action {
                if let Some(SessionActivity::Action(old)) =
                    recent.iter().find(|a| a.id() == action.id)
                {
                    action.started_at = old.started_at;
                    if action.text == old.text && action.status == old.status {
                        action.updated_at = old.updated_at;
                    }
                }
                upsert(recent, SessionActivity::Action(action));
            }
            for old in recent.iter_mut() {
                if let SessionActivity::Request(r) = old
                    && r.status == RequestStatus::Pending
                    && !active_requests.iter().any(|a| a.id() == r.id)
                {
                    r.status = RequestStatus::Resolved;
                    r.updated_at = now();
                }
            }
            for mut incoming in active_requests {
                if let Some(old) = recent.iter().find(|a| a.id() == incoming.id()) {
                    incoming.set_times(old.started_at(), old.updated_at());
                    if let (SessionActivity::Request(old), SessionActivity::Request(next)) =
                        (old, &mut incoming)
                        && (old.approval != next.approval || old.interaction != next.interaction)
                    {
                        next.updated_at = now();
                    }
                }
                upsert(recent, incoming);
            }
            trim(recent);
            let approval = recent.iter().find_map(|a| {
                if let SessionActivity::Request(r) = a
                    && r.status == RequestStatus::Pending
                {
                    Some(r.approval.clone())
                } else {
                    None
                }
            });
            result.insert(
                id.clone(),
                View {
                    activities: recent.clone(),
                    activity: if approval.is_some() {
                        Activity::Confirmation
                    } else {
                        activity
                    },
                    #[cfg(test)]
                    approval,
                },
            );
        }
        result
    }
}
pub(super) fn activity(status: &Value) -> Option<Activity> {
    match status["type"].as_str()? {
        "idle" => Some(Activity::Idle),
        "active" => Some(
            if status["activeFlags"].as_array().is_some_and(|flags| {
                flags
                    .iter()
                    .any(|f| f == "waitingOnApproval" || f == "waitingOnUserInput")
            }) {
                Activity::Confirmation
            } else {
                Activity::Working
            },
        ),
        "systemError" => Some(Activity::Unknown),
        _ => None,
    }
}
pub(super) fn body(request: &Value) -> String {
    let p = &request["params"];
    if request["method"] == "item/tool/requestUserInput"
        || request["method"] == async_questions::METHOD
    {
        let questions = requests::interaction(request).questions;
        if !questions.is_empty() {
            return questions
                .iter()
                .map(|question| question.title.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
        }
    }
    if request["method"] == "item/tool/requestOptionPicker" {
        let mut text = p["question"]
            .as_str()
            .unwrap_or("Choose an option in Codex.")
            .to_owned();
        for option in p["options"].as_array().into_iter().flatten() {
            if let Some(label) = option["label"].as_str() {
                text.push_str(&format!("\n• {label}"));
                if let Some(description) = option["description"].as_str().filter(|s| !s.is_empty())
                {
                    text.push_str(&format!(" — {description}"));
                }
            }
        }
        return text;
    }
    let reason = ["reason", "justification", "message"]
        .iter()
        .find_map(|key| p[key].as_str().filter(|s| !s.trim().is_empty()));
    if request["method"] == "item/permissions/requestApproval" {
        return format!(
            "{}\n\n{}",
            reason.unwrap_or("Allow the requested permissions?"),
            serde_json::to_string_pretty(&p["permissions"]).unwrap_or_default()
        );
    }
    if let Some(text) = reason {
        return crate::app::core::shorten(text, 2000);
    }
    match request["method"].as_str() {
        Some("item/commandExecution/requestApproval") => "Allow this command?".into(),
        Some("item/fileChange/requestApproval") => "Allow these file changes?".into(),
        _ => "Codex is waiting for your confirmation or input.".into(),
    }
}
#[cfg(test)]
fn approval_method(request: &Value) -> Option<&'static str> {
    if !request["id"].is_string() && !request["id"].is_number() {
        return None;
    }
    match request["method"].as_str()? {
        "item/commandExecution/requestApproval" => {
            Some("thread-follower-command-approval-decision")
        }
        "item/fileChange/requestApproval" => Some("thread-follower-file-approval-decision"),
        _ => None,
    }
}
fn exact_request(state: &Value, ticket: &Ticket) -> bool {
    state["id"] == ticket.session
        && (state["requests"]
            .as_array()
            .is_some_and(|requests| requests.contains(&ticket.request))
            || async_questions::pending(state).as_ref() == Some(&ticket.request))
        && ticket.request["params"]["threadId"] == ticket.session
}
/// Only called by an explicit Allow click. Re-read from the same owner first.
impl Runtime {
    pub fn allow(&self, token: &str) -> bool {
        self.respond_inner(token, &Response::Action("allow".into()), &mut false)
            .unwrap_or(false)
    }
    pub fn allow_for(&self, session: &crate::agents::sessions::Session) -> bool {
        let Some(approval) = &session.approval() else {
            return false;
        };
        let valid = self
            .state
            .borrow()
            .tickets
            .get(&approval.id)
            .is_some_and(|ticket| {
                ticket.session == session.id
                    && body(&ticket.request) == approval.body
                    && approval.can_allow
            });
        valid && self.allow(&approval.id)
    }
    /// Send only to the already running owner; never resume a second agent.
    pub fn send_message(&self, session: &str, text: &str) -> Result<()> {
        let mut attempted = false;
        let result = (|| -> Result<bool> {
            let mut connection = Connection::connect(&self.socket)?;
            let owner = connection.owner(session)?;
            connection.follow(session, &owner, true)?;
            let deadline = Instant::now() + SNAPSHOT_TIMEOUT;
            while Instant::now() < deadline {
                let Some(message) = connection.next()? else {
                    continue;
                };
                if !supported_stream(&message)
                    || message["sourceClientId"] != owner
                    || message["params"]["conversationId"] != session
                    || message["params"]["hostId"] != "local"
                    || message["params"]["change"]["type"] != "snapshot"
                {
                    continue;
                }
                let state = &message["params"]["change"]["conversationState"];
                anyhow::ensure!(
                    state["id"] == session,
                    "Session owner returned a different conversation."
                );
                let client_id = client_message_id()?;
                let (method, version, params) =
                    async_questions::reply_envelope(state, text, &client_id)
                        .context("This session is not accepting messages right now.")?;
                attempted = true;
                let result = connection.request(method, params, version, Some(&owner))?;
                return Ok(admission(&result, false, &mut attempted));
            }
            anyhow::bail!("No live message connection to this session.")
        })();
        match result {
            Ok(true) => Ok(()),
            Ok(false) if !attempted => {
                anyhow::bail!("Codex rejected the message. It was not sent.")
            }
            Err(error) if !attempted => Err(error.context("Message not sent")),
            _ => anyhow::bail!(
                "Could not confirm delivery. Check the agent before sending this message again."
            ),
        }
    }
    pub fn interaction_for(&self, session: &crate::agents::sessions::Session) -> Interaction {
        let Some(approval) = &session.approval() else {
            return Interaction::default();
        };
        self.state
            .borrow()
            .tickets
            .get(&approval.id)
            .filter(|ticket| ticket.session == session.id && body(&ticket.request) == approval.body)
            .map(|ticket| requests::interaction(&ticket.request))
            .unwrap_or_default()
    }
    pub fn respond_for(
        &self,
        session: &crate::agents::sessions::Session,
        response: &Response,
    ) -> bool {
        let Some(approval) = &session.approval() else {
            return false;
        };
        let valid = self
            .state
            .borrow()
            .tickets
            .get(&approval.id)
            .is_some_and(|ticket| {
                ticket.session == session.id && body(&ticket.request) == approval.body
            });
        valid
            && self
                .respond_inner(&approval.id, response, &mut false)
                .unwrap_or(false)
    }
    pub fn deliver_for(
        &self,
        session: &crate::agents::sessions::Session,
        response: &Response,
    ) -> crate::agents::interaction::Delivery {
        use crate::agents::interaction::Delivery;
        let Some(approval) = &session.approval() else {
            return Delivery::NotSent;
        };
        if !self
            .state
            .borrow()
            .tickets
            .get(&approval.id)
            .is_some_and(|t| t.session == session.id && body(&t.request) == approval.body)
        {
            return Delivery::NotSent;
        }
        let mut attempted = false;
        match self.respond_inner(&approval.id, response, &mut attempted) {
            Ok(true) => Delivery::Confirmed,
            _ if attempted => Delivery::Uncertain,
            _ => Delivery::NotSent,
        }
    }
    fn respond_inner(
        &self,
        token: &str,
        response: &Response,
        attempted: &mut bool,
    ) -> Result<bool> {
        let Some(ticket) = self.state.borrow().tickets.get(token).cloned() else {
            return Ok(false);
        };
        let asynchronous = ticket.request["method"] == async_questions::METHOD;
        let wire = requests::follower_response(&ticket.request, &ticket.session, response);
        let async_text = if asynchronous {
            async_questions::reply_text(&ticket.request, response)
        } else {
            None
        };
        if wire.is_none() && async_text.is_none() {
            return Ok(false);
        }
        let mut connection = Connection::connect(&self.socket)?;
        let owner = connection.owner(&ticket.session)?;
        if owner != ticket.owner {
            return Ok(false);
        }
        connection.follow(&ticket.session, &owner, true)?;
        let deadline = Instant::now() + SNAPSHOT_TIMEOUT;
        while Instant::now() < deadline {
            let Some(message) = connection.next()? else {
                continue;
            };
            if !supported_stream(&message)
                || message["sourceClientId"] != owner
                || message["params"]["conversationId"] != ticket.session
                || message["params"]["hostId"] != "local"
            {
                continue;
            }
            let change = &message["params"]["change"];
            if change["type"] != "snapshot" {
                continue;
            }
            if !exact_request(&change["conversationState"], &ticket) {
                return Ok(false);
            }
            if let Some(text) = async_text {
                let client_id = client_message_id()?;
                let Some((method, version, params)) = async_questions::reply_envelope(
                    &change["conversationState"],
                    &text,
                    &client_id,
                ) else {
                    return Ok(false);
                };
                *attempted = true;
                let result = connection.request(method, params, version, Some(&owner))?;
                return Ok(admission(&result, false, attempted));
            }
            let Some((method, params)) = wire else {
                return Ok(false);
            };
            let mut confirmed = HashMap::new();
            if !apply_change(&mut confirmed, &ticket.session, &owner, change) {
                return Ok(false);
            }
            *attempted = true;
            let result = connection.request(method, params, 1, Some(&owner))?;
            if !admission(&result, true, attempted) {
                return Ok(false);
            }
            // The owner returns ok even when its strict request lookup finds nothing.
            // A transport acknowledgement alone must not make the UI report Sent.
            connection.follow(&ticket.session, &owner, true)?;
            let deadline = Instant::now() + SNAPSHOT_TIMEOUT;
            while Instant::now() < deadline {
                let Some(message) = connection.next()? else {
                    continue;
                };
                if !supported_stream(&message)
                    || message["sourceClientId"] != owner
                    || message["params"]["conversationId"] != ticket.session
                    || message["params"]["hostId"] != "local"
                {
                    continue;
                }
                if !apply_change(
                    &mut confirmed,
                    &ticket.session,
                    &owner,
                    &message["params"]["change"],
                ) {
                    connection.follow(&ticket.session, &owner, true)?;
                    continue;
                }
                if Self::request_resolved(&confirmed, &ticket) {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        Ok(false)
    }

    fn request_resolved(live: &HashMap<String, Live>, ticket: &Ticket) -> bool {
        live.get(&ticket.session)
            .filter(|current| current.owner == ticket.owner)
            .and_then(|current| current.state["requests"].as_array())
            .is_some_and(|requests| {
                !requests.iter().any(|request| {
                    request["id"] == ticket.request["id"]
                        && request["method"] == ticket.request["method"]
                })
            })
    }
}

fn socket_path(home: &std::path::Path) -> std::path::PathBuf {
    home.join("ipc/ipc.sock")
}

// A received rejection proves non-admission. A missing/malformed response does not.
pub(super) fn client_message_id() -> Result<String> {
    let mut random = [0u8; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    random[6] = (random[6] & 0x0f) | 0x40;
    random[8] = (random[8] & 0x3f) | 0x80;
    let hex = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let client_id = format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    );

    Ok(client_id)
}
fn admission(result: &Value, approval: bool, attempted: &mut bool) -> bool {
    let accepted =
        result["resultType"] == "success" && (!approval || result["result"]["ok"] == true);
    if result["resultType"] == "error"
        || (approval && result["resultType"] == "success" && result["result"]["ok"] == false)
    {
        *attempted = false;
    }
    accepted
}

struct Connection {
    socket: UnixStream,
    input: BufReader<UnixStream>,
    header: [u8; 4],
    header_len: usize,
    header_started: Option<Instant>,
    failed: bool,
    queued: VecDeque<Value>,
    client: String,
    next_id: u64,
    follows: HashMap<String, String>,
}
impl Connection {
    fn connect(path: &std::path::Path) -> Result<Self> {
        let meta = fs::symlink_metadata(path)?;
        // SAFETY: geteuid has no preconditions.
        anyhow::ensure!(
            meta.file_type().is_socket()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0,
            "Untrusted Codex IPC endpoint"
        );
        let mut connection = Self::from_socket(UnixStream::connect(path)?)?;
        let reply =
            connection.request("initialize", json!({"clientType":"agent-monitor"}), 0, None)?;
        connection.client = reply["result"]["clientId"]
            .as_str()
            .context("IPC initialization failed")?
            .into();
        Ok(connection)
    }
    fn from_socket(socket: UnixStream) -> Result<Self> {
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        socket.set_write_timeout(Some(Duration::from_millis(250)))?;
        Ok(Self {
            input: BufReader::with_capacity(65536, socket.try_clone()?),
            socket,
            header: [0; 4],
            header_len: 0,
            header_started: None,
            failed: false,
            queued: VecDeque::new(),
            client: format!("agent-monitor-{}", std::process::id()),
            next_id: 0,
            follows: HashMap::new(),
        })
    }
    fn send(&mut self, value: Value) -> Result<()> {
        anyhow::ensure!(!self.failed, "IPC connection is unusable");
        let data = serde_json::to_vec(&value)?;
        anyhow::ensure!(data.len() <= MAX_REQUEST, "IPC message too large");
        self.socket.write_all(&(data.len() as u32).to_le_bytes())?;
        self.socket.write_all(&data)?;
        Ok(())
    }
    fn read(&mut self) -> Result<Option<Value>> {
        anyhow::ensure!(!self.failed, "IPC connection is unusable");
        let result = self.read_frame();
        if result.is_err() {
            // A partial JSON decoder cannot be resumed after an error. Never
            // interpret leftover frame bytes as another header/request.
            self.failed = true;
        }
        result
    }
    fn read_frame(&mut self) -> Result<Option<Value>> {
        if let Some(started) = self.header_started {
            anyhow::ensure!(started.elapsed() < SNAPSHOT_TIMEOUT, "IPC header timed out");
        }
        if self.header_len < 4 {
            match self.input.read(&mut self.header[self.header_len..]) {
                Ok(0) => anyhow::bail!("IPC disconnected"),
                Ok(size) => {
                    self.header_started.get_or_insert_with(Instant::now);
                    self.header_len += size;
                    if self.header_len < 4 {
                        return Ok(None);
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error.into()),
            }
        }
        let length = u32::from_le_bytes(self.header) as usize;
        anyhow::ensure!(length <= MAX_FRAME, "IPC snapshot too large");
        // Limit before buffering: the JSON decoder cannot eat the next frame.
        // Short socket timeouts are retried inside one bounded decode; restarting
        // a serde decoder after a timeout would lose its position in the JSON.
        let mut frame = (&mut self.input).take(length as u64);
        let value = projection::decode_reader(FrameRead {
            input: &mut frame,
            deadline: Instant::now() + SNAPSHOT_TIMEOUT,
        })?;
        anyhow::ensure!(frame.limit() == 0, "Truncated IPC frame");
        self.header_len = 0;
        self.header_started = None;
        Ok(Some(value))
    }
    fn handle_discovery(&mut self, message: &Value) -> Result<bool> {
        if message["type"] == "client-discovery-request" {
            self.send(json!({"type":"client-discovery-response","requestId":message["requestId"],"response":{"canHandle":false}}))?;
            return Ok(true);
        }
        // This observer never advertises ownership or handles agent operations.
        if message["type"] == "request" {
            self.send(json!({"type":"response","requestId":message["requestId"],"resultType":"error","error":"observer-only"}))?;
            return Ok(true);
        }
        Ok(false)
    }
    fn next(&mut self) -> Result<Option<Value>> {
        if let Some(message) = self.queued.pop_front() {
            return Ok(Some(message));
        }
        let Some(message) = self.read()? else {
            return Ok(None);
        };
        if self.handle_discovery(&message)? {
            Ok(None)
        } else {
            Ok(Some(message))
        }
    }
    fn begin_request(
        &mut self,
        method: &str,
        params: Value,
        version: u64,
        target: Option<&str>,
    ) -> Result<(String, Instant)> {
        self.next_id += 1;
        let id = format!("aim-{}", self.next_id);
        let timeout = if matches!(
            method,
            "thread-follower-start-turn" | "thread-follower-steer-turn"
        ) {
            8000
        } else if method == "thread-owner-discovery" {
            // A response can arrive behind an image-heavy snapshot on the
            // same framed stream; allow time to drain and project that frame.
            5000
        } else {
            1000
        };
        let mut message = json!({"type":"request","requestId":id,"sourceClientId":self.client,"method":method,"version":version,"params":params,"timeoutMs":timeout});
        if let Some(target) = target {
            message["targetClientId"] = target.into();
        }
        self.send(message)?;
        let deadline = Instant::now() + Duration::from_millis(timeout + 500);
        Ok((id, deadline))
    }
    fn request(
        &mut self,
        method: &str,
        params: Value,
        version: u64,
        target: Option<&str>,
    ) -> Result<Value> {
        let (id, deadline) = self.begin_request(method, params, version, target)?;
        while Instant::now() < deadline {
            let Some(message) = self.read()? else {
                continue;
            };
            if self.handle_discovery(&message)? {
                continue;
            }
            if message["type"] == "response" && message["requestId"] == id {
                return Ok(message);
            }
            if message["type"] == "broadcast" {
                anyhow::ensure!(self.queued.len() < 512, "IPC observer fell behind");
                self.queued.push_back(message);
            }
        }
        anyhow::bail!("IPC read timed out")
    }
    fn owner(&mut self, session: &str) -> Result<String> {
        let reply = self.request(
            "thread-owner-discovery",
            json!({"hostId":"local","conversationId":session}),
            1,
            None,
        )?;
        anyhow::ensure!(reply["resultType"] == "success", "No active thread owner");
        Ok(reply["handledByClientId"]
            .as_str()
            .context("Missing thread owner")?
            .into())
    }
    fn follow(&mut self, session: &str, owner: &str, following: bool) -> Result<()> {
        self.send(json!({"type":"broadcast","method":"thread-stream-following-changed","version":1,"sourceClientId":self.client,"targetClientIds":[owner],"params":{"hostId":"local","conversationId":session,"following":following}}))?;
        if following {
            self.follows.insert(session.into(), owner.into());
        } else {
            self.follows.remove(session);
        }
        Ok(())
    }
}
/// A partial frame owns the decoder until completion or its absolute deadline.
/// This runs on the observer/provider worker, never on the UI/monitor thread.
struct FrameRead<R> {
    input: R,
    deadline: Instant,
}
impl<R: Read> Read for FrameRead<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            if Instant::now() >= self.deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "IPC frame timed out",
                ));
            }
            match self.input.read(bytes) {
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                result => return result,
            }
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        for (session, owner) in self.follows.clone() {
            let _ = self.follow(&session, &owner, false);
        }
    }
}
fn watch(
    wanted_receiver: &mut tokio::sync::watch::Receiver<HashSet<String>>,
    published: &tokio::sync::watch::Sender<Arc<HashMap<String, Live>>>,
    socket: &std::path::Path,
    updates: &tokio::sync::Notify,
) -> Result<()> {
    let mut connection = Connection::connect(socket)?;
    let mut live = HashMap::<String, Live>::new();
    let mut owners: HashMap<String, String> = HashMap::new();
    let mut awaiting_snapshot: HashMap<String, Instant> = HashMap::new();
    let mut owner_requests: HashMap<String, (String, Instant)> = HashMap::new();
    let mut checked = Instant::now() - Duration::from_secs(6);
    let mut last_wanted = HashSet::new();
    loop {
        if wanted_receiver.has_changed().is_err() {
            return Ok(());
        }
        let wanted = wanted_receiver.borrow_and_update().clone();
        if wanted.is_empty() {
            return Ok(());
        }
        if checked.elapsed() >= Duration::from_secs(5) || wanted != last_wanted {
            last_wanted = wanted.clone();
            for session in owners
                .keys()
                .filter(|s| !wanted.contains(*s))
                .cloned()
                .collect::<Vec<_>>()
            {
                if let Some(owner) = owners.remove(&session) {
                    connection.follow(&session, &owner, false)?;
                }
                live.remove(&session);
                awaiting_snapshot.remove(&session);
            }
            owner_requests.retain(|_, (session, deadline)| {
                wanted.contains(session) && Instant::now() < *deadline
            });
            for session in wanted {
                if owner_requests
                    .values()
                    .any(|(pending, _)| pending == &session)
                {
                    continue;
                }
                let (request, deadline) = connection.begin_request(
                    "thread-owner-discovery",
                    json!({"hostId":"local","conversationId":session}),
                    1,
                    None,
                )?;
                owner_requests.insert(request, (session, deadline));
            }
            checked = Instant::now();
        }
        publish_live(published, &live, updates);
        let Some(message) = connection.next()? else {
            continue;
        };
        if message["type"] == "response" {
            let Some((session, _)) = message["requestId"]
                .as_str()
                .and_then(|id| owner_requests.remove(id))
            else {
                continue;
            };
            if !wanted_receiver.borrow().contains(&session) {
                continue;
            }
            if message["resultType"] == "success"
                && let Some(owner) = message["handledByClientId"].as_str()
            {
                if owners.get(&session).map(String::as_str) != Some(owner) {
                    if let Some(old) = owners.insert(session.clone(), owner.into()) {
                        connection.follow(&session, &old, false)?;
                    }
                    live.remove(&session);
                    awaiting_snapshot.remove(&session);
                }
                if needs_snapshot(
                    live.contains_key(&session),
                    awaiting_snapshot.get(&session).copied(),
                    Instant::now(),
                ) {
                    connection.follow(&session, owner, true)?;
                    awaiting_snapshot.insert(session, Instant::now());
                }
            } else {
                if let Some(owner) = owners.remove(&session) {
                    connection.follow(&session, &owner, false)?;
                }
                live.remove(&session);
                awaiting_snapshot.remove(&session);
            }
        } else if message["method"] == "ipc-connection-reset" {
            owners.clear();
            owner_requests.clear();
            live.clear();
            awaiting_snapshot.clear();
            checked = Instant::now() - Duration::from_secs(6);
        } else if message["method"] == "thread-stream-following-status-requested" {
            if let Some(session) = message["params"]["conversationId"].as_str()
                && message["params"]["hostId"] == "local"
                && let Some(owner) = owners.get(session)
                && message["sourceClientId"] == *owner
            {
                connection.follow(session, owner, true)?;
                awaiting_snapshot.insert(session.into(), Instant::now());
            }
        } else if message["method"] == "client-status-changed"
            && message["params"]["status"] == "disconnected"
        {
            if let Some(client) = message["params"]["clientId"].as_str() {
                owners.retain(|_, owner| owner != client);
                live.retain(|_, s| s.owner != client);
                awaiting_snapshot.retain(|session, _| owners.contains_key(session));
                checked = Instant::now() - Duration::from_secs(6);
            }
        } else if supported_stream(&message) {
            let Some(session) = message["params"]["conversationId"].as_str() else {
                continue;
            };
            let Some(owner) = owners.get(session) else {
                continue;
            };
            if message["sourceClientId"] != *owner || message["params"]["hostId"] != "local" {
                continue;
            }
            if !wanted_receiver.borrow().contains(session) {
                continue;
            }
            if !apply_change(&mut live, session, owner, &message["params"]["change"]) {
                awaiting_snapshot.remove(session);
                checked = Instant::now() - Duration::from_secs(6);
            } else {
                awaiting_snapshot.remove(session);
            }
            publish_live(published, &live, updates);
        }
    }
}
fn publish_live(
    published: &tokio::sync::watch::Sender<Arc<HashMap<String, Live>>>,
    live: &HashMap<String, Live>,
    updates: &tokio::sync::Notify,
) {
    let changed = {
        let old = published.borrow();
        old.len() != live.len()
            || live.iter().any(|(id, view)| {
                old.get(id).is_none_or(|before| {
                    before.owner != view.owner
                        || before.revision != view.revision
                        || !Arc::ptr_eq(&before.state, &view.state)
                })
            })
    };
    if changed {
        published.send_replace(Arc::new(live.clone()));
        updates.notify_one();
    }
}
fn needs_snapshot(has_live: bool, awaiting: Option<Instant>, now: Instant) -> bool {
    !has_live && awaiting.is_none_or(|sent| now.duration_since(sent) >= SNAPSHOT_TIMEOUT)
}
fn apply_change(
    live: &mut HashMap<String, Live>,
    session: &str,
    owner: &str,
    change: &Value,
) -> bool {
    if change["type"] == "snapshot" {
        let state = &change["conversationState"];
        let Some(revision) = change["revision"].as_u64() else {
            live.remove(session);
            return false;
        };
        if state["id"] != session || !state["requests"].is_array() {
            live.remove(session);
            return false;
        }
        live.insert(
            session.into(),
            Live {
                owner: owner.into(),
                revision,
                state: Arc::new(async_questions::project(state)),
            },
        );
        return true;
    }
    let Some(current) = live.get_mut(session) else {
        return false;
    };
    if current.owner != owner
        || change["type"] != "patches"
        || change["baseRevision"].as_u64() != Some(current.revision)
    {
        live.remove(session);
        return false;
    }
    let Some(patches) = change["patches"].as_array() else {
        live.remove(session);
        return false;
    };
    for patch in patches {
        let Some(path) = patch["path"].as_array() else {
            live.remove(session);
            return false;
        };
        let value = if matches!(
            path.first().and_then(Value::as_str),
            Some("requests" | "threadRuntimeStatus")
        ) {
            patch["value"].clone()
        } else if let Some(value) = async_questions::project_patch(path, &patch["value"]) {
            value
        } else {
            continue;
        };
        if !apply_patch(
            Arc::make_mut(&mut current.state),
            path,
            patch["op"].as_str().unwrap_or(""),
            &value,
        ) {
            live.remove(session);
            return false;
        }
    }
    let Some(revision) = change["revision"].as_u64() else {
        live.remove(session);
        return false;
    };
    current.revision = revision;
    true
}
fn apply_patch(value: &mut Value, path: &[Value], op: &str, replacement: &Value) -> bool {
    let Some((key, rest)) = path.split_first() else {
        return false;
    };
    if let Some(object) = value.as_object_mut() {
        let Some(key) = key.as_str() else {
            return false;
        };
        if rest.is_empty() {
            match op {
                "add" | "replace" => {
                    object.insert(key.into(), replacement.clone());
                }
                "remove" => {
                    if object.remove(key).is_none() {
                        return false;
                    }
                }
                _ => return false,
            };
            true
        } else {
            object
                .get_mut(key)
                .is_some_and(|v| apply_patch(v, rest, op, replacement))
        }
    } else if let Some(array) = value.as_array_mut() {
        let Some(index) = key.as_u64().and_then(|v| usize::try_from(v).ok()) else {
            return false;
        };
        if rest.is_empty() {
            match op {
                "add" if index <= array.len() => array.insert(index, replacement.clone()),
                "replace" if index < array.len() => array[index] = replacement.clone(),
                "remove" if index < array.len() => {
                    array.remove(index);
                }
                _ => return false,
            };
            true
        } else {
            array
                .get_mut(index)
                .is_some_and(|v| apply_patch(v, rest, op, replacement))
        }
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn chat_messages_target_the_existing_owner_and_never_retry_uncertain_delivery() {
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        fn read(stream: &mut UnixStream) -> Value {
            let mut size = [0u8; 4];
            stream.read_exact(&mut size).unwrap();
            let mut data = vec![0; u32::from_le_bytes(size) as usize];
            stream.read_exact(&mut data).unwrap();
            serde_json::from_slice(&data).unwrap()
        }
        fn write(stream: &mut UnixStream, value: Value) {
            let data = serde_json::to_vec(&value).unwrap();
            stream
                .write_all(&(data.len() as u32).to_le_bytes())
                .unwrap();
            stream.write_all(&data).unwrap();
        }
        for (version, status, outcome) in [
            (11, "idle", "success"),
            (11, "active", "success"),
            (11, "active", "error"),
            (11, "idle", "disconnect"),
            (13, "idle", "success"),
            (13, "active", "success"),
            (13, "active", "error"),
            (13, "idle", "disconnect"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("ipc.sock");
            let listener = UnixListener::bind(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let init = read(&mut socket);
                assert_eq!(init["method"], "initialize");
                write(
                    &mut socket,
                    json!({"type":"response","requestId":init["requestId"],"resultType":"success","result":{"clientId":"monitor"}}),
                );
                let discovery = read(&mut socket);
                assert_eq!(discovery["method"], "thread-owner-discovery");
                assert_eq!(discovery["params"]["conversationId"], "chat");
                write(
                    &mut socket,
                    json!({"type":"response","requestId":discovery["requestId"],"resultType":"success","handledByClientId":"owner"}),
                );
                let follow = read(&mut socket);
                assert_eq!(follow["method"], "thread-stream-following-changed");
                assert_eq!(follow["targetClientIds"], json!(["owner"]));
                write(
                    &mut socket,
                    json!({"type":"broadcast","method":"thread-stream-state-changed","sourceClientId":"owner","version":version,"params":{"hostId":"local","conversationId":"chat","change":{"type":"snapshot","revision":1,"conversationState":{"id":"chat","requests":[],"threadRuntimeStatus":{"type":status}}}}}),
                );
                let request = read(&mut socket);
                assert_eq!(request["targetClientId"], "owner");
                assert_eq!(request["params"]["conversationId"], "chat");
                if status == "idle" {
                    assert_eq!(request["method"], "thread-follower-start-turn");
                    assert_eq!(request["version"], 2);
                    assert_eq!(
                        request["params"]["turnStart"]["request"]["input"][0]["text"],
                        "Hello from Telegram"
                    );
                    assert_eq!(
                        request["params"]["turnStart"]["context"]["inheritThreadSettings"],
                        true
                    );
                } else {
                    assert_eq!(request["method"], "thread-follower-steer-turn");
                    assert_eq!(request["version"], 1);
                    assert_eq!(request["params"]["input"][0]["text"], "Hello from Telegram");
                }
                if outcome != "disconnect" {
                    write(
                        &mut socket,
                        json!({"type":"response","requestId":request["requestId"],"resultType":outcome,"result":{}}),
                    );
                }
            });
            let (wanted, _requests) = tokio::sync::watch::channel(HashSet::new());
            let (_published, observed) = tokio::sync::watch::channel(Arc::new(HashMap::new()));
            let runtime = Runtime {
                socket: path,
                wanted,
                observed,
                state: RefCell::new(State::default()),
                epoch: 0,
            };
            let result = runtime.send_message("chat", "Hello from Telegram");
            server.join().unwrap();
            match outcome {
                "success" => assert!(result.is_ok(), "{result:?}"),
                "error" => assert!(result.unwrap_err().to_string().contains("rejected")),
                _ => assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("Could not confirm delivery")
                ),
            }
        }
    }
    #[test]
    fn explicit_rejections_are_not_confused_with_unknown_delivery() {
        for (value, approval) in [
            (serde_json::json!({"resultType":"error"}), false),
            (serde_json::json!({"resultType":"error"}), true),
            (
                serde_json::json!({"resultType":"success","result":{"ok":false}}),
                true,
            ),
        ] {
            let mut attempted = true;
            assert!(!super::admission(&value, approval, &mut attempted));
            assert!(!attempted);
        }
        let mut attempted = true;
        assert!(!super::admission(
            &serde_json::json!({}),
            true,
            &mut attempted
        ));
        assert!(attempted);
        assert!(super::admission(
            &serde_json::json!({"resultType":"success","result":{"ok":true}}),
            true,
            &mut attempted
        ));
    }
    #[test]
    fn socket_follows_the_injected_codex_home() {
        assert_eq!(
            super::socket_path(std::path::Path::new("/custom/codex")),
            std::path::Path::new("/custom/codex/ipc/ipc.sock")
        );
    }

    use super::*;
    fn request() -> Value {
        json!({"method":"item/commandExecution/requestApproval","id":82,"params":{"threadId":"chat","turnId":"turn","reason":"Allow file update?","command":"original"}})
    }
    #[test]
    fn waiting_flag_without_an_addressed_request_does_not_invent_an_approval() {
        let mut live = HashMap::new();
        let mut update = snapshot();
        update["conversationState"]["requests"] = json!([]);
        apply_change(&mut live, "chat", "owner", &update);
        let (published, observed) = tokio::sync::watch::channel(Arc::new(live.clone()));
        let (wanted, _requests) = tokio::sync::watch::channel(HashSet::new());
        let runtime = Runtime {
            socket: Default::default(),
            wanted,
            observed,
            state: RefCell::new(State::default()),
            epoch: 0,
        };
        let ids = HashSet::from(["chat".to_owned()]);
        let view = runtime.views(&ids).remove("chat").unwrap();
        assert_eq!(view.activity, Activity::Confirmation);
        assert!(view.approval.is_none());
        update["revision"] = json!(5);
        update["conversationState"]["requests"] = json!([request()]);
        apply_change(&mut live, "chat", "owner", &update);
        published.send_replace(Arc::new(live));
        assert!(
            runtime.views(&ids)["chat"].approval.is_some(),
            "real requests must still arrive"
        );
    }
    #[test]
    fn approval_resume_keeps_current_caption_even_when_item_predates_latest_message() {
        use crate::domain::session::{Client, ConnectionState, Session};
        use crate::domain::session_activity::*;
        let state = json!({"id":"chat","requests":[request()],"threadRuntimeStatus":{"type":"active","activeFlags":["waitingOnApproval"]},"turns":[{"turnId":"turn","turnStartedAtMs":1000,"items":[{"type":"agentMessage","id":"before"},{"type":"commandExecution","id":"command","command":"python3 - <<'PY'\nprint('checks')\nPY","status":"inProgress"}]}]});
        let mut live = HashMap::from([(
            "chat".into(),
            Live {
                owner: "owner".into(),
                revision: 1,
                state: Arc::new(state.clone()),
            },
        )]);
        let (published, observed) = tokio::sync::watch::channel(Arc::new(live.clone()));
        let (wanted, _requests) = tokio::sync::watch::channel(HashSet::new());
        let runtime = Runtime {
            socket: Default::default(),
            wanted,
            observed,
            state: RefCell::new(State::default()),
            epoch: 0,
        };
        let ids = HashSet::from(["chat".into()]);
        let waiting = runtime.views(&ids).remove("chat").unwrap();
        assert_eq!(waiting.activity, Activity::Confirmation);
        let mut session = Session {
            agent: "codex",
            id: "chat".into(),
            title: "Caption regression".into(),
            cwd: "/demo".into(),
            workspace: None,
            git_branch: None,
            model: None,
            model_display_name: None,
            reasoning_effort: None,
            updated_at: 0,
            client: Client::VSCode,
            activity: waiting.activity,
            target: None,
            connection: ConnectionState::live(true),
            activities: waiting.activities,
        };
        session.upsert_activity(SessionActivity::Message(SessionMessage {
            id: "rollout:before".into(),
            started_at: millis(2000),
            updated_at: millis(2000),
            role: MessageRole::Agent,
            text: "Running the checks next.".into(),
            images: vec![],
        }));
        let mut active = state;
        active["requests"] = json!([]);
        active["threadRuntimeStatus"]["activeFlags"] = json!([]);
        live.get_mut("chat").unwrap().state = Arc::new(active.clone());
        published.send_replace(Arc::new(live.clone()));
        let view = runtime.views(&ids).remove("chat").unwrap();
        session.activity = view.activity;
        session.observe_activities(view.activities);
        assert_eq!(
            session.work_caption(),
            Some("Running python3 - <<'PY' print('checks') PY")
        );
        assert_eq!(
            session.activities.front().unwrap().started_at(),
            millis(1000),
            "retain the original chronological start time"
        );
        active["turns"][0]["items"][1]["status"] = json!("completed");
        active["turns"][0]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"reasoning","id":"empty","summary":[]}));
        live.get_mut("chat").unwrap().state = Arc::new(active.clone());
        published.send_replace(Arc::new(live.clone()));
        let view = runtime.views(&ids).remove("chat").unwrap();
        session.observe_activities(view.activities);
        assert_eq!(
            session.work_caption(),
            Some("Running python3 - <<'PY' print('checks') PY")
        );
        active["turns"][0]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"agentMessage","id":"after"}));
        live.get_mut("chat").unwrap().state = Arc::new(active);
        published.send_replace(Arc::new(live));
        let view = runtime.views(&ids).remove("chat").unwrap();
        session.observe_activities(view.activities);
        assert_eq!(
            session.progress(),
            None,
            "a new owner message clears the current caption"
        );
    }

    #[test]
    fn owner_wire_patches_retain_the_last_caption_until_the_next_agent_message() {
        fn wire(change: Value) -> Value {
            projection::decode(&serde_json::to_vec(&json!({"params":{"change":change}})).unwrap())
                .unwrap()["params"]["change"]
                .clone()
        }
        let mut live = HashMap::new();
        let snapshot = wire(
            json!({"type":"snapshot","revision":1,"conversationState":{"id":"chat","requests":[],"threadRuntimeStatus":{"type":"active","activeFlags":[]},"turns":[],"turnHistory":{"history":{"entitiesByKey":{"turn:live":{"turnId":"live","turnStartedAtMs":1,"items":[{"type":"commandExecution","id":"command","command":"cargo test --offline","status":"inProgress"}]}}}}}}),
        );
        assert!(apply_change(&mut live, "chat", "owner", &snapshot));
        let base = vec![
            json!("turnHistory"),
            json!("history"),
            json!("entitiesByKey"),
            json!("turn:live"),
            json!("items"),
        ];
        let mut status = base.clone();
        status.extend([json!(0), json!("status")]);
        let mut thought = base.clone();
        thought.push(json!(1));
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &wire(
                json!({"type":"patches","baseRevision":1,"revision":2,"patches":[{"op":"replace","path":status,"value":"completed"},{"op":"add","path":thought,"value":{"id":"thought","type":"reasoning","summary":[],"content":["RAW REASONING"]}}]})
            )
        ));
        assert_eq!(
            super::super::progress::current(&live["chat"].state).as_deref(),
            Some("Running cargo test --offline")
        );
        let mut message = base;
        message.push(json!(2));
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &wire(
                json!({"type":"patches","baseRevision":2,"revision":3,"patches":[{"op":"add","path":message,"value":{"type":"agentMessage","id":"answer","text":"New message"}}]})
            )
        ));
        assert!(super::super::progress::current(&live["chat"].state).is_none());
        assert!(!live["chat"].state.to_string().contains("RAW REASONING"));
    }
    #[test]
    fn stream_versions_are_explicit_for_observation_and_response_validation() {
        for version in [json!(11), json!(13)] {
            let mut message = json!({"method":"thread-stream-state-changed","version":version});
            assert!(supported_stream(&message));
            message["method"] = json!("unrelated-event");
            assert!(!supported_stream(&message));
        }
        for version in [json!(10), json!(12), json!(14), json!("13"), Value::Null] {
            assert!(!supported_stream(
                &json!({"method":"thread-stream-state-changed","version":version})
            ));
        }
    }
    fn snapshot() -> Value {
        json!({"type":"snapshot","revision":4,"conversationState":{"id":"chat","requests":[request()],"threadRuntimeStatus":{"type":"active","activeFlags":["waitingOnApproval"]},"turnHistory":{"ignored":"large history"}}})
    }
    #[test]
    fn native_pending_request_overrides_the_transcript_working_status_and_clears_on_resolution() {
        let mut live = HashMap::new();
        assert!(apply_change(&mut live, "chat", "owner", &snapshot()));
        let state = &live["chat"].state;
        assert_eq!(
            activity(&state["threadRuntimeStatus"]),
            Some(Activity::Confirmation)
        );
        assert_eq!(body(&state["requests"][0]), "Allow file update?");
        assert!(
            state["turnHistory"]["history"]["entitiesByKey"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &json!({"type":"patches","baseRevision":4,"revision":5,"patches":[{"op":"remove","path":["requests",0]},{"op":"replace","path":["threadRuntimeStatus","activeFlags"],"value":[]}]})
        ));
        assert_eq!(
            activity(&live["chat"].state["threadRuntimeStatus"]),
            Some(Activity::Working)
        );
        assert_eq!(live["chat"].state["requests"], json!([]));
    }
    #[test]
    fn fresh_snapshot_clears_a_resolved_approval_even_when_its_patch_was_missed() {
        let mut live = HashMap::new();
        apply_change(&mut live, "chat", "owner", &snapshot());
        let mut resolved = snapshot();
        resolved["revision"] = json!(20);
        resolved["conversationState"]["requests"] = json!([]);
        resolved["conversationState"]["threadRuntimeStatus"] = json!({"type":"idle"});
        assert!(apply_change(&mut live, "chat", "owner", &resolved));
        assert_eq!(live["chat"].state["requests"], json!([]));
        assert_eq!(
            activity(&live["chat"].state["threadRuntimeStatus"]),
            Some(Activity::Idle)
        );
    }
    #[test]
    fn async_answer_turn_does_not_issue_a_new_interaction_ticket() {
        let raw = json!({"id":"chat","requests":[],"threadRuntimeStatus":{"type":"idle"},"turns":[{"turnId":"one","turnStartedAtMs":1,"items":[{"type":"agentMessage","id":"call","questions":[{"title":"Pick?","options":["A","B"]}]}]}]});
        let mut live = HashMap::new();
        apply_change(
            &mut live,
            "chat",
            "owner",
            &json!({"type":"snapshot","revision":1,"conversationState":raw}),
        );
        let (published, observed) = tokio::sync::watch::channel(Arc::new(live.clone()));
        let (wanted, _requests) = tokio::sync::watch::channel(HashSet::new());
        let runtime = Runtime {
            socket: std::path::PathBuf::new(),
            wanted,
            observed,
            state: RefCell::new(State::default()),
            epoch: 0,
        };
        let ids = HashSet::from(["chat".to_owned()]);
        let initial = runtime.views(&ids)["chat"].approval.clone().unwrap();
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &json!({"type":"patches","baseRevision":1,"revision":2,"patches":[
                {"op":"replace","path":["threadRuntimeStatus"],"value":{"type":"active","activeFlags":[]}},
                {"op":"add","path":["turns",1],"value":{"turnId":"two","turnStartedAtMs":2,"items":[]}}
            ]})
        ));
        published.send_replace(Arc::new(live));
        assert_eq!(
            runtime.views(&ids)["chat"].approval.as_ref(),
            Some(&initial)
        );
        assert_eq!(runtime.state.borrow().next_ticket, 1);
    }
    #[test]
    fn async_questions_follow_history_patches_and_old_tickets_fail_after_a_reply() {
        let raw = json!({"id":"chat","requests":[],"threadRuntimeStatus":{"type":"idle"},"turnHistory":{"history":{"entitiesByKey":{"turn:one":{"turnId":"one","turnStartedAtMs":1,"items":[{"type":"userMessage","id":"u"},{"type":"agentMessage","id":"call","questions":null}]}}}}});
        let mut live = HashMap::new();
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &json!({"type":"snapshot","revision":1,"conversationState":raw})
        ));
        assert!(async_questions::pending(&live["chat"].state).is_none());
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &json!({"type":"patches","baseRevision":1,"revision":2,"patches":[{"op":"replace","path":["turnHistory","history","entitiesByKey","turn:one","items",1,"questions"],"value":[{"title":"Pick?","options":["A","B"]}]}]})
        ));
        let request = async_questions::pending(&live["chat"].state).unwrap();
        let ticket = Ticket {
            session: "chat".into(),
            owner: "owner".into(),
            request,
        };
        assert!(exact_request(&live["chat"].state, &ticket));
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &json!({"type":"patches","baseRevision":2,"revision":3,"patches":[{"op":"add","path":["turnHistory","history","entitiesByKey","turn:two"],"value":{"turnId":"two","turnStartedAtMs":2,"items":[{"type":"userMessage","id":"reply","content":[{"type":"text","text":"New input"}]}]}}]})
        ));
        assert!(!exact_request(&live["chat"].state, &ticket));
        assert!(async_questions::pending(&live["chat"].state).is_none());
    }
    #[test]
    fn accepted_vscode_steering_reply_patch_retires_the_async_ticket() {
        let raw = json!({"id":"chat","requests":[],"threadRuntimeStatus":{"type":"active","activeFlags":[]},"turns":[{"turnId":"one","turnStartedAtMs":1,"items":[{"type":"agentMessage","id":"call","questions":[{"title":"Pick?","options":["A","B"]}]}]}]});
        let mut live = HashMap::new();
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &json!({"type":"snapshot","revision":1,"conversationState":raw})
        ));
        let request = async_questions::pending(&live["chat"].state).unwrap();
        let id = request["params"]["questions"][0]["id"].as_str().unwrap();
        let text = format!(
            "<send_user_message_question_reply>{}</send_user_message_question_reply>",
            json!([{"questionItemId":id,"answer":"A"}])
        );
        let ticket = Ticket {
            session: "chat".into(),
            owner: "owner".into(),
            request,
        };
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &json!({"type":"patches","baseRevision":1,"revision":2,"patches":[{"op":"add","path":["turns",0,"items",1],"value":{"type":"steeringUserMessage","id":"reply","status":"pending","input":[{"type":"text","text":text}]}}]})
        ));
        assert!(exact_request(&live["chat"].state, &ticket));
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &json!({"type":"patches","baseRevision":2,"revision":3,"patches":[{"op":"replace","path":["turns",0,"items",1,"status"],"value":"accepted"}]})
        ));
        assert!(!exact_request(&live["chat"].state, &ticket));
        assert!(async_questions::pending(&live["chat"].state).is_none());
    }
    #[test]
    fn stale_revisions_are_discarded_instead_of_reusing_old_approvals() {
        let mut live = HashMap::new();
        apply_change(&mut live, "chat", "owner", &snapshot());
        assert!(!apply_change(
            &mut live,
            "chat",
            "owner",
            &json!({"type":"patches","baseRevision":3,"revision":5,"patches":[]})
        ));
        assert!(!live.contains_key("chat"));
        assert!(!apply_change(
            &mut live,
            "different-thread",
            "owner",
            &snapshot()
        ));
    }
    #[test]
    fn an_allow_ticket_requires_the_same_full_request_and_thread_and_never_accepts_extended_permissions()
     {
        let ticket = Ticket {
            session: "chat".into(),
            owner: "owner".into(),
            request: request(),
        };
        let state = snapshot()["conversationState"].clone();
        assert!(exact_request(&state, &ticket));
        let mut changed = state.clone();
        changed["requests"][0]["params"]["command"] = "different".into();
        assert!(!exact_request(&changed, &ticket));
        changed = state.clone();
        changed["id"] = "other".into();
        assert!(!exact_request(&changed, &ticket));
        assert!(approval_method(&request()).is_some());
        assert!(
            approval_method(&json!({"id":82,"method":"item/permissions/requestApproval"}))
                .is_none()
        );
        assert!(approval_method(&json!({"id":82,"method":"item/tool/requestUserInput"})).is_none());
    }
    #[test]
    fn approval_waits_for_slow_fresh_snapshot_and_resolution_before_reporting_success() {
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        fn read(socket: &mut UnixStream) -> Value {
            let mut header = [0; 4];
            socket.read_exact(&mut header).unwrap();
            let mut bytes = vec![0; u32::from_le_bytes(header) as usize];
            socket.read_exact(&mut bytes).unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }
        fn write(socket: &mut UnixStream, value: Value) {
            let bytes = serde_json::to_vec(&value).unwrap();
            socket
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .unwrap();
            socket.write_all(&bytes).unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ipc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(12)))
                .unwrap();
            let init = read(&mut socket);
            write(
                &mut socket,
                json!({"type":"response","requestId":init["requestId"],"resultType":"success","result":{"clientId":"monitor"}}),
            );
            let discovery = read(&mut socket);
            assert_eq!(discovery["method"], "thread-owner-discovery");
            write(
                &mut socket,
                json!({"type":"response","requestId":discovery["requestId"],"resultType":"success","handledByClientId":"owner"}),
            );
            let follow = read(&mut socket);
            assert_eq!(follow["params"]["following"], true);
            // Exceeds the old two-second cutoff before any permission is sent.
            thread::sleep(Duration::from_millis(2200));
            write(
                &mut socket,
                json!({"type":"broadcast","method":"thread-stream-state-changed","sourceClientId":"owner","version":13,"params":{"hostId":"local","conversationId":"chat","change":snapshot()}}),
            );
            let decision = read(&mut socket);
            assert_eq!(
                decision["method"],
                "thread-follower-command-approval-decision"
            );
            assert_eq!(decision["targetClientId"], "owner");
            assert_eq!(decision["params"]["requestId"], 82);
            assert_eq!(decision["params"]["decision"], "accept");
            write(
                &mut socket,
                json!({"type":"response","requestId":decision["requestId"],"resultType":"success","result":{"ok":true}}),
            );
            let follow = read(&mut socket);
            assert_eq!(follow["params"]["following"], true);
            // An RPC acknowledgement is insufficient; wait for owner evidence.
            thread::sleep(Duration::from_millis(2200));
            let mut resolved = snapshot();
            resolved["revision"] = 5.into();
            resolved["conversationState"]["requests"] = json!([]);
            write(
                &mut socket,
                json!({"type":"broadcast","method":"thread-stream-state-changed","sourceClientId":"owner","version":13,"params":{"hostId":"local","conversationId":"chat","change":resolved}}),
            );
        });
        let (wanted, _requests) = tokio::sync::watch::channel(HashSet::new());
        let (_published, observed) = tokio::sync::watch::channel(Arc::new(HashMap::new()));
        let runtime = Runtime {
            socket: path,
            wanted,
            observed,
            state: RefCell::new(State::default()),
            epoch: 0,
        };
        runtime.state.borrow_mut().tickets.insert(
            "ticket".into(),
            Ticket {
                session: "chat".into(),
                owner: "owner".into(),
                request: request(),
            },
        );
        let mut attempted = false;
        let start = Instant::now();
        assert!(
            runtime
                .respond_inner("ticket", &Response::Action("allow".into()), &mut attempted)
                .unwrap()
        );
        assert!(attempted);
        assert!(start.elapsed() >= Duration::from_millis(4400));
        server.join().unwrap();
    }
    #[test]
    fn reconnect_preserves_ticket_identity_without_authorizing_stale_state() {
        let (wanted, _requests) = tokio::sync::watch::channel(HashSet::new());
        let (published, observed) = tokio::sync::watch::channel(Arc::new(HashMap::new()));
        let runtime = Runtime {
            socket: std::path::PathBuf::new(),
            wanted,
            observed,
            state: RefCell::new(State::default()),
            epoch: 0,
        };
        let ids = HashSet::from(["chat".to_owned()]);
        let mut live = HashMap::new();
        assert!(apply_change(&mut live, "chat", "owner", &snapshot()));
        published.send_replace(Arc::new(live.clone()));
        let token = runtime.views(&ids)["chat"]
            .approval
            .as_ref()
            .unwrap()
            .id
            .clone();
        published.send_replace(Arc::new(HashMap::new()));
        assert!(runtime.views(&ids).is_empty());
        published.send_replace(Arc::new(live.clone()));
        assert_eq!(
            runtime.views(&ids)["chat"].approval.as_ref().unwrap().id,
            token
        );
        // A changed command must acquire a new identity, even with the same wire id.
        Arc::make_mut(&mut live.get_mut("chat").unwrap().state)["requests"][0]["params"]["command"] =
            "changed".into();
        published.send_replace(Arc::new(live.clone()));
        assert_ne!(
            runtime.views(&ids)["chat"].approval.as_ref().unwrap().id,
            token
        );
        Arc::make_mut(&mut live.get_mut("chat").unwrap().state)["requests"] = json!([]);
        published.send_replace(Arc::new(live));
        runtime.views(&ids);
        assert!(runtime.state.borrow().tickets.is_empty());
    }
    #[test]
    fn observer_streams_image_heavy_frames_above_outgoing_limit() {
        let (socket, mut peer) = UnixStream::pair().unwrap();
        let prefix = br#"{"method":"thread-stream-state-changed","params":{"change":{"type":"snapshot","conversationState":{"id":"chat","requests":[],"turns":[{"items":[{"type":"tool","output":""#;
        let suffix = br#""}]}]}}}}"#;
        let image_size = 33 * 1024 * 1024;
        let length = prefix.len() + image_size + suffix.len();
        assert!(length > MAX_REQUEST && length < MAX_FRAME);
        let writer = thread::spawn(move || {
            peer.write_all(&(length as u32).to_le_bytes()).unwrap();
            peer.write_all(prefix).unwrap();
            let chunk = [b'x'; 65536];
            for _ in 0..image_size / chunk.len() {
                peer.write_all(&chunk).unwrap();
            }
            peer.write_all(suffix).unwrap();
        });
        let mut connection = Connection::from_socket(socket).unwrap();
        let value = connection.read().unwrap().unwrap();
        writer.join().unwrap();
        assert!(serde_json::to_vec(&value).unwrap().len() < 1024);
        assert_eq!(connection.input.capacity(), 65536);
        let oversized = json!({"image":"x".repeat(MAX_REQUEST)});
        assert!(
            connection.send(oversized).is_err(),
            "outgoing limit must remain unchanged"
        );
    }
    #[test]
    fn framed_reader_handles_fragmented_headers_and_rejects_oversized_messages() {
        let (socket, mut peer) = UnixStream::pair().unwrap();
        let mut c = Connection::from_socket(socket).unwrap();
        let value = json!({"type":"response","requestId":"test"});
        let data = serde_json::to_vec(&value).unwrap();
        let header = (data.len() as u32).to_le_bytes();
        peer.write_all(&header[..2]).unwrap();
        assert!(c.read().unwrap().is_none());
        peer.write_all(&header[2..]).unwrap();
        peer.write_all(&data).unwrap();
        assert_eq!(c.read().unwrap(), Some(value));
        peer.write_all(&((MAX_FRAME + 1) as u32).to_le_bytes())
            .unwrap();
        assert!(c.read().is_err());
    }

    #[test]
    fn streamed_frames_retry_short_timeouts_and_leave_the_next_frame_intact() {
        let (socket, mut peer) = UnixStream::pair().unwrap();
        let mut connection = Connection::from_socket(socket).unwrap();
        let first = json!({"type":"response","requestId":"one","result":{"ok":true}});
        let second = json!({"type":"response","requestId":"two"});
        let a = serde_json::to_vec(&first).unwrap();
        let b = serde_json::to_vec(&second).unwrap();
        let writer = thread::spawn(move || {
            peer.write_all(&(a.len() as u32).to_le_bytes()).unwrap();
            peer.write_all(&a[..3]).unwrap();
            thread::sleep(Duration::from_millis(150)); // exceeds socket timeout
            peer.write_all(&a[3..]).unwrap();
            peer.write_all(&(b.len() as u32).to_le_bytes()).unwrap();
            peer.write_all(&b).unwrap();
        });
        assert_eq!(connection.read().unwrap(), Some(first));
        assert_eq!(connection.read().unwrap(), Some(second));
        writer.join().unwrap();
    }

    #[test]
    fn malformed_truncated_and_stalled_streams_cannot_be_reused() {
        for (body, extra) in [(b"{}".as_slice(), 1), (b"{invalid}".as_slice(), 0)] {
            let (socket, mut peer) = UnixStream::pair().unwrap();
            let mut connection = Connection::from_socket(socket).unwrap();
            peer.write_all(&((body.len() + extra) as u32).to_le_bytes())
                .unwrap();
            peer.write_all(body).unwrap();
            drop(peer);
            assert!(connection.read().is_err());
            assert!(connection.failed);
            assert!(
                connection
                    .read()
                    .unwrap_err()
                    .to_string()
                    .contains("unusable")
            );
            assert!(connection.send(json!({})).is_err());
        }
        struct Stalled;
        impl Read for Stalled {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::WouldBlock))
            }
        }
        let mut frame = FrameRead {
            input: Stalled,
            deadline: Instant::now() + Duration::from_millis(10),
        };
        assert_eq!(
            frame.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        let (socket, mut peer) = UnixStream::pair().unwrap();
        let mut connection = Connection::from_socket(socket).unwrap();
        peer.write_all(&[5]).unwrap();
        assert!(connection.read().unwrap().is_none());
        connection.header_started = Some(Instant::now() - SNAPSHOT_TIMEOUT);
        assert!(connection.read().is_err());
        assert!(connection.failed);
    }
    #[test]
    fn an_rpc_acknowledgement_is_not_resolution_until_the_exact_request_is_gone() {
        let request = json!({"id":42,"method":"item/commandExecution/requestApproval","params":{"threadId":"chat"}});
        let ticket = Ticket {
            session: "chat".into(),
            owner: "owner".into(),
            request: request.clone(),
        };
        let mut live = HashMap::new();
        assert!(!Runtime::request_resolved(&live, &ticket));
        let pending = json!({"type":"snapshot","revision":1,"conversationState":{"id":"chat","requests":[request],"threadRuntimeStatus":{"type":"active"}}});
        assert!(apply_change(&mut live, "chat", "owner", &pending));
        assert!(!Runtime::request_resolved(&live, &ticket));
        let removed = json!({"type":"patches","baseRevision":1,"revision":2,"patches":[{"op":"remove","path":["requests",0]}]});
        assert!(apply_change(&mut live, "chat", "owner", &removed));
        assert!(Runtime::request_resolved(&live, &ticket));
        live.get_mut("chat").unwrap().owner = "other owner".into();
        assert!(!Runtime::request_resolved(&live, &ticket));
    }

    #[test]
    fn only_missing_or_timed_out_snapshots_are_retried() {
        let now = Instant::now();
        assert!(needs_snapshot(false, None, now));
        assert!(!needs_snapshot(false, Some(now), now));
        assert!(needs_snapshot(false, Some(now - SNAPSHOT_TIMEOUT), now));
        assert!(!needs_snapshot(true, None, now));
        assert!(!needs_snapshot(true, Some(now - SNAPSHOT_TIMEOUT), now));
    }

    #[test]
    fn projected_frames_preserve_async_replies_and_indices_through_ignored_deltas() {
        fn decoded(change: Value) -> Value {
            let frame = projection::decode(&serde_json::to_vec(&json!({"type":"broadcast","method":"thread-stream-state-changed","params":{"change":change}})).unwrap()).unwrap();
            frame["params"]["change"].clone()
        }
        let raw = json!({"id":"chat","requests":[],"threadRuntimeStatus":{"type":"idle"},"turns":[{"turnId":"one","turnStartedAtMs":1,"items":[
            {"type":"tool","output":"discarded tool text","content":[{"type":"image","url":"discarded image"}]},
            {"type":"agentMessage","id":"call","text":"discarded prose","questions":[{"title":"First?","options":["A","B"]},{"title":"Second?","options":["C","D"]}]}
        ]}]});
        let mut live = HashMap::new();
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &decoded(json!({"type":"snapshot","revision":1,"conversationState":raw}))
        ));
        let request = async_questions::pending(&live["chat"].state).unwrap();
        let questions = request["params"]["questions"].as_array().unwrap();
        assert_eq!(questions.len(), 2);
        // An ignored tool delta still advances the revision; item1 remains item1.
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &decoded(
                json!({"type":"patches","baseRevision":1,"revision":2,"patches":[{"op":"replace","path":["turns",0,"items",0,"output"],"value":"more tool text"}]})
            )
        ));
        assert_eq!(
            async_questions::pending(&live["chat"].state),
            Some(request.clone())
        );
        let reply = format!(
            "<send_user_message_question_reply>{}</send_user_message_question_reply>",
            json!([{"questionItemId":questions[0]["id"],"answer":"A"}])
        );
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &decoded(
                json!({"type":"patches","baseRevision":2,"revision":3,"patches":[{"op":"add","path":["turns",0,"items",2],"value":{"type":"steeringUserMessage","id":"reply","status":"pending","input":[{"type":"image","url":"discarded image"},{"type":"text","text":reply}]}}]})
            )
        ));
        assert_eq!(async_questions::pending(&live["chat"].state), Some(request));
        assert!(apply_change(
            &mut live,
            "chat",
            "owner",
            &decoded(
                json!({"type":"patches","baseRevision":3,"revision":4,"patches":[{"op":"replace","path":["turns",0,"items",2,"status"],"value":"accepted"}]})
            )
        ));
        let remaining = async_questions::pending(&live["chat"].state).unwrap();
        assert_eq!(
            remaining["params"]["questions"].as_array().unwrap().len(),
            1
        );
        assert_eq!(remaining["params"]["questions"][0]["question"], "Second?");
        let state = serde_json::to_string(live["chat"].state.as_ref()).unwrap();
        assert!(!state.contains("discarded"));
        assert!(!state.contains("more tool text"));
    }

    #[test]
    fn a_silent_owner_does_not_delay_other_subscriptions_or_request_patches() {
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        fn read(stream: &mut UnixStream) -> Value {
            let mut size = [0; 4];
            stream.read_exact(&mut size).unwrap();
            let mut bytes = vec![0; u32::from_le_bytes(size) as usize];
            stream.read_exact(&mut bytes).unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }
        fn write(stream: &mut UnixStream, value: Value) {
            let bytes = serde_json::to_vec(&value).unwrap();
            stream
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .unwrap();
            stream.write_all(&bytes).unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ipc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let (wanted, mut requests) =
            tokio::sync::watch::channel(HashSet::from(["silent".into(), "healthy".into()]));
        let (published, observed) = tokio::sync::watch::channel(Arc::new(HashMap::new()));
        let worker = thread::spawn(move || {
            watch(
                &mut requests,
                &published,
                &path,
                &tokio::sync::Notify::new(),
            )
        });
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let init = read(&mut socket);
        write(
            &mut socket,
            json!({"type":"response","requestId":init["requestId"],"resultType":"success","result":{"clientId":"observer"}}),
        );
        for _ in 0..2 {
            let request = read(&mut socket);
            assert_eq!(request["method"], "thread-owner-discovery");
            if request["params"]["conversationId"] == "healthy" {
                write(
                    &mut socket,
                    json!({"type":"response","requestId":request["requestId"],"resultType":"success","handledByClientId":"owner"}),
                );
            }
        }
        let follow = read(&mut socket);
        assert_eq!(follow["method"], "thread-stream-following-changed");
        assert_eq!(follow["params"]["conversationId"], "healthy");
        let mut snapshot = snapshot();
        snapshot["conversationState"]["id"] = json!("healthy");
        write(
            &mut socket,
            json!({"type":"broadcast","method":"thread-stream-state-changed","version":13,"sourceClientId":"owner","params":{"conversationId":"healthy","hostId":"local","change":snapshot}}),
        );
        write(
            &mut socket,
            json!({"type":"broadcast","method":"thread-stream-state-changed","version":13,"sourceClientId":"owner","params":{"conversationId":"healthy","hostId":"local","change":{"type":"patches","baseRevision":4,"revision":5,"patches":[{"op":"replace","path":["requests",0,"params","reason"],"value":"Latest approval"}]}}}),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while !observed
            .borrow()
            .get("healthy")
            .is_some_and(|s| s.revision == 5)
        {
            assert!(
                Instant::now() < deadline,
                "silent discovery blocked the healthy request stream"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!observed.borrow().contains_key("silent"));
        drop(wanted);
        worker.join().unwrap().unwrap();
    }
    #[test]
    fn subscription_uses_patches_and_resynchronizes_gaps_resets_and_owner_changes() {
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        fn read(stream: &mut UnixStream) -> Value {
            let mut size = [0; 4];
            stream.read_exact(&mut size).unwrap();
            let mut bytes = vec![0; u32::from_le_bytes(size) as usize];
            stream.read_exact(&mut bytes).unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }
        fn write(stream: &mut UnixStream, value: Value) {
            let bytes = serde_json::to_vec(&value).unwrap();
            stream
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .unwrap();
            stream.write_all(&bytes).unwrap();
        }
        fn discover(stream: &mut UnixStream, owner: &str) {
            let request = read(stream);
            assert_eq!(request["method"], "thread-owner-discovery");
            write(
                stream,
                json!({"type":"response","requestId":request["requestId"],"resultType":"success","handledByClientId":owner}),
            );
        }
        fn follow(stream: &mut UnixStream, owner: &str) {
            let request = read(stream);
            assert_eq!(request["method"], "thread-stream-following-changed");
            assert_eq!(request["params"]["following"], true);
            assert_eq!(request["targetClientIds"], json!([owner]));
        }
        fn state(stream: &mut UnixStream, owner: &str, change: Value) {
            write(
                stream,
                json!({"type":"broadcast","method":"thread-stream-state-changed","version":STREAM_VERSION,"sourceClientId":owner,"params":{"conversationId":"chat","hostId":"local","change":change}}),
            );
        }
        fn wait_revision(
            observed: &tokio::sync::watch::Receiver<Arc<HashMap<String, Live>>>,
            revision: u64,
            owner: &str,
        ) {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if observed
                    .borrow()
                    .get("chat")
                    .is_some_and(|live| live.revision == revision && live.owner == owner)
                {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "observer did not reach {owner}:{revision}"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ipc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let (wanted, mut requests) = tokio::sync::watch::channel(HashSet::from(["chat".into()]));
        let (published, observed) = tokio::sync::watch::channel(Arc::new(HashMap::new()));
        let worker = thread::spawn(move || {
            watch(
                &mut requests,
                &published,
                &path,
                &tokio::sync::Notify::new(),
            )
        });
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(7)))
            .unwrap();
        let init = read(&mut socket);
        write(
            &mut socket,
            json!({"type":"response","requestId":init["requestId"],"resultType":"success","result":{"clientId":"observer"}}),
        );
        discover(&mut socket, "owner");
        follow(&mut socket, "owner");
        state(&mut socket, "owner", snapshot());
        wait_revision(&observed, 4, "owner");

        // Lightweight periodic owner discovery must NOT reaffirm a healthy
        // subscription or trigger another large snapshot, even with a request.
        discover(&mut socket, "owner");
        socket
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        assert!(
            matches!(socket.read(&mut [0u8; 1]), Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
        );
        socket
            .set_read_timeout(Some(Duration::from_secs(7)))
            .unwrap();
        state(
            &mut socket,
            "owner",
            json!({"type":"patches","baseRevision":4,"revision":5,"patches":[{"op":"remove","path":["requests",0]}]}),
        );
        wait_revision(&observed, 5, "owner");
        assert!(
            observed.borrow()["chat"].state["requests"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        // An undelivered revision invalidates the cached permission and requests
        // exactly a replacement snapshot, rather than guessing a patch result.
        state(
            &mut socket,
            "owner",
            json!({"type":"patches","baseRevision":6,"revision":7,"patches":[]}),
        );
        discover(&mut socket, "owner");
        follow(&mut socket, "owner");
        let mut resolved = snapshot();
        resolved["revision"] = json!(8);
        resolved["conversationState"]["requests"] = json!([]);
        state(&mut socket, "owner", resolved.clone());
        wait_revision(&observed, 8, "owner");

        // Owner-side subscription recovery is explicit, not a timer snapshot.
        write(
            &mut socket,
            json!({"type":"broadcast","method":"thread-stream-following-status-requested","sourceClientId":"owner","params":{"conversationId":"chat","hostId":"local"}}),
        );
        follow(&mut socket, "owner");
        state(&mut socket, "owner", resolved.clone());

        write(
            &mut socket,
            json!({"type":"broadcast","method":"client-status-changed","params":{"status":"disconnected","clientId":"owner"}}),
        );
        discover(&mut socket, "new-owner");
        follow(&mut socket, "new-owner");
        state(&mut socket, "owner", snapshot()); // stale owner cannot revive it
        state(&mut socket, "new-owner", resolved.clone());
        wait_revision(&observed, 8, "new-owner");
        assert!(
            observed.borrow()["chat"].state["requests"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        write(
            &mut socket,
            json!({"type":"broadcast","method":"ipc-connection-reset"}),
        );
        discover(&mut socket, "new-owner");
        follow(&mut socket, "new-owner");
        state(&mut socket, "new-owner", resolved);
        wait_revision(&observed, 8, "new-owner");
        wanted.send_replace(HashSet::new());
        worker.join().unwrap().unwrap();
    }
}
