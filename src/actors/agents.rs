//! One resource-owning adapter actor for each agent provider.
use super::monitor::{AgentCommand, Command, Handle};
use crate::app::core;
use std::time::Duration;
use tokio::sync::mpsc;
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
                AgentCommand::History {
                    session,
                    cursor,
                    generation,
                } => {
                    let result = provider
                        .previous_message(&session, cursor.as_deref())
                        .map_err(|e| e.to_string());
                    let _ = handle.publish_from_worker(Command::HistoryObserved {
                        session: session.key(),
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
