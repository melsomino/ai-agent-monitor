//! Agent-independent session snapshots and backend dispatch.
use crate::agents::interaction::{Interaction, Response};
pub use crate::domain::session::{Activity, Approval, Client, Session};
use anyhow::{Context, Result};

/// A provider-owned opaque token plus the information needed by the UI.
/// The provider must revalidate this snapshot before sending any response.
/// Backend boundary: discovery is read-only; approval is an explicit action.
pub trait SessionProvider: Send {
    fn id(&self) -> &'static str;
    fn sessions(&self) -> Result<Vec<Session>>;
    #[cfg(test)]
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
    fn deliver(
        &self,
        session: &Session,
        response: &Response,
    ) -> crate::agents::interaction::Delivery {
        if self.respond(session, response) {
            crate::agents::interaction::Delivery::Confirmed
        } else {
            crate::agents::interaction::Delivery::NotSent
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
        let mut branches = std::collections::HashMap::new();
        for session in &mut sessions {
            session.git_branch = branches
                .entry(session.working_directory().to_owned())
                .or_insert_with_key(|directory| crate::app::git::branch(directory))
                .clone();
        }
        for session in &mut sessions {
            if let Some(provider) = self.provider(session.agent) {
                session.interaction_data = provider.interaction(session);
                session.target = Some(provider.open_target(session));
                #[cfg(target_os = "macos")]
                if session.client == Client::Terminal {
                    session.target = Some(crate::macos::terminal::target(session));
                }
            }
        }
        Ok(sessions)
    }

    #[cfg(test)]
    fn pending_count(&self) -> usize {
        self.0
            .iter()
            .map(|provider| provider.pending_count())
            .sum::<usize>()
    }

    #[cfg(test)]
    fn open_target(&self, session: &Session) -> Option<String> {
        self.provider(session.agent)
            .map(|provider| provider.open_target(session))
    }

    #[cfg(test)]
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

pub fn adapters() -> Vec<Box<dyn SessionProvider>> {
    vec![
        Box::new(crate::agents::codex::Codex::default()),
        Box::new(crate::agents::claude::Claude),
        Box::new(crate::agents::cline::Cline),
    ]
}
pub fn observe(provider: &dyn SessionProvider) -> Result<Vec<Session>> {
    Registry(&[provider]).sessions()
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
                authoritative: true,
                interaction_data: Default::default(),
                target: None,
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
        #[cfg(test)]
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
            #[cfg(test)]
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
