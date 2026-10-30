//! Native notifications reconcile their own bindings against Monitor state.
use crate::macos::native::{NotificationInput, notify_passive, remove_session_notification};
use objc2_user_notifications::UNNotificationDefaultActionIdentifier;
pub async fn run(
    monitor: crate::actors::monitor::Handle,
    mut input: tokio::sync::mpsc::Receiver<NotificationInput>,
    ui: crate::macos::bridge::Sender,
) {
    use crate::macos::{
        notification_actions::{Bindings, Kind},
        notification_categories::Notice,
    };
    use std::collections::{HashSet, VecDeque};
    let mut snapshots = monitor.subscribe();
    let mut bindings = Bindings::default();
    let (mut known, mut notices) = (HashSet::new(), HashSet::new());
    let (receipts, mut completed) = tokio::sync::mpsc::channel::<String>(128);
    let mut queued = VecDeque::new();
    let mut busy = false;
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
        for session in &snapshot.sessions {
            if let Some(key) = session.approval_key()
                && !known.contains(&key)
                && let Some(plan) = bindings.bind(session)
            {
                let approval = session.approval.as_ref().unwrap();
                queued.push_back((
                    plan,
                    Notice {
                        key,
                        title: session.title.clone(),
                        body: approval.body.clone(),
                        target: session.open_target().unwrap_or_default(),
                        token: approval.id.clone(),
                    },
                ));
            }
        }
        known = active;
        for notice in &snapshot.notices {
            if notices.insert(notice.id) {
                notify_passive(&notice.content);
            }
        }
        if snapshot.stopping {
            for key in &known {
                remove_session_notification(key);
            }
            break;
        }
        if !busy {
            while let Some((plan, notice)) = queued.pop_front() {
                if bindings.active(&notice.key) {
                    busy = true;
                    crate::macos::notification_categories::deliver(
                        plan,
                        notice,
                        bindings.plans(),
                        receipts.clone(),
                    );
                    break;
                }
            }
        }
        tokio::select! {
            changed=snapshots.changed()=>if changed.is_err(){break;},
            receipt=completed.recv()=>if let Some(key)=receipt{busy=false;if !bindings.active(&key){remove_session_notification(&key);}},
            event=input.recv()=>{
                let Some(event)=event else{break;};
                if let Some(binding)=bindings.lookup(&event.key,&event.category){
                    if let Some(index)=event.action.strip_prefix("AGENT_REPLY_").and_then(|i|i.parse::<usize>().ok()){
                        if let Some(response)=binding.plan.response(index,event.text.as_deref()){let _=monitor.submit(&binding.session,response);}
                        else {let form=matches!(binding.plan.buttons.get(index).map(|b|&b.kind),Some(Kind::Form|Kind::More|Kind::Text));let _=ui.try_send(crate::macos::bridge::Event::ShowRequest{session:Box::new(binding.session),form,draft:None});}
                    }else if event.action==unsafe{UNNotificationDefaultActionIdentifier.to_string()}{let form=!binding.session.interaction().questions.is_empty();let _=ui.try_send(crate::macos::bridge::Event::ShowRequest{session:Box::new(binding.session),form,draft:None});}
                }else if event.action==unsafe{UNNotificationDefaultActionIdentifier.to_string()}{let _=ui.try_send(crate::macos::bridge::Event::Platform(crate::macos::bridge::PlatformCommand::Open(event.target)));}
            }
        }
    }
    for key in &known {
        remove_session_notification(key);
    }
}
