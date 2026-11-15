//! The resident's single state owner. Commands are one-way; consumers observe snapshots.
use crate::app::{core, ipc::Owner};
use crate::domain::{
    interaction::{Interaction, Response},
    session::{Activity, Approval, Client, Session},
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, watch};

pub use state::*;
mod releases;
mod state;
pub use handle::Handle;
mod handle;
pub(crate) use command::AgentCommand;
pub use command::Command;
mod command;
pub(crate) async fn run(
    mut rx: mpsc::Receiver<Command>,
    published: watch::Sender<Arc<State>>,
    mut agents: HashMap<&'static str, mpsc::Sender<AgentCommand>>,
    telegram: mpsc::Sender<crate::actors::human::telegram::Event>,
    ui: crate::platform::Sender,
) {
    let mut state = State::default();
    let mut releases = releases::Checker::new().await;
    state.versions = releases.state();
    if state.versions != published.borrow().versions {
        state.revision += 1;
    }
    published.send_replace(Arc::new(state.clone()));
    let mut preferences_loaded = false;
    let mut stopped = HashSet::new();
    let mut discovered = Vec::new();
    let mut sources = HashMap::<&'static str, Vec<Session>>::new();
    let mut resolved = HashMap::<String, Approval>::new();
    loop {
        let command = tokio::select! {
            command=rx.recv()=>match command {Some(command)=>command,None=>break},
            versions=releases.next()=>{
                state.versions=versions;state.revision+=1;
                published.send_replace(Arc::new(state.clone()));continue;
            }
        };
        let previous_sessions = state.sessions.clone();
        match command {
            Command::ForgetHistory { client } => {
                state.history.retain(|key, _| key.client != client);
            }
            Command::FinishedSeen { completions } => {
                for (key, generation) in completions {
                    if state.unseen_finished.get(&key) == Some(&generation)
                        && state
                            .sessions
                            .iter()
                            .any(|s| s.key() == key && s.activity == Activity::Idle)
                    {
                        state.unseen_finished.remove(&key);
                    }
                }
            }
            Command::Observe {
                source,
                sessions,
                usage,
            } => {
                if stopped.contains(source) {
                    continue;
                }
                let usage_changed = source == "codex" && state.usage != usage;
                if source == "codex" {
                    state.usage = usage.clone();
                }
                match sessions {
                    Ok(mut sessions) => {
                        sessions.sort_by_key(Session::key);
                        if sources.get(source) == Some(&sessions)
                            && !state.agent_errors.contains_key(source)
                            && !usage_changed
                        {
                            continue;
                        }
                        let previous = &state.sessions;
                        let observed = sessions
                            .into_iter()
                            .map(|mut session| {
                                if let Some(old) =
                                    previous.iter().find(|s| s.key() == session.key())
                                {
                                    let replayed_closed_request = session.pending_request().is_some_and(|request| {
                                        old.activities.iter().any(|activity| {
                                            matches!(activity, crate::domain::session_activity::SessionActivity::Request(previous)
                                                if previous.status != crate::domain::session_activity::RequestStatus::Pending
                                                    && previous.approval == request.approval
                                                    && previous.interaction == request.interaction)
                                        })
                                    });
                                    session.merge_previous(old);
                                    if session.approval().is_some() {
                                        session.activity = Activity::Confirmation;
                                    } else if replayed_closed_request
                                        && session.activity == Activity::Confirmation
                                        && !resolved.contains_key(&session.key())
                                    {
                                        session.activity = Activity::Working;
                                    }
                                }
                                session
                            })
                            .collect();
                        sources.insert(source, observed);
                        discovered = sources.values().flatten().cloned().collect();
                        discovered.sort_by_key(Session::key);
                        state.agent_errors.remove(source);
                    }
                    Err(e) => {
                        state.agent_errors.insert(source.into(), e);
                    }
                }
                state.discovery_error = (!state.agent_errors.is_empty()).then(|| {
                    state
                        .agent_errors
                        .values()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ")
                });
            }
            Command::AgentStopped { source, error } => {
                stopped.insert(source);
                agents.remove(source);
                for send in state.message_sends.values_mut() {
                    if send.session.starts_with(&format!("{source}:"))
                        && send.status == MessageStatus::Sending
                    {
                        send.status = MessageStatus::Failed(error.clone());
                    }
                }
                if let Some(sessions) = sources.get_mut(source) {
                    for session in sessions {
                        session.set_authoritative(false);
                        session.activity = Activity::Unknown;
                        if let Some(key) = session.approval_key() {
                            state
                                .submissions
                                .insert(key, Submission::Failed(error.clone()));
                        }
                    }
                }
                for (key, history) in &mut state.history {
                    if key.session.starts_with(&format!("{source}:")) {
                        let history = Arc::make_mut(history);
                        history.loading = false;
                        history.error = Some(error.clone());
                    }
                }
                discovered = sources.values().flatten().cloned().collect();
                discovered.sort_by_key(Session::key);
                state.agent_errors.insert(source.into(), error);
                state.discovery_error = Some(
                    state
                        .agent_errors
                        .values()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; "),
                );
            }
            #[cfg(target_os = "linux")]
            Command::SetAnimation(enabled) => {
                state.preferences.animation = enabled;
                let _ = ui.try_send(crate::platform::Event::Platform(
                    crate::platform::PlatformCommand::Animation(enabled),
                ));
            }
            Command::SetApplicationEnabled { id, enabled } => {
                if state.preferences.applications.set_enabled(&id, enabled) {
                    let _ = ui.try_send(crate::platform::Event::Platform(
                        crate::platform::PlatformCommand::Applications(
                            state.preferences.applications.clone(),
                        ),
                    ));
                }
            }
            Command::Hook {
                key,
                session,
                target,
            } => {
                let mut session = *session;
                session.target = Some(target);
                state.hooks.insert(
                    key,
                    HookState {
                        session,
                        decision: None,
                    },
                );
            }
            Command::HookGone(key) => {
                state.hooks.remove(&key);
            }
            Command::CancelSession { agent, id } => {
                state
                    .hooks
                    .retain(|_, h| h.session.agent != agent || h.session.id != id);
            }
            Command::SendMessage { id, session, text } => {
                if state.message_sends.contains_key(&id) {
                    continue;
                }
                while state.message_sends.len() >= 256 {
                    let finished = state
                        .message_sends
                        .iter()
                        .find(|(_, send)| send.status != MessageStatus::Sending)
                        .map(|(id, _)| id.clone());
                    if let Some(id) = finished {
                        state.message_sends.remove(&id);
                    } else {
                        break;
                    }
                }
                let current = state.sessions.iter().find(|s| s.key() == session).cloned();
                let error = if text.trim().is_empty() || text.chars().count() > 16000 {
                    Some("Enter a message of 1–16000 characters.")
                } else if current.is_none() {
                    Some("This session is no longer open.")
                } else if !state
                    .preferences
                    .applications
                    .enabled(current.as_ref().unwrap().client)
                {
                    Some("This application is hidden in monitor settings.")
                } else if !current.as_ref().unwrap().can_send_message() {
                    Some("No live message connection to this session. Open it in the agent app.")
                } else if state.message_sends.len() >= 256 {
                    Some("Message queue is busy. Try again later.")
                } else {
                    None
                };
                let status = if let Some(error) = error {
                    MessageStatus::Failed(error.into())
                } else if let Some(current) = current {
                    match agents.get(current.agent).map(|agent| {
                        agent.try_send(AgentCommand::SendMessage {
                            id: id.clone(),
                            session: Box::new(current),
                            text,
                        })
                    }) {
                        Some(Ok(())) => MessageStatus::Sending,
                        _ => MessageStatus::Failed("Agent message queue is unavailable.".into()),
                    }
                } else {
                    unreachable!()
                };
                state
                    .message_sends
                    .insert(id, MessageSend { session, status });
            }
            Command::MessageSent {
                id,
                session,
                result,
            } => {
                if let Some(send) = state.message_sends.get_mut(&id)
                    && send.session == session
                    && send.status == MessageStatus::Sending
                {
                    send.status = match result {
                        Ok(()) => MessageStatus::Sent,
                        Err(error) => MessageStatus::Failed(error),
                    };
                }
            }
            Command::Submit { session, response } => {
                if let Some(current) = state
                    .sessions
                    .iter()
                    .find(|s| {
                        s.key() == session.key()
                            && s.approval().is_some()
                            && s.approval() == session.approval()
                            && s.interaction() == session.interaction()
                            && s.authoritative()
                            && s.approval().is_some_and(|a| a.expires > now())
                    })
                    .cloned()
                {
                    let key = current.approval_key().unwrap();
                    if !matches!(
                        state.submissions.get(&key),
                        Some(Submission::Sending | Submission::Reconciling)
                    ) && valid_response(&current, &response)
                    {
                        if let Some(hook) = state.hooks.get_mut(&current.approval().unwrap().id) {
                            if hook.decision.is_none() {
                                hook.decision = Some(response);
                                state.submissions.insert(key, Submission::Sending);
                            }
                        } else {
                            let Some(agent) = agents.get(current.agent) else {
                                continue;
                            };
                            match agent.try_reserve() {
                                Ok(permit) => {
                                    state.submissions.insert(key, Submission::Sending);
                                    permit.send(AgentCommand::Respond {
                                        session: Box::new(current),
                                        response,
                                    });
                                }
                                Err(_) => {
                                    state.submissions.insert(
                                        key,
                                        Submission::Failed(
                                            "Agent queue is busy. Try again.".into(),
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Command::Delivery {
                key,
                approval,
                delivery,
            } => {
                if let Some(session) = state
                    .sessions
                    .iter()
                    .find(|s| s.key() == key && s.approval() == Some(&approval))
                {
                    let request = session.approval_key().unwrap();
                    if delivery == crate::agents::interaction::Delivery::Confirmed {
                        resolved.insert(key, approval);
                        state.submissions.remove(&request);
                    } else if delivery == crate::agents::interaction::Delivery::Uncertain {
                        state.submissions.insert(request, Submission::Reconciling);
                    } else {
                        state.submissions.insert(
                            request,
                            Submission::Failed(
                                "Could not send the answer. The request may have changed.".into(),
                            ),
                        );
                    }
                }
            }
            Command::Passive(content) => {
                state.notices.push(Notice {
                    id: state.revision,
                    content,
                });
                if state.notices.len() > 64 {
                    state.notices.remove(0);
                }
            }
            Command::Refresh => {
                for agent in agents.values() {
                    let _ = agent.try_send(AgentCommand::Refresh);
                }
            }
            Command::ShowWindow => {
                let _ = ui.try_send(crate::platform::Event::Toggle);
            }
            Command::Open(key) => {
                if let Some(target) = state
                    .sessions
                    .iter()
                    .find(|s| s.key() == key)
                    .and_then(Session::open_target)
                {
                    let _ = ui.try_send(crate::platform::Event::Platform(
                        crate::platform::PlatformCommand::Open(target),
                    ));
                }
            }
            Command::History {
                key,
                cursor,
                generation,
            } => {
                if let Some(value) = state
                    .sessions
                    .iter()
                    .find(|s| s.key() == key.session)
                    .cloned()
                {
                    // Retried commands for the same consumer/request must not
                    // dispatch a second backend read or reset completed state.
                    if state
                        .history
                        .get(&key)
                        .is_some_and(|h| h.generation == generation)
                    {
                        continue;
                    }
                    let history = state.history.entry(key.clone()).or_default();
                    let history = Arc::make_mut(history);
                    if cursor.is_none() {
                        *history = History::default();
                    }
                    history.loading = true;
                    history.error = None;
                    history.generation = generation;
                    match agents.get(value.agent) {
                        None => {
                            history.loading = false;
                            history.error = Some(
                                "Agent is unavailable. Restart the monitor to reconnect.".into(),
                            );
                        }
                        Some(agent) => {
                            if agent
                                .try_send(AgentCommand::History {
                                    key,
                                    session: Box::new(value),
                                    cursor,
                                    generation,
                                })
                                .is_err()
                            {
                                history.loading = false;
                                history.error = Some("History queue is busy".into());
                            }
                        }
                    }
                }
            }
            Command::HistoryObserved {
                key,
                generation,
                result,
            } => {
                if let Some(h) = state.history.get_mut(&key)
                    && h.generation == generation
                    && h.loading
                {
                    let h = Arc::make_mut(h);
                    h.loading = false;
                    match result {
                        Ok(Some(message)) => {
                            h.cursor = Some(message.cursor.clone());
                            h.messages.insert(0, message);
                            // Keep the most recently requested history page's images.
                            let mut image_bytes = 0;
                            for message in &mut h.messages {
                                image_bytes +=
                                    message.images.iter().map(|i| i.data.len()).sum::<usize>();
                                if image_bytes > 40 * 1024 * 1024 {
                                    message.images.clear();
                                }
                            }
                        }
                        Ok(None) => h.exhausted = true,
                        Err(e) => h.error = Some(e),
                    }
                }
            }
            Command::Telegram(event) => {
                let _ = telegram.try_send(event);
            }
            Command::TelegramObserved(status) => {
                if state.telegram == status {
                    continue;
                }
                state.telegram = status;
            }
            #[cfg(target_os = "macos")]
            Command::Platform(effect) => {
                let _ = ui.try_send(crate::platform::Event::Platform(effect));
            }
            Command::IntegrationsObserved { rows, busy, note } => {
                state.integrations = rows;
                state.integration_busy = busy;
                state.integration_note = note;
            }
            Command::PreferencesObserved(mut preferences) => {
                // Persistence acknowledgements must not overwrite newer application commands.
                if preferences_loaded {
                    preferences.applications = state.preferences.applications.clone();
                }
                preferences_loaded = true;
                state.preferences = preferences;
            }
            Command::Shutdown => {
                state.stopping = true;
                state.hooks.clear();
            }
        }
        if preferences_loaded
            && state
                .preferences
                .applications
                .observe(discovered.iter().map(|s| s.client))
        {
            let _ = ui.try_send(crate::platform::Event::Platform(
                crate::platform::PlatformCommand::Applications(
                    state.preferences.applications.clone(),
                ),
            ));
        }
        state.sessions = discovered.clone();
        for session in &mut state.sessions {
            if let Some(approval) = resolved.get(&session.key()) {
                if session.approval() == Some(approval) {
                    session.settle_request(
                        approval,
                        crate::domain::session_activity::RequestStatus::Answered,
                    );
                }
                if session.activity == Activity::Confirmation && session.approval().is_none() {
                    session.activity = Activity::Working;
                }
            }
        }
        resolved.retain(|key, approval| {
            discovered.iter().any(|s| {
                s.key() == *key
                    && (s.approval() == Some(approval)
                        || (s.approval().is_none() && s.activity == Activity::Confirmation))
            })
        });
        let now = now();
        for hook in state.hooks.values().filter(|h| {
            h.decision.is_none() && h.session.approval().is_some_and(|a| a.expires > now)
        }) {
            if let Some(s) = state
                .sessions
                .iter_mut()
                .find(|s| s.key() == hook.session.key())
            {
                s.set_approval(hook.session.approval().cloned());
                s.set_interaction(hook.session.interaction().clone());
                s.activity = Activity::Confirmation;
                s.connection = crate::domain::session::ConnectionState::live(false);
            } else {
                state.sessions.push(hook.session.clone());
            }
        }
        state
            .history
            .retain(|key, _| state.sessions.iter().any(|s| s.key() == key.session));
        let active: HashSet<_> = state
            .sessions
            .iter()
            .filter_map(Session::approval_key)
            .collect();
        state.submissions.retain(|key, _| active.contains(key));
        state.reconcile_finished(&previous_sessions);
        state.revision += 1;
        published.send_replace(Arc::new(state.clone()));
        if state.stopping {
            break;
        }
    }
}
fn valid_response(session: &Session, response: &Response) -> bool {
    let interaction = session.interaction();
    match response {
        Response::Action(id) => interaction.actions.iter().any(|a| a.id == *id),
        Response::Answers(answers) => interaction.valid_answers(answers),
    }
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn hook_session(
    content: &core::Notification,
    event: &serde_json::Value,
    owner: Option<&Owner>,
    key: &str,
) -> Session {
    let agent = if event["_agent"] == "claude" {
        "claude"
    } else {
        "codex"
    };
    {
        let mut __session = Session {
            agent,
            id: event["session_id"].as_str().unwrap_or(key).into(),
            title: content.title.clone(),
            cwd: event["cwd"]
                .as_str()
                .map(Into::into)
                .unwrap_or_else(core::home),
            workspace: None,
            git_branch: None,
            model: None,
            model_display_name: None,
            reasoning_effort: None,
            updated_at: now() as i64,
            client: if owner.is_some_and(|o| o.2 == "vscode") {
                Client::VSCode
            } else {
                Client::Terminal
            },
            activity: Activity::Confirmation,
            target: Some(content.target.clone()),
            connection: if true {
                crate::domain::session::ConnectionState::live(true)
            } else {
                Default::default()
            },
            activities: Default::default(),
        };
        __session.seed_snapshot(
            String::new(),
            vec![],
            None,
            Some(Approval {
                id: key.into(),
                body: content.body.clone(),
                expires: now() + core::WAIT.as_secs(),
                can_allow: true,
            }),
            Interaction {
                command: event["tool_input"]["command"].as_str().map(str::to_owned),
                ..if event["_test_extended_choices"] == true {
                    Interaction {
                        actions: [
                            ("allow", "Allow once"),
                            ("test:always", "Allow always"),
                            ("test:deny", "Deny"),
                        ]
                        .into_iter()
                        .map(|(id, label)| crate::domain::interaction::Action {
                            id: id.into(),
                            label: label.into(),
                            detail: if id == "test:always" {
                                event["_test_always_command"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .into()
                            } else {
                                String::new()
                            },
                        })
                        .collect(),
                        ..Interaction::default()
                    }
                } else {
                    Interaction::allow()
                }
            },
        );
        __session
    }
}

#[cfg(test)]
mod tests;
