use super::*;
use crate::agents::interaction::Delivery;
use std::time::Duration;

#[test]
fn hook_session_preserves_command_for_approval_display() {
    let command = "printf '%s\\n' 'a long command for the approval preview'";
    let content = core::Notification {
        title: "Test".into(),
        body: "Allow?".into(),
        target: String::new(),
    };
    let event = serde_json::json!({
        "_agent": "claude",
        "tool_input": {"command": command}
    });
    let session = hook_session(&content, &event, None, "test");
    assert_eq!(session.interaction_data.command.as_deref(), Some(command));
    let plain = hook_session(&content, &serde_json::json!({}), None, "plain");
    assert_eq!(plain.interaction_data.command, None);
    let choices = hook_session(
        &content,
        &serde_json::json!({"_test_extended_choices": true, "_test_always_command": "git -C /long/project/path log"}),
        None,
        "choices",
    );
    assert_eq!(
        choices
            .interaction_data
            .actions
            .iter()
            .map(|a| a.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Allow once", "Allow always", "Deny"]
    );
    assert_eq!(
        choices.interaction_data.actions[1].detail,
        "git -C /long/project/path log"
    );
    assert!(choices.interaction_data.actions[0].detail.is_empty());
}

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
    _ui: mpsc::Receiver<crate::platform::Event>,
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
    async fn settle(&self) -> Arc<State> {
        let title = format!("test barrier after {}", self.handle.snapshot().revision);
        self.handle
            .enqueue(Command::Passive(core::Notification {
                title: title.clone(),
                body: String::new(),
                target: String::new(),
            }))
            .await
            .unwrap();
        self.wait(|state| {
            state
                .notices
                .iter()
                .any(|notice| notice.content.title == title)
        })
        .await
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn active_session(id: &str, activity: Activity) -> Session {
    let mut value = session(id, "");
    value.approval = None;
    value.interaction_data = Interaction::default();
    value.activity = activity;
    value
}

#[tokio::test]
async fn finished_sessions_stay_unseen_until_acknowledged() {
    let h = Harness::new();
    let idle = active_session("one", Activity::Idle);
    h.observe(vec![idle.clone()]).await;
    let initial = h.wait(|state| state.sessions.len() == 1).await;
    assert!(
        initial.unseen_finished.is_empty(),
        "initial idle discovery is not a completion"
    );
    h.observe(vec![active_session("one", Activity::Working)])
        .await;
    h.wait(|state| state.sessions[0].activity == Activity::Working)
        .await;
    h.observe(vec![idle.clone()]).await;
    let finished = h.wait(|state| state.unseen_finished.len() == 1).await;
    let generation = finished.unseen_finished[&idle.key()];
    h.handle.enqueue(Command::Refresh).await.unwrap();
    let refreshed = h.wait(|state| state.revision > finished.revision).await;
    assert_eq!(refreshed.unseen_finished[&idle.key()], generation);
    h.handle
        .enqueue(Command::FinishedSeen {
            completions: refreshed.unseen_finished.clone(),
        })
        .await
        .unwrap();
    h.wait(|state| state.unseen_finished.is_empty()).await;
    h.observe(vec![idle]).await;
    assert!(h.handle.snapshot().unseen_finished.is_empty());
}

#[tokio::test]
async fn stale_seen_acknowledgement_cannot_clear_a_new_completion_or_running_session() {
    let h = Harness::new();
    let idle = active_session("one", Activity::Idle);
    let working = active_session("one", Activity::Working);
    h.observe(vec![working.clone()]).await;
    h.wait(|state| state.sessions.len() == 1).await;
    h.observe(vec![idle.clone()]).await;
    let first = h.wait(|state| !state.unseen_finished.is_empty()).await;
    h.observe(vec![working.clone()]).await;
    let running = h
        .wait(|state| state.sessions[0].activity == Activity::Working)
        .await;
    assert!(running.unseen_finished.is_empty());
    h.handle
        .enqueue(Command::FinishedSeen {
            completions: first.unseen_finished.clone(),
        })
        .await
        .unwrap();
    let acknowledged = h.wait(|state| state.revision > running.revision).await;
    assert_eq!(acknowledged.sessions[0].activity, Activity::Working);
    h.observe(vec![idle.clone()]).await;
    let second = h.wait(|state| !state.unseen_finished.is_empty()).await;
    assert_ne!(
        first.unseen_finished[&idle.key()],
        second.unseen_finished[&idle.key()]
    );
    h.handle
        .enqueue(Command::FinishedSeen {
            completions: first.unseen_finished.clone(),
        })
        .await
        .unwrap();
    let after = h.wait(|state| state.revision > second.revision).await;
    assert_eq!(after.unseen_finished, second.unseen_finished);
    h.observe(vec![]).await;
    h.wait(|state| state.sessions.is_empty() && state.unseen_finished.is_empty())
        .await;
}

#[tokio::test]
async fn acknowledging_one_finished_session_does_not_clear_other_sessions() {
    let h = Harness::new();
    h.observe(vec![
        active_session("one", Activity::Working),
        active_session("two", Activity::Working),
    ])
    .await;
    h.wait(|state| state.sessions.len() == 2).await;
    h.observe(vec![
        active_session("one", Activity::Idle),
        active_session("two", Activity::Idle),
    ])
    .await;
    let finished = h.wait(|state| state.unseen_finished.len() == 2).await;
    h.handle
        .enqueue(Command::FinishedSeen {
            completions: HashMap::from([(
                "codex:one".into(),
                finished.unseen_finished["codex:one"],
            )]),
        })
        .await
        .unwrap();
    let seen = h.wait(|state| state.unseen_finished.len() == 1).await;
    assert!(seen.unseen_finished.contains_key("codex:two"));
}

#[tokio::test]
async fn unavailable_or_confirmation_sessions_do_not_report_finished() {
    let h = Harness::new();
    h.observe(vec![active_session("one", Activity::Working)])
        .await;
    h.wait(|state| state.sessions.len() == 1).await;
    let mut idle = active_session("one", Activity::Idle);
    idle.authoritative = false;
    h.observe(vec![idle]).await;
    let unavailable = h.wait(|state| !state.sessions[0].authoritative).await;
    assert!(unavailable.unseen_finished.is_empty());
    h.observe(vec![session("one", "request")]).await;
    h.wait(|state| state.sessions[0].activity == Activity::Confirmation)
        .await;
    h.observe(vec![active_session("one", Activity::Idle)]).await;
    let resolved = h
        .wait(|state| state.sessions[0].activity == Activity::Idle)
        .await;
    assert!(resolved.unseen_finished.is_empty());
}

#[tokio::test]
async fn unseen_completion_survives_temporary_connection_loss_without_changing_generation() {
    let h = Harness::new();
    h.observe(vec![active_session("one", Activity::Working)])
        .await;
    h.wait(|s| s.sessions.len() == 1).await;
    h.observe(vec![active_session("one", Activity::Idle)]).await;
    let finished = h.wait(|s| s.unseen_finished.len() == 1).await;
    let generation = finished.unseen_finished["codex:one"];
    for activity in [Activity::Idle, Activity::Unknown, Activity::Working] {
        let mut disconnected = active_session("one", activity);
        disconnected.authoritative = false;
        h.observe(vec![disconnected]).await;
        let state = h.settle().await;
        assert_eq!(state.unseen_finished.get("codex:one"), Some(&generation));
    }
    h.observe(vec![active_session("one", Activity::Idle)]).await;
    let restored = h.wait(|s| s.sessions[0].authoritative).await;
    assert_eq!(restored.unseen_finished.get("codex:one"), Some(&generation));
    h.observe(vec![active_session("one", Activity::Working)])
        .await;
    h.wait(|s| s.sessions[0].activity == Activity::Working && s.unseen_finished.is_empty())
        .await;
}

#[tokio::test]
async fn window_and_telegram_history_are_independent_even_with_matching_generations() {
    let mut h = Harness::new();
    h.observe(vec![active_session("one", Activity::Idle)]).await;
    h.wait(|s| s.sessions.len() == 1).await;
    let window = HistoryKey::window("codex:one".into());
    let telegram = HistoryKey::telegram("codex:one".into());
    for (key, cursor) in [
        (window.clone(), None),
        (telegram.clone(), Some("telegram-before".into())),
    ] {
        h.handle
            .enqueue(Command::History {
                key,
                cursor,
                generation: 1,
            })
            .await
            .unwrap();
    }
    let loading = h.settle().await;
    assert!(loading.history[&window].loading && loading.history[&telegram].loading);
    for expected in [&window, &telegram] {
        let Some(AgentCommand::History {
            key, generation, ..
        }) = h.agents.recv().await
        else {
            panic!("history command expected");
        };
        assert_eq!(&key, expected);
        assert_eq!(generation, 1);
    }
    for (key, text) in [
        (telegram.clone(), "Telegram message"),
        (window.clone(), "Window message"),
    ] {
        h.handle
            .enqueue(Command::HistoryObserved {
                key,
                generation: 1,
                result: Ok(Some(crate::domain::history::Message {
                    cursor: text.into(),
                    text: text.into(),
                })),
            })
            .await
            .unwrap();
    }
    let done = h.settle().await;
    assert_eq!(done.history[&window].messages.len(), 1);
    assert_eq!(done.history[&window].messages[0].text, "Window message");
    assert_eq!(done.history[&telegram].messages.len(), 1);
    assert_eq!(done.history[&telegram].messages[0].text, "Telegram message");
    // A duplicate completion and a repeated command cannot append the page again.
    h.handle
        .enqueue(Command::HistoryObserved {
            key: window.clone(),
            generation: 1,
            result: Ok(Some(crate::domain::history::Message {
                cursor: "Window message".into(),
                text: "Duplicate".into(),
            })),
        })
        .await
        .unwrap();
    h.handle
        .enqueue(Command::History {
            key: window.clone(),
            generation: 1,
            cursor: None,
        })
        .await
        .unwrap();
    let repeated = h.settle().await;
    assert_eq!(repeated.history[&window].messages.len(), 1);
    assert_eq!(repeated.history[&window].messages[0].text, "Window message");
    assert!(h.agents.try_recv().is_err());
}

#[tokio::test]
async fn history_requests_after_adapter_stop_publish_errors_for_both_clients() {
    let h = Harness::new();
    h.observe(vec![active_session("one", Activity::Idle)]).await;
    h.wait(|s| s.sessions.len() == 1).await;
    h.handle
        .enqueue(Command::AgentStopped {
            source: "codex",
            error: "Disconnected".into(),
        })
        .await
        .unwrap();
    h.wait(|s| !s.sessions[0].authoritative).await;
    let keys = [
        HistoryKey::window("codex:one".into()),
        HistoryKey::telegram("codex:one".into()),
    ];
    for key in &keys {
        h.handle
            .enqueue(Command::History {
                key: key.clone(),
                cursor: None,
                generation: 1,
            })
            .await
            .unwrap();
    }
    let failed = h
        .wait(|s| {
            keys.iter().all(|key| {
                s.history
                    .get(key)
                    .is_some_and(|history| !history.loading && history.error.is_some())
            })
        })
        .await;
    for key in &keys {
        assert!(
            failed.history[key]
                .error
                .as_deref()
                .unwrap()
                .contains("unavailable")
        );
    }
    // A late worker result cannot overwrite the published failure.
    h.handle
        .enqueue(Command::HistoryObserved {
            key: keys[0].clone(),
            generation: 1,
            result: Ok(None),
        })
        .await
        .unwrap();
    let late = h.settle().await;
    assert!(!late.history[&keys[0]].exhausted);
    assert!(late.history[&keys[0]].error.is_some());
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
            key: HistoryKey::window(s.key()),
            cursor: None,
            generation: 2,
        })
        .await
        .unwrap();
    h.wait(|state| {
        state
            .history
            .get(&HistoryKey::window(s.key()))
            .is_some_and(|h| h.loading)
    })
    .await;
    h.handle
        .enqueue(Command::HistoryObserved {
            key: HistoryKey::window(s.key()),
            generation: 1,
            result: Ok(None),
        })
        .await
        .unwrap();
    h.handle
        .enqueue(Command::HistoryObserved {
            key: HistoryKey::window(s.key()),
            generation: 2,
            result: Err("Read failed".into()),
        })
        .await
        .unwrap();
    let state = h
        .wait(|state| state.history[&HistoryKey::window(s.key())].error.is_some())
        .await;
    assert!(!state.history[&HistoryKey::window(s.key())].loading);
    assert!(!state.history[&HistoryKey::window(s.key())].exhausted);
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

#[tokio::test]
async fn changed_request_context_rejects_an_old_control_even_when_approval_id_matches() {
    let mut h = Harness::new();
    let mut original = session("one", "approval");
    original.interaction_data.command = Some("echo original".into());
    h.observe(vec![original.clone()]).await;
    h.wait(|s| s.sessions.len() == 1).await;
    let mut current = original.clone();
    current.interaction_data.command = Some("echo changed".into());
    h.observe(vec![current.clone()]).await;
    h.wait(|s| s.sessions[0].interaction_data.command.as_deref() == Some("echo changed"))
        .await;
    h.handle
        .submit(&original, Response::Action("allow".into()))
        .unwrap();
    let state = h.settle().await;
    assert!(state.submissions.is_empty());
    assert!(h.agents.try_recv().is_err());
    h.handle
        .submit(&current, Response::Action("allow".into()))
        .unwrap();
    h.wait(|s| !s.submissions.is_empty()).await;
    assert!(matches!(
        h.agents.recv().await,
        Some(AgentCommand::Respond { .. })
    ));
}
