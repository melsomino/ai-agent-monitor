use super::*;
use crate::agents::interaction::Delivery;
use std::time::Duration;
fn session(id: &str, request: &str) -> Session {
    Session {
        agent: "codex",
        id: id.into(),
        title: id.into(),
        cwd: "/work".into(),
        workspace: None,
        git_branch: None,
        updated_at: 0,
        client: Client::VSCode,
        activity: Activity::Confirmation,
        last_message: "Last message".into(),
        interaction_data: Interaction::allow(),
        target: Some("vscode://file/work".into()),
        authoritative: true,
        approval: Some(Approval {
            id: request.into(),
            body: "Allow?".into(),
            expires: u64::MAX,
            can_allow: true,
        }),
    }
}
struct Harness {
    handle: Handle,
    agents: mpsc::Receiver<AgentCommand>,
    _ui: mpsc::Receiver<crate::macos::bridge::Event>,
    _telegram: mpsc::Receiver<crate::actors::human::telegram::Event>,
    task: tokio::task::JoinHandle<()>,
}
impl Harness {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel(256);
        let (published, state) = watch::channel(Arc::new(State::default()));
        let handle = Handle { tx, state };
        let (agent, agents) = mpsc::channel(64);
        let (ui, events) = mpsc::channel(128);
        let (telegram, telegram_events) = mpsc::channel(128);
        let task = tokio::spawn(run(
            rx,
            published,
            HashMap::from([("codex", agent)]),
            telegram,
            ui,
        ));
        Self {
            handle,
            agents,
            _ui: events,
            _telegram: telegram_events,
            task,
        }
    }
    async fn observe(&self, sessions: Vec<Session>) {
        self.handle
            .enqueue(Command::Observe {
                source: "codex",
                sessions: Ok(sessions),
                usage: None,
            })
            .await
            .unwrap();
    }
    async fn wait(&self, predicate: impl Fn(&State) -> bool) -> Arc<State> {
        let mut receiver = self.handle.subscribe();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let state = receiver.borrow_and_update().clone();
                if predicate(&state) {
                    return state;
                }
                receiver.changed().await.unwrap();
            }
        })
        .await
        .expect("state transition did not arrive")
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[tokio::test]
async fn state_subscriptions_include_existing_questions_and_both_humans_see_resolution() {
    let h = Harness::new();
    let s = session("one", "request");
    h.observe(vec![s.clone()]).await;
    let initial = h.wait(|state| state.pending_count() == 1).await;
    let late = h.handle.subscribe();
    assert_eq!(late.borrow().sessions, initial.sessions);
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    h.wait(|state| state.submissions.get(&s.approval_key().unwrap()) == Some(&Submission::Sending))
        .await;
    h.handle
        .enqueue(Command::Delivery {
            key: s.key(),
            approval: s.approval.clone().unwrap(),
            delivery: Delivery::Confirmed,
        })
        .await
        .unwrap();
    let state = h.wait(|state| state.pending_count() == 0).await;
    assert_eq!(late.borrow().revision, state.revision);
    assert!(
        initial.sessions[0].approval.is_some(),
        "previous snapshots are immutable"
    );
}
#[tokio::test]
async fn simultaneous_answers_create_only_one_backend_operation() {
    let mut h = Harness::new();
    let s = session("one", "request");
    h.observe(vec![s.clone()]).await;
    h.wait(|s| s.pending_count() == 1).await;
    let answer = Response::Action("allow".into());
    h.handle.submit(&s, answer.clone()).unwrap();
    h.handle.submit(&s, answer).unwrap();
    h.wait(|state| state.revision >= 3).await;
    assert!(matches!(
        h.agents.recv().await,
        Some(AgentCommand::Respond { .. })
    ));
    assert!(h.agents.try_recv().is_err());
}
#[tokio::test]
async fn invalid_answers_and_expired_or_stale_requests_never_reach_the_adapter() {
    let mut h = Harness::new();
    let s = session("one", "request");
    h.observe(vec![s.clone()]).await;
    h.wait(|s| s.pending_count() == 1).await;
    h.handle
        .submit(&s, Response::Action("invented".into()))
        .unwrap();
    let stale = session("one", "older-request");
    h.handle
        .submit(&stale, Response::Action("allow".into()))
        .unwrap();
    let mut expired = session("expired", "request2");
    expired.approval.as_mut().unwrap().expires = 0;
    h.observe(vec![s, expired.clone()]).await;
    h.wait(|s| s.sessions.len() == 2).await;
    h.handle
        .submit(&expired, Response::Action("allow".into()))
        .unwrap();
    h.wait(|s| s.revision >= 5).await;
    assert!(h.agents.try_recv().is_err());
}
#[tokio::test]
async fn uncertain_delivery_does_not_allow_an_automatic_or_duplicate_retry() {
    let mut h = Harness::new();
    let s = session("one", "request");
    h.observe(vec![s.clone()]).await;
    h.wait(|s| s.pending_count() == 1).await;
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    let _ = h.agents.recv().await;
    h.handle
        .enqueue(Command::Delivery {
            key: s.key(),
            approval: s.approval.clone().unwrap(),
            delivery: Delivery::Uncertain,
        })
        .await
        .unwrap();
    h.wait(|state| {
        state.submissions.get(&s.approval_key().unwrap()) == Some(&Submission::Reconciling)
    })
    .await;
    let revision = h.handle.snapshot().revision;
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    h.wait(|state| state.revision > revision).await;
    assert!(h.agents.try_recv().is_err());
    let mut resolved = s;
    resolved.approval = None;
    h.observe(vec![resolved]).await;
    h.wait(|state| state.pending_count() == 0).await;
}
#[tokio::test]
async fn missing_stream_observation_preserves_question_but_blocks_sending() {
    let mut h = Harness::new();
    let s = session("one", "request");
    h.observe(vec![s.clone()]).await;
    h.wait(|s| s.pending_count() == 1).await;
    let mut uncertain = s.clone();
    uncertain.authoritative = false;
    uncertain.approval = None;
    uncertain.interaction_data = Interaction::default();
    h.observe(vec![uncertain]).await;
    let state = h.wait(|s| !s.sessions[0].authoritative).await;
    assert!(state.sessions[0].approval.is_some());
    let rev = state.revision;
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    h.wait(|s| s.revision > rev).await;
    assert!(h.agents.try_recv().is_err());
    h.observe(vec![]).await;
    h.wait(|s| s.sessions.is_empty()).await;
}
#[tokio::test]
async fn one_agent_failure_does_not_remove_another_agents_sessions() {
    let h = Harness::new();
    let first = session("codex-session", "one");
    let mut second = session("claude-session", "two");
    second.agent = "claude";
    h.observe(vec![first]).await;
    h.handle
        .enqueue(Command::Observe {
            source: "claude",
            sessions: Ok(vec![second]),
            usage: None,
        })
        .await
        .unwrap();
    h.wait(|s| s.sessions.len() == 2).await;
    h.handle
        .enqueue(Command::Observe {
            source: "claude",
            sessions: Err("Temporarily unavailable".into()),
            usage: None,
        })
        .await
        .unwrap();
    let state = h.wait(|s| s.agent_errors.contains_key("claude")).await;
    assert_eq!(state.sessions.len(), 2);
}
#[tokio::test]
async fn late_delivery_cannot_resolve_a_replacement_question() {
    let h = Harness::new();
    let old = session("one", "old");
    h.observe(vec![old.clone()]).await;
    h.wait(|s| s.pending_count() == 1).await;
    let new = session("one", "new");
    h.observe(vec![new.clone()]).await;
    h.wait(|s| s.sessions[0].approval == new.approval).await;
    let rev = h.handle.snapshot().revision;
    h.handle
        .enqueue(Command::Delivery {
            key: old.key(),
            approval: old.approval.unwrap(),
            delivery: Delivery::Confirmed,
        })
        .await
        .unwrap();
    let state = h.wait(|s| s.revision > rev).await;
    assert_eq!(state.sessions[0].approval, new.approval);
}
#[tokio::test]
async fn hooks_are_resolved_through_state_and_disconnect_revokes_the_request() {
    let h = Harness::new();
    let content = core::Notification {
        title: "Hook".into(),
        body: "Allow?".into(),
        target: String::new(),
    };
    let s = hook_session(
        &content,
        &serde_json::json!({"session_id":"one"}),
        None,
        "hook:first",
    );
    h.handle
        .enqueue(Command::Hook {
            key: "hook:first".into(),
            session: Box::new(s.clone()),
            target: String::new(),
        })
        .await
        .unwrap();
    h.wait(|s| s.pending_count() == 1).await;
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    h.wait(|s| s.hooks["hook:first"].decision.is_some() && s.pending_count() == 0)
        .await;
    h.handle
        .enqueue(Command::HookGone("hook:first".into()))
        .await
        .unwrap();
    h.wait(|s| s.hooks.is_empty()).await;
    let rev = h.handle.snapshot().revision;
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    h.wait(|s| s.revision > rev).await;
    assert!(h.handle.snapshot().hooks.is_empty());
}
#[tokio::test]
async fn history_ignores_late_generations_and_reports_loading_in_state() {
    let h = Harness::new();
    let s = session("one", "request");
    h.observe(vec![s.clone()]).await;
    h.wait(|s| s.pending_count() == 1).await;
    h.handle
        .enqueue(Command::History {
            session: s.key(),
            cursor: None,
            generation: 2,
        })
        .await
        .unwrap();
    h.wait(|state| state.history.get(&s.key()).is_some_and(|h| h.loading))
        .await;
    h.handle
        .enqueue(Command::HistoryObserved {
            session: s.key(),
            generation: 1,
            result: Ok(None),
        })
        .await
        .unwrap();
    h.handle
        .enqueue(Command::HistoryObserved {
            session: s.key(),
            generation: 2,
            result: Err("Read failed".into()),
        })
        .await
        .unwrap();
    let state = h
        .wait(|state| state.history[&s.key()].error.is_some())
        .await;
    assert!(!state.history[&s.key()].loading);
    assert!(!state.history[&s.key()].exhausted);
}
#[tokio::test]
async fn shutdown_is_published_and_releases_active_hooks() {
    let h = Harness::new();
    let s = session("one", "request");
    h.handle
        .enqueue(Command::Hook {
            key: "request".into(),
            session: Box::new(s),
            target: String::new(),
        })
        .await
        .unwrap();
    h.wait(|s| s.pending_count() == 1).await;
    h.handle.enqueue(Command::Shutdown).await.unwrap();
    let state = h.wait(|s| s.stopping).await;
    assert!(state.hooks.is_empty());
}

#[tokio::test]
async fn a_full_adapter_mailbox_reports_state_error_without_losing_the_question() {
    let h = Harness::new();
    let s = session("one", "request");
    h.observe(vec![s.clone()]).await;
    h.wait(|s| s.pending_count() == 1).await;
    for _ in 0..64 {
        h.handle.enqueue(Command::Refresh).await.unwrap();
    }
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    let state = h
        .wait(|state| {
            matches!(
                state.submissions.get(&s.approval_key().unwrap()),
                Some(Submission::Failed(_))
            )
        })
        .await;
    assert_eq!(state.pending_count(), 1);
}
#[tokio::test]
async fn disconnected_hook_cannot_accept_a_late_human_answer() {
    let mut h = Harness::new();
    let s = session("one", "request");
    h.handle
        .enqueue(Command::Hook {
            key: "request".into(),
            session: Box::new(s.clone()),
            target: String::new(),
        })
        .await
        .unwrap();
    h.wait(|s| s.pending_count() == 1).await;
    h.handle
        .enqueue(Command::HookGone("request".into()))
        .await
        .unwrap();
    h.wait(|s| s.pending_count() == 0).await;
    let revision = h.handle.snapshot().revision;
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    h.wait(|s| s.revision > revision).await;
    assert!(h.handle.snapshot().hooks.is_empty());
    assert!(h.agents.try_recv().is_err());
}
#[tokio::test]
async fn failed_delivery_stays_visible_and_requires_an_explicit_retry() {
    let mut h = Harness::new();
    let s = session("one", "request");
    h.observe(vec![s.clone()]).await;
    h.wait(|s| s.pending_count() == 1).await;
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    let _ = h.agents.recv().await;
    h.handle
        .enqueue(Command::Delivery {
            key: s.key(),
            approval: s.approval.clone().unwrap(),
            delivery: Delivery::NotSent,
        })
        .await
        .unwrap();
    h.wait(|state| {
        matches!(
            state.submissions.get(&s.approval_key().unwrap()),
            Some(Submission::Failed(_))
        )
    })
    .await;
    assert!(h.agents.try_recv().is_err());
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    assert!(matches!(
        h.agents.recv().await,
        Some(AgentCommand::Respond { .. })
    ));
}

#[tokio::test]
async fn stopped_adapter_marks_sessions_and_pending_operations_unavailable() {
    let h = Harness::new();
    let s = session("one", "request");
    let key = s.approval_key().unwrap();
    h.observe(vec![s.clone()]).await;
    h.wait(|s| s.pending_count() == 1).await;
    h.handle
        .submit(&s, Response::Action("allow".into()))
        .unwrap();
    h.wait(|s| s.submissions.get(&key) == Some(&Submission::Sending))
        .await;
    h.handle
        .enqueue(Command::AgentStopped {
            source: "codex",
            error: "Adapter stopped".into(),
        })
        .await
        .unwrap();
    let state = h.wait(|s| s.agent_errors.contains_key("codex")).await;
    assert!(!state.sessions[0].authoritative);
    assert_eq!(state.sessions[0].activity, Activity::Unknown);
    assert!(matches!(
        state.submissions.get(&key),
        Some(Submission::Failed(_))
    ));
    h.observe(vec![s]).await;
    h.handle.enqueue(Command::Refresh).await.unwrap();
    let updated = h.wait(|s| s.revision > state.revision).await;
    assert!(!updated.sessions[0].authoritative);
}

#[tokio::test]
async fn rapid_application_changes_survive_stale_persistence_acknowledgements() {
    let h = Harness::new();
    let mut preferences = crate::domain::preferences::Preferences::default();
    preferences
        .applications
        .observe([Client::VSCode, Client::Terminal]);
    h.handle
        .enqueue(Command::PreferencesObserved(preferences.clone()))
        .await
        .unwrap();
    for client in [Client::VSCode, Client::Terminal] {
        h.handle
            .enqueue(Command::SetApplicationEnabled {
                id: client.application_id(),
                enabled: false,
            })
            .await
            .unwrap();
    }
    let state = h
        .wait(|s| {
            !s.preferences.applications.enabled(Client::VSCode)
                && !s.preferences.applications.enabled(Client::Terminal)
        })
        .await;
    h.handle
        .enqueue(Command::PreferencesObserved(preferences))
        .await
        .unwrap();
    let updated = h.wait(|s| s.revision > state.revision).await;
    assert!(!updated.preferences.applications.enabled(Client::VSCode));
    assert!(!updated.preferences.applications.enabled(Client::Terminal));
}

#[tokio::test]
async fn sending_chat_text_is_idempotent_and_delivery_is_published_in_state() {
    let mut h = Harness::new();
    let s = session("one", "request");
    h.observe(vec![s.clone()]).await;
    h.wait(|state| state.sessions.len() == 1).await;
    for _ in 0..2 {
        h.handle
            .enqueue(Command::SendMessage {
                id: "telegram:42:7".into(),
                session: s.key(),
                text: "Continue with the tests".into(),
            })
            .await
            .unwrap();
    }
    h.wait(|state| {
        state
            .message_sends
            .get("telegram:42:7")
            .is_some_and(|send| send.status == MessageStatus::Sending)
    })
    .await;
    let Some(AgentCommand::SendMessage {
        id,
        session: target,
        text,
    }) = h.agents.recv().await
    else {
        panic!("expected a message operation")
    };
    assert_eq!(target.key(), s.key());
    assert_eq!(text, "Continue with the tests");
    assert!(h.agents.try_recv().is_err());
    h.handle
        .enqueue(Command::MessageSent {
            id: id.clone(),
            session: "codex:other".into(),
            result: Ok(()),
        })
        .await
        .unwrap();
    h.handle
        .enqueue(Command::MessageSent {
            id,
            session: s.key(),
            result: Ok(()),
        })
        .await
        .unwrap();
    h.wait(|state| {
        state
            .message_sends
            .get("telegram:42:7")
            .is_some_and(|send| send.status == MessageStatus::Sent)
    })
    .await;
}
#[tokio::test]
async fn stale_empty_and_disconnected_chat_messages_never_reach_an_agent() {
    let mut h = Harness::new();
    let mut s = session("one", "request");
    s.authoritative = false;
    h.observe(vec![s.clone()]).await;
    h.wait(|state| state.sessions.len() == 1).await;
    for (id, target, text) in [
        ("stale", "codex:gone", "Hello"),
        ("empty", "codex:one", "  "),
        ("disconnected", "codex:one", "Hello"),
    ] {
        h.handle
            .enqueue(Command::SendMessage {
                id: id.into(),
                session: target.into(),
                text: text.into(),
            })
            .await
            .unwrap();
    }
    h.wait(|state| state.message_sends.len() == 3).await;
    assert!(
        h.handle
            .snapshot()
            .message_sends
            .values()
            .all(|send| matches!(send.status, MessageStatus::Failed(_)))
    );
    assert!(h.agents.try_recv().is_err());
}
#[tokio::test]
async fn adapter_failure_clears_inflight_chat_sending_status() {
    let h = Harness::new();
    let s = session("one", "request");
    h.observe(vec![s.clone()]).await;
    h.wait(|state| state.sessions.len() == 1).await;
    h.handle
        .enqueue(Command::SendMessage {
            id: "message".into(),
            session: s.key(),
            text: "Hello".into(),
        })
        .await
        .unwrap();
    h.wait(|state| {
        state
            .message_sends
            .get("message")
            .is_some_and(|send| send.status == MessageStatus::Sending)
    })
    .await;
    h.handle
        .enqueue(Command::AgentStopped {
            source: "codex",
            error: "Disconnected".into(),
        })
        .await
        .unwrap();
    h.wait(|state| {
        state
            .message_sends
            .get("message")
            .is_some_and(|send| matches!(send.status, MessageStatus::Failed(_)))
    })
    .await;
}
