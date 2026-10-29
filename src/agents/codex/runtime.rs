//! Observe the existing owner without resuming threads. Explicit replies are forwarded
//! to that owner; async question replies use its normal user-message admission.
use super::{async_questions, requests};
use crate::agents::interaction::{Interaction, Response};
use crate::agents::sessions::{Activity, Approval};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    sync::{Condvar, Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const STREAM_VERSION: u64 = 11;
const MAX_FRAME: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug)]
struct Live {
    owner: String,
    revision: u64,
    state: Value,
}
#[derive(Clone)]
struct Ticket {
    session: String,
    owner: String,
    request: Value,
}
#[derive(Default)]
struct State {
    wanted: HashSet<String>,
    live: HashMap<String, Live>,
    tickets: HashMap<String, Ticket>,
    next_ticket: u64,
}
struct Observer {
    state: Mutex<State>,
    ready: Condvar,
    epoch: u128,
}
static OBSERVER: OnceLock<Observer> = OnceLock::new();
fn observer() -> &'static Observer {
    OBSERVER.get_or_init(|| {
        thread::spawn(|| {
            // OnceLock is published before the worker reads it.
            loop {
                if let Some(observer) = OBSERVER.get() {
                    if watch(observer).is_err() {
                        let mut state = observer.state.lock().unwrap();
                        state.live.clear();
                        state.tickets.clear();
                        observer.ready.notify_all();
                    }
                    thread::sleep(Duration::from_secs(2));
                } else {
                    thread::yield_now();
                }
            }
        });
        Observer {
            state: Mutex::new(State::default()),
            ready: Condvar::new(),
            epoch: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        }
    })
}
#[derive(Clone)]
pub struct View {
    pub activity: Activity,
    pub approval: Option<Approval>,
}

pub fn views(ids: &HashSet<String>) -> HashMap<String, View> {
    if ids.is_empty() && OBSERVER.get().is_none() {
        return HashMap::new();
    }
    let observer = observer();
    let mut state = observer.state.lock().unwrap();
    state.wanted = ids.clone();
    state.live.retain(|id, _| ids.contains(id));
    if ids.is_empty() {
        state.tickets.clear();
        return HashMap::new();
    }
    if !ids.iter().any(|id| state.live.contains_key(id)) {
        state = observer
            .ready
            .wait_timeout_while(state, Duration::from_millis(500), |state| {
                ids.iter().any(|id| !state.live.contains_key(id))
            })
            .unwrap()
            .0;
    }
    // A token is only an opaque reference to an exact request in this process.
    let current = state.live.clone();
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
                .find(|(_, t)| t.session == *id && t.owner == live.owner && t.request == *request)
                .map(|(token, _)| token.clone())
                .unwrap_or_else(|| {
                    state.next_ticket += 1;
                    let token = format!(
                        "ipc:{}:{}:{}",
                        std::process::id(),
                        observer.epoch,
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
pub fn pending_count(exclude: &HashSet<String>) -> usize {
    let Some(observer) = OBSERVER.get() else {
        return 0;
    };
    observer
        .state
        .lock()
        .unwrap()
        .live
        .iter()
        .filter(|(id, s)| {
            !exclude.contains(*id)
                && (activity(&s.state["threadRuntimeStatus"]) == Some(Activity::Confirmation)
                    || async_questions::pending(&s.state).is_some())
        })
        .count()
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
pub fn allow(token: &str) -> bool {
    respond_inner(token, &Response::Action("allow".into())).unwrap_or(false)
}
pub fn allow_for(session: &crate::agents::sessions::Session) -> bool {
    let Some(approval) = &session.approval else {
        return false;
    };
    let Some(observer) = OBSERVER.get() else {
        return false;
    };
    let valid = observer
        .state
        .lock()
        .unwrap()
        .tickets
        .get(&approval.id)
        .is_some_and(|ticket| {
            ticket.session == session.id
                && body(&ticket.request) == approval.body
                && approval.can_allow
        });
    valid && allow(&approval.id)
}
pub fn interaction_for(session: &crate::agents::sessions::Session) -> Interaction {
    let Some(approval) = &session.approval else {
        return Interaction::default();
    };
    let Some(observer) = OBSERVER.get() else {
        return Interaction::default();
    };
    observer
        .state
        .lock()
        .unwrap()
        .tickets
        .get(&approval.id)
        .filter(|ticket| ticket.session == session.id && body(&ticket.request) == approval.body)
        .map(|ticket| requests::interaction(&ticket.request))
        .unwrap_or_default()
}
pub fn respond_for(session: &crate::agents::sessions::Session, response: &Response) -> bool {
    let Some(approval) = &session.approval else {
        return false;
    };
    let Some(observer) = OBSERVER.get() else {
        return false;
    };
    let valid = observer
        .state
        .lock()
        .unwrap()
        .tickets
        .get(&approval.id)
        .is_some_and(|ticket| {
            ticket.session == session.id && body(&ticket.request) == approval.body
        });
    valid && respond_inner(&approval.id, response).unwrap_or(false)
}
fn respond_inner(token: &str, response: &Response) -> Result<bool> {
    let Some(observer) = OBSERVER.get() else {
        return Ok(false);
    };
    let Some(ticket) = observer.state.lock().unwrap().tickets.get(token).cloned() else {
        return Ok(false);
    };
    let asynchronous = ticket.request["method"] == async_questions::METHOD;
    let wire = requests::wire_response(&ticket.request, response);
    let async_text = if asynchronous {
        async_questions::reply_text(&ticket.request, response)
    } else {
        None
    };
    if wire.is_none() && async_text.is_none() {
        return Ok(false);
    }
    let mut connection = Connection::connect()?;
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
            let Some((method, version, params)) =
                async_questions::reply_envelope(&change["conversationState"], &text, &client_id)
            else {
                return Ok(false);
            };
            let result = connection.request(method, params, version, Some(&owner))?;
            return Ok(result["resultType"] == "success");
        }
        let Some((method, field, value)) = wire else {
            return Ok(false);
        };
        let id = &ticket.request["id"];
        let request_id = id
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| id.to_string());
        let mut params = json!({"conversationId":ticket.session,"requestId":request_id});
        params[field] = value;
        let result = connection.request(method, params, 1, Some(&owner))?;
        return Ok(result["resultType"] == "success" && result["result"]["ok"] == true);
    }
    Ok(false)
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
    fn connect() -> Result<Self> {
        let path = crate::app::core::home().join(".codex/ipc/ipc.sock");
        let meta = fs::symlink_metadata(&path)?;
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
fn watch(observer: &Observer) -> Result<()> {
    let mut connection = Connection::connect()?;
    let mut owners: HashMap<String, String> = HashMap::new();
    let mut checked = Instant::now() - Duration::from_secs(6);
    let mut last_wanted = HashSet::new();
    loop {
        let wanted = observer.state.lock().unwrap().wanted.clone();
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
                observer.state.lock().unwrap().live.remove(&session);
            }
            for session in wanted {
                if let Ok(owner) = connection.owner(&session) {
                    if owners.get(&session) != Some(&owner) {
                        if let Some(old) = owners.insert(session.clone(), owner.clone()) {
                            connection.follow(&session, &old, false)?;
                        }
                        observer.state.lock().unwrap().live.remove(&session);
                    }
                    // Reaffirming a subscription also requests a fresh snapshot.
                    // A missed patch or an owner-side subscription reset must not
                    // leave a resolved approval cached indefinitely.
                    connection.follow(&session, &owner, true)?;
                } else {
                    owners.remove(&session);
                    observer.state.lock().unwrap().live.remove(&session);
                }
            }
            checked = Instant::now();
        }
        let Some(message) = connection.next()? else {
            continue;
        };
        if message["method"] == "ipc-connection-reset" {
            owners.clear();
            let mut state = observer.state.lock().unwrap();
            state.live.clear();
            state.tickets.clear();
            observer.ready.notify_all();
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
                observer
                    .state
                    .lock()
                    .unwrap()
                    .live
                    .retain(|_, s| s.owner != client);
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
            let mut state = observer.state.lock().unwrap();
            if !state.wanted.contains(session) {
                continue;
            }
            if !apply_change(
                &mut state.live,
                session,
                owner,
                &message["params"]["change"],
            ) {
                checked = Instant::now() - Duration::from_secs(6);
            }
            observer.ready.notify_all();
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
                state: async_questions::project(state),
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
            &mut current.state,
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
}
