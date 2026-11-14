//! One resource-owning adapter actor for each agent provider.
use super::monitor::{AgentCommand, Command, Handle};
use crate::app::core;
use std::time::Duration;
use tokio::sync::mpsc;
pub(crate) async fn supervise(
    provider: Box<dyn crate::agents::sessions::SessionProvider>,
    rx: mpsc::Receiver<AgentCommand>,
    monitor: Handle,
) {
    let source = provider.id();
    let result = tokio::spawn(run(provider, rx, monitor.clone())).await;
    if !monitor.snapshot().stopping {
        let error = match result {
            Ok(()) => format!("{source} adapter stopped. Restart the monitor to reconnect."),
            Err(error) => format!("{source} adapter failed: {error}"),
        };
        let _ = monitor
            .enqueue(Command::AgentStopped { source, error })
            .await;
    }
}
pub(crate) async fn run(
    mut provider: Box<dyn crate::agents::sessions::SessionProvider>,
    mut rx: mpsc::Receiver<AgentCommand>,
    monitor: Handle,
) {
    let source = provider.id();
    let live_updates = provider.live_updates();
    let mut cached: Option<Vec<crate::agents::sessions::Session>> = None;
    let mut cached_usage = None;
    let mut usage_reader = crate::agents::codex::usage::Reader::default();
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        let cmd = tokio::select! {
            cmd=rx.recv()=>match cmd{Some(c)=>c,None=>break},
            _=tick.tick()=>AgentCommand::Refresh,
            _=async { match &live_updates { Some(updates)=>updates.notified().await, None=>std::future::pending::<()>().await } }=>AgentCommand::LiveRefresh,
        };
        let handle = monitor.clone();
        let task = tokio::task::spawn_blocking(move || {
            match cmd {
                AgentCommand::Refresh => {
                    let sessions =
                        crate::agents::sessions::observe(&*provider).map_err(|e| e.to_string());
                    let usage = if source == "codex" {
                        usage_reader.latest(&core::codex_home())
                    } else {
                        None
                    };
                    cached = sessions.as_ref().ok().cloned();
                    cached_usage = usage.clone();
                    let _ = handle.publish_from_worker(Command::Observe {
                        source,
                        sessions,
                        usage,
                    });
                }
                AgentCommand::LiveRefresh => {
                    publish_live(&*provider, &mut cached, &cached_usage, &handle);
                }
                AgentCommand::Respond { session, response } => {
                    if let Some(approval) = session.approval().cloned() {
                        let delivery = provider.deliver(&session, &response);
                        let _ = handle.publish_from_worker(Command::Delivery {
                            key: session.key(),
                            approval,
                            delivery,
                        });
                        publish_live(&*provider, &mut cached, &cached_usage, &handle);
                    }
                }
                AgentCommand::SendMessage { id, session, text } => {
                    let result = provider
                        .send_message(&session, &text)
                        .map_err(|error| error.to_string());
                    let _ = handle.publish_from_worker(Command::MessageSent {
                        id,
                        session: session.key(),
                        result,
                    });
                }
                AgentCommand::History {
                    key,
                    session,
                    cursor,
                    generation,
                } => {
                    let result = provider
                        .previous_message(&session, cursor.as_deref())
                        .map_err(|e| e.to_string());
                    let _ = handle.publish_from_worker(Command::HistoryObserved {
                        key,
                        generation,
                        result,
                    });
                }
            }
            (provider, usage_reader, cached, cached_usage)
        });
        match task.await {
            Ok((new_provider, new_usage, new_cached, new_cached_usage)) => {
                provider = new_provider;
                usage_reader = new_usage;
                cached = new_cached;
                cached_usage = new_cached_usage;
            }
            Err(_) => break,
        }
        if monitor.snapshot().stopping {
            break;
        }
    }
}

fn publish_live(
    provider: &dyn crate::agents::sessions::SessionProvider,
    cached: &mut Option<Vec<crate::agents::sessions::Session>>,
    usage: &Option<crate::agents::codex::usage::Usage>,
    handle: &Handle,
) {
    if let Some(sessions) = cached {
        let previous = sessions.clone();
        provider.refresh_live(sessions);
        if *sessions != previous {
            let _ = handle.publish_from_worker(Command::Observe {
                source: provider.id(),
                sessions: Ok(sessions.clone()),
                usage: usage.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::sessions::{Session, SessionProvider};
    struct Broken;
    impl SessionProvider for Broken {
        fn id(&self) -> &'static str {
            "broken"
        }
        fn sessions(&self) -> anyhow::Result<Vec<Session>> {
            panic!("test adapter failure")
        }
        fn pending_count(&self) -> usize {
            0
        }
        fn open_target(&self, _: &Session) -> String {
            String::new()
        }
        fn allow(&self, _: &Session) -> bool {
            false
        }
    }
    #[tokio::test]
    async fn adapter_panic_is_reported_to_monitor() {
        let (tx, mut events) = mpsc::channel(8);
        let handle = Handle {
            tx,
            state: tokio::sync::watch::channel(std::sync::Arc::new(
                crate::actors::monitor::State::default(),
            ))
            .1,
        };
        let (_port, rx) = mpsc::channel(8);
        tokio::time::timeout(
            Duration::from_secs(2),
            supervise(Box::new(Broken), rx, handle),
        )
        .await
        .unwrap();
        assert!(matches!(
            events.recv().await,
            Some(Command::AgentStopped {
                source: "broken",
                ..
            })
        ));
    }
    #[tokio::test]
    async fn live_requests_and_confirmed_status_publish_without_another_discovery_scan() {
        use crate::domain::{
            interaction::{Delivery, Interaction, Response},
            session::{Activity, Approval, Client},
        };
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        struct LiveFake {
            state: Arc<Mutex<Session>>,
            updates: Arc<tokio::sync::Notify>,
            scans: Arc<AtomicUsize>,
        }
        impl SessionProvider for LiveFake {
            fn id(&self) -> &'static str {
                "live-test"
            }
            fn sessions(&self) -> anyhow::Result<Vec<Session>> {
                self.scans.fetch_add(1, Ordering::SeqCst);
                Ok(vec![self.state.lock().unwrap().clone()])
            }
            fn pending_count(&self) -> usize {
                0
            }
            fn open_target(&self, _: &Session) -> String {
                String::new()
            }
            fn allow(&self, _: &Session) -> bool {
                false
            }
            fn live_updates(&self) -> Option<Arc<tokio::sync::Notify>> {
                Some(self.updates.clone())
            }
            fn refresh_live(&self, sessions: &mut [Session]) {
                sessions[0] = self.state.lock().unwrap().clone();
            }
            fn deliver(&self, _: &Session, _: &Response) -> Delivery {
                let mut s = self.state.lock().unwrap();
                s.set_approval(None);
                s.set_interaction(Interaction::default());
                s.activity = Activity::Working;
                self.updates.notify_one();
                Delivery::Confirmed
            }
        }
        async fn observed(events: &mut mpsc::Receiver<Command>) -> Vec<Session> {
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if let Some(Command::Observe {
                        sessions: Ok(sessions),
                        ..
                    }) = events.recv().await
                    {
                        return sessions;
                    }
                }
            })
            .await
            .expect("live update must not wait for the two-second scan")
        }
        let state = Arc::new(Mutex::new({
            let mut __session = Session {
                agent: "live-test",
                id: "terminal".into(),
                title: "Test".into(),
                cwd: "/tmp".into(),
                workspace: None,
                git_branch: None,
                model: None,
                model_display_name: None,
                reasoning_effort: None,
                updated_at: 0,
                client: Client::Terminal,
                activity: Activity::Working,
                target: None,
                connection: if true {
                    crate::domain::session::ConnectionState::live(true)
                } else {
                    Default::default()
                },
                activities: Default::default(),
            };
            __session.seed_snapshot(String::new(), vec![], None, None, Interaction::default());
            __session
        }));
        let scans = Arc::new(AtomicUsize::new(0));
        let updates = Arc::new(tokio::sync::Notify::new());
        let (tx, mut events) = mpsc::channel(32);
        let handle = Handle {
            tx,
            state: tokio::sync::watch::channel(Arc::new(crate::actors::monitor::State::default()))
                .1,
        };
        let (commands, rx) = mpsc::channel(8);
        let task = tokio::spawn(run(
            Box::new(LiveFake {
                state: state.clone(),
                updates: updates.clone(),
                scans: scans.clone(),
            }),
            rx,
            handle,
        ));
        assert!(observed(&mut events).await[0].approval().is_none());
        {
            let mut s = state.lock().unwrap();
            s.activity = Activity::Confirmation;
            s.set_approval(Some(Approval {
                id: "request".into(),
                body: "Confirm".into(),
                expires: u64::MAX,
                can_allow: true,
            }));
            s.set_interaction(Interaction::allow());
        }
        updates.notify_one();
        let pending = observed(&mut events).await.remove(0);
        assert_eq!(pending.activity, Activity::Confirmation);
        assert!(pending.approval().is_some());
        assert_eq!(scans.load(Ordering::SeqCst), 1);
        commands
            .send(AgentCommand::Respond {
                session: Box::new(pending),
                response: Response::Action("allow".into()),
            })
            .await
            .unwrap();
        let cleared = observed(&mut events).await.remove(0);
        assert!(cleared.approval().is_none());
        assert_eq!(cleared.activity, Activity::Working);
        assert_eq!(scans.load(Ordering::SeqCst), 1);
        // An unchanged wake must not republish the same request/state.
        updates.notify_one();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), events.recv())
                .await
                .is_err()
        );
        drop(commands);
        task.await.unwrap();
    }
}
