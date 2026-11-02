//! Typed D-Bus bridge to the small GNOME Shell panel extension.
use super::ui::UiEvent;
use crate::actors::monitor::State;
use gtk::{gio, glib, prelude::*};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc::Sender},
};

pub struct Panel {
    connection: gio::DBusConnection,
    registration: Option<gio::RegistrationId>,
    owner: Option<gio::OwnerId>,
    watch: Option<Box<dyn FnOnce()>>,
    values: Arc<Mutex<HashMap<String, glib::Variant>>>,
}
impl Panel {
    pub fn new(events: Sender<UiEvent>) -> Result<Self, glib::Error> {
        let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
        let xml = r#"<node><interface name="io.github.melsomino.AIAgentMonitor"><method name="Toggle"/><method name="Show"/><method name="Hide"/><method name="ShortcutFailed"><arg type="s" direction="in"/></method><property name="Visible" type="b" access="read"/><property name="Working" type="u" access="read"/><property name="Pending" type="u" access="read"/><property name="Finished" type="u" access="read"/><property name="Animation" type="b" access="read"/><property name="Shortcut" type="s" access="read"/></interface></node>"#;
        let node = gio::DBusNodeInfo::for_xml(xml)?;
        let interface = node
            .lookup_interface("io.github.melsomino.AIAgentMonitor")
            .unwrap();
        let values = Arc::new(Mutex::new(HashMap::from([
            ("Visible".into(), false.to_variant()),
            ("Working".into(), 0u32.to_variant()),
            ("Pending".into(), 0u32.to_variant()),
            ("Finished".into(), 0u32.to_variant()),
            ("Animation".into(), true.to_variant()),
            ("Shortcut".into(), "".to_variant()),
        ])));
        let read = values.clone();
        let calls = events.clone();
        let registration = connection.register_object(
            "/io/github/melsomino/AIAgentMonitor",
            &interface,
            move |_, _, _, _, method, parameters, invocation| {
                let event = match method {
                    "Toggle" => UiEvent::Toggle,
                    "Show" => UiEvent::Show,
                    "Hide" => UiEvent::Hide,
                    "ShortcutFailed" => UiEvent::ShortcutError(
                        parameters
                            .child_value(0)
                            .get::<String>()
                            .unwrap_or_else(|| "Could not register shortcut".into()),
                    ),
                    _ => {
                        invocation.return_dbus_error(
                            "io.github.melsomino.AIAgentMonitor.UnknownMethod",
                            "Unsupported panel command",
                        );
                        return;
                    }
                };
                let _ = calls.send(event);
                invocation.return_value(None);
            },
            move |_, _, _, _, name| {
                read.lock()
                    .unwrap()
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| false.to_variant())
            },
            |_, _, _, _, _, _| false,
        )?;
        let owner = gio::bus_own_name_on_connection(
            &connection,
            "io.github.melsomino.AIAgentMonitor",
            gio::BusNameOwnerFlags::NONE,
            |_, _| {},
            |_, _| {},
        );
        let appeared = events.clone();
        let watch = gio::bus_watch_name_on_connection(
            &connection,
            "io.github.melsomino.AIAgentMonitor.Panel",
            gio::BusNameWatcherFlags::NONE,
            move |_, _, _| {
                let _ = appeared.send(UiEvent::Attached(true));
            },
            move |_, _| {
                let _ = events.send(UiEvent::Attached(false));
            },
        );
        Ok(Self {
            connection,
            registration: Some(registration),
            owner: Some(owner),
            watch: Some(Box::new(move || gio::bus_unwatch_name(watch))),
            values,
        })
    }
    pub fn activate_editor(&self, target: &str, timestamp: u32) -> bool {
        if !target.starts_with("vscode://file/") {
            return false;
        }
        self.connection
            .call_sync(
                Some("io.github.melsomino.AIAgentMonitor.Panel"),
                "/io/github/melsomino/AIAgentMonitor/Panel",
                "io.github.melsomino.AIAgentMonitor.Panel",
                "ActivateEditor",
                Some(&(target, timestamp).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                1000,
                gio::Cancellable::NONE,
            )
            .ok()
            .and_then(|reply| reply.get::<(bool,)>())
            .is_some_and(|(active,)| active)
    }
    pub fn publish(&self, state: &State, visible: bool) {
        let sessions = state
            .sessions
            .iter()
            .filter(|s| state.preferences.applications.enabled(s.client))
            .collect::<Vec<_>>();
        let shortcut = state
            .preferences
            .shortcut
            .as_ref()
            .map(|s| {
                gtk::accelerator_name(
                    s.key_code,
                    gtk::gdk::ModifierType::from_bits_truncate(s.modifiers),
                )
                .map(|s| s.to_string())
                .unwrap_or_default()
            })
            .unwrap_or_default();
        let next: HashMap<String, glib::Variant> = HashMap::from([
            ("Visible".into(), visible.to_variant()),
            (
                "Working".into(),
                (sessions
                    .iter()
                    .filter(|s| s.activity == crate::domain::session::Activity::Working)
                    .count() as u32)
                    .to_variant(),
            ),
            (
                "Pending".into(),
                (sessions.iter().filter(|s| s.approval.is_some()).count() as u32).to_variant(),
            ),
            (
                "Finished".into(),
                (sessions
                    .iter()
                    .filter(|s| state.unseen_finished.contains_key(&s.key()))
                    .count() as u32)
                    .to_variant(),
            ),
            ("Animation".into(), state.preferences.animation.to_variant()),
            ("Shortcut".into(), shortcut.to_variant()),
        ]);
        let mut values = self.values.lock().unwrap();
        let changed = next
            .iter()
            .filter(|(k, v)| values.get(*k) != Some(*v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<HashMap<String, glib::Variant>>();
        *values = next;
        if !changed.is_empty() {
            let args = (
                "io.github.melsomino.AIAgentMonitor",
                changed,
                Vec::<String>::new(),
            )
                .to_variant();
            let _ = self.connection.emit_signal(
                None,
                "/io/github/melsomino/AIAgentMonitor",
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                Some(&args),
            );
        }
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        if let Some(w) = self.watch.take() {
            w();
        }
        if let Some(o) = self.owner.take() {
            gio::bus_unown_name(o);
        }
        if let Some(r) = self.registration.take() {
            let _ = self.connection.unregister_object(r);
        }
    }
}
