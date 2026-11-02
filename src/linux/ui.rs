//! Main-thread GTK owner. Only typed monitor commands cross the actor boundary.
use super::{model, notifications, panel, requests, settings, style, widgets};
use crate::{
    actors::monitor,
    domain::{
        layout,
        session::{Activity, Session},
    },
    platform,
};
use gtk::{gdk, glib, pango, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};

pub enum UiEvent {
    Toggle,
    Show,
    Hide,
    Attached(bool),
    Reveal(String),
    ShortcutError(String),
}
pub struct Desktop {
    pub window: gtk::Window,
    handle: monitor::Handle,
    state: RefCell<Arc<monitor::State>>,
    snapshots: RefCell<tokio::sync::watch::Receiver<Arc<monitor::State>>>,
    platform: RefCell<tokio::sync::mpsc::Receiver<platform::Event>>,
    events: RefCell<mpsc::Receiver<UiEvent>>,
    panel: panel::Panel,
    list: gtk::Box,
    scroll: gtk::ScrolledWindow,
    pinned: gtk::DrawingArea,
    groups: RefCell<Vec<(gtk::Box, Option<gtk::gdk_pixbuf::Pixbuf>)>>,
    footer: gtk::Box,
    usage: gtk::Box,
    filter_label: gtk::Label,
    empty: gtk::Label,
    error: gtk::Label,
    rows: RefCell<HashMap<String, Rc<Row>>>,
    order: RefCell<Vec<String>>,
    disclosure: RefCell<model::Disclosure>,
    query: RefCell<String>,
    activation_time: Cell<u32>,
    input: gtk::IMMulticontext,
    settings: RefCell<Option<Rc<settings::Settings>>>,
    notifications: RefCell<notifications::Notifications>,
    attached: Cell<bool>,
    phase: Rc<Cell<f64>>,
    timer: RefCell<Option<glib::SourceId>>,
    provider: RefCell<gtk::CssProvider>,
    last_frame_height: Cell<i32>,
    closed: Cell<bool>,
}
impl Desktop {
    pub fn new(
        handle: monitor::Handle,
        platform: tokio::sync::mpsc::Receiver<platform::Event>,
    ) -> anyhow::Result<Rc<Self>> {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("AI Agent Monitor");
        window.set_role("ai-agent-monitor-panel");
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_skip_taskbar_hint(true);
        window.set_type_hint(gdk::WindowTypeHint::Utility);
        window.set_default_size(408, 1);
        style::class(&window, "monitor");
        style::class(&window, "monitor-popover");
        window.set_app_paintable(true);
        if let Some(visual) =
            gtk::prelude::WidgetExt::screen(&window).and_then(|screen| screen.rgba_visual())
        {
            window.set_visual(Some(&visual));
        }
        let provider = style::install(&window);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        style::class(&root, "popover-surface");
        root.set_margin_start(8);
        root.set_margin_end(8);
        root.set_margin_top(4);
        root.set_margin_bottom(12);
        window.add(&root);
        let content = root.clone();
        window.connect_draw(move |window, c| {
            let _ = c.save();
            c.set_operator(gtk::cairo::Operator::Source);
            c.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = c.paint();
            c.set_operator(gtk::cairo::Operator::Over);
            let width = window.allocated_width() as f64;
            let height = window.allocated_height() as f64;
            for spread in (1..=6).rev() {
                let spread = spread as f64;
                c.set_source_rgba(0.0, 0.0, 0.0, 0.018);
                widgets::rounded(
                    c,
                    8.0 - spread,
                    7.0 - spread,
                    width - 16.0 + spread * 2.0,
                    height - 16.0 + spread * 2.0,
                    18.0 + spread,
                );
                let _ = c.fill();
            }
            widgets::rounded(c, 8.0, 4.0, width - 16.0, height - 16.0, 18.0);
            c.clip();
            gtk::render_background(
                &content.style_context(),
                c,
                8.0,
                4.0,
                width - 16.0,
                height - 16.0,
            );
            window.propagate_draw(&content, c);
            let _ = c.restore();
            let _ = c.save();
            let foreground = window.style_context().color(gtk::StateFlags::NORMAL);
            c.set_source_rgba(
                foreground.red(),
                foreground.green(),
                foreground.blue(),
                0.09,
            );
            c.set_line_width(1.0);
            widgets::rounded(c, 8.5, 4.5, width - 17.0, height - 17.0, 17.5);
            let _ = c.stroke();
            let _ = c.restore();
            glib::Propagation::Stop
        });
        let error = widgets::label("", "error", true);
        error.set_no_show_all(true);
        root.pack_start(&error, false, false, 6);
        let overlay = gtk::Overlay::new();
        let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_overlay_scrolling(true);
        scroll.set_propagate_natural_height(false);
        scroll.set_min_content_height(1);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.set_size_request(384, -1);
        style::class(&list, "session-list");
        scroll.add(&list);
        overlay.add(&scroll);
        let pinned = widgets::mouse_transparent_layer();
        overlay.add_overlay(&pinned);
        overlay.set_overlay_pass_through(&pinned, true);
        root.pack_start(&overlay, true, true, 0);
        let empty = widgets::label("No open sessions", "empty", true);
        list.pack_start(&empty, false, false, 0);
        let footer = gtk::Box::new(gtk::Orientation::Vertical, 6);
        style::class(&footer, "footer");
        let filter_label = widgets::label("", "filter", false);
        filter_label.set_no_show_all(true);
        footer.pack_start(&filter_label, false, false, 0);
        let usage = gtk::Box::new(gtk::Orientation::Vertical, 4);
        footer.pack_start(&usage, false, false, 0);
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        bottom.pack_start(
            &widgets::label(&format!("v{}", crate::app::core::VERSION), "version", false),
            true,
            true,
            0,
        );
        let gear = widgets::icon_button("emblem-system-symbolic", "Settings");
        let quit = widgets::icon_button("system-shutdown-symbolic", "Quit AI Agent Monitor");
        gear.set_can_focus(false);
        quit.set_can_focus(false);
        bottom.pack_end(&quit, false, false, 0);
        bottom.pack_end(&gear, false, false, 0);
        footer.pack_start(&bottom, false, false, 0);
        root.pack_end(&footer, false, false, 0);
        let (sender, events) = mpsc::channel();
        let panel = panel::Panel::new(sender.clone())?;
        let notifications = notifications::Notifications::new(handle.clone(), sender.clone());
        let snapshots = handle.subscribe();
        let state = handle.snapshot();
        let this = Rc::new(Self {
            window,
            handle,
            state: RefCell::new(state),
            snapshots: RefCell::new(snapshots),
            platform: RefCell::new(platform),
            events: RefCell::new(events),
            panel,
            list,
            scroll,
            pinned,
            groups: RefCell::new(Vec::new()),
            footer,
            usage,
            filter_label,
            empty,
            error,
            rows: RefCell::new(HashMap::new()),
            order: RefCell::new(Vec::new()),
            disclosure: RefCell::new(model::Disclosure::default()),
            query: RefCell::new(String::new()),
            activation_time: Cell::new(0),
            input: gtk::IMMulticontext::new(),
            settings: RefCell::new(None),
            notifications: RefCell::new(notifications),
            attached: Cell::new(false),
            phase: Rc::new(Cell::new(0.0)),
            timer: RefCell::new(None),
            provider: RefCell::new(provider),
            last_frame_height: Cell::new(0),
            closed: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        this.pinned.connect_draw(move |_, c| {
            if let Some(ui) = weak.upgrade() {
                ui.draw_application_icons(c);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&this);
        this.input.connect_commit(move |_, text| {
            if let Some(ui) = weak.upgrade() {
                ui.query.borrow_mut().push_str(text);
                gtk::prelude::GtkWindowExt::set_focus(&ui.window, gtk::Widget::NONE);
                ui.refresh();
            }
        });
        let input = this.input.clone();
        this.window
            .connect_realize(move |window| input.set_client_window(window.window().as_ref()));
        let weak = Rc::downgrade(&this);
        gear.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.open_settings();
            }
        });
        quit.connect_clicked(|_| gtk::main_quit());
        let weak = Rc::downgrade(&this);
        this.window.connect_delete_event(move |_, _| {
            if let Some(ui) = weak.upgrade() {
                ui.hide();
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&this);
        this.window.connect_key_press_event(move |_, event| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |ui| ui.key(event))
        });
        let weak = Rc::downgrade(&this);
        this.window.connect_focus_out_event(move |_, _| {
            let weak = weak.clone();
            glib::timeout_add_local_once(Duration::from_millis(100), move || {
                if let Some(ui) = weak.upgrade() {
                    if !ui.window.is_active()
                        && !ui
                            .settings
                            .borrow()
                            .as_ref()
                            .is_some_and(|s| s.window.is_active())
                    {
                        ui.hide();
                    }
                }
            });
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&this);
        this.scroll.vadjustment().connect_value_changed(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.pin_icons();
            }
        });
        let weak = Rc::downgrade(&this);
        this.list.connect_size_allocate(move |_, _| {
            if let Some(ui) = weak.upgrade() {
                ui.pin_icons();
            }
        });
        if let Some(settings) = gtk::Settings::default() {
            for property in ["gtk-theme-name", "gtk-application-prefer-dark-theme"] {
                let weak = Rc::downgrade(&this);
                settings.connect_notify_local(Some(property), move |_, _| {
                    if let Some(ui) = weak.upgrade() {
                        if let Some(screen) = gdk::Screen::default() {
                            gtk::StyleContext::remove_provider_for_screen(
                                &screen,
                                &*ui.provider.borrow(),
                            );
                        }
                        ui.provider.replace(style::install(&ui.window));
                        ui.refresh();
                    }
                });
            }
        }
        let weak = Rc::downgrade(&this);
        let timer = glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(ui) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if ui.closed.get() {
                return glib::ControlFlow::Break;
            }
            ui.tick();
            glib::ControlFlow::Continue
        });
        this.timer.replace(Some(timer));
        root.show_all();
        this.error.hide();
        this.filter_label.hide();

        settings::check_integrations(this.handle.clone(), false);
        this.refresh();
        Ok(this)
    }
    fn tick(self: &Rc<Self>) {
        let changed = match self.snapshots.borrow().has_changed() {
            Ok(changed) => changed,
            Err(_) => {
                gtk::main_quit();
                return;
            }
        };
        if changed {
            let state = self.snapshots.borrow_mut().borrow_and_update().clone();
            self.state.replace(state);
            self.refresh();
        }
        loop {
            let event = self.events.borrow().try_recv().ok();
            let Some(event) = event else {
                break;
            };
            match event {
                UiEvent::Toggle => {
                    if self.window.is_visible() {
                        self.hide()
                    } else {
                        self.show()
                    }
                }
                UiEvent::Show => self.show(),
                UiEvent::Hide => self.hide(),
                UiEvent::Attached(active) => self.attached.set(active),
                UiEvent::ShortcutError(error) => {
                    let mut preferences = self.handle.snapshot().preferences.clone();
                    preferences.error = Some(error);
                    let _ = self
                        .handle
                        .send(monitor::Command::PreferencesObserved(preferences));
                }
                UiEvent::Reveal(key) => {
                    if !self.window.is_visible() {
                        self.show();
                    }
                    self.set_expanded(&key, true, false);
                    self.focus_request(&key);
                }
            }
        }
        loop {
            let event = self.platform.borrow_mut().try_recv().ok();
            let Some(event) = event else {
                break;
            };
            match event {
                platform::Event::Toggle => self.show(),
                platform::Event::Platform(platform::PlatformCommand::Open(target)) => {
                    let timestamp = self.activation_time.replace(0);
                    if self.attached.get() && self.panel.activate_editor(&target, timestamp) {
                        self.hide();
                        continue;
                    }
                    match super::open_session(&target, timestamp) {
                        Ok(()) => self.hide(),
                        Err(error) => {
                            self.error.set_text(&error.to_string());
                            self.error.show();
                        }
                    }
                }
                platform::Event::Platform(platform::PlatformCommand::Applications(
                    applications,
                )) => {
                    let mut p = self.handle.snapshot().preferences.clone();
                    p.applications = applications;
                    if let Err(e) = settings::save_preferences(&p) {
                        self.error
                            .set_text(&format!("Could not save application preferences: {e}"));
                        self.error.show();
                    }
                }
                platform::Event::Platform(platform::PlatformCommand::Animation(animation)) => {
                    let mut p = self.handle.snapshot().preferences.clone();
                    p.animation = animation;
                    let _ = settings::save_preferences(&p);
                }
            }
        }
        if self.window.is_visible() {
            self.phase
                .set((self.phase.get() + 0.08) % std::f64::consts::TAU);
            for row in self.rows.borrow().values() {
                if row.session.borrow().activity == Activity::Working {
                    row.status_slot.queue_draw();
                }
            }
            self.resize_to_content();
        }
    }
    pub fn show(self: &Rc<Self>) {
        if !self.window.is_visible() {
            let state = self.handle.snapshot();
            let eligible = model::eligible(&state, "");
            self.query.borrow_mut().clear();
            self.disclosure.borrow_mut().show(&eligible);
            self.refresh();
            self.resize_to_content();
            self.window.show();
        }
        self.window.present();
        self.panel.publish(&self.state.borrow(), true);
        let request = self
            .order
            .borrow()
            .iter()
            .find(|key| {
                self.rows
                    .borrow()
                    .get(*key)
                    .is_some_and(|row| row.session.borrow().approval.is_some())
            })
            .cloned();
        if let Some(key) = request {
            let weak = Rc::downgrade(self);
            glib::idle_add_local_once(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.focus_request(&key);
                }
            });
        } else {
            gtk::prelude::GtkWindowExt::set_focus(&self.window, gtk::Widget::NONE);
        }
    }
    pub fn hide(&self) {
        self.window.hide();
        self.input.reset();
        self.query.borrow_mut().clear();
        self.disclosure.borrow_mut().close();
        gtk::prelude::GtkWindowExt::set_focus(&self.window, gtk::Widget::NONE);
        for row in self.rows.borrow().values() {
            row.revealer.set_reveal_child(false);
            row.card.borrow_mut().take();
            for child in row.card_slot.children() {
                row.card_slot.remove(&child);
            }
            row.card_token.borrow_mut().clear();
        }
        self.panel.publish(&self.state.borrow(), false);
    }
    pub fn close(&self) {
        self.closed.set(true);
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        if let Some(settings) = self.settings.borrow_mut().take() {
            settings.window.hide();
        }
        self.window.hide();
    }
    fn refresh(self: &Rc<Self>) {
        let state = self.state.borrow().clone();
        if state.stopping {
            gtk::main_quit();
            return;
        }
        self.notifications.borrow_mut().update(&state);
        self.panel.publish(&state, self.window.is_visible());
        if let Some(settings) = self.settings.borrow().as_ref() {
            settings.sync(&state);
        }
        let sessions = model::eligible(&state, &self.query.borrow());
        if self.disclosure.borrow().open {
            self.disclosure
                .borrow_mut()
                .observe(&model::eligible(&state, ""));
        }
        let keys = sessions.iter().map(Session::key).collect::<Vec<_>>();
        let old_order = self.order.borrow().clone();
        // Removed or filtered rows are detached; their current-request drafts survive live updates.
        let live = state
            .sessions
            .iter()
            .map(Session::key)
            .collect::<std::collections::HashSet<_>>();
        let removed = self
            .rows
            .borrow()
            .keys()
            .filter(|k| !live.contains(*k))
            .cloned()
            .collect::<Vec<_>>();
        for key in removed {
            if let Some(row) = self.rows.borrow_mut().remove(&key) {
                self.clear_focus_inside(&row.root.clone().upcast());
                if let Some(parent) = row
                    .root
                    .parent()
                    .and_then(|w| w.downcast::<gtk::Container>().ok())
                {
                    parent.remove(&row.root);
                }
            }
        }
        if keys != old_order {
            for (group, _) in self.groups.borrow_mut().drain(..) {
                for child in group.children() {
                    group.remove(&child);
                }
                self.list.remove(&group);
            }
            let mut group: Option<(String, gtk::Box)> = None;
            for s in &sessions {
                let id = s.client.application_id();
                if group.as_ref().is_none_or(|(last, _)| last != &id) {
                    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    if group.is_some() {
                        style::class(&container, "following-group");
                    }
                    self.list.pack_start(&container, false, false, 0);
                    let icon = widgets::application_pixbuf(s.client);
                    self.groups.borrow_mut().push((container.clone(), icon));
                    group = Some((id, container));
                }
                let row = self
                    .rows
                    .borrow_mut()
                    .entry(s.key())
                    .or_insert_with(|| Row::new(self, s))
                    .clone();
                group
                    .as_ref()
                    .unwrap()
                    .1
                    .pack_start(&row.root, false, false, 0);
            }
            self.order.replace(keys);
        }
        for s in &sessions {
            let row = self.rows.borrow().get(&s.key()).cloned().unwrap();
            row.update(self, s, &state);
        }
        self.empty
            .set_text(model::empty_text(&state, &self.query.borrow()));
        self.list.show_all();
        self.empty.set_visible(sessions.is_empty());
        self.pinned.show_all();
        self.filter_label
            .set_text(&format!("Filter: {}", self.query.borrow()));
        self.filter_label
            .set_visible(!self.query.borrow().is_empty());
        self.error.set_text(
            state
                .discovery_error
                .as_deref()
                .or(state.preferences.error.as_deref())
                .unwrap_or(""),
        );
        self.error.set_visible(!self.error.text().is_empty());
        self.update_usage(&state);
        self.resize_to_content();
        self.pin_icons();
    }
    fn set_expanded(self: &Rc<Self>, key: &str, expanded: bool, acknowledge: bool) {
        self.disclosure
            .borrow_mut()
            .expanded
            .insert(key.into(), expanded);
        if acknowledge {
            self.seen(key);
        }
        if let Some(row) = self.rows.borrow().get(key) {
            row.revealer.set_reveal_child(expanded);
        }
        self.refresh();
    }
    fn seen(&self, key: &str) {
        if let Some(generation) = self.state.borrow().unseen_finished.get(key).copied() {
            let _ = self.handle.send(monitor::Command::FinishedSeen {
                completions: HashMap::from([(key.into(), generation)]),
            });
        }
    }
    fn open_settings(&self) {
        if self.settings.borrow().is_none() {
            self.settings
                .replace(Some(settings::Settings::new(self.handle.clone())));
        }
        if let Some(settings) = self.settings.borrow().as_ref() {
            settings.window.show();
            settings.window.present();
        }
    }
    fn clear_focus_inside(&self, widget: &gtk::Widget) {
        if let Some(focus) = gtk::prelude::GtkWindowExt::focused_widget(&self.window) {
            if focus == *widget || focus.is_ancestor(widget) {
                gtk::prelude::GtkWindowExt::set_focus(&self.window, gtk::Widget::NONE);
            }
        }
    }
    fn focus_request(&self, key: &str) {
        if let Some(row) = self.rows.borrow().get(key) {
            if let Some(card) = row.card.borrow().as_ref() {
                if let Some(target) = card.targets.iter().find(|w| w.is_sensitive()) {
                    target.grab_focus();
                    self.reveal_target(target);
                }
            }
        }
    }
    fn targets(&self) -> Vec<gtk::Widget> {
        let rows = self.rows.borrow();
        let disclosure = self.disclosure.borrow();
        let mut targets = Vec::new();
        for key in self.order.borrow().iter() {
            if let Some(row) = rows.get(key) {
                targets.push(row.title.clone().upcast());
                if disclosure.expanded.get(key) == Some(&true) {
                    if let Some(card) = row.card.borrow().as_ref() {
                        targets.extend(card.targets.iter().filter(|w| w.is_sensitive()).cloned());
                    }
                }
            }
        }
        targets
    }
    fn reveal_target(&self, target: &gtk::Widget) {
        if let Some((_, y)) = target.translate_coordinates(&self.list, 0, 0) {
            let a = self.scroll.vadjustment();
            a.clamp_page(y as f64, (y + target.allocated_height()) as f64);
        }
    }
    fn key(self: &Rc<Self>, event: &gdk::EventKey) -> glib::Propagation {
        use gdk::keys::constants as key;
        let value = event.keyval();
        let focus = gtk::prelude::GtkWindowExt::focused_widget(&self.window);
        if value == key::Escape {
            self.hide();
            return glib::Propagation::Stop;
        }
        if value != key::Up
            && value != key::Down
            && focus
                .as_ref()
                .is_some_and(|w| w.is::<gtk::Entry>() || w.is::<gtk::TextView>())
        {
            return glib::Propagation::Proceed;
        }
        if value == key::Up || value == key::Down {
            let targets = self.targets();
            if targets.is_empty() {
                return glib::Propagation::Stop;
            }
            let current = focus
                .as_ref()
                .and_then(|f| targets.iter().position(|w| w == f));
            let index = match (current, value == key::Down) {
                (Some(i), true) => (i + 1) % targets.len(),
                (Some(i), false) => (i + targets.len() - 1) % targets.len(),
                (None, true) => 0,
                (None, false) => targets.len() - 1,
            };
            targets[index].grab_focus();
            self.reveal_target(&targets[index]);
            return glib::Propagation::Stop;
        }
        if value == key::Left || value == key::Right {
            if let Some(focus) = &focus {
                let title = self
                    .rows
                    .borrow()
                    .values()
                    .find(|r| r.title.clone().upcast::<gtk::Widget>() == *focus)
                    .map(|r| r.session.borrow().key());
                if let Some(title) = title {
                    self.set_expanded(&title, value == key::Right, true);
                    return glib::Propagation::Stop;
                }
                if focus.is::<gtk::RadioButton>() {
                    return glib::Propagation::Proceed;
                }
                if let Some(parent) = focus.parent().and_then(|w| w.downcast::<gtk::Box>().ok()) {
                    if parent.orientation() == gtk::Orientation::Horizontal {
                        let siblings = self
                            .targets()
                            .into_iter()
                            .filter(|w| w.parent() == Some(parent.clone().upcast()))
                            .collect::<Vec<_>>();
                        if let Some(i) = siblings.iter().position(|w| w == focus) {
                            let next = if value == key::Right {
                                (i + 1) % siblings.len()
                            } else {
                                (i + siblings.len() - 1) % siblings.len()
                            };
                            siblings[next].grab_focus();
                            return glib::Propagation::Stop;
                        }
                    }
                }
            }
            return glib::Propagation::Proceed;
        }
        if value == key::Return || value == key::KP_Enter {
            if let Some(button) = focus.and_then(|w| w.downcast::<gtk::Button>().ok()) {
                button.emit_clicked();
                return glib::Propagation::Stop;
            }
        }
        if value == key::space
            && gtk::prelude::GtkWindowExt::focused_widget(&self.window)
                .is_some_and(|w| w.is::<gtk::Button>())
        {
            return glib::Propagation::Proceed;
        }
        if event.state().intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::MOD1_MASK
                | gdk::ModifierType::SUPER_MASK,
        ) {
            return glib::Propagation::Proceed;
        }
        if self.input.filter_keypress(event) {
            return glib::Propagation::Stop;
        }
        if value == key::BackSpace {
            model::pop_grapheme(&mut self.query.borrow_mut());
            self.refresh();
            return glib::Propagation::Stop;
        }
        if let Some(ch) = value.to_unicode() {
            if !ch.is_control() {
                self.query.borrow_mut().push(ch);
                gtk::prelude::GtkWindowExt::set_focus(&self.window, gtk::Widget::NONE);
                self.refresh();
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    }
    fn resize_to_content(&self) {
        let monitor = gdk::Display::default().and_then(|d| {
            self.window
                .window()
                .map(|w| d.monitor_at_window(&w))
                .flatten()
                .or_else(|| d.primary_monitor())
                .or_else(|| d.monitor(0))
        });
        let max = monitor
            .map(|m| {
                ((m.geometry().height() as f64 * 0.8 - 20.0) as i32).min(m.workarea().height() - 24)
            })
            .unwrap_or(640)
            .max(120);
        let content = self.list.preferred_height_for_width(384).1;
        let footer = self.footer.preferred_height_for_width(392).1;
        let error = if self.error.is_visible() {
            self.error.preferred_height().1 + 12
        } else {
            0
        };
        let height = (content + footer + error + 16).clamp(footer + 52, max);
        if self.last_frame_height.replace(height) != height {
            self.window.resize(408, height);
        }
    }
    fn pin_icons(&self) {
        self.pinned.queue_draw();
    }
    fn draw_application_icons(&self, c: &gtk::cairo::Context) {
        use gtk::gdk::prelude::GdkContextExt;
        let top = self.scroll.vadjustment().value();
        for (group, icon) in self.groups.borrow().iter() {
            let a = group.allocation();
            let start = group
                .children()
                .first()
                .and_then(|row| row.translate_coordinates(&self.list, 0, 0))
                .map_or(a.y(), |(_, y)| y);
            let y = layout::sticky_y(start as f64, (a.y() + a.height()) as f64, top) - top;
            if (a.y() + a.height()) as f64 > top
                && (a.y() as f64) < top + self.scroll.allocated_height() as f64
            {
                if let Some(icon) = icon {
                    let icon_y = (y + 6.0).round();
                    if let Some(bg) = self.window.style_context().lookup_color("theme_bg_color") {
                        c.set_source_rgba(bg.red(), bg.green(), bg.blue(), bg.alpha());
                        c.rectangle(8.0, icon_y, 24.0, 24.0);
                        let _ = c.fill();
                    }
                    c.set_source_pixbuf(icon, 8.0, icon_y);
                    let _ = c.paint();
                }
            }
        }
    }
    fn update_usage(&self, state: &monitor::State) {
        for c in self.usage.children() {
            self.usage.remove(&c);
        }
        if let Some(usage) = &state.usage {
            for w in &usage.windows {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
                row.pack_start(
                    &widgets::label(&format!("Codex {}", w.label()), "usage", false),
                    true,
                    true,
                    0,
                );
                row.pack_start(
                    &widgets::label(
                        &w.remaining(monitor::now() as i64)
                            .map(|p| format!("{p}%"))
                            .unwrap_or_else(|| "—".into()),
                        "usage",
                        false,
                    ),
                    false,
                    false,
                    0,
                );
                let date = w
                    .resets_at
                    .and_then(|t| glib::DateTime::from_unix_local(t).ok())
                    .and_then(|d| {
                        d.format(if w.minutes >= 1440 { "%b %-d" } else { "%H:%M" })
                            .ok()
                    })
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "—".into());
                let date = widgets::label(&date, "usage", false);
                date.set_tooltip_text(
                    w.resets_at
                        .map(|t| format!("Reset timestamp: {t}\nObserved: {}", usage.observed_at))
                        .as_deref(),
                );
                row.pack_start(&date, false, false, 0);
                self.usage.pack_start(&row, false, false, 0);
            }
        } else {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
            row.pack_start(
                &widgets::label("Codex usage", "usage", false),
                true,
                true,
                0,
            );
            row.pack_start(&widgets::label("—", "usage", false), false, false, 0);
            row.pack_start(&widgets::label("—", "usage", false), false, false, 0);
            self.usage.pack_start(&row, false, false, 0);
        }
        self.usage.show_all();
    }
}

struct Row {
    root: gtk::Overlay,
    title: gtk::Button,
    title_label: gtk::Label,
    robot: gtk::Button,
    chevron: gtk::Button,
    status_slot: gtk::Box,
    revealer: gtk::Revealer,
    detail: gtk::Box,
    messages: gtk::Box,
    card_slot: gtk::Box,
    card: RefCell<Option<requests::Card>>,
    card_token: RefCell<String>,
    metadata: gtk::Box,
    feedback: gtk::Label,
    session: RefCell<Session>,
    messages_token: RefCell<String>,
}
impl Row {
    fn new(ui: &Rc<Desktop>, session: &Session) -> Rc<Self> {
        let root = gtk::Overlay::new();
        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add(&main);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        style::class(&header, "session-header");
        main.pack_start(&header, false, false, 0);
        let robot = gtk::Button::new();
        style::class(&robot, "robot-button");
        robot.set_can_focus(false);
        header.pack_start(&robot, false, false, 0);
        let title = gtk::Button::new();
        style::class(&title, "session-title");
        let title_label = widgets::label("", "", false);
        title_label.set_ellipsize(pango::EllipsizeMode::End);
        title.add(&title_label);
        header.pack_start(&title, false, false, 0);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.pack_start(&spacer, true, true, 0);
        let status_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.pack_start(&status_slot, false, false, 0);
        let chevron = widgets::icon_button("pan-end-symbolic", "Show details");
        chevron.set_can_focus(false);
        header.pack_end(&chevron, false, false, 0);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        chevron.set_image(Some(&widgets::chevron(&revealer)));
        main.pack_start(&revealer, false, false, 0);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 8);
        detail.set_margin_top(4);
        detail.set_margin_bottom(8);
        revealer.add(&detail);
        let messages = gtk::Box::new(gtk::Orientation::Vertical, 10);
        detail.pack_start(&messages, false, false, 0);
        let card_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card_slot.set_margin_start(34);
        card_slot.set_margin_end(12);
        detail.pack_start(&card_slot, false, false, 0);
        let feedback = widgets::label("", "error", true);
        feedback.set_margin_start(34);
        feedback.set_no_show_all(true);
        detail.pack_start(&feedback, false, false, 0);
        let metadata = gtk::Box::new(gtk::Orientation::Vertical, 1);
        metadata.set_margin_start(34);
        metadata.set_margin_end(12);
        detail.pack_start(&metadata, false, false, 0);
        let row = Rc::new(Self {
            root,
            title,
            title_label,
            robot,
            chevron,
            status_slot,
            revealer,
            detail,
            messages,
            card_slot,
            card: RefCell::new(None),
            card_token: RefCell::new(String::new()),
            metadata,
            feedback,
            session: RefCell::new(session.clone()),
            messages_token: RefCell::new(String::new()),
        });
        let thread = widgets::mouse_transparent_layer();
        thread.set_size_request(34, -1);
        thread.set_halign(gtk::Align::Start);
        row.root.add_overlay(&thread);
        row.root.set_overlay_pass_through(&thread, true);
        let weak = Rc::downgrade(&row);
        thread.connect_draw(move |w, c| {
            let Some(row) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !row.revealer.reveals_child() && !row.revealer.is_child_revealed() {
                return glib::Propagation::Proceed;
            }
            let fg = w.style_context().color(gtk::StateFlags::NORMAL);
            let mut last = row
                .robot
                .image()
                .and_then(|image| {
                    image.translate_coordinates(&row.root, 0, image.allocated_height())
                })
                .map_or(23.5, |(_, y)| y as f64);
            let line_to = |start: f64, end: f64| {
                if end > start {
                    c.set_source_rgba(fg.red(), fg.green(), fg.blue(), 0.22);
                    c.set_line_width(0.5);
                    c.move_to(14.0, start);
                    c.line_to(14.0, end);
                    let _ = c.stroke();
                }
            };
            for (index, message) in row.messages.children().iter().enumerate() {
                if let Some((_, y)) = message.translate_coordinates(&row.root, 0, 0) {
                    let y = y as f64 + 8.0;
                    if index > 0 {
                        line_to(last, y - 2.5);
                        c.set_source_rgba(fg.red(), fg.green(), fg.blue(), 0.25);
                        c.arc(14.0, y, 2.5, 0.0, std::f64::consts::TAU);
                        let _ = c.fill();
                        // Leave the marker's contour clear before continuing.
                        last = y + 2.5;
                    }
                }
            }
            if row.session.borrow().approval.is_some() {
                if let Some((_, y)) = row.card_slot.translate_coordinates(&row.root, 0, 0) {
                    let y = y as f64 + 20.0;
                    line_to(last, y - 8.0);
                    c.set_source_rgb(1.0, 0.584, 0.0);
                    c.arc(14.0, y, 8.0, 0.0, std::f64::consts::TAU);
                    let _ = c.fill();
                    c.set_source_rgb(1.0, 1.0, 1.0);
                    c.set_line_width(1.6);
                    c.move_to(14.0, y - 4.0);
                    c.line_to(14.0, y + 0.8);
                    let _ = c.stroke();
                    c.arc(14.0, y + 4.0, 0.8, 0.0, std::f64::consts::TAU);
                    let _ = c.fill();
                    last = y + 8.0;
                }
            }
            if let Some((_, y)) =
                row.metadata
                    .translate_coordinates(&row.root, 0, row.metadata.allocated_height())
            {
                line_to(last, y as f64);
            }
            glib::Propagation::Proceed
        });
        for button in [&row.robot, &row.chevron] {
            let weak = Rc::downgrade(ui);
            let key = session.key();
            button.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    let expanded = ui
                        .disclosure
                        .borrow()
                        .expanded
                        .get(&key)
                        .copied()
                        .unwrap_or(false);
                    ui.set_expanded(&key, !expanded, true);
                }
            });
        }
        let weak = Rc::downgrade(ui);
        let key = session.key();
        row.title.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.activation_time.set(gtk::current_event_time());
                ui.seen(&key);
                if let Some(row) = ui.rows.borrow().get(&key) {
                    ui.handle.open(&row.session.borrow());
                }
            }
        });
        row
    }
    fn update(self: &Rc<Self>, ui: &Rc<Desktop>, session: &Session, state: &monitor::State) {
        self.session.replace(session.clone());
        self.title_label
            .set_markup(&model::title_markup(&session.title, &ui.query.borrow()));
        let layout = self.title.create_pango_layout(Some(&session.title));
        let width = layout.pixel_size().0.min(
            if session.activity == Activity::Idle
                && !state.unseen_finished.contains_key(&session.key())
            {
                284
            } else {
                250
            },
        );
        self.title_label.set_size_request(width, 22);
        self.title
            .set_tooltip_text(Some(&format!("Switch to {}", session.client.label())));
        let expanded = ui.disclosure.borrow().expanded.get(&session.key()) == Some(&true);
        self.revealer.set_transition_duration(
            if gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations()) {
                240
            } else {
                0
            },
        );
        self.revealer.set_reveal_child(expanded);

        self.robot
            .set_tooltip_text(Some(if expanded { "Collapse" } else { "Show details" }));
        self.robot
            .set_image(Some(&widgets::robot(session.activity)));
        for c in self.status_slot.children() {
            self.status_slot.remove(&c);
        }
        let finished = state.unseen_finished.contains_key(&session.key());
        if finished || session.activity != Activity::Idle {
            let icon = widgets::status(session.activity, finished, ui.phase.clone());
            if finished {
                let button = gtk::Button::new();
                style::class(&button, "icon-button");
                button.set_can_focus(false);
                button.set_tooltip_text(Some("Mark as seen"));
                button.add(&icon);
                let weak = Rc::downgrade(ui);
                let key = session.key();
                let generation = state.unseen_finished[&key];
                button.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        let _ = ui.handle.send(monitor::Command::FinishedSeen {
                            completions: HashMap::from([(key.clone(), generation)]),
                        });
                    }
                });
                self.status_slot.pack_start(&button, false, false, 0);
            } else {
                self.status_slot.pack_start(&icon, false, false, 4);
            }
        }
        let history = state
            .history
            .get(&monitor::HistoryKey::window(session.key()));
        let mut messages = history
            .map(|h| {
                h.messages
                    .iter()
                    .map(|m| m.text.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        messages.push(session.last_message.clone());
        let token = format!("{:?}:{}", messages, style::dark(&ui.window));
        if *self.messages_token.borrow() != token {
            self.messages_token.replace(token);
            for child in self.messages.children() {
                self.messages.remove(&child);
            }
            for (index, text) in messages.iter().enumerate() {
                let event = gtk::EventBox::new();
                event.set_visible_window(false);
                event.add_events(
                    gdk::EventMask::ENTER_NOTIFY_MASK | gdk::EventMask::LEAVE_NOTIFY_MASK,
                );
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                event.add(&line);
                let gutter = gtk::Box::new(gtk::Orientation::Vertical, 0);
                gutter.set_size_request(34, -1);
                let gutter_target = gtk::EventBox::new();
                gutter_target.set_visible_window(false);
                gutter_target.add(&gutter);
                line.pack_start(&gutter_target, false, false, 0);
                let message = widgets::message(text, style::dark(&ui.window));
                message.set_size_request(326, -1);
                line.pack_start(&message, false, false, 0);
                if index == 0 {
                    let previous = widgets::icon_button("pan-up-symbolic", "Previous message");
                    previous.style_context().remove_class("icon-button");
                    style::class(&previous, "history-button");
                    previous.set_can_focus(false);
                    // The gutter is 34px wide; the thread is at x=14 and text
                    // starts at x=34. Center the arrow at x=24 in that gap.
                    previous.set_size_request(16, -1);
                    previous.set_halign(gtk::Align::End);
                    previous.set_valign(gtk::Align::Fill);
                    previous.set_margin_end(2);
                    // Keep the glyph realized and the target allocated while hidden.
                    // Hiding the button itself also hid its image and changed layout
                    // as the pointer crossed into the gutter.
                    previous.set_opacity(0.0);
                    previous.set_sensitive(!history.is_some_and(|h| h.loading || h.exhausted));
                    gutter.pack_start(&previous, true, true, 0);
                    let click = previous.clone();
                    gutter_target.add_events(gdk::EventMask::BUTTON_PRESS_MASK);
                    gutter_target.connect_button_press_event(move |_, event| {
                        if event.button() == 1 {
                            click.emit_clicked();
                            glib::Propagation::Stop
                        } else {
                            glib::Propagation::Proceed
                        }
                    });
                    let decoration = previous.downgrade();
                    event.add_tick_callback(move |message, _| {
                        let Some(decoration) = decoration.upgrade() else {
                            return glib::ControlFlow::Break;
                        };
                        // Crossing events do not bubble through selectable labels.
                        // Sample the shared pointer coordinates so entering the text,
                        // the gutter, or scrolling the row gives the same result.
                        let hovered = message
                            .toplevel()
                            .and_then(|top| {
                                let window = top.window()?;
                                let pointer = window.display().default_seat()?.pointer()?;
                                let (_, x, y, _) = window.device_position(&pointer);
                                let (left, top) = message.translate_coordinates(&top, 0, 0)?;
                                Some(
                                    x >= left
                                        && x < left + message.allocated_width()
                                        && y >= top
                                        && y < top + message.allocated_height(),
                                )
                            })
                            .unwrap_or(false);
                        decoration.set_opacity(if hovered { 1.0 } else { 0.0 });
                        glib::ControlFlow::Continue
                    });
                    let weak = Rc::downgrade(ui);
                    let key = session.key();
                    previous.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            ui.seen(&key);
                            let state = ui.handle.snapshot();
                            let hkey = monitor::HistoryKey::window(key.clone());
                            let history = state.history.get(&hkey);
                            if !history.is_some_and(|h| h.loading || h.exhausted) {
                                let _ = ui.handle.send(monitor::Command::History {
                                    key: hkey,
                                    cursor: history.and_then(|h| h.cursor.clone()),
                                    generation: history.map_or(1, |h| h.generation + 1),
                                });
                            }
                        }
                    });
                }
                self.messages.pack_start(&event, false, false, 0);
            }
        }
        let request_token = format!("{:?}:{:?}", session.approval, session.interaction_data);
        if *self.card_token.borrow() != request_token {
            self.card_token.replace(request_token);
            ui.clear_focus_inside(&self.card_slot.clone().upcast());
            self.card.borrow_mut().take();
            for c in self.card_slot.children() {
                self.card_slot.remove(&c);
            }
            if session.approval.is_some() {
                let card = requests::Card::new(session, &ui.handle);
                self.card_slot.pack_start(&card.widget, false, false, 0);
                self.card.replace(Some(card));
            }
        }
        let submission = session
            .approval_key()
            .and_then(|key| state.submissions.get(&key));
        if let Some(card) = self.card.borrow().as_ref() {
            card.widget.set_sensitive(
                session.authoritative
                    && session
                        .approval
                        .as_ref()
                        .is_none_or(|a| a.expires > monitor::now())
                    && !matches!(
                        submission,
                        Some(monitor::Submission::Sending | monitor::Submission::Reconciling)
                    ),
            );
        }
        let feedback = if session
            .approval
            .as_ref()
            .is_some_and(|a| a.expires <= monitor::now())
        {
            "This request has expired. Open it in the agent app."
        } else {
            match submission {
                Some(monitor::Submission::Failed(e)) => e.as_str(),
                Some(monitor::Submission::Sending | monitor::Submission::Reconciling) => {
                    "Sending answer…"
                }
                _ => history.and_then(|h| h.error.as_deref()).unwrap_or(""),
            }
        };
        self.feedback.set_text(feedback);
        self.feedback.set_visible(!feedback.is_empty());
        for c in self.metadata.children() {
            self.metadata.remove(&c);
        }
        let home = crate::app::core::home();
        let cwd = session.working_directory();
        let short = cwd
            .strip_prefix(&home)
            .map(|p| format!("~/{}", p.display()))
            .unwrap_or_else(|_| cwd.display().to_string());
        let folder = widgets::meta("folder-symbolic", &short);
        folder.set_tooltip_text(Some(&cwd.display().to_string()));
        self.metadata.pack_start(&folder, false, false, 0);
        if let Some(branch) = &session.git_branch {
            self.metadata.pack_start(
                &widgets::meta("vcs-branch-symbolic", branch),
                false,
                false,
                0,
            );
        }
        self.detail.show_all();
        self.feedback.set_visible(!feedback.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        interaction::{Choice, Interaction, Question},
        session::{Approval, Client},
    };
    fn session(id: &str, title: &str, client: Client, activity: Activity) -> Session {
        Session{agent:"codex",id:id.into(),title:title.into(),cwd:"/tmp/native-monitor".into(),workspace:None,git_branch:Some("ubuntu".into()),updated_at:1,client,activity,last_message:"## Native Rust interface\n\nThe **session** is ready. [Documentation](https://example.com)".into(),interaction_data:Interaction::default(),target:Some("vscode://file/tmp/native-monitor".into()),authoritative:true,approval:None}
    }
    fn drain() {
        let until = std::time::Instant::now() + Duration::from_millis(350);
        while std::time::Instant::now() < until {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn previous_button(widget: &gtk::Widget) -> Option<gtk::Button> {
        if widget.tooltip_text().as_deref() == Some("Previous message") {
            return widget.clone().downcast().ok();
        }
        widget
            .clone()
            .downcast::<gtk::Container>()
            .ok()?
            .children()
            .iter()
            .find_map(previous_button)
    }
    #[test]
    #[ignore = "requires a graphical session or Xvfb and an isolated D-Bus session"]
    fn native_widgets_follow_style_guide_and_preserve_request_snapshots() {
        gtk::init().unwrap();
        let mut request = session(
            "question",
            "Review test scope",
            Client::Terminal,
            Activity::Confirmation,
        );
        request.approval = Some(Approval {
            id: "request-one".into(),
            body: "Choose the test scope".into(),
            expires: u64::MAX,
            can_allow: false,
        });
        request.interaction_data = Interaction {
            command: Some("cargo test --locked --workspace\necho next\necho more".into()),
            actions: vec![],
            questions: vec![Question {
                id: "scope".into(),
                title: "Choose the test scope".into(),
                choices: vec![Choice {
                    label: "Full suite".into(),
                    description: "All tests".into(),
                }],
                freeform: true,
                secret: false,
            }],
        };
        let mut state = monitor::State::default();
        state.sessions = vec![
            request.clone(),
            session("ten", "Task 10", Client::VSCode, Activity::Idle),
            session("two", "Task 2", Client::VSCode, Activity::Working),
        ];
        state
            .preferences
            .applications
            .observe(state.sessions.iter().map(|s| s.client));
        let (tx, mut commands) = tokio::sync::mpsc::channel(256);
        let (_published, state_rx) = tokio::sync::watch::channel(Arc::new(state));
        let handle = monitor::Handle {
            tx,
            state: state_rx,
        };
        let (_platform, platform_rx) = tokio::sync::mpsc::channel(128);
        let ui = Desktop::new(handle, platform_rx).unwrap();
        ui.show();
        drain();
        let width = ui.window.allocated_width();
        let height = ui.window.allocated_height();
        let mut surface =
            gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height).unwrap();
        {
            let context = gtk::cairo::Context::new(&surface).unwrap();
            ui.window.draw(&context);
        }
        let stride = surface.stride() as usize;
        let pixels = surface.data().unwrap();
        let alpha = |x: usize, y: usize| {
            let offset = y * stride + x * 4;
            u32::from_ne_bytes(pixels[offset..offset + 4].try_into().unwrap()) >> 24
        };
        for (x, y) in [
            (0, 0),
            (width - 1, 0),
            (0, height - 1),
            (width - 1, height - 1),
        ] {
            assert_eq!(
                alpha(x as usize, y as usize),
                0,
                "popover corners are transparent, including the footer"
            );
        }
        assert_eq!(
            alpha(width as usize / 2, height as usize - 14),
            255,
            "footer remains opaque inside the rounded surface"
        );
        drop(pixels);
        assert!(
            ui.pinned.window().unwrap().is_pass_through(),
            "application artwork must pass mouse events to the session robot"
        );
        // Move the actual X11 pointer over the app artwork, rather than emitting a
        // synthetic enter signal directly on the button: both overlay windows
        // must let GTK pick the robot underneath.
        if std::env::var_os("DISPLAY").is_some() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            let robot = ui.rows.borrow()["codex:two"].robot.clone();
            let window = ui.window.window().unwrap();
            let display = window.display();
            let pointer = display.default_seat().unwrap().pointer().unwrap();
            for cycle in 0..3 {
                if cycle > 0 {
                    robot.emit_clicked();
                    drain();
                }
                let (_, origin_x, origin_y) = window.origin();
                for (dx, dy) in [(14, 14), (2, 14), (25, 14), (14, 2), (14, 25)] {
                    let (x, y) = robot.translate_coordinates(&ui.window, dx, dy).unwrap();
                    pointer.warp(&window.screen(), origin_x + 200, origin_y + y);
                    display.sync();
                    drain();
                    assert!(
                        !robot.state_flags().contains(gtk::StateFlags::PRELIGHT),
                        "hover must clear outside the robot, cycle {cycle}"
                    );
                    pointer.warp(&window.screen(), origin_x + x, origin_y + y);
                    display.sync();
                    drain();
                    assert!(
                        robot.state_flags().contains(gtk::StateFlags::PRELIGHT),
                        "the full circle must hover after disclosure, cycle {cycle}, point {dx},{dy}"
                    );
                }
            }
            let row = ui.rows.borrow()["codex:two"].clone();
            let previous = previous_button(row.messages.upcast_ref()).unwrap();
            assert!(previous.image().unwrap().is_visible());
            let (left, _) = previous.translate_coordinates(&row.root, 0, 0).unwrap();
            assert_eq!(
                left * 2 + previous.allocated_width(),
                48,
                "history arrow is centered between thread x=14 and text x=34"
            );
            assert_eq!(previous.allocated_width(), 16);
            assert_eq!(
                previous.allocated_height(),
                row.messages.children()[0].allocated_height(),
                "history hover spans the message height"
            );
            let (_, origin_x, origin_y) = window.origin();
            let (x, y) = previous
                .translate_coordinates(&ui.window, 8, previous.allocated_height() / 2)
                .unwrap();
            pointer.warp(&window.screen(), origin_x + 100, origin_y + y);
            display.sync();
            drain();
            assert_eq!(
                previous.opacity(),
                1.0,
                "message hover reveals history control"
            );
            pointer.warp(&window.screen(), origin_x + x, origin_y + y);
            display.sync();
            drain();
            assert_eq!(
                previous.opacity(),
                1.0,
                "history control stays visible in the gutter"
            );
            previous.emit_clicked();
            assert!(
                std::iter::from_fn(|| commands.try_recv().ok())
                    .any(|command| matches!(command, monitor::Command::History { .. })),
                "previous message requests earlier history"
            );
            pointer.warp(&window.screen(), origin_x + 200, origin_y + 2);
            display.sync();
            drain();
            assert_eq!(
                previous.opacity(),
                0.0,
                "leaving the message hides history control"
            );
        }
        assert_eq!(
            *ui.order.borrow(),
            vec!["codex:two", "codex:ten", "codex:question"]
        );
        assert_eq!(
            ui.window.size().0,
            408,
            "the 392px native menu reserves 8px on each side for its shadow"
        );
        assert!(
            ui.window.size().1 <= 700,
            "long content is confined to the screen"
        );
        assert_eq!(
            ui.disclosure.borrow().expanded.get("codex:question"),
            Some(&true)
        );
        assert_eq!(
            ui.disclosure.borrow().expanded.get("codex:two"),
            Some(&true)
        );
        let row = ui.rows.borrow()["codex:question"].clone();
        for row in ui.rows.borrow().values() {
            for button in [&row.robot, &row.chevron] {
                assert_eq!(button.allocated_width(), 28);
                assert_eq!(button.allocated_height(), 28);
            }
            if let Some(status) = row.status_slot.children().first() {
                let (_, status_y) = status.translate_coordinates(&row.root, 0, 0).unwrap();
                let (_, robot_y) = row.robot.translate_coordinates(&row.root, 0, 0).unwrap();
                assert!(
                    (status_y * 2 + status.allocated_height()
                        - robot_y * 2
                        - row.robot.allocated_height())
                    .abs()
                        <= 1
                );
            }
        }
        let first = row.card.borrow().as_ref().unwrap().targets[0]
            .clone()
            .downcast::<gtk::Button>()
            .unwrap();
        first.emit_clicked();
        let mut submitted = None;
        while let Ok(command) = commands.try_recv() {
            if let monitor::Command::Submit { session, response } = command {
                submitted = Some((session, response));
            }
        }
        let (snapshot, response) = submitted.expect("single-question preset submits directly");
        assert_eq!(snapshot.approval, request.approval);
        assert!(
            matches!(response,crate::domain::interaction::Response::Answers(a) if a["scope"]==vec!["Full suite"])
        );
        let entry = row
            .card
            .borrow()
            .as_ref()
            .unwrap()
            .targets
            .iter()
            .find_map(|w| w.clone().downcast::<gtk::Entry>().ok())
            .unwrap();
        entry.set_text("Native draft");
        ui.refresh();
        assert_eq!(entry.text(), "Native draft");
        ui.query.replace("task 2".into());
        ui.refresh();
        assert_eq!(ui.order.borrow().len(), 1);
        ui.query.borrow_mut().clear();
        ui.refresh();
        assert_eq!(entry.text(), "Native draft");
        if let Some(window) = ui.window.window() {
            if let Some(pixbuf) = window.pixbuf(0, 0, window.width(), window.height()) {
                let _ = pixbuf.savev("/tmp/ai-agent-monitor-native.png", "png", &[]);
            }
        }
        let mut multi = request.clone();
        multi.interaction_data.command = None;
        multi.interaction_data.questions = vec![
            Question {
                id: "interface".into(),
                title: "Which interface should we refine next?".into(),
                choices: vec![
                    Choice {
                        label: "Monitor window".into(),
                        description: String::new(),
                    },
                    Choice {
                        label: "Top-panel icon".into(),
                        description: String::new(),
                    },
                ],
                freeform: true,
                secret: false,
            },
            Question {
                id: "theme".into(),
                title: "Which theme should we check first?".into(),
                choices: vec![
                    Choice {
                        label: "Dark theme".into(),
                        description: String::new(),
                    },
                    Choice {
                        label: "Light theme".into(),
                        description: String::new(),
                    },
                ],
                freeform: true,
                secret: false,
            },
        ];
        multi.approval.as_mut().unwrap().body = multi.interaction_data.questions[0].title.clone();
        let card = requests::Card::new(&multi, &ui.handle);
        let preview = gtk::Window::new(gtk::WindowType::Toplevel);
        style::class(&preview, "monitor");
        preview.set_default_size(326, -1);
        preview.add(&card.widget);
        preview.show_all();
        drain();
        let inputs = card
            .targets
            .iter()
            .filter_map(|w| w.clone().downcast::<gtk::Entry>().ok())
            .collect::<Vec<_>>();
        for input in &inputs {
            let custom_radio = input
                .parent()
                .unwrap()
                .downcast::<gtk::Box>()
                .unwrap()
                .children()
                .into_iter()
                .find_map(|child| child.downcast::<gtk::RadioButton>().ok())
                .unwrap();
            assert!(
                !custom_radio.can_focus(),
                "custom field owns its keyboard focus"
            );
            assert!(
                !card.targets.contains(&custom_radio.upcast()),
                "custom radio is excluded from answer traversal"
            );
        }
        inputs[0].set_text("Session history");
        let light = card
            .targets
            .iter()
            .find_map(|w| {
                w.clone()
                    .downcast::<gtk::RadioButton>()
                    .ok()
                    .filter(|r| r.label().as_deref() == Some("Light theme"))
            })
            .unwrap();
        light.set_active(true);
        drain();
        if let Some(window) = preview.window() {
            window
                .pixbuf(0, 0, window.width(), window.height())
                .unwrap()
                .savev("/tmp/ai-agent-monitor-multi-question.png", "png", &[])
                .unwrap();
        }
        card.targets
            .last()
            .unwrap()
            .clone()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        let answers = std::iter::from_fn(|| commands.try_recv().ok())
            .find_map(|command| match command {
                monitor::Command::Submit {
                    session,
                    response: crate::domain::interaction::Response::Answers(answers),
                } => {
                    assert_eq!(session.interaction_data, multi.interaction_data);
                    Some(answers)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(answers["interface"], vec!["Session history"]);
        assert_eq!(answers["theme"], vec!["Light theme"]);
        preview.close();
        ui.open_settings();
        drain();
        assert!(ui.settings.borrow().as_ref().unwrap().window.is_visible());
        ui.settings.borrow().as_ref().unwrap().window.hide();
        ui.query.replace("filter".into());
        ui.hide();
        assert!(ui.query.borrow().is_empty());
        assert!(ui.disclosure.borrow().expanded.is_empty());
        assert!(!ui.window.is_visible());
        ui.show();
        drain();
        assert_eq!(
            ui.disclosure.borrow().expanded.get("codex:question"),
            Some(&true)
        );
        ui.close();
    }
}
