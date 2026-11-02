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
    let mut usage_reader = crate::agents::codex::usage::Reader::default();
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        let cmd = tokio::select! {cmd=rx.recv()=>match cmd{Some(c)=>c,None=>break},_=tick.tick()=>AgentCommand::Refresh};
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
                    let _ = handle.publish_from_worker(Command::Observe {
                        source,
                        sessions,
                        usage,
                    });
                }
                AgentCommand::Respond { session, response } => {
                    if let Some(approval) = session.approval.clone() {
                        let delivery = provider.deliver(&session, &response);
                        let _ = handle.publish_from_worker(Command::Delivery {
                            key: session.key(),
                            approval,
                            delivery,
                        });
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
            (provider, usage_reader)
        });
        match task.await {
            Ok((new_provider, new_usage)) => {
                provider = new_provider;
                usage_reader = new_usage;
            }
            Err(_) => break,
        }
        if monitor.snapshot().stopping {
            break;
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
}
