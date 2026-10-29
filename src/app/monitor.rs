//! Resident-owned requests. Window, forms and notifications share `respond`.
use crate::{
    agents::{
        self,
        interaction::Response,
        sessions::{Activity, Approval, Client, Session},
    },
    app::{
        core,
        ipc::{self, Owner, Request},
    },
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static HOST: AtomicBool = AtomicBool::new(false);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static STORE: OnceLock<HookStore> = OnceLock::new();
static RESPONDING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
thread_local! { static ORIGIN: RefCell<Option<Owner>> = const { RefCell::new(None) }; }
fn store() -> &'static HookStore {
    STORE.get_or_init(HookStore::default)
}
pub fn set_host() {
    HOST.store(true, Ordering::Relaxed);
    #[cfg(target_os = "macos")]
    crate::macos::telegram::start();
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn token() -> String {
    format!(
        "hook:{}:{}:{}",
        std::process::id(),
        agents::tracking::now(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone)]
enum Resolution {
    Pending,
    Answer(Response),
    Cancelled,
}

struct Entry {
    session: Session,
    target: String,
    deadline: Instant,
    peer: Option<std::os::unix::net::UnixStream>,
    answer: Mutex<Resolution>,
    changed: Condvar,
}
#[derive(Default)]
struct HookStore {
    entries: Mutex<HashMap<String, Arc<Entry>>>,
}
impl HookStore {
    fn insert(&self, session: Session, target: String, timeout: Duration) -> Arc<Entry> {
        let entry = Arc::new(Entry {
            session,
            target,
            deadline: Instant::now() + timeout,
            peer: ipc::peer(),
            answer: Mutex::new(Resolution::Pending),
            changed: Condvar::new(),
        });
        self.entries.lock().unwrap().insert(
            entry.session.approval_key().expect("pending request"),
            entry.clone(),
        );
        entry
    }
    fn active(&self) -> Vec<Session> {
        let entries = self.entries.lock().unwrap();
        let mut active: Vec<_> = entries
            .values()
            .filter(|entry| {
                Instant::now() < entry.deadline
                    && entry.peer.as_ref().is_none_or(ipc::peer_alive)
                    && matches!(*entry.answer.lock().unwrap(), Resolution::Pending)
            })
            .collect();
        active.sort_by_key(|entry| entry.deadline);
        active
            .into_iter()
            .map(|entry| entry.session.clone())
            .collect()
    }

    fn respond(&self, session: &Session, response: &Response) -> bool {
        let Some(key) = session.approval_key() else {
            return false;
        };
        let entries = self.entries.lock().unwrap();
        let Some(entry) = entries.get(&key) else {
            return false;
        };
        if entry.session.agent != session.agent
            || entry.session.id != session.id
            || entry.session.approval != session.approval
            || Instant::now() >= entry.deadline
            || !entry.peer.as_ref().is_none_or(ipc::peer_alive)
            || !matches!(response, Response::Action(action) if action == "allow" || action == "open")
        {
            return false;
        }
        let mut answer = entry.answer.lock().unwrap();
        if !matches!(*answer, Resolution::Pending) {
            return false;
        }
        *answer = Resolution::Answer(response.clone());
        entry.changed.notify_all();
        true
    }
    fn wait(&self, entry: &Entry, connected: impl Fn() -> bool) -> Option<Response> {
        let mut answer = entry.answer.lock().unwrap();
        loop {
            match &*answer {
                Resolution::Answer(answer) => return Some(answer.clone()),
                Resolution::Cancelled => return None,
                Resolution::Pending => {}
            }
            if Instant::now() >= entry.deadline || !connected() {
                return None;
            }
            let duration = entry
                .deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));
            answer = entry.changed.wait_timeout(answer, duration).unwrap().0;
        }
    }
    fn cancel_session(&self, agent: &str, id: &str) {
        for entry in self
            .entries
            .lock()
            .unwrap()
            .values()
            .filter(|entry| entry.session.agent == agent && entry.session.id == id)
        {
            let mut answer = entry.answer.lock().unwrap();
            if matches!(*answer, Resolution::Pending) {
                *answer = Resolution::Cancelled;
                entry.changed.notify_all();
            }
        }
    }
    fn remove(&self, entry: &Entry) {
        self.entries
            .lock()
            .unwrap()
            .remove(&entry.session.approval_key().expect("pending request"));
    }
}

pub fn hook_sessions() -> Vec<Session> {
    store().active()
}
pub fn hook_ids(agent: &str) -> HashSet<String> {
    hook_sessions()
        .into_iter()
        .filter(|s| s.agent == agent)
        .map(|s| s.id)
        .collect()
}
pub fn hook_pending(session: &Session) -> bool {
    if !session
        .approval
        .as_ref()
        .is_some_and(|a| a.id.starts_with("hook:"))
    {
        return true;
    }
    hook_sessions().iter().any(|current| {
        current.agent == session.agent
            && current.id == session.id
            && current.approval == session.approval
    })
}

/// Opening a hook's editor returns control to the agent's own prompt, without approval.
pub fn defer_hook(key: &str) {
    let session = store()
        .entries
        .lock()
        .unwrap()
        .get(key)
        .map(|entry| entry.session.clone());
    if let Some(session) = session {
        respond(&session, &Response::Action("open".into()));
    }
}

pub fn hook_target(session: &Session) -> Option<String> {
    let key = session.approval_key()?;
    store()
        .entries
        .lock()
        .unwrap()
        .get(&key)
        .map(|entry| entry.target.clone())
}

/// The single response boundary for every UI surface and every agent backend.
pub fn respond(session: &Session, response: &Response) -> bool {
    if !HOST.load(Ordering::Relaxed) {
        return false;
    }
    let Some(key) = session.approval_key() else {
        return false;
    };
    let responding = RESPONDING.get_or_init(Mutex::default);
    if !responding.lock().unwrap().insert(key.clone()) {
        return false;
    }
    struct InFlight<'a>(&'a Mutex<HashSet<String>>, String);
    impl Drop for InFlight<'_> {
        fn drop(&mut self) {
            self.0.lock().unwrap().remove(&self.1);
        }
    }
    let _flight = InFlight(responding, key.clone());
    let sent = if session
        .approval
        .as_ref()
        .is_some_and(|a| a.id.starts_with("hook:"))
    {
        store().respond(session, response)
    } else {
        session.respond_agent(response)
    };
    if sent {
        #[cfg(target_os = "macos")]
        crate::macos::telegram::resolved(&key);
        #[cfg(target_os = "macos")]
        dispatch2::DispatchQueue::main().exec_async(move || {
            crate::macos::native::remove_session_notification(&key);
            crate::macos::menu::refresh_after_response();
        });
    }
    sent
}

pub fn ask(
    content: &core::Notification,
    event: &Value,
    approval: bool,
) -> Result<Option<core::Action>> {
    anyhow::ensure!(
        HOST.load(Ordering::Relaxed),
        "Requests must be handled by the resident monitor"
    );
    if !approval {
        #[cfg(target_os = "macos")]
        crate::macos::native::notify_passive(content);
        return Ok(None);
    }
    let agent = match event["_agent"].as_str().unwrap_or("codex") {
        "codex" => "codex",
        "claude" => "claude",
        _ => bail!("Unsupported hook agent"),
    };
    let token = token();
    let session = Session {
        agent,
        id: event["session_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&token)
            .to_owned(),
        title: content.title.clone(),
        cwd: event["cwd"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(core::home),
        workspace: None,
        git_branch: None,
        updated_at: now() as i64,
        client: ORIGIN.with(|owner| {
            if owner
                .borrow()
                .as_ref()
                .is_some_and(|owner| owner.2 == "vscode")
            {
                Client::VSCode
            } else {
                Client::Terminal
            }
        }),
        activity: Activity::Confirmation,
        last_message: String::new(),
        approval: Some(Approval {
            id: token,
            body: content.body.clone(),
            expires: now() + core::WAIT.as_secs(),
            can_allow: true,
        }),
    };
    let entry = store().insert(session, content.target.clone(), core::WAIT);
    #[cfg(target_os = "macos")]
    {
        let session = entry.session.clone();
        dispatch2::DispatchQueue::main().exec_async(move || {
            crate::macos::native::notify_session(&session);
            crate::macos::menu::refresh_after_response();
        });
    }
    let answer = store().wait(&entry, ipc::connected);
    store().remove(&entry);
    #[cfg(target_os = "macos")]
    {
        let key = entry.session.approval_key().unwrap();
        dispatch2::DispatchQueue::main().exec_async(move || {
            crate::macos::native::remove_session_notification(&key);
            crate::macos::menu::refresh_after_response();
        });
    }
    Ok(match answer {
        Some(Response::Action(action)) if action == "allow" => Some(core::Action::Allow),
        Some(Response::Action(action)) if action == "open" => Some(core::Action::Open),
        _ => None,
    })
}

pub fn handle(request: Request) -> Result<Value> {
    let result = match request {
        Request::Setup => {
            #[cfg(target_os = "macos")]
            {
                json!({"allowed": crate::macos::native::setup_notifications()?})
            }
            #[cfg(not(target_os = "macos"))]
            {
                bail!("Notifications require macOS")
            }
        }
        Request::NotificationSettings => {
            #[cfg(target_os = "macos")]
            {
                crate::macos::native::notification_settings()?
            }
            #[cfg(not(target_os = "macos"))]
            {
                bail!("Notifications require macOS")
            }
        }
        Request::Sessions => {
            let sessions = agents::sessions::open()?;
            json!({"monitor":{"pid":std::process::id(),"version":core::VERSION}, "sessions":sessions.iter().map(|session| json!({
                "agent":session.agent,"id":session.id,"title":session.title,"cwd":session.cwd,"workspace":session.workspace,"gitBranch":session.git_branch,"updatedAt":session.updated_at,
                "client":session.client.label(),"status":session.activity.label(),"lastMessage":session.last_message,
                "approval":session.approval.as_ref().map(|request| json!({"id":request.id,"body":request.body,"expiresAt":request.expires,"canAllow":request.can_allow}))
            })).collect::<Vec<_>>()})
        }
        Request::Hook { event, owner } => {
            ORIGIN.with(|slot| *slot.borrow_mut() = owner);
            core::decision(core::deliver(&event, true)?, false).unwrap_or(Value::Null)
        }
        Request::AgentHook {
            agent,
            event,
            input,
            owner,
        } => {
            ORIGIN.with(|slot| *slot.borrow_mut() = Some(owner.clone()));
            let root = agents::tracking::root();
            if agent == "claude"
                && matches!(
                    input["hook_event_name"].as_str(),
                    Some(
                        "SessionEnd"
                            | "Stop"
                            | "StopFailure"
                            | "PostToolUse"
                            | "PostToolUseFailure"
                            | "UserPromptSubmit"
                    )
                )
                && let Some(id) = input["session_id"].as_str()
            {
                store().cancel_session("claude", id);
            }
            match agent.as_str() {
                "claude" => {
                    agents::claude::hook(&input, &root, owner, true)?.unwrap_or(Value::Null)
                }
                "cline" => agents::cline::hook(&input, event.as_deref(), &root, owner)?,
                _ => bail!("Unknown agent"),
            }
        }
        Request::Test {
            approval,
            session_id,
        } => {
            let mut event = json!({"hook_event_name":"PermissionRequest", "cwd":core::home(), "_agent":"codex"});
            if let Some(id) = session_id {
                event["session_id"] = id.into();
            }
            let content = core::Notification {
                title: if event["session_id"].is_string() { core::chat_title(&event, &core::codex_home()) } else { "AI Agent Monitor · Test".into() },
                body: "Click Allow to test the monitor's response path. No agent command will be approved.".into(), target: String::new(),
            };
            let action = ask(&content, &event, approval)?;
            json!({"allowed":action == Some(core::Action::Allow), "test":true})
        }
        Request::Notice {
            title,
            body,
            target,
        } => {
            ask(
                &core::Notification {
                    title,
                    body,
                    target,
                },
                &Value::Null,
                false,
            )?;
            Value::Null
        }
    };
    #[cfg(target_os = "macos")]
    dispatch2::DispatchQueue::main().exec_async(crate::macos::menu::refresh_after_response);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(id: &str) -> Session {
        Session {
            agent: "codex",
            id: "one".into(),
            title: "Test".into(),
            cwd: "/project".into(),
            workspace: None,
            git_branch: None,
            updated_at: 0,
            client: Client::Terminal,
            activity: Activity::Confirmation,
            last_message: String::new(),
            approval: Some(Approval {
                id: id.into(),
                body: "Allow?".into(),
                expires: u64::MAX,
                can_allow: true,
            }),
        }
    }
    #[test]
    fn a_request_has_one_answer_regardless_of_which_surface_submits_it() {
        let store = HookStore::default();
        let original = session("hook:one");
        let entry = store.insert(original.clone(), String::new(), Duration::from_secs(1));
        assert_eq!(store.active().len(), 1);
        assert!(!store.respond(&original, &Response::Action("invented".into())));
        let mut stale = original.clone();
        stale.approval.as_mut().unwrap().body = "changed".into();
        assert!(!store.respond(&stale, &Response::Action("allow".into())));
        assert!(store.respond(&original, &Response::Action("allow".into())));
        assert!(!store.respond(&original, &Response::Action("allow".into())));
        assert!(store.active().is_empty());
        assert_eq!(
            store.wait(&entry, || true),
            Some(Response::Action("allow".into()))
        );
        store.remove(&entry);
        assert!(!store.respond(&original, &Response::Action("allow".into())));
    }
    #[test]
    fn timeout_disconnect_and_a_new_request_cannot_reuse_an_old_answer() {
        let store = HookStore::default();
        let old = session("hook:old");
        let expired = store.insert(old.clone(), String::new(), Duration::ZERO);
        assert!(!store.respond(&old, &Response::Action("allow".into())));
        assert_eq!(store.wait(&expired, || true), None);
        store.remove(&expired);
        let next = session("hook:new");
        let pending = store.insert(next.clone(), String::new(), Duration::from_secs(1));
        assert!(!store.respond(&old, &Response::Action("allow".into())));
        assert_eq!(store.wait(&pending, || false), None);
        store.remove(&pending);
        assert!(store.active().is_empty());
    }
    #[test]
    fn simultaneous_window_and_notification_answers_are_delivered_once() {
        let store = Arc::new(HookStore::default());
        let original = session("hook:concurrent");
        let entry = store.insert(original.clone(), String::new(), Duration::from_secs(1));
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let tasks: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                let session = original.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.respond(&session, &Response::Action("allow".into()))
                })
            })
            .collect();
        barrier.wait();
        assert_eq!(
            tasks
                .into_iter()
                .filter_map(|t| t.join().ok())
                .filter(|sent| *sent)
                .count(),
            1
        );
        assert_eq!(
            store.wait(&entry, || true),
            Some(Response::Action("allow".into()))
        );
        store.remove(&entry);
    }
    #[test]
    fn a_finished_session_cancels_its_pending_hook_without_an_answer() {
        let store = HookStore::default();
        let original = session("hook:cancelled");
        let entry = store.insert(original.clone(), String::new(), Duration::from_secs(1));
        store.cancel_session("claude", "one");
        assert_eq!(store.active().len(), 1);
        store.cancel_session("codex", "one");
        assert!(store.active().is_empty());
        assert!(!store.respond(&original, &Response::Action("allow".into())));
        assert_eq!(store.wait(&entry, || true), None);
        store.remove(&entry);
    }
    #[test]
    fn a_disconnected_hook_cannot_be_answered_from_a_stale_notification() {
        let store = HookStore::default();
        let original = session("hook:disconnected");
        let (client, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let entry = Arc::new(Entry {
            session: original.clone(),
            target: String::new(),
            deadline: Instant::now() + Duration::from_secs(1),
            peer: Some(peer),
            answer: Mutex::new(Resolution::Pending),
            changed: Condvar::new(),
        });
        store
            .entries
            .lock()
            .unwrap()
            .insert(original.approval_key().unwrap(), entry.clone());
        assert_eq!(store.active().len(), 1);
        drop(client);
        assert!(store.active().is_empty());
        assert!(!store.respond(&original, &Response::Action("allow".into())));
        assert_eq!(
            store.wait(&entry, || entry.peer.as_ref().is_none_or(ipc::peer_alive)),
            None
        );
        store.remove(&entry);
    }
    #[test]
    fn opening_the_editor_defers_a_hook_without_granting_permission() {
        let store = HookStore::default();
        let original = session("hook:open");
        let entry = store.insert(original.clone(), String::new(), Duration::from_secs(1));
        assert!(store.respond(&original, &Response::Action("open".into())));
        assert_eq!(
            store.wait(&entry, || true),
            Some(Response::Action("open".into()))
        );
        assert!(!store.respond(&original, &Response::Action("allow".into())));
        assert!(core::decision(Some(core::Action::Open), false).is_none());
        store.remove(&entry);
    }
}
