use super::{style, widgets};
use crate::{
    actors::{human::telegram, monitor},
    app::core,
    domain::{
        integrations::{Phase, Row},
        preferences::{Applications, LoginStatus, Preferences},
        shortcut::Shortcut,
    },
};
use anyhow::Result;
use gtk::{gdk, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    fs,
    rc::Rc,
};

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    applications: Applications,
    animation: bool,
    #[serde(default)]
    shortcut: Option<Shortcut>,
}
pub fn load_preferences() -> Preferences {
    let mut p = Preferences::default();
    if let Ok(bytes) = fs::read(core::state_home().join("preferences.json")) {
        if let Ok(saved) = serde_json::from_slice::<Saved>(&bytes) {
            p.applications = saved.applications;
            p.animation = saved.animation;
            p.shortcut = saved.shortcut;
        }
    }
    p.login = if autostart_path().is_file() {
        LoginStatus::Enabled
    } else {
        LoginStatus::Disabled
    };
    p
}
pub fn save_preferences(p: &Preferences) -> Result<()> {
    let dir = core::state_home();
    let mut f = tempfile::NamedTempFile::new_in(&dir)?;
    serde_json::to_writer(
        &mut f,
        &Saved {
            applications: p.applications.clone(),
            animation: p.animation,
            shortcut: p.shortcut.clone(),
        },
    )?;
    f.persist(dir.join("preferences.json"))?;
    Ok(())
}
fn autostart_path() -> std::path::PathBuf {
    core::xdg_home("XDG_CONFIG_HOME", ".config").join("autostart/ai-agent-monitor.desktop")
}
fn set_autostart(enabled: bool) -> Result<()> {
    let path = autostart_path();
    if !enabled {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        return Ok(());
    }
    fs::create_dir_all(path.parent().unwrap())?;
    let exe = std::env::current_exe()?
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    write!(
        file,
        "[Desktop Entry]\nType=Application\nName=AI Agent Monitor\nExec=\"{exe}\" background\nIcon=ai-agent-monitor\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )?;
    file.persist(path)?;
    Ok(())
}
fn publish_preferences(handle: &monitor::Handle, change: impl FnOnce(&mut Preferences)) {
    let mut p = handle.snapshot().preferences.clone();
    change(&mut p);
    if let Err(e) = save_preferences(&p) {
        p.error = Some(format!("Could not save preferences: {e}"));
    }
    let _ = handle.send(monitor::Command::PreferencesObserved(p));
}
pub struct Settings {
    pub window: gtk::Window,
    stack: gtk::Stack,
    handle: monitor::Handle,
    apps: gtk::Box,
    integrations: gtk::Box,
    telegram: gtk::Box,
    telegram_note: RefCell<Option<gtk::Label>>,
    apps_signature: RefCell<String>,
    integration_signature: RefCell<String>,
    telegram_signature: RefCell<String>,
    auto_open: Rc<Cell<bool>>,
    previous_link: RefCell<Option<String>>,
    error: gtk::Label,
    height: Cell<i32>,
}
impl Settings {
    pub fn new(handle: monitor::Handle) -> Rc<Self> {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("AI Agent Monitor Settings");
        window.set_default_size(560, 1);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.add(&root);
        let toolbar = gtk::Toolbar::new();
        toolbar.set_toolbar_style(gtk::ToolbarStyle::Both);
        toolbar.set_show_arrow(false);
        root.pack_start(&toolbar, false, false, 0);
        let stack = gtk::Stack::new();
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        root.pack_start(&stack, true, true, 0);
        let general = page();
        let apps = page();
        let integrations = page();
        let telegram = page();
        for (id, title, icon, pane) in [
            (
                "general",
                "General",
                "preferences-system-symbolic",
                &general,
            ),
            (
                "applications",
                "Applications",
                "view-app-grid-symbolic",
                &apps,
            ),
            (
                "integrations",
                "Integrations",
                "application-x-addon-symbolic",
                &integrations,
            ),
            ("telegram", "Telegram", "mail-send-symbolic", &telegram),
        ] {
            stack.add_named(pane, id);
            let image = gtk::Image::from_icon_name(Some(icon), gtk::IconSize::LargeToolbar);
            let button = gtk::ToolButton::new(Some(&image), Some(title));
            button.set_is_important(true);
            let target = stack.clone();
            let id = id.to_string();
            button.connect_clicked(move |_| target.set_visible_child_name(&id));
            toolbar.insert(&button, -1);
        }
        let error = widgets::label("", "error", true);
        root.pack_end(&error, false, false, 6);
        let this = Rc::new(Self {
            window,
            stack,
            handle,
            apps,
            integrations,
            telegram,
            telegram_note: RefCell::new(None),
            apps_signature: RefCell::new(String::new()),
            integration_signature: RefCell::new(String::new()),
            telegram_signature: RefCell::new(String::new()),
            auto_open: Rc::new(Cell::new(false)),
            previous_link: RefCell::new(None),
            error,
            height: Cell::new(0),
        });
        this.general(&general);
        let weak = Rc::downgrade(&this);
        this.stack.connect_visible_child_notify(move |_| {
            if let Some(s) = weak.upgrade() {
                s.resize();
            }
        });
        this.window.connect_delete_event(|w, _| {
            w.hide();
            glib::Propagation::Stop
        });
        this.sync(&this.handle.snapshot());
        this.window.show_all();
        this.error.hide();
        this.resize();
        this
    }
    fn general(&self, pane: &gtk::Box) {
        let state = self.handle.snapshot();
        let login = gtk::CheckButton::with_label("Launch at login");
        login.set_active(state.preferences.login == LoginStatus::Enabled);
        pane.pack_start(&login, false, false, 0);
        pane.pack_start(
            &widgets::label(
                "Start the monitor when you sign in to Ubuntu.",
                "secondary",
                true,
            ),
            false,
            false,
            0,
        );
        let h = self.handle.clone();
        login.connect_toggled(move |b| {
            let enabled = b.is_active();
            let result = set_autostart(enabled);
            publish_preferences(&h, |p| {
                p.login = if enabled {
                    LoginStatus::Enabled
                } else {
                    LoginStatus::Disabled
                };
                if let Err(e) = result {
                    p.error = Some(format!("Could not change launch at login: {e}"));
                    p.login = LoginStatus::Unavailable;
                }
            });
        });
        let animation = gtk::CheckButton::with_label("Menu bar animation");
        animation.set_active(state.preferences.animation);
        let h = self.handle.clone();
        animation.connect_toggled(move |b| {
            let _ = h.send(monitor::Command::SetAnimation(b.is_active()));
        });
        pane.pack_start(&animation, false, false, 0);
        pane.pack_start(
            &widgets::label("Keyboard shortcut", "question", false),
            false,
            false,
            4,
        );
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let text = state
            .preferences
            .shortcut
            .as_ref()
            .map(|s| s.key_name.as_str())
            .unwrap_or("Record shortcut");
        let recorder = widgets::action(text);
        let recording = Rc::new(Cell::new(false));
        let r = recording.clone();
        recorder.connect_clicked(move |b| {
            r.set(true);
            b.set_label("Press a shortcut…");
            b.grab_focus();
        });
        let h = self.handle.clone();
        let r = recording.clone();
        recorder.connect_key_press_event(move |b, event| {
            if !r.get() {
                return glib::Propagation::Proceed;
            }
            if event.keyval() == gdk::keys::constants::Escape {
                r.set(false);
                b.set_label(
                    h.snapshot()
                        .preferences
                        .shortcut
                        .as_ref()
                        .map(|s| s.key_name.as_str())
                        .unwrap_or("Record shortcut"),
                );
                return glib::Propagation::Stop;
            }
            let mods = event.state()
                & (gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::MOD1_MASK
                    | gdk::ModifierType::SUPER_MASK
                    | gdk::ModifierType::SHIFT_MASK);
            if !mods.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::MOD1_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) {
                b.set_label("Use Control, Alt, or Super");
                return glib::Propagation::Stop;
            }
            let key = event.keyval();
            let name = gtk::accelerator_get_label(*key, mods)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Shortcut".into());
            b.set_label(&name);
            r.set(false);
            publish_preferences(&h, |p| {
                p.shortcut = Some(Shortcut {
                    key_code: *key,
                    modifiers: mods.bits(),
                    key_name: name,
                })
            });
            glib::Propagation::Stop
        });
        row.pack_start(&recorder, false, false, 0);
        let clear = widgets::action("Clear");
        let h = self.handle.clone();
        let b = recorder.clone();
        clear.connect_clicked(move |_| {
            b.set_label("Record shortcut");
            publish_preferences(&h, |p| p.shortcut = None);
        });
        row.pack_start(&clear, false, false, 0);
        pane.pack_start(&row, false, false, 0);
        pane.pack_start(&widgets::label("Control, Alt, or Super is required. The GNOME panel extension handles the global shortcut.","secondary",true),false,false,0);
    }
    pub fn sync(&self, state: &monitor::State) {
        self.error
            .set_text(state.preferences.error.as_deref().unwrap_or(""));
        self.error.set_visible(state.preferences.error.is_some());
        let signature = serde_json::to_string(&state.preferences.applications).unwrap_or_default();
        if *self.apps_signature.borrow() != signature {
            self.apps_signature.replace(signature);
            clear(&self.apps);
            for app in state.preferences.applications.list() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                let client = state
                    .sessions
                    .iter()
                    .find(|s| s.client.application_id() == app.id)
                    .map(|s| s.client)
                    .unwrap_or_default();
                row.pack_start(&widgets::application_icon(client), false, false, 0);
                row.pack_start(&widgets::label(&app.name, "", false), true, true, 0);
                let switch = gtk::Switch::new();
                switch.set_active(app.enabled);
                switch.set_valign(gtk::Align::Center);
                let id = app.id.clone();
                let h = self.handle.clone();
                switch.connect_state_set(move |_, enabled| {
                    let _ = h.send(monitor::Command::SetApplicationEnabled {
                        id: id.clone(),
                        enabled,
                    });
                    glib::Propagation::Proceed
                });
                row.pack_end(&switch, false, false, 0);
                self.apps.pack_start(&row, false, false, 0);
            }
            self.apps.show_all();
        }
        let signature = format!(
            "{:?}:{}:{}",
            state.integrations, state.integration_busy, state.integration_note
        );
        if *self.integration_signature.borrow() != signature {
            self.integration_signature.replace(signature);
            clear(&self.integrations);
            for row in &state.integrations {
                let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                header.pack_start(
                    &widgets::label(row.agent.name(), "question", false),
                    false,
                    false,
                    0,
                );
                header.pack_start(
                    &widgets::label(
                        &format!(
                            "{} {}",
                            if row.phase == Phase::Installed {
                                "✓"
                            } else {
                                "!"
                            },
                            row.status()
                        ),
                        "secondary",
                        false,
                    ),
                    false,
                    false,
                    0,
                );
                self.integrations.pack_start(&header, false, false, 0);
                self.integrations.pack_start(
                    &widgets::label(&row.detail, "secondary", true),
                    false,
                    false,
                    0,
                );
            }
            let setup = widgets::action(if state.integration_busy {
                "Setting up…"
            } else {
                "Check & set up"
            });
            setup.set_sensitive(!state.integration_busy);
            let h = self.handle.clone();
            setup.connect_clicked(move |_| check_integrations(h.clone(), true));
            self.integrations.pack_start(&setup, false, false, 0);
            self.integrations.pack_start(
                &widgets::label(&state.integration_note, "secondary", true),
                false,
                false,
                0,
            );
            self.integrations.show_all();
        }
        self.sync_telegram(&state.telegram);
        self.resize();
    }
    fn sync_telegram(&self, status: &crate::domain::telegram::Status) {
        let signature = format!(
            "{}:{}:{}:{}",
            status.connected, status.paired, status.setting_up, status.bot_username
        );
        if *self.telegram_signature.borrow() != signature {
            self.telegram_signature.replace(signature);
            clear(&self.telegram);
            self.telegram_note.replace(None);
            if telegram_ready(status) {
                self.telegram.pack_start(
                    &widgets::label("Telegram is connected", "question", false),
                    false,
                    false,
                    0,
                );
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.pack_start(
                    &widgets::label(&format!("@{}", status.bot_username), "", false),
                    false,
                    false,
                    0,
                );
                let test = widgets::action("Test");
                let h = self.handle.clone();
                test.connect_clicked(move |_| {
                    let _ = h.send(monitor::Command::Telegram(telegram::Event::Test));
                });
                row.pack_start(&test, false, false, 0);
                let open = widgets::action("Open");
                let target = format!("https://t.me/{}", status.bot_username);
                open.connect_clicked(move |_| {
                    let _ = super::open(&target);
                });
                row.pack_start(&open, false, false, 0);
                self.telegram.pack_start(&row, false, false, 0);
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let change = widgets::action("Change bot");
                let h = self.handle.clone();
                change.connect_clicked(move |_| {
                    let _ = h.send(monitor::Command::Telegram(telegram::Event::BeginSetup));
                });
                row.pack_start(&change, false, false, 0);
                let unlink = widgets::action("Unlink bot");
                let h = self.handle.clone();
                unlink.connect_clicked(move |_| {
                    let _ = h.send(monitor::Command::Telegram(telegram::Event::Disconnect));
                });
                row.pack_start(&unlink, false, false, 0);
                self.telegram.pack_start(&row, false, false, 0);
            } else {
                self.telegram_setup(status.connected && status.paired);
            }
            let note = widgets::label("", "secondary", true);
            self.telegram.pack_start(&note, false, false, 0);
            self.telegram_note.replace(Some(note));
            self.telegram.show_all();
        }
        if let Some(note) = self.telegram_note.borrow().as_ref() {
            let routine = format!("@{} · Connected", status.bot_username);
            note.set_text(if telegram_ready(status) && status.text == routine {
                ""
            } else {
                &status.text
            });
        }
        if self.auto_open.get() && status.link != *self.previous_link.borrow() {
            if let Some(link) = &status.link {
                let _ = super::open(link);
                self.auto_open.set(false);
            }
        }
        self.previous_link.replace(status.link.clone());
    }
    fn telegram_setup(&self, replacing: bool) {
        self.telegram.pack_start(
            &widgets::label("Set up Telegram integration", "question", false),
            false,
            false,
            0,
        );
        self.telegram.pack_start(&widgets::label("We chose personal Telegram bots over shared ones to protect your privacy. Your bot connects directly to the app running on your computer, so your conversations never pass through our servers — only Telegram's.","secondary",true),false,false,0);
        self.telegram.pack_start(
            &widgets::label(
                "You can paste an existing bot's token directly.",
                "secondary",
                true,
            ),
            false,
            false,
            0,
        );
        if replacing {
            self.telegram.pack_start(
                &widgets::label(
                    "Your current bot stays connected until the replacement is activated.",
                    "secondary",
                    true,
                ),
                false,
                false,
                0,
            );
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.pack_start(
            &widgets::label("1. Open BotFather and send /newbot.", "", true),
            true,
            true,
            0,
        );
        let open = widgets::action("Open");
        open.connect_clicked(|_| {
            let _ = super::open("https://t.me/BotFather");
        });
        row.pack_start(&open, false, false, 0);
        self.telegram.pack_start(&row, false, false, 0);
        self.telegram.pack_start(
            &widgets::label("2. Choose the bot's name and username.", "", true),
            false,
            false,
            0,
        );
        let grid = gtk::Grid::new();
        grid.set_column_spacing(8);
        grid.set_row_spacing(6);
        let name = gtk::Entry::new();
        name.set_text("AI Agent Monitor");
        name.set_hexpand(true);
        let username = gtk::Entry::new();
        username.set_text(&suggest_username(false));
        username.set_hexpand(true);
        for (index, caption, field) in [(0, "Bot name", &name), (1, "Bot username", &username)] {
            grid.attach(&widgets::label(caption, "secondary", false), 0, index, 1, 1);
            grid.attach(field, 1, index, 1, 1);
            let copy = widgets::action("Copy");
            let f = field.clone();
            copy.connect_clicked(move |_| {
                gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_text(&f.text())
            });
            grid.attach(&copy, 3, index, 1, 1);
        }
        let regenerate = widgets::icon_button("view-refresh-symbolic", "Regenerate username");
        let u = username.clone();
        regenerate.connect_clicked(move |_| u.set_text(&suggest_username(true)));
        grid.attach(&regenerate, 2, 1, 1, 1);
        self.telegram.pack_start(&grid, false, false, 0);
        grid.attach(
            &widgets::label(
                "3. Copy the token from BotFather and paste it below.",
                "",
                true,
            ),
            0,
            2,
            4,
            1,
        );
        grid.attach(
            &widgets::label(
                "The token is in BotFather's last message after creating the bot.",
                "secondary",
                true,
            ),
            0,
            3,
            4,
            1,
        );
        let token = gtk::Entry::new();
        token.set_max_length(256);
        token.set_visibility(false);
        token.set_placeholder_text(Some("Paste the entire token from BotFather"));
        token.set_hexpand(true);
        grid.attach(&widgets::label("Bot token", "secondary", false), 0, 4, 1, 1);
        grid.attach(&token, 1, 4, 1, 1);
        let paste = widgets::action("Paste");
        let e = token.clone();
        paste.connect_clicked(move |_| {
            let e = e.clone();
            gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).request_text(move |_, text| {
                if let Some(text) = text {
                    e.set_text(text);
                }
            });
        });
        grid.attach(&paste, 3, 4, 1, 1);
        self.telegram.pack_start(
            &widgets::label(
                "Your bot token will be stored securely in Ubuntu's keyring.",
                "secondary",
                true,
            ),
            false,
            false,
            0,
        );
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let connect = widgets::action("Connect");
        let h = self.handle.clone();
        let auto = self.auto_open.clone();
        connect.connect_clicked(move |_| {
            auto.set(true);
            let _ = h.send(monitor::Command::Telegram(telegram::Event::Connect(
                token.text().to_string(),
            )));
        });
        row.pack_start(&connect, false, false, 0);
        let cancel = widgets::action("Cancel");
        let h = self.handle.clone();
        let auto = self.auto_open.clone();
        cancel.connect_clicked(move |_| {
            auto.set(false);
            let _ = h.send(monitor::Command::Telegram(telegram::Event::CancelSetup));
        });
        row.pack_start(&cancel, false, false, 0);
        self.telegram.pack_start(&row, false, false, 0);
    }
    fn resize(&self) {
        if self.window.is_visible() {
            let height = self
                .window
                .child()
                .map(|root| root.preferred_height_for_width(560).1)
                .unwrap_or(180);
            if self.height.replace(height) != height {
                let (x, y) = self.window.position();
                self.window.resize(560, height.max(1));
                self.window.move_(x, y);
            }
        }
    }
}
fn page() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 10);
    style::class(&page, "settings-page");
    page
}
fn clear(container: &gtk::Box) {
    for c in container.children() {
        container.remove(&c);
    }
}
fn suggest_username(regenerate: bool) -> String {
    let name = glib::real_name().to_string_lossy().to_string();
    let c = std::ffi::CString::new(name).unwrap_or_default();
    // SAFETY: valid nul-terminated input, GLib returns an owned nul-terminated string.
    let latin: glib::GString = unsafe {
        glib::translate::from_glib_full(glib::ffi::g_str_to_ascii(c.as_ptr(), std::ptr::null()))
    };
    let mut base = String::new();
    for ch in latin.chars() {
        if ch.is_ascii_alphanumeric() {
            base.push(ch.to_ascii_lowercase());
        } else if !base.is_empty() && !base.ends_with('_') {
            base.push('_');
        }
    }
    if base.trim_matches('_').is_empty() {
        base = "my".into();
    }
    let suffix = if regenerate {
        format!("_{}_bot", &crate::app::ids::nonce()[..4])
    } else {
        "_bot".into()
    };
    base.truncate(32 - 8 - suffix.len());
    format!("aiagmon_{}{suffix}", base.trim_end_matches('_'))
}
pub fn check_integrations(handle: monitor::Handle, install: bool) {
    let _ = handle.send(monitor::Command::IntegrationsObserved {
        rows: handle.snapshot().integrations.clone(),
        busy: true,
        note: String::new(),
    });
    std::thread::spawn(move || {
        let executable = core::bundle_executable();
        let integrations = crate::app::setup::discover(
            &core::home(),
            core::codex_home(),
            crate::agents::claude::home(),
        );
        let mut rows = Vec::new();
        for integration in integrations {
            let result = if install {
                integration
                    .install(&executable)
                    .and_then(|_| integration.installed(&executable))
            } else {
                integration.installed(&executable)
            };
            let (phase, detail) = match result {
                Ok(true) => (Phase::Installed, integration.agent.next_step().into()),
                Ok(false) => (Phase::Detected, "Hooks are not installed yet.".into()),
                Err(e) => (Phase::Failed, format!("{e:#}")),
            };
            rows.push(Row {
                agent: integration.agent,
                phase,
                detail,
                directory: integration.directory.display().to_string(),
            });
        }
        let _ = handle.publish_from_worker(monitor::Command::IntegrationsObserved {
            rows,
            busy: false,
            note: if install {
                "Setup complete. Restart agents that need hooks.".into()
            } else {
                String::new()
            },
        });
    });
}

fn telegram_ready(status: &crate::domain::telegram::Status) -> bool {
    status.connected && status.paired && !status.setting_up
}
#[cfg(test)]
mod tests {
    #[test]
    fn a_valid_token_is_not_a_completed_telegram_connection() {
        let mut status = crate::domain::telegram::Status {
            connected: true,
            ..Default::default()
        };
        assert!(!super::telegram_ready(&status));
        status.paired = true;
        assert!(super::telegram_ready(&status));
        status.setting_up = true;
        assert!(!super::telegram_ready(&status));
    }
}
