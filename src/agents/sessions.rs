//! Agent-independent session snapshots and backend dispatch.
use crate::agents::interaction::{Interaction, Response};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A provider-owned opaque token plus the information needed by the UI.
/// The provider must revalidate this snapshot before sending any response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub id: String,
    pub body: String,
    pub expires: u64,
    pub can_allow: bool,
}

/// Backend boundary: discovery is read-only; approval is an explicit action.
pub trait SessionProvider: Sync {
    fn id(&self) -> &'static str;
    fn sessions(&self) -> Result<Vec<Session>>;
    fn pending_count(&self) -> usize;
    fn previous_message(
        &self,
        _session: &Session,
        _before: Option<&str>,
    ) -> Result<Option<crate::agents::history::Message>> {
        Ok(None)
    }
    fn open_target(&self, session: &Session) -> String;
    fn allow(&self, session: &Session) -> bool;
    fn interaction(&self, session: &Session) -> Interaction {
        if session
            .approval
            .as_ref()
            .is_some_and(|request| request.can_allow)
        {
            Interaction::allow()
        } else {
            Interaction::default()
        }
    }
    fn respond(&self, session: &Session, response: &Response) -> bool {
        response == &Response::Action("allow".into())
            && session
                .approval
                .as_ref()
                .is_some_and(|request| request.can_allow)
            && self.allow(session)
    }
}

/// Registry also serves as the dispatch boundary for session actions.
struct Registry<'a>(&'a [&'a dyn SessionProvider]);

impl Registry<'_> {
    fn provider(&self, id: &str) -> Option<&dyn SessionProvider> {
        self.0.iter().copied().find(|provider| provider.id() == id)
    }

    fn sessions(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();
        for provider in self.0 {
            let discovered = provider
                .sessions()
                .with_context(|| format!("{} session discovery", provider.id()))?;
            anyhow::ensure!(
                discovered
                    .iter()
                    .all(|session| session.agent == provider.id()),
                "Session provider returned a foreign agent ID"
            );
            sessions.extend(discovered);
        }
        for pending in crate::app::monitor::hook_sessions() {
            if let Some(session) = sessions
                .iter_mut()
                .find(|session| session.key() == pending.key())
            {
                session.activity = Activity::Confirmation;
                session.approval = pending.approval;
            } else {
                sessions.push(pending);
            }
        }
        let mut branches = std::collections::HashMap::new();
        for session in &mut sessions {
            session.git_branch = branches
                .entry(session.working_directory().to_owned())
                .or_insert_with_key(|directory| crate::app::git::branch(directory))
                .clone();
        }
        Ok(sessions)
    }

    fn pending_count(&self) -> usize {
        self.0
            .iter()
            .map(|provider| provider.pending_count())
            .sum::<usize>()
            + crate::app::monitor::hook_sessions().len()
    }

    fn open_target(&self, session: &Session) -> Option<String> {
        self.provider(session.agent)
            .map(|provider| provider.open_target(session))
    }

    fn allow(&self, session: &Session) -> bool {
        session
            .approval
            .as_ref()
            .is_some_and(|approval| approval.can_allow)
            && self
                .provider(session.agent)
                .is_some_and(|provider| provider.allow(session))
    }
}

fn registry() -> Registry<'static> {
    Registry(crate::agents::providers())
}

pub fn open() -> Result<Vec<Session>> {
    registry().sessions()
}

pub fn pending_count() -> usize {
    registry().pending_count()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Client {
    Terminal,
    VSCode,
    Desktop {
        name: &'static str,
        app_path: &'static str,
    },
    #[default]
    Unknown,
}
impl Client {
    /// Stable settings identity independent of the displayed group name.
    pub fn application_id(self) -> String {
        match self {
            Self::VSCode => "com.microsoft.VSCode".into(),
            Self::Terminal => "terminal".into(),
            Self::Desktop { app_path, .. } => format!("desktop:{app_path}"),
            Self::Unknown => "other".into(),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::VSCode => "VS Code",
            Self::Desktop { name, .. } => name,
            Self::Unknown => "Unknown",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Activity {
    Working,
    Idle,
    Confirmation,
    #[default]
    Unknown,
}
impl Activity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Idle => "Idle",
            Self::Confirmation => "Needs response",
            Self::Unknown => "Unknown status",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub agent: &'static str,
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub workspace: Option<PathBuf>,
    pub git_branch: Option<String>,
    pub updated_at: i64,
    pub client: Client,
    pub activity: Activity,
    pub last_message: String,
    pub approval: Option<Approval>,
}

impl Session {
    pub fn agent_label(&self) -> &str {
        match self.agent {
            "codex" => "Codex",
            "claude" => "Claude Code",
            "cline" => "Cline",
            other => other,
        }
    }

    pub fn project_path(&self) -> &Path {
        self.workspace.as_deref().unwrap_or(&self.cwd)
    }

    pub fn working_directory(&self) -> &Path {
        match self.workspace.as_deref() {
            Some(path)
                if path
                    .extension()
                    .is_some_and(|extension| extension == "code-workspace") =>
            {
                // A multi-root workspace file does not identify one project directory.
                &self.cwd
            }
            _ => self.project_path(),
        }
    }

    /// UI identity includes the backend; native session IDs may overlap.
    pub fn key(&self) -> String {
        format!("{}:{}", self.agent, self.id)
    }

    /// Request identity is independent of the session's next approval.
    pub fn approval_key(&self) -> Option<String> {
        self.approval
            .as_ref()
            .map(|approval| format!("{}:{}", self.agent, approval.id))
    }

    pub fn open_target(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        if self.client == Client::Terminal {
            return Some(crate::macos::terminal::target(self));
        }
        crate::app::monitor::hook_target(self).or_else(|| registry().open_target(self))
    }

    pub fn interaction(&self) -> Interaction {
        registry()
            .provider(self.agent)
            .map(|provider| provider.interaction(self))
            .unwrap_or_default()
    }

    pub fn previous_message(
        &self,
        before: Option<&str>,
    ) -> Result<Option<crate::agents::history::Message>> {
        registry()
            .provider(self.agent)
            .context("Unknown agent")?
            .previous_message(self, before)
    }

    pub fn respond(&self, response: &Response) -> bool {
        crate::app::monitor::respond(self, response)
    }

    pub(crate) fn respond_agent(&self, response: &Response) -> bool {
        if response == &Response::Action("allow".into()) {
            return registry().allow(self);
        }
        self.approval.is_some()
            && registry()
                .provider(self.agent)
                .is_some_and(|provider| provider.respond(self, response))
    }

    #[cfg(test)]
    pub fn allow(&self) -> bool {
        self.respond(&Response::Action("allow".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeAgent {
        id: &'static str,
        allowed: Mutex<Vec<String>>,
    }
    impl FakeAgent {
        fn new(id: &'static str) -> Self {
            Self {
                id,
                allowed: Mutex::new(Vec::new()),
            }
        }
    }
    impl SessionProvider for FakeAgent {
        fn id(&self) -> &'static str {
            self.id
        }
        fn sessions(&self) -> Result<Vec<Session>> {
            Ok(vec![Session {
                agent: self.id,
                id: "same-chat".into(),
                title: "Chat".into(),
                cwd: "/project".into(),
                workspace: None,
                git_branch: None,
                updated_at: 0,
                client: Client::Terminal,
                activity: Activity::Confirmation,
                last_message: "Answer".into(),
                approval: Some(Approval {
                    id: "same-request".into(),
                    body: "Approve?".into(),
                    expires: u64::MAX,
                    can_allow: true,
                }),
            }])
        }
        fn pending_count(&self) -> usize {
            1
        }
        fn open_target(&self, session: &Session) -> String {
            format!("{}://{}", self.id, session.id)
        }
        fn allow(&self, session: &Session) -> bool {
            self.allowed.lock().unwrap().push(session.id.clone());
            true
        }
    }

    #[test]
    fn overlapping_native_ids_route_to_their_own_agent_and_have_separate_ui_keys() {
        let first = FakeAgent::new("first");
        let second = FakeAgent::new("second");
        let registry = Registry(&[&first, &second]);
        let sessions = registry.sessions().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id, sessions[1].id);
        assert_ne!(sessions[0].key(), sessions[1].key());
        assert_ne!(sessions[0].approval_key(), sessions[1].approval_key());
        assert_eq!(registry.pending_count(), 2);
        assert_eq!(
            registry.open_target(&sessions[1]).as_deref(),
            Some("second://same-chat")
        );
        assert!(first.allowed.lock().unwrap().is_empty());
        assert!(second.allowed.lock().unwrap().is_empty());
        assert!(registry.allow(&sessions[1]));
        assert!(first.allowed.lock().unwrap().is_empty());
        assert_eq!(*second.allowed.lock().unwrap(), ["same-chat"]);
    }

    #[test]
    fn unknown_agent_and_sessions_without_approval_cannot_dispatch_allow() {
        let first = FakeAgent::new("first");
        let registry = Registry(&[&first]);
        let mut session = registry.sessions().unwrap().remove(0);
        session.agent = "unregistered";
        assert!(registry.open_target(&session).is_none());
        assert!(!registry.allow(&session));
        session.agent = "first";
        session.approval.as_mut().unwrap().can_allow = false;
        assert!(!registry.allow(&session));
        assert!(first.allowed.lock().unwrap().is_empty());
        session.approval = None;
        assert!(!registry.allow(&session));
        assert!(first.allowed.lock().unwrap().is_empty());
    }

    #[test]
    fn discovery_rejects_sessions_claiming_another_provider() {
        struct WrongAgent;
        impl SessionProvider for WrongAgent {
            fn id(&self) -> &'static str {
                "wrong"
            }
            fn sessions(&self) -> Result<Vec<Session>> {
                FakeAgent::new("other").sessions()
            }
            fn pending_count(&self) -> usize {
                0
            }
            fn open_target(&self, _: &Session) -> String {
                panic!("must not dispatch")
            }
            fn allow(&self, _: &Session) -> bool {
                panic!("must not dispatch")
            }
        }
        assert!(Registry(&[&WrongAgent]).sessions().is_err());
    }
}
