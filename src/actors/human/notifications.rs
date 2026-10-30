//! Native notifications reconcile their own bindings against Monitor state.
use crate::{
    actors::monitor,
    macos::{
        bridge::{Event, Sender},
        native::{NotificationInput, notify_passive, remove_session_notification},
        notification_actions::{Binding, Bindings, Kind, Plan},
        notification_categories::{Notice, Receipt},
    },
};
use objc2_user_notifications::UNNotificationDefaultActionIdentifier;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

#[derive(Default)]
struct DeliveryQueue {
    queued: super::retry::Queue<(Plan, Notice)>,
    in_flight: Option<(Plan, Notice)>,
}
impl DeliveryQueue {
    fn complete(&mut self, receipt: Receipt, active: bool, now: Instant) {
        if !self
            .in_flight
            .as_ref()
            .is_some_and(|(_, notice)| notice.key == receipt.key)
        {
            return;
        }
        let delivery = self.in_flight.take().unwrap();
        if let Err(error) = receipt.result {
            eprintln!("ai-agent-monitor: {error}");
            if active {
                self.queued.push(delivery, now + Duration::from_secs(5));
            }
        }
    }
}

// Navigation remains a Monitor command, just like clicking the session row.
async fn present_button(binding: Binding, index: usize, monitor: &monitor::Handle, ui: &Sender) {
    match binding.plan.buttons.get(index).map(|button| &button.kind) {
        Some(Kind::Editor) => {
            if let Err(error) = monitor
                .enqueue(monitor::Command::Open(binding.session.key()))
                .await
            {
                eprintln!("ai-agent-monitor: {error}");
            }
        }
        Some(kind) => {
            let form = matches!(kind, Kind::Form | Kind::More | Kind::Text);
            let _ = ui
                .send(Event::ShowRequest {
                    session: Box::new(binding.session),
                    form,
                    draft: None,
                })
                .await;
        }
        None => {}
    }
}

async fn submit_reply(
    monitor: &monitor::Handle,
    session: &crate::domain::session::Session,
    response: crate::domain::interaction::Response,
) {
    if let Err(error) = monitor
        .enqueue(monitor::Command::Submit {
            session: Box::new(session.clone()),
            response,
        })
        .await
    {
        eprintln!("ai-agent-monitor: {error}");
    }
}

pub async fn run(
    monitor: monitor::Handle,
    mut input: tokio::sync::mpsc::Receiver<NotificationInput>,
    ui: Sender,
) {
    let mut snapshots = monitor.subscribe();
    let mut bindings = Bindings::default();
    let (mut known, mut notices) = (HashSet::new(), HashSet::new());
    let (receipts, mut completed) = tokio::sync::mpsc::channel::<Receipt>(128);
    let mut delivery = DeliveryQueue::default();
    loop {
        let snapshot = snapshots.borrow_and_update().clone();
        let active: HashSet<_> = snapshot
            .sessions
            .iter()
            .filter_map(|s| s.approval_key())
            .collect();
        for key in known.difference(&active) {
            bindings.remove(key);
            remove_session_notification(key);
        }
        delivery
            .queued
            .retain(|(plan, notice)| bindings.lookup(&notice.key, &plan.category()).is_some());
        for session in &snapshot.sessions {
            if let Some(key) = session.approval_key()
                && !known.contains(&key)
                && let Some(plan) = bindings.bind(session)
            {
                let approval = session.approval.as_ref().unwrap();
                delivery.queued.push(
                    (
                        plan,
                        Notice {
                            key,
                            title: session.title.clone(),
                            body: approval.body.clone(),
                            target: session.open_target().unwrap_or_default(),
                            token: approval.id.clone(),
                        },
                    ),
                    Instant::now(),
                );
            }
        }
        known = active;
        for notice in &snapshot.notices {
            if notices.insert(notice.id) {
                notify_passive(&notice.content);
            }
        }
        if snapshot.stopping {
            break;
        }
        if delivery.in_flight.is_none()
            && let Some((plan, notice)) = delivery.queued.pop_ready(Instant::now())
        {
            delivery.in_flight = Some((plan.clone(), notice.clone()));
            crate::macos::notification_categories::deliver(
                plan,
                notice,
                bindings.plans(),
                receipts.clone(),
            );
        }
        // Wake at the retry deadline even when the Monitor state is unchanged.
        let due = delivery
            .in_flight
            .is_none()
            .then(|| delivery.queued.next_due())
            .flatten();
        tokio::select! {
            changed = snapshots.changed() => if changed.is_err() { break; },
            _ = async {
                match due {
                    Some(due) => tokio::time::sleep_until(due.into()).await,
                    None => std::future::pending::<()>().await,
                }
            } => {},
            receipt = completed.recv() => if let Some(receipt) = receipt {
                let active = delivery.in_flight.as_ref().is_some_and(|(plan, notice)| bindings.lookup(&notice.key, &plan.category()).is_some());
                if !active { remove_session_notification(&receipt.key); }
                delivery.complete(receipt, active, Instant::now());
            },
            event = input.recv() => {
                let Some(event) = event else { break; };
                if let Some(binding) = bindings.lookup(&event.key, &event.category) {
                    if let Some(index) = event.action.strip_prefix("AGENT_REPLY_").and_then(|i| i.parse::<usize>().ok()) {
                        if let Some(response) = binding.plan.response(index, event.text.as_deref()) {
                            submit_reply(&monitor, &binding.session, response).await;
                        } else { present_button(binding, index, &monitor, &ui).await; }
                    } else if event.action == unsafe { UNNotificationDefaultActionIdentifier.to_string() } {
                        let form = !binding.session.interaction().questions.is_empty();
                        let _ = ui.try_send(Event::ShowRequest { session: Box::new(binding.session), form, draft: None });
                    }
                } else if event.action == unsafe { UNNotificationDefaultActionIdentifier.to_string() } {
                    let _ = ui.try_send(Event::Platform(crate::macos::bridge::PlatformCommand::Open(event.target)));
                }
            }
        }
    }
    for key in &known {
        remove_session_notification(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        interaction::Interaction,
        session::{Activity, Approval, Client, Session},
    };
    fn notice() -> Notice {
        Notice {
            key: "request".into(),
            title: "Chat".into(),
            body: "Continue?".into(),
            target: "target".into(),
            token: "ticket".into(),
        }
    }
    fn session() -> Session {
        Session {
            agent: "codex",
            id: "chat".into(),
            title: "Chat".into(),
            cwd: "/work".into(),
            workspace: None,
            git_branch: None,
            updated_at: 0,
            client: Client::VSCode,
            activity: Activity::Confirmation,
            last_message: String::new(),
            interaction_data: Interaction::default(),
            target: Some("target".into()),
            authoritative: true,
            approval: Some(Approval {
                id: "ticket".into(),
                body: "Answer in editor".into(),
                expires: u64::MAX,
                can_allow: false,
            }),
        }
    }
    #[tokio::test]
    async fn notification_reply_waits_for_capacity_without_losing_the_answer() {
        use std::future::Future;
        let (tx, mut commands) = tokio::sync::mpsc::channel(1);
        let monitor = monitor::Handle {
            tx,
            state: tokio::sync::watch::channel(std::sync::Arc::new(monitor::State::default())).1,
        };
        monitor.send(monitor::Command::Refresh).unwrap();
        let session = session();
        let response = crate::domain::interaction::Response::Action("allow".into());
        let mut pending = Box::pin(submit_reply(&monitor, &session, response.clone()));
        std::future::poll_fn(|cx| {
            assert!(pending.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert!(matches!(
            commands.recv().await,
            Some(monitor::Command::Refresh)
        ));
        pending.await;
        assert!(
            matches!(commands.try_recv(), Ok(monitor::Command::Submit { session: sent, response: answer }) if sent.key() == session.key() && answer == response)
        );
    }
    #[test]
    fn failed_delivery_retries_with_same_actions_after_backoff() {
        let now = Instant::now();
        let plan = Plan::new(&Interaction::allow());
        let category = plan.category();
        let mut queue = DeliveryQueue {
            in_flight: Some((plan, notice())),
            ..Default::default()
        };
        queue.complete(
            Receipt {
                key: "request".into(),
                result: Err("temporarily unavailable".into()),
            },
            true,
            now,
        );
        assert!(queue.in_flight.is_none());
        assert!(
            queue
                .queued
                .pop_ready(now + Duration::from_secs(4))
                .is_none()
        );
        let retry = queue
            .queued
            .pop_ready(now + Duration::from_secs(5))
            .unwrap();
        assert_eq!(retry.0.category(), category);
        queue.in_flight = Some(retry);
        queue.complete(
            Receipt {
                key: "request".into(),
                result: Ok(()),
            },
            true,
            now,
        );
        assert!(queue.queued.next_due().is_none());
    }
    #[test]
    fn resolved_request_is_not_retried_after_late_failure() {
        let mut queue = DeliveryQueue {
            in_flight: Some((Plan::new(&Interaction::allow()), notice())),
            ..Default::default()
        };
        queue.complete(
            Receipt {
                key: "request".into(),
                result: Err("failure".into()),
            },
            false,
            Instant::now(),
        );
        assert!(queue.in_flight.is_none());
        assert!(queue.queued.next_due().is_none());
    }
    #[tokio::test]
    async fn editor_button_commands_monitor_navigation_instead_of_opening_menu() {
        let session = session();
        let key = session.key();
        let binding = Binding {
            plan: Plan::new(&session.interaction()),
            session,
        };
        let (tx, mut commands) = tokio::sync::mpsc::channel(4);
        let monitor = monitor::Handle {
            tx,
            state: tokio::sync::watch::channel(std::sync::Arc::new(monitor::State::default())).1,
        };
        let (ui, mut events) = tokio::sync::mpsc::channel(4);
        present_button(binding, 0, &monitor, &ui).await;
        assert!(matches!(commands.try_recv(), Ok(monitor::Command::Open(value)) if value == key));
        assert!(events.try_recv().is_err());
    }
}
