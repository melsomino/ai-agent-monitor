//! Observe the existing owner without resuming threads. Explicit replies are forwarded
//! to that owner; async question replies use its normal user-message admission.
use super::{async_questions, requests};
use crate::agents::interaction::{Interaction, Response};
use crate::agents::sessions::{Activity, Approval};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const STREAM_VERSION: u64 = 11;
const MAX_FRAME: usize = 32 * 1024 * 1024;
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
                if watch(&mut requests, &published, &watched_socket).is_err() {
                    published.send_replace(Arc::new(HashMap::new()));
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
    pub approval: Option<Approval>,
}

impl Runtime {
    pub fn views(&self, ids: &HashSet<String>) -> HashMap<String, View> {
        if *self.wanted.borrow() != *ids {
            self.wanted.send_replace(ids.clone());
        }
        let current = self.observed.borrow().clone();
        let mut state = self.state.borrow_mut();
        state.tickets.retain(|_, t| {
            current.get(&t.session).is_some_and(|s| {
                s.owner == t.owner
                    && s.state["requests"]
                        .as_array()
                        .is_some_and(|requests| requests.contains(&t.request))
                    || (s.owner == t.owner
                        && async_questions::pending(&s.state).as_ref() == Some(&t.request))
            })
        });
        let mut result = HashMap::new();
        for id in ids {
            let Some(live) = current.get(id) else {
                continue;
            };
            let Some(activity) = activity(&live.state["threadRuntimeStatus"]) else {
                continue;
            };
            let async_request = async_questions::pending(&live.state);
            let request = live.state["requests"]
                .as_array()
                .and_then(|requests| {
                    requests.iter().find(|r| {
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
                })
                .or(async_request.as_ref());
            let approval = if let Some(request) = request {
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
                Some(Approval {
                    id: token,
                    body: body(request),
                    expires: u64::MAX,
                    can_allow: requests::interaction(request)
                        .actions
                        .iter()
                        .any(|action| action.id == "allow"),
                })
            } else if activity == Activity::Confirmation {
                Some(Approval {
                    id: format!("ipc-view:{id}"),
                    body: "Codex is waiting for your confirmation or input.".into(),
                    expires: u64::MAX,
                    can_allow: false,
                })
            } else {
                None
            };
            result.insert(
                id.clone(),
                View {
                    activity: if approval.is_some() {
                        Activity::Confirmation
                    } else {
                        activity
                    },
                    approval,
                },
            );
        }
        result
    }
}
fn activity(status: &Value) -> Option<Activity> {
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
fn body(request: &Value) -> String {
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
        let Some(approval) = &session.approval else {
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
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                let Some(message) = connection.next()? else {
                    continue;
                };
                if message["method"] != "thread-stream-state-changed"
                    || message["sourceClientId"] != owner
                    || message["version"] != STREAM_VERSION
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
        let Some(approval) = &session.approval else {
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
        let Some(approval) = &session.approval else {
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
        let Some(approval) = &session.approval else {
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
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let Some(message) = connection.next()? else {
                continue;
            };
            if message["method"] != "thread-stream-state-changed"
                || message["sourceClientId"] != owner
                || message["version"] != STREAM_VERSION
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
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                let Some(message) = connection.next()? else {
                    continue;
                };
                if message["method"] != "thread-stream-state-changed"
                    || message["sourceClientId"] != owner
                    || message["version"] != STREAM_VERSION
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
fn client_message_id() -> Result<String> {
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
    bytes: Vec<u8>,
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
        let socket = UnixStream::connect(path)?;
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        socket.set_write_timeout(Some(Duration::from_millis(250)))?;
        let mut connection = Self {
            socket,
            bytes: Vec::new(),
            queued: VecDeque::new(),
            client: format!("agent-monitor-{}", std::process::id()),
            next_id: 0,
            follows: HashMap::new(),
        };
        let reply =
            connection.request("initialize", json!({"clientType":"agent-monitor"}), 0, None)?;
        connection.client = reply["result"]["clientId"]
            .as_str()
            .context("IPC initialization failed")?
            .into();
        Ok(connection)
    }
    fn send(&mut self, value: Value) -> Result<()> {
        let data = serde_json::to_vec(&value)?;
        anyhow::ensure!(data.len() <= MAX_FRAME, "IPC message too large");
        self.socket.write_all(&(data.len() as u32).to_le_bytes())?;
        self.socket.write_all(&data)?;
        Ok(())
    }
    fn read(&mut self) -> Result<Option<Value>> {
        if self.bytes.len() >= 4 {
            let length = u32::from_le_bytes(self.bytes[..4].try_into().unwrap()) as usize;
            anyhow::ensure!(length <= MAX_FRAME, "IPC snapshot too large");
            if self.bytes.len() >= length + 4 {
                let value = serde_json::from_slice(&self.bytes[4..length + 4])?;
                self.bytes.drain(..length + 4);
                return Ok(Some(value));
            }
        }
        let mut data = [0u8; 65536];
        match self.socket.read(&mut data) {
            Ok(0) => anyhow::bail!("IPC disconnected"),
            Ok(size) => {
                self.bytes.extend_from_slice(&data[..size]);
                Ok(None)
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
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
    fn request(
        &mut self,
        method: &str,
        params: Value,
        version: u64,
        target: Option<&str>,
    ) -> Result<Value> {
        self.next_id += 1;
        let id = format!("aim-{}", self.next_id);
        let timeout = if matches!(
            method,
            "thread-follower-start-turn" | "thread-follower-steer-turn"
        ) {
            8000
        } else {
            1000
        };
        let mut message = json!({"type":"request","requestId":id,"sourceClientId":self.client,"method":method,"version":version,"params":params,"timeoutMs":timeout});
        if let Some(target) = target {
            message["targetClientId"] = target.into();
        }
        self.send(message)?;
        let deadline = Instant::now() + Duration::from_millis(timeout + 500);
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
) -> Result<()> {
    let mut connection = Connection::connect(socket)?;
    let mut live = HashMap::<String, Live>::new();
    let mut owners: HashMap<String, String> = HashMap::new();
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
            }
            for session in wanted {
                if let Ok(owner) = connection.owner(&session) {
                    if owners.get(&session) != Some(&owner) {
                        if let Some(old) = owners.insert(session.clone(), owner.clone()) {
                            connection.follow(&session, &old, false)?;
                        }
                        live.remove(&session);
                    }
                    // Reaffirming a subscription also requests a fresh snapshot.
                    // A missed patch or an owner-side subscription reset must not
                    // leave a resolved approval cached indefinitely.
                    connection.follow(&session, &owner, true)?;
                } else {
                    owners.remove(&session);
                    live.remove(&session);
                }
            }
            checked = Instant::now();
        }
        published.send_replace(Arc::new(live.clone()));
        let Some(message) = connection.next()? else {
            continue;
        };
        if message["method"] == "ipc-connection-reset" {
            owners.clear();
            live.clear();
            checked = Instant::now() - Duration::from_secs(6);
        } else if message["method"] == "thread-stream-following-status-requested" {
            if let Some(session) = message["params"]["conversationId"].as_str()
                && message["params"]["hostId"] == "local"
                && let Some(owner) = owners.get(session)
                && message["sourceClientId"] == *owner
            {
                connection.follow(session, owner, true)?;
            }
        } else if message["method"] == "client-status-changed"
            && message["params"]["status"] == "disconnected"
        {
            if let Some(client) = message["params"]["clientId"].as_str() {
                owners.retain(|_, owner| owner != client);
                live.retain(|_, s| s.owner != client);
                checked = Instant::now() - Duration::from_secs(6);
            }
        } else if message["method"] == "thread-stream-state-changed"
            && message["version"] == STREAM_VERSION
        {
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
                checked = Instant::now() - Duration::from_secs(6);
            }
            published.send_replace(Arc::new(live.clone()));
        }
    }
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
        for (status, outcome) in [
            ("idle", "success"),
            ("active", "success"),
            ("active", "error"),
            ("idle", "disconnect"),
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
                    json!({"type":"broadcast","method":"thread-stream-state-changed","sourceClientId":"owner","version":STREAM_VERSION,"params":{"hostId":"local","conversationId":"chat","change":{"type":"snapshot","revision":1,"conversationState":{"id":"chat","requests":[],"threadRuntimeStatus":{"type":status}}}}}),
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
    fn framed_reader_handles_fragmented_headers_and_rejects_oversized_messages() {
        let (socket, mut peer) = UnixStream::pair().unwrap();
        let mut c = Connection {
            socket,
            bytes: Vec::new(),
            queued: VecDeque::new(),
            client: "test".into(),
            next_id: 0,
            follows: HashMap::new(),
        };
        let value = json!({"type":"response","requestId":"test"});
        let data = serde_json::to_vec(&value).unwrap();
        let header = (data.len() as u32).to_le_bytes();
        peer.write_all(&header[..2]).unwrap();
        assert!(c.read().unwrap().is_none());
        peer.write_all(&header[2..]).unwrap();
        peer.write_all(&data).unwrap();
        assert!(c.read().unwrap().is_none());
        assert_eq!(c.read().unwrap(), Some(value));
        c.bytes = ((MAX_FRAME + 1) as u32).to_le_bytes().to_vec();
        assert!(c.read().is_err());
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
}
