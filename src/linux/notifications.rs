//! Freedesktop notifications share request identities and retirement with the monitor.
use super::ui::UiEvent;
use crate::{
    actors::monitor,
    domain::{interaction::Response, session::Session},
};
use gtk::{gio, prelude::*};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    rc::Rc,
    sync::mpsc::Sender,
};
struct Notice {
    id: Option<u32>,
    session: Session,
    actions: HashMap<String, Response>,
}
pub struct Notifications {
    connection: Option<gio::DBusConnection>,
    notices: Rc<RefCell<HashMap<String, Notice>>>,
    seen: HashSet<String>,
    passive_seen: HashSet<u64>,
    passive_ids: Rc<RefCell<HashSet<u32>>>,
    subscription: Option<gio::SignalSubscriptionId>,
    icon: std::path::PathBuf,
}
impl Notifications {
    pub fn new(monitor: monitor::Handle, events: Sender<UiEvent>) -> Self {
        let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok();
        let notices = Rc::new(RefCell::new(HashMap::<String, Notice>::new()));
        let passive_ids = Rc::new(RefCell::new(HashSet::new()));
        let subscription = connection.as_ref().map(|bus| {
            let passive_ids = passive_ids.clone();
            let notices = notices.clone();
            bus.signal_subscribe(
                Some("org.freedesktop.Notifications"),
                Some("org.freedesktop.Notifications"),
                Some("ActionInvoked"),
                Some("/org/freedesktop/Notifications"),
                None,
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, args| {
                    if let Some((id, action)) = args.get::<(u32, String)>() {
                        if let Some(notice) = notices.borrow().values().find(|n| n.id == Some(id)) {
                            if let Some(response) = notice.actions.get(&action) {
                                let _ = monitor.submit(&notice.session, response.clone());
                            } else {
                                let _ = events.send(UiEvent::Reveal(notice.session.key()));
                            }
                        } else if passive_ids.borrow().contains(&id) {
                            let _ = events.send(UiEvent::Show);
                        }
                    }
                },
            )
        });
        let icon = crate::app::core::state_home().join("AppIcon-robots.png");
        let _ = std::fs::write(&icon, include_bytes!("../../assets/AppIcon-robots.png"));
        Self {
            connection,
            notices,
            seen: HashSet::new(),
            passive_seen: HashSet::new(),
            passive_ids,
            subscription,
            icon,
        }
    }
    pub fn update(&mut self, state: &monitor::State) {
        for notice in &state.notices {
            if self.passive_seen.insert(notice.id) {
                if let Some(bus) = &self.connection {
                    let args = (
                        "AI Agent Monitor",
                        0u32,
                        self.icon.to_string_lossy().as_ref(),
                        notice.content.title.as_str(),
                        notice.content.body.as_str(),
                        vec!["default", "Open monitor"],
                        HashMap::<String, gtk::glib::Variant>::new(),
                        -1i32,
                    )
                        .to_variant();
                    let ids = self.passive_ids.clone();
                    bus.call(
                        Some("org.freedesktop.Notifications"),
                        "/org/freedesktop/Notifications",
                        "org.freedesktop.Notifications",
                        "Notify",
                        Some(&args),
                        None,
                        gio::DBusCallFlags::NONE,
                        3000,
                        gio::Cancellable::NONE,
                        move |result| {
                            if let Ok(result) = result {
                                if let Some((id,)) = result.get::<(u32,)>() {
                                    let mut ids = ids.borrow_mut();
                                    ids.insert(id);
                                    if ids.len() > 128 {
                                        ids.clear();
                                        ids.insert(id);
                                    }
                                }
                            }
                        },
                    );
                }
            }
        }
        self.passive_seen
            .retain(|id| state.notices.iter().any(|n| n.id == *id));
        let active = state
            .sessions
            .iter()
            .filter(|s| {
                state.preferences.applications.enabled(s.client)
                    && s.approval
                        .as_ref()
                        .is_some_and(|a| a.expires > monitor::now())
            })
            .map(|s| (token(s), s))
            .collect::<HashMap<_, _>>();
        let gone = self
            .notices
            .borrow()
            .keys()
            .filter(|k| !active.contains_key(*k))
            .cloned()
            .collect::<Vec<_>>();
        for key in gone {
            if let Some(n) = self.notices.borrow_mut().remove(&key) {
                if let Some(id) = n.id {
                    self.close(id);
                }
            }
        }
        for (key, session) in active {
            if self.seen.contains(&key)
                || state
                    .submissions
                    .contains_key(&session.approval_key().unwrap())
            {
                continue;
            }
            self.seen.insert(key.clone());
            if self.seen.len() > 1024 {
                self.seen.retain(|k| self.notices.borrow().contains_key(k));
                self.seen.insert(key.clone());
            }
            let Some(bus) = &self.connection else {
                continue;
            };
            let interaction = session.interaction();
            let mut actions = HashMap::new();
            let mut captions = vec!["default".to_owned(), "Open monitor".to_owned()];
            for (i, action) in interaction.actions.iter().enumerate().take(4) {
                let id = format!("action-{i}");
                captions.extend([id.clone(), action.label.clone()]);
                actions.insert(id, Response::Action(action.id.clone()));
            }
            if interaction.questions.len() == 1 {
                let q = &interaction.questions[0];
                for (i, c) in q.choices.iter().enumerate().take(4) {
                    let id = format!("answer-{i}");
                    captions.extend([id.clone(), c.label.clone()]);
                    actions.insert(
                        id,
                        Response::Answers(BTreeMap::from([(q.id.clone(), vec![c.label.clone()])])),
                    );
                }
            }
            if interaction.questions.len() > 1
                || interaction
                    .questions
                    .first()
                    .is_some_and(|q| q.freeform || q.choices.len() > 4)
            {
                captions.extend(["questions".into(), "Answer questions…".into()]);
            }
            let body = session
                .approval
                .as_ref()
                .map(|a| a.body.as_str())
                .unwrap_or_default();
            let hints = HashMap::from([("desktop-entry", "ai-agent-monitor".to_variant())]);
            let args = (
                "AI Agent Monitor",
                0u32,
                self.icon.to_string_lossy().as_ref(),
                session.title.as_str(),
                body,
                captions,
                hints,
                -1i32,
            )
                .to_variant();
            self.notices.borrow_mut().insert(
                key.clone(),
                Notice {
                    id: None,
                    session: session.clone(),
                    actions,
                },
            );
            let notices = self.notices.clone();
            let bus_copy = bus.clone();
            bus.call(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "Notify",
                Some(&args),
                None,
                gio::DBusCallFlags::NONE,
                3000,
                gio::Cancellable::NONE,
                move |result| {
                    if let Ok(result) = result {
                        if let Some((id,)) = result.get::<(u32,)>() {
                            if let Some(n) = notices.borrow_mut().get_mut(&key) {
                                n.id = Some(id);
                            } else {
                                close(&bus_copy, id);
                            }
                        }
                    }
                },
            );
        }
    }
    fn close(&self, id: u32) {
        if let Some(bus) = &self.connection {
            close(bus, id);
        }
    }
}
fn token(session: &Session) -> String {
    format!(
        "{}:{:?}:{:?}",
        session.key(),
        session.approval,
        session.interaction_data
    )
}
fn close(bus: &gio::DBusConnection, id: u32) {
    bus.call(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "CloseNotification",
        Some(&(id,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        3000,
        gio::Cancellable::NONE,
        |_| {},
    );
}
impl Drop for Notifications {
    fn drop(&mut self) {
        if let (Some(bus), Some(id)) = (&self.connection, self.subscription.take()) {
            bus.signal_unsubscribe(id);
        }
        for n in self.notices.borrow().values() {
            if let Some(id) = n.id {
                self.close(id);
            }
        }
    }
}
