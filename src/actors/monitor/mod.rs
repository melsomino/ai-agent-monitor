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
mod state;
pub use handle::Handle;
mod handle;
pub(crate) use command::AgentCommand;
pub use command::Command;
mod command;
#[cfg(target_os = "macos")]
pub(crate) async fn run(
    mut rx: mpsc::Receiver<Command>,
    published: watch::Sender<Arc<State>>,
    mut agents: HashMap<&'static str, mpsc::Sender<AgentCommand>>,
    telegram: mpsc::Sender<crate::actors::human::telegram::Event>,
    ui: crate::macos::bridge::Sender,
) {
    let mut state = State::default();
    let mut preferences_loaded = false;
    let mut stopped = HashSet::new();
    let mut discovered = Vec::new();
    let mut sources = HashMap::<&'static str, Vec<Session>>::new();
    let mut resolved = HashMap::<String, Approval>::new();
    while let Some(command) = rx.recv().await {
        match command {
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
                                if !session.authoritative
                                    && session.approval.is_none()
                                    && let Some(old) =
                                        previous.iter().find(|s| s.key() == session.key())
                                {
                                    session.approval = old.approval.clone();
                                    session.interaction_data = old.interaction_data.clone();
                                    if session.approval.is_some() {
                                        session.activity = Activity::Confirmation;
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
                if let Some(sessions) = sources.get_mut(source) {
                    for session in sessions {
                        session.authoritative = false;
                        session.activity = Activity::Unknown;
                        if let Some(key) = session.approval_key() {
                            state
                                .submissions
                                .insert(key, Submission::Failed(error.clone()));
                        }
                    }
                }
                for (key, history) in &mut state.history {
                    if key.starts_with(&format!("{source}:")) {
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
            Command::SetApplicationEnabled { id, enabled } => {
                if state.preferences.applications.set_enabled(&id, enabled) {
                    let _ = ui.try_send(crate::macos::bridge::Event::Platform(
                        crate::macos::bridge::PlatformCommand::Applications(
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
            Command::Submit { session, response } => {
                if let Some(current) = state
                    .sessions
                    .iter()
                    .find(|s| {
                        s.key() == session.key()
                            && s.approval.is_some()
                            && s.approval == session.approval
                            && s.authoritative
                            && s.approval.as_ref().is_some_and(|a| a.expires > now())
                    })
                    .cloned()
                {
                    let key = current.approval_key().unwrap();
                    if !matches!(
                        state.submissions.get(&key),
                        Some(Submission::Sending | Submission::Reconciling)
                    ) && valid_response(&current, &response)
                    {
                        if let Some(hook) =
                            state.hooks.get_mut(&current.approval.as_ref().unwrap().id)
                        {
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
                    .find(|s| s.key() == key && s.approval.as_ref() == Some(&approval))
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
            Command::Open(key) => {
                if let Some(target) = state
                    .sessions
                    .iter()
                    .find(|s| s.key() == key)
                    .and_then(Session::open_target)
                {
                    let _ = ui.try_send(crate::macos::bridge::Event::Platform(
                        crate::macos::bridge::PlatformCommand::Open(target),
                    ));
                }
            }
            Command::History {
                session,
                cursor,
                generation,
            } => {
                if let Some(value) = state.sessions.iter().find(|s| s.key() == session).cloned() {
                    let history = state.history.entry(session.clone()).or_default();
                    let history = Arc::make_mut(history);
                    if cursor.is_none() {
                        *history = History::default();
                    }
                    history.loading = true;
                    history.error = None;
                    history.generation = generation;
                    let Some(agent) = agents.get(value.agent) else {
                        continue;
                    };
                    if agent
                        .try_send(AgentCommand::History {
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
            Command::HistoryObserved {
                session,
                generation,
                result,
            } => {
                if let Some(h) = state.history.get_mut(&session)
                    && h.generation == generation
                {
                    let h = Arc::make_mut(h);
                    h.loading = false;
                    match result {
                        Ok(Some(message)) => {
                            h.cursor = Some(message.cursor.clone());
                            h.messages.insert(0, message);
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
            Command::Platform(effect) => {
                let _ = ui.try_send(crate::macos::bridge::Event::Platform(effect));
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
            let _ = ui.try_send(crate::macos::bridge::Event::Platform(
                crate::macos::bridge::PlatformCommand::Applications(
                    state.preferences.applications.clone(),
                ),
            ));
        }
        state.sessions = discovered.clone();
        for session in &mut state.sessions {
            if resolved
                .get(&session.key())
                .is_some_and(|a| session.approval.as_ref() == Some(a))
            {
                session.approval = None;
                session.interaction_data = Interaction::default();
            }
        }
        resolved.retain(|key, approval| {
            discovered
                .iter()
                .any(|s| s.key() == *key && s.approval.as_ref() == Some(approval))
        });
        let now = now();
        for hook in state.hooks.values().filter(|h| {
            h.decision.is_none() && h.session.approval.as_ref().is_some_and(|a| a.expires > now)
        }) {
            if let Some(s) = state
                .sessions
                .iter_mut()
                .find(|s| s.key() == hook.session.key())
            {
                s.approval = hook.session.approval.clone();
                s.interaction_data = hook.session.interaction_data.clone();
                s.activity = Activity::Confirmation;
                s.authoritative = true;
            } else {
                state.sessions.push(hook.session.clone());
            }
        }
        state
            .history
            .retain(|key, _| state.sessions.iter().any(|s| s.key() == *key));
        let active: HashSet<_> = state
            .sessions
            .iter()
            .filter_map(Session::approval_key)
            .collect();
        state.submissions.retain(|key, _| active.contains(key));
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
    Session {
        authoritative: true,
        agent,
        id: event["session_id"].as_str().unwrap_or(key).into(),
        title: content.title.clone(),
        cwd: event["cwd"]
            .as_str()
            .map(Into::into)
            .unwrap_or_else(core::home),
        workspace: None,
        git_branch: None,
        updated_at: now() as i64,
        client: if owner.is_some_and(|o| o.2 == "vscode") {
            Client::VSCode
        } else {
            Client::Terminal
        },
        activity: Activity::Confirmation,
        last_message: String::new(),
        interaction_data: Interaction::allow(),
        target: Some(content.target.clone()),
        approval: Some(Approval {
            id: key.into(),
            body: content.body.clone(),
            expires: now() + core::WAIT.as_secs(),
            can_allow: true,
        }),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
