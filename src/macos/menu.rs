//! Resident menu bar item and native sessions menu.
use super::visual::{
    self, BRAND_HEIGHT, GlassBox, UsageTrack, UsageTrackState, appearance_color, command_text,
    draw_menu_tint, glass_box, hover_fill_color, primary_text_color, request_color,
    secondary_text_color, working_status_color,
};
use crate::macos::session_layout::{
    BUTTON_HORIZONTAL_INSET, BUTTON_MIN_HEIGHT, BUTTON_VERTICAL_INSET,
};
use crate::{
    agents::{
        codex::usage,
        interaction::{Action, Response},
        sessions::{Activity, Client, Session},
    },
    app::core,
    macos::{
        answers, hotkey, inline_questions, motion, native, pointer, search, session_layout,
        session_robot::SessionRobot, session_thread::SessionThread, settings,
    },
};
use anyhow::{Context, Result};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    time::Instant,
};

const MENU_WIDTH: f64 = 460.0;

const SESSION_HEADER_HEIGHT: f64 = session_layout::ROW_HEIGHT;
const SESSION_TEXT_LEFT: f64 = session_layout::TEXT_LEFT;
const DETAIL_WIDTH: f64 = session_layout::DETAIL_WIDTH;
const LIST_BOTTOM_PADDING: f64 = 14.0;
const SESSION_HOVER_PADDING: f64 = 0.0;
const SESSION_HEADER_HOVER_LEFT: f64 = SESSION_TEXT_LEFT - 9.0;
const MESSAGE_TOP_GAP: f64 = 4.0;
const BUTTON_HOVER_PADDING: f64 = 0.0;
const ICON_BUTTON_SIZE: f64 = 28.0;

#[derive(Default, PartialEq, Eq)]
struct RowStatus {
    submissions: HashMap<String, crate::actors::monitor::Submission>,
    messages: HashMap<String, crate::actors::monitor::MessageSend>,
    finished: HashSet<String>,
}

#[derive(PartialEq)]
struct FooterRenderKey {
    windows: Vec<crate::domain::usage::Window>,
    remaining: Vec<Option<u32>>,
    resets: crate::domain::usage::Resets,
    expanded: bool,
    error: Option<String>,
    query: String,
    width: f64,
}

#[derive(Clone, Default)]
struct MessageHistory {
    messages: Vec<crate::agents::history::Message>,
    cursor: Option<String>,
    anchor: String,
    anchor_images: Vec<crate::domain::media::Image>,
    loading: bool,
    exhausted: bool,
    error: Option<String>,
    ticket: u64,
}

impl MessageHistory {
    fn enqueued(&mut self, result: Result<(), String>) {
        if let Err(error) = result {
            self.loading = false;
            self.error = Some(error);
        }
    }
}

#[derive(Clone)]
enum ControlKind {
    Reply(Response),
    Form,
    Open,
    Native,
    Composer,
    ContextToggle(String),
}
#[derive(Clone)]
struct RequestControl {
    session: Session,
    label: String,
    detail: String,
    kind: ControlKind,
}
fn request_controls(session: &Session) -> Vec<RequestControl> {
    if session.approval().is_none() {
        return vec![];
    }
    let interaction = session.interaction();
    let mut controls: Vec<_> = interaction
        .actions
        .into_iter()
        .map(|Action { id, label, detail }| RequestControl {
            session: session.clone(),
            label,
            detail,
            kind: ControlKind::Reply(Response::Action(id)),
        })
        .collect();
    if let [question] = interaction.questions.as_slice()
        && !question.secret
    {
        for choice in &question.choices {
            controls.push(RequestControl {
                session: session.clone(),
                label: choice.label.clone(),
                detail: choice.description.clone(),
                kind: ControlKind::Reply(Response::Answers(std::collections::BTreeMap::from([(
                    question.id.clone(),
                    vec![choice.label.clone()],
                )]))),
            });
        }
    }
    if !interaction.questions.is_empty()
        && (interaction.questions.len() > 1
            || interaction
                .questions
                .iter()
                .any(|question| question.freeform || question.secret))
    {
        controls.push(RequestControl {
            session: session.clone(),
            label: if controls.is_empty() {
                "Answer…"
            } else {
                "Other answer…"
            }
            .into(),
            detail: String::new(),
            kind: ControlKind::Form,
        });
    }
    if controls.is_empty() {
        controls.push(RequestControl {
            session: session.clone(),
            label: "Answer in editor…".into(),
            detail: "This request type must be answered in the original client.".into(),
            kind: ControlKind::Open,
        });
    }
    controls
}

#[derive(Default)]
pub struct State {
    // The resident owns only the status item and services. Each menu opening
    // gets a separate controller, retained while detached and disposed on close.
    popup: RefCell<Option<Retained<MenuController>>>,
    popup_owner: OnceCell<objc2::rc::Weak<MenuController>>,
    detach_pending: Cell<bool>,
    reattaching: Cell<bool>,
    resume_editors_on_attach: Cell<bool>,
    detached_window: RefCell<Option<Retained<NSWindow>>>,
    detached_layout: Cell<bool>,
    pinned_width: Cell<f64>,
    detached_frame: Cell<Option<NSRect>>,
    pin_button: OnceCell<Retained<NSButton>>,
    pin_background: OnceCell<Retained<NSBox>>,
    monitor: OnceCell<crate::actors::monitor::Handle>,
    revision: Cell<u64>,
    row_status: RefCell<Option<RowStatus>>,
    footer_render_key: RefCell<Option<FooterRenderKey>>,
    popup_width: Cell<f64>,
    ui_sender: OnceCell<crate::macos::bridge::Sender>,
    ui_incoming: OnceCell<RefCell<tokio::sync::mpsc::Receiver<crate::macos::bridge::Event>>>,
    forms: RefCell<Vec<Retained<answers::AnswerForm>>>,
    platform: OnceCell<RefCell<crate::macos::bridge::Platform>>,
    snapshot_timer: OnceCell<Retained<NSTimer>>,
    status: OnceCell<Retained<NSStatusItem>>,
    status_content: OnceCell<Retained<StatusSnapshotView>>,
    status_artwork: OnceCell<Retained<NSView>>,
    status_frames: RefCell<HashMap<[u8; 3], [Retained<NSImage>; 2]>>,
    status_badge: OnceCell<Retained<NSBox>>,
    status_count: OnceCell<Retained<NSTextField>>,
    status_working_count: OnceCell<Retained<NSTextField>>,
    status_finished_count: OnceCell<Retained<NSTextField>>,
    status_finished_badge: OnceCell<Retained<NSBox>>,
    status_icon: OnceCell<Retained<NSImageView>>,
    status_base_images: OnceCell<[Retained<NSImage>; 2]>,
    status_eyes: OnceCell<[Retained<NSImageView>; 2]>,
    status_eye_animation: RefCell<motion::WorkingEyes>,
    status_painted_eyes: Cell<Option<(bool, [f64; 2])>>,
    status_badge_snapshot: Cell<Option<(usize, usize, usize, bool)>>,
    status_timer: RefCell<Option<Retained<NSTimer>>>,
    settings_shell: OnceCell<super::inline_settings::Shell>,
    settings_tabs: OnceCell<Retained<NSTabViewController>>,
    settings_active: Cell<bool>,
    settings_selected: Cell<usize>,
    web_settings: OnceCell<Retained<super::web_settings::WebSettings>>,
    settings_session_scroll: Cell<NSPoint>,
    applications: RefCell<settings::Applications>,
    applications_list: OnceCell<Retained<SessionListView>>,
    applications_controller: OnceCell<Retained<NSViewController>>,
    applications_scroll: OnceCell<Retained<NSScrollView>>,
    applications_heading: OnceCell<Retained<NSTextField>>,
    integrations_controller: OnceCell<Retained<NSViewController>>,
    integrations_scroll: OnceCell<Retained<NSScrollView>>,
    integrations_heading: OnceCell<Retained<NSTextField>>,
    integrations_list: OnceCell<Retained<SessionListView>>,
    integrations_button: OnceCell<Retained<NSButton>>,
    integrations_note: OnceCell<Retained<NSTextField>>,
    integrations_snapshot: RefCell<Option<Vec<crate::macos::onboarding::Row>>>,
    telegram_username: OnceCell<Retained<NSTextField>>,
    telegram_auto_open: RefCell<crate::macos::telegram_setup::AutoOpen>,
    telegram_setup_view: OnceCell<Retained<NSView>>,
    telegram_connected_view: OnceCell<Retained<NSView>>,
    telegram_controller: OnceCell<Retained<NSViewController>>,
    telegram_tab_item: OnceCell<Retained<NSTabViewItem>>,
    telegram_bot_label: OnceCell<Retained<NSTextField>>,
    telegram_connected_title: OnceCell<Retained<NSTextField>>,
    telegram_connected_status: OnceCell<Retained<NSTextField>>,
    telegram_connected_test: OnceCell<Retained<NSButton>>,
    telegram_connected_open: OnceCell<Retained<NSButton>>,
    telegram_token: OnceCell<Retained<NSSecureTextField>>,
    telegram_status: OnceCell<Retained<NSTextField>>,
    telegram_pair: OnceCell<Retained<NSButton>>,
    telegram_disconnect: OnceCell<Retained<NSButton>>,
    login_switch: OnceCell<Retained<NSButton>>,
    animation_switch: OnceCell<Retained<NSButton>>,
    login_note: OnceCell<Retained<NSTextField>>,
    login_settings_button: OnceCell<Retained<NSButton>>,
    shortcut_button: OnceCell<Retained<NSButton>>,
    shortcut_clear: OnceCell<Retained<NSButton>>,
    shortcut_note: OnceCell<Retained<NSTextField>>,
    shortcut_error: RefCell<Option<String>>,
    shortcut_monitor: RefCell<Option<Retained<AnyObject>>>,
    shortcut_registration: RefCell<Option<hotkey::Registration>>,
    shortcut_applied: RefCell<Option<hotkey::Shortcut>>,
    menu: OnceCell<Retained<NSMenu>>,
    menu_item: OnceCell<Retained<NSMenuItem>>,
    menu_close_shortcut: OnceCell<Retained<NSMenuItem>>,
    menu_open: Cell<bool>,
    list: OnceCell<Retained<SessionListView>>,
    root: OnceCell<Retained<PanelView>>,
    backdrop: OnceCell<Retained<visual::MeshView>>,
    toolbar: OnceCell<Retained<NSView>>,
    release: OnceCell<Retained<super::release_link::ReleaseLink>>,
    footer: OnceCell<Retained<NSView>>,
    footer_height: Cell<f64>,
    footer_extension: OnceCell<Retained<visual::MeshView>>,
    header_extension: OnceCell<Retained<visual::MeshView>>,
    scroll: OnceCell<Retained<SessionScrollView>>,
    content_height: Cell<f64>,
    expanded: RefCell<HashSet<String>>,
    opened_approvals: RefCell<HashSet<String>>,
    opened_working: RefCell<HashSet<String>>,
    reveal: RefCell<HashMap<String, f64>>,
    motion: RefCell<Option<motion::Disclosure>>,
    animation_epoch: Cell<u64>,
    working_epoch: OnceCell<Instant>,
    layout: RefCell<Vec<LayoutRow>>,
    caption: OnceCell<Retained<NSTextField>>,
    overview: OnceCell<Retained<NSTextField>>,
    all: RefCell<Vec<Session>>,
    visible: RefCell<Vec<Session>>,
    filter_query: RefCell<String>,
    filter_dirty: Cell<bool>,
    usage: RefCell<Option<usage::Usage>>,
    usage_resets_expanded: Cell<bool>,
    usage_resets_pending: Cell<Option<u64>>,
    usage_resets_spinner: OnceCell<Retained<NSProgressIndicator>>,
    usage_resets_spinning: Cell<bool>,
    usage_resets_error: RefCell<Option<String>>,
    usage_resets_scroll: RefCell<Option<Retained<NSScrollView>>>,
    lock: OnceCell<File>,
    routing_scroll: Cell<bool>,
    buttons: RefCell<Vec<ButtonHover>>,
    request_controls: RefCell<HashMap<String, RequestControl>>,
    question_panels: RefCell<HashMap<String, Retained<inline_questions::Panel>>>,
    composers: RefCell<HashMap<String, Retained<super::session_composer::Composer>>>,
    composer_drafts: RefCell<HashMap<String, super::session_composer::Draft>>,
    question_drafts: RefCell<HashMap<String, inline_questions::Draft>>,
    work_glows: RefCell<Vec<Retained<super::work_caption::WorkGlow>>>,
    expanded_contexts: RefCell<HashSet<String>>,
    responding: RefCell<HashSet<String>>,
    message_history: RefCell<HashMap<String, MessageHistory>>,
    history_generation: Cell<u64>,
    toolbar_buttons: RefCell<Vec<ButtonHover>>,
    hovered: Cell<Option<pointer::Target>>,
    keyboard_focus: RefCell<Option<String>>,
    focus_answer_on_open: Cell<bool>,
    keyboard_handler: RefCell<Option<hotkey::MenuKeyboard>>,
    pointer_timer: RefCell<Option<Retained<NSTimer>>>,
    resize_cursor_active: Cell<bool>,
    hand_cursor_active: Cell<bool>,
    previous_menu_application: RefCell<Option<Retained<NSRunningApplication>>>,
    last_pointer: Cell<Option<NSPoint>>,
}

impl State {
    fn acknowledge_finished(&self, key: &str) {
        let Some(monitor) = self.monitor.get() else {
            return;
        };
        let snapshot = monitor.snapshot();
        let Some(&generation) = snapshot.unseen_finished.get(key) else {
            return;
        };
        if snapshot
            .sessions
            .iter()
            .any(|session| session.key() == key && session.activity == Activity::Idle)
        {
            let _ = monitor.send(crate::actors::monitor::Command::FinishedSeen {
                completions: HashMap::from([(key.to_owned(), generation)]),
            });
        }
    }
    fn working_count(&self) -> usize {
        let applications = self.applications.borrow();
        self.all
            .borrow()
            .iter()
            .filter(|session| {
                session.activity == Activity::Working && applications.enabled(session.client)
            })
            .count()
    }
    fn finished_count(&self) -> usize {
        let snapshot = self.monitor.get().map(|monitor| monitor.snapshot());
        let applications = self.applications.borrow();
        snapshot
            .map(|snapshot| {
                snapshot
                    .sessions
                    .iter()
                    .filter(|session| {
                        applications.enabled(session.client)
                            && snapshot.unseen_finished.contains_key(&session.key())
                    })
                    .count()
            })
            .unwrap_or(0)
    }
    fn filtered_sessions(&self) -> Vec<Session> {
        let query = self.filter_query.borrow();
        let applications = self.applications.borrow();
        self.all
            .borrow()
            .iter()
            .filter(|session| {
                applications.enabled(session.client) && search::matches(&session.title, &query)
            })
            .cloned()
            .collect()
    }

    fn reset_disclosures(&self) {
        self.animation_epoch
            .set(self.animation_epoch.get().wrapping_add(1));
        self.motion.borrow_mut().take();
        self.expanded.borrow_mut().clear();
        self.reveal.borrow_mut().clear();
        self.opened_approvals.borrow_mut().clear();
        self.opened_working.borrow_mut().clear();
        self.message_history.borrow_mut().clear();
        self.expanded_contexts.borrow_mut().clear();
        self.history_generation
            .set(self.history_generation.get().wrapping_add(1));
    }

    fn expand_active_sessions(&self, sessions: &[Session]) -> bool {
        if !self.menu_open.get() {
            return false;
        }
        let mut changed = false;
        for session in sessions {
            let newly_active = if let Some(key) = session.approval_key() {
                self.opened_approvals.borrow_mut().insert(key)
            } else if session.activity == Activity::Working {
                self.opened_working.borrow_mut().insert(session.key())
            } else {
                false
            };
            if newly_active {
                self.expanded.borrow_mut().insert(session.key());
                self.reveal.borrow_mut().insert(session.key(), 1.0);
                if let Some(motion) = self.motion.borrow_mut().as_mut() {
                    motion.show(&session.key());
                }
                changed = true;
            }
        }
        changed
    }
}

#[derive(Default, Debug)]
struct StatusContentState {
    images: RefCell<Option<[Retained<NSImage>; 2]>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. UI and all state are
    // confined to the main thread; retained controls outlive their weak targets.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    pub struct MenuController;

    unsafe impl NSObjectProtocol for MenuController {}
    // SAFETY: The retained main-thread controller implements NSMenuDelegate.
    unsafe impl NSMenuDelegate for MenuController {
        #[unsafe(method(menuWillOpen:))]
        fn will_open(&self, _menu: &NSMenu) {
            if self.ivars().root.get().is_none() {
                let existing = self.ivars().popup.borrow().clone();
                if let Some(popup) = existing {
                    if popup.is_detached() {
                        _menu.cancelTracking();
                        // Leave native menu tracking before activating the window.
                        unsafe { self.performSelector_withObject_afterDelay(sel!(showDetached:), None, 0.0); }
                        return;
                    }
                    self.ivars().menu_open.set(true);
                    popup.will_open(sel!(menuWillOpen:), _menu);
                    return;
                }
                self.ivars().menu_open.set(true);
                self.ivars().shortcut_registration.borrow_mut().take();
                let popup = self.create_popup(_menu);
                *self.ivars().popup.borrow_mut() = Some(popup.clone());
                popup.will_open(sel!(menuWillOpen:), _menu);
                return;
            }
            self.acquire_menu_cursor_owner();
            if let Some(screen) = self.ivars().status.get().and_then(|s| s.button(self.mtm()))
                .and_then(|b| b.window()).and_then(|w| w.screen()) {
                let maximum = (screen.visibleFrame().size.width - 8.0).max(MENU_WIDTH);
                if self.menu_width() > maximum { self.change_width(maximum); }
            }
            self.ivars().menu_open.set(true);
            self.ivars().focus_answer_on_open.set(!self.ivars().reattaching.replace(false));
            // NSMenu handles this shortcut as a native key equivalent while it
            // tracks. Keeping Carbon registered here can buffer the key until
            // tracking ends, reopening the menu instead of closing it.
            self.ivars().shortcut_registration.borrow_mut().take();
            self.receive_sessions();
            self.filter();
            self.refresh();
            self.start_pointer_tracking();
        }
        #[unsafe(method(menuDidClose:))]
        fn did_close(&self, _menu: &NSMenu) {
            self.ivars().menu_open.set(false);
            if self.ivars().root.get().is_none() {
                let popup = self.ivars().popup.borrow().clone();
                if let Some(popup) = popup {
                    if popup.ivars().detach_pending.replace(false) {
                        popup.detach_window();
                        self.install_shortcut();
                        return;
                    }
                    if popup.is_detached() { return; }
                    self.ivars().popup.borrow_mut().take();
                    popup.did_close(sel!(menuDidClose:), _menu);
                    self.save_popup_data(&popup);
                    *self.ivars().telegram_auto_open.borrow_mut() =
                        std::mem::take(&mut *popup.ivars().telegram_auto_open.borrow_mut());
                    if let Some(item) = popup.ivars().menu_item.get() {
                        item.setView(None);
                    }
                    if let Some(root) = popup.ivars().root.get() {
                        root.removeFromSuperview();
                    }
                    popup.shutdown();
                }
                _menu.removeAllItems();
                add_popup_placeholder(_menu, self.mtm());
                self.install_shortcut();
                return;
            }
            self.close_panel();
            self.restore_menu_application();
            self.install_shortcut();
        }
    }

    // SAFETY: The popup controller is retained by its resident while the window exists.
    unsafe impl NSWindowDelegate for MenuController {
        #[unsafe(method(windowWillClose:))]
        fn detached_closed(&self, _notification: &NSNotification) {
            // Removing the resident's last strong popup reference must not
            // destroy the delegate while this AppKit callback is still running.
            let retained: Retained<Self> = Retained::from(self);
            retained.dispose_detached();
        }
        #[unsafe(method(windowDidResize:))]
        fn detached_resized(&self, _notification: &NSNotification) {
            self.layout_detached();
            self.remember_detached_frame();
        }
        #[unsafe(method(windowDidMove:))]
        fn detached_moved(&self, _notification: &NSNotification) {
            self.remember_detached_frame();
        }
    }

    impl MenuController {
        #[unsafe(method(togglePin:))]
        fn toggle_pin(&self, _sender: Option<&AnyObject>) {
            if self.is_detached() {
                self.reattach_window();
            } else {
                self.ivars().detach_pending.set(true);
                self.dismiss_menu();
            }
        }
        #[unsafe(method(showDetached:))]
        fn show_detached(&self, _sender: Option<&AnyObject>) {
            let popup = self.ivars().popup.borrow().clone();
            if let Some(popup) = popup {
                popup.focus_detached();
            }
        }
        #[unsafe(method(showPinned:))]
        fn show_pinned(&self, _sender: Option<&AnyObject>) { self.show(); }
        #[unsafe(method(toggleMenuFromHotkey:))]
        fn toggle_menu_from_hotkey(&self, _timer: &NSTimer) {
            if self.ivars().menu_open.get() { self.dismiss_menu(); } else { self.show(); }
        }

        #[unsafe(method(toggleUsageResets:))]
        fn toggle_usage_resets(&self, _sender: Option<&AnyObject>) {
            let open=!self.ivars().usage_resets_expanded.get();
            self.ivars().usage_resets_expanded.set(open);
            if open {
                let current=&self.monitor().snapshot().usage_resets;
                let generation=if current.phase==crate::domain::usage::ResetPhase::Loading{current.generation}else{current.generation.saturating_add(1)};
                self.ivars().usage_resets_pending.set(Some(generation));
                let error=if current.phase==crate::domain::usage::ResetPhase::Loading{None}else{self.monitor().send(crate::actors::monitor::Command::ReadUsageResets).err()};
                if error.is_some(){self.ivars().usage_resets_pending.set(None);}
                *self.ivars().usage_resets_error.borrow_mut()=error;
            } else { self.ivars().usage_resets_pending.set(None); }
            self.render_footer();
            self.update_hover();
        }

        #[unsafe(method(closeMenu:))]
        fn close_menu(&self, _sender: Option<&AnyObject>) {
            let window = self.ivars().detached_window.borrow().clone();
            if let Some(window) = window { window.performClose(None); } else { self.dismiss_menu(); }
        }
        #[unsafe(method(animateStatus:))]
        fn animate_status(&self, _timer: &NSTimer) {
            self.ivars().status_eye_animation.borrow_mut().tick();
            self.paint_status_eyes();
        }

        #[unsafe(method(pollState:))]
        fn poll_state(&self, timer: &NSTimer) {
            if self.ivars().popup_owner.get().is_some() && !self.ivars().menu_open.get() {
                return;
            }
            if self.ivars().root.get().is_some() && self.mouse_tracking_active() {
                // Do not replace native controls inside their mouse tracking loop.
                // The resident still publishes status counters and notifications.
                return;
            }
            self.drain_ui();
            self.receive_sessions();
            let popup = self.ivars().popup.borrow().clone();
            if let Some(popup) = popup {
                popup.poll_state(sel!(pollState:), timer);
            }
            if let Some(web) = self.ivars().web_settings.get() {
                let height = web.frame().size.height;
                web.poll(self.ivars().menu_open.get()
                    && self.ivars().settings_active.get()
                    && self.ivars().settings_selected.get() == 4);
                let changed = web.take_layout_changed();
                if height != web.frame().size.height || changed {
                    self.layout_settings();
                }
            }
        }
        #[unsafe(method(pollPointer:))]
        fn poll_pointer(&self, _timer: &NSTimer) {
            self.constrain_menu_to_work_area();
            self.sync_header_extension();
            self.sync_footer_extension();
            self.receive_sessions();
            if self.mouse_tracking_active() {
                self.sync_action_cursor();
                return;
            }
            self.start_keyboard_tracking();
            self.advance_disclosure();
            self.advance_robot_eyes();
            self.sample_pointer();
            let panels: Vec<_> = self.ivars().question_panels.borrow().values().cloned().collect();
            let now = Instant::now();
            let mut focus_changed = false;
            for panel in panels {
                if let Some(id) = panel.take_focus_change().or_else(|| panel.editing_target()) {
                    let changed = self.ivars().keyboard_focus.borrow().as_ref() != Some(&id);
                    if changed {
                        *self.ivars().keyboard_focus.borrow_mut() = Some(id);
                        focus_changed = true;
                    }
                }
                panel.advance_carets(now);
            }
            let composers: Vec<_> = self.ivars().composers.borrow().values().cloned().collect();
            let mut composer_resized = false;
            for composer in composers {
                composer_resized |= composer.resize_to_fit();
                if composer.editing() {
                    let id = composer.id();
                    if self.ivars().keyboard_focus.borrow().as_ref() != Some(&id) {
                        *self.ivars().keyboard_focus.borrow_mut() = Some(id);
                        focus_changed = true;
                    }
                }
                composer.paint(self.ivars().keyboard_focus.borrow().as_deref());
            }
            if composer_resized { self.render_rows(&self.ivars().filtered_sessions()); }
            if focus_changed { self.apply_hover(self.ivars().hovered.get()); }
        }

        #[unsafe(method(showSessions:))]
        fn show_sessions(&self, _sender: Option<&AnyObject>) {
            self.show();
        }

        #[unsafe(method(expandSession:))]
        fn expand_session(&self, sender: Option<&NSButton>) {
            let Some(id) = sender.and_then(|button| button.identifier()).map(|id| id.to_string()) else { return; };
            self.begin_disclosure(&id);
        }

        #[unsafe(method(markFinishedSeen:))]
        fn mark_finished_seen(&self, sender: &NSButton) {
            if let Some(key) = sender.identifier() {
                self.ivars().acknowledge_finished(&key.to_string());
            }
        }

        #[unsafe(method(previousMessage:))]
        fn previous_message(&self, sender: &NSButton) {
            let Some(key) = sender.identifier().map(|id| id.to_string()) else { return; };
            let state = self.ivars();
            let Some(session) = state.all.borrow().iter().find(|session| session.key() == key).cloned() else { return; };
            state.acknowledge_finished(&key);
            let ticket = state.history_generation.get().wrapping_add(1);
            state.history_generation.set(ticket);
            let before = {
                let mut histories = state.message_history.borrow_mut();
                let history = histories.entry(key.clone()).or_default();
                if history.loading || history.exhausted { return; }
                if history.cursor.is_none() {
                    history.anchor = session.last_message().to_owned();
                    history.anchor_images = session.images().to_vec();
                }
                history.loading = true;
                history.error = None;
                history.ticket = ticket;
                history.cursor.clone()
            };
            self.render_rows(&state.filtered_sessions());
            let result = self.monitor().send(crate::actors::monitor::Command::History{key:crate::actors::monitor::HistoryKey::window(key.clone()),cursor:before,generation:ticket});
            if let Err(error) = result {
                if let Some(history) = state.message_history.borrow_mut().get_mut(&key) {
                    history.enqueued(Err(error));
                }
                self.render_rows(&state.filtered_sessions());
            }
        }

        #[unsafe(method(respondToRequest:))]
        fn respond_to_request(&self, sender: Option<&NSButton>) {
            let Some(id) = sender.and_then(|button| button.identifier()).map(|id| id.to_string()) else { return; };
            let control = self.ivars().request_controls.borrow().get(&id).cloned();
            let Some(control) = control else { return; };
            match control.kind {
                ControlKind::ContextToggle(key) => {
                    let mut expanded = self.ivars().expanded_contexts.borrow_mut();
                    if !expanded.remove(&key) { expanded.insert(key); }
                    drop(expanded);
                    self.render_rows(&self.ivars().filtered_sessions());
                    self.update_hover();
                }
                ControlKind::Native | ControlKind::Composer => {},
                ControlKind::Form => { self.dismiss_menu(); self.show_form(control.session,None); }
                ControlKind::Open => {
                    self.ivars().acknowledge_finished(&control.session.key());
                    self.dismiss_menu(); self.monitor().open(&control.session);
                }
                ControlKind::Reply(response) => {
                    if let Err(error)=self.monitor().submit(&control.session,response) && let Some(button)=sender {button.setToolTip(Some(&NSString::from_str(&error)));}

                }
            }
        }

        #[unsafe(method(openSession:))]
        fn open_session(&self, sender: Option<&NSButton>) {
            let Some(id) = sender.and_then(|button| button.identifier()) else { return; };
            let data = self.ivars().all.borrow();
            if let Some(session) = data.iter().find(|session| session.key() == id.to_string()) {
                self.ivars().acknowledge_finished(&session.key());
                self.dismiss_menu();
                self.monitor().open(session);
            }
        }

        #[unsafe(method(openProject:))]
        fn open_project(&self, _sender: Option<&AnyObject>) {
            self.dismiss_menu();
            if let Some(url) = NSURL::URLWithString(&NSString::from_str(core::PROJECT_URL)) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }

        #[unsafe(method(settings:))]
        fn settings(&self, _sender: Option<&AnyObject>) {
            self.show_settings();
        }

        #[unsafe(method(backToSessions:))]
        fn back_to_sessions(&self, _sender: Option<&AnyObject>) { self.hide_settings(); }

        #[unsafe(method(selectSettingsTab:))]
        fn select_settings_tab(&self, sender: &NSButton) {
            self.end_shortcut_recording(false);
            if let Some(window)=self.ivars().root.get().and_then(|v|v.window()) { unsafe {window.endEditingFor(None);} }
            self.ivars().settings_selected.set(sender.tag().clamp(0,4) as usize);
            self.layout_settings();
        }

        #[unsafe(method(setupIntegrations:))]
        fn setup_integrations(&self, _sender: Option<&AnyObject>) {
            let _=self.monitor().send(crate::actors::monitor::Command::Platform(crate::macos::bridge::PlatformCommand::Setup(true)));
            self.refresh_integration_settings();
        }

        #[unsafe(method(connectTelegram:))]
        fn connect_telegram(&self, _sender: Option<&AnyObject>) {
            if let Some(field) = self.ivars().telegram_token.get() {
                let token = field.stringValue().to_string();
                field.setStringValue(ns_string!(""));
                self.ivars().telegram_auto_open.borrow_mut().request(self.monitor().snapshot().telegram.link.clone());
                let _=self.monitor().send(crate::actors::monitor::Command::Telegram(crate::actors::human::telegram::Event::Connect(token)));
                self.refresh_telegram_settings();
            }
        }
        #[unsafe(method(pasteTelegramToken:))]
        fn paste_telegram_token(&self, _sender: Option<&AnyObject>) {
            if let Some(field) = self.ivars().telegram_token.get() {
                // Read only plain text; never log or persist clipboard contents.
                if let Some(text) = NSPasteboard::generalPasteboard().stringForType(unsafe { NSPasteboardTypeString }) {
                    field.setStringValue(&text);
                    if let Some(window) = field.window() { window.makeFirstResponder(Some(field)); }
                }
            }
        }
        #[unsafe(method(disconnectTelegram:))]
        fn disconnect_telegram(&self, _sender: Option<&AnyObject>) {
            self.ivars().telegram_auto_open.borrow_mut().cancel();
            if let Some(field) = self.ivars().telegram_token.get() { field.setStringValue(ns_string!("")); }
            let status = self.monitor().snapshot().telegram.clone();
            let event = if status.paired && status.setting_up { crate::actors::human::telegram::Event::CancelSetup } else { crate::actors::human::telegram::Event::Disconnect };
            let _=self.monitor().send(crate::actors::monitor::Command::Telegram(event));
        }
        #[unsafe(method(removeTelegram:))]
        fn remove_telegram(&self, _sender: Option<&AnyObject>) {
            self.ivars().telegram_auto_open.borrow_mut().cancel();
            let _ = self.monitor().send(crate::actors::monitor::Command::Telegram(crate::actors::human::telegram::Event::Disconnect));
        }
        #[unsafe(method(changeTelegram:))]
        fn change_telegram(&self, _sender: Option<&AnyObject>) {
            self.ivars().telegram_auto_open.borrow_mut().cancel();
            let _ = self.monitor().send(crate::actors::monitor::Command::Telegram(crate::actors::human::telegram::Event::BeginSetup));
        }
        #[unsafe(method(openBotFather:))]
        fn open_bot_father(&self, _sender: Option<&AnyObject>) {
            let workspace = NSWorkspace::sharedWorkspace();
            if let Some(url) = NSURL::URLWithString(ns_string!("tg://resolve?domain=BotFather&text=%2Fnewbot"))
                && workspace.openURL(&url) {
                return;
            }
            if let Some(url) = NSURL::URLWithString(ns_string!("https://t.me/BotFather?text=%2Fnewbot")) {
                workspace.openURL(&url);
            }
        }
        #[unsafe(method(copyBotValue:))]
        fn copy_bot_value(&self, sender: &NSButton) {
            let text = if sender.tag() == 0 {
                NSString::from_str("AI Agent Monitor")
            } else if let Some(field) = self.ivars().telegram_username.get() {
                field.stringValue()
            } else { return; };
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.clearContents();
            // SAFETY: AppKit's standard string pasteboard type is a valid constant.
            pasteboard.setString_forType(&text, unsafe { NSPasteboardTypeString });
        }
        #[unsafe(method(regenerateBotUsername:))]
        fn regenerate_bot_username(&self, _sender: Option<&AnyObject>) {
            if let Some(field) = self.ivars().telegram_username.get() {
                let value = NSString::from_str(&crate::macos::telegram_setup::username(true));
                field.setStringValue(&value);
                field.setToolTip(Some(&value));
            }
        }
        #[unsafe(method(pairTelegram:))]
        fn pair_telegram(&self, _sender: Option<&AnyObject>) {
            let status = self.monitor().snapshot().telegram.clone();
            let link = status.link.or_else(|| (!status.bot_username.is_empty()).then(|| format!("https://t.me/{}", status.bot_username)));
            if let Some(link) = link { open_telegram_bot(&link); }
        }
        #[unsafe(method(testTelegram:))]
        fn test_telegram(&self, _sender: Option<&AnyObject>) {
            let _=self.monitor().send(crate::actors::monitor::Command::Telegram(crate::actors::human::telegram::Event::Test));
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, sender: &NSButton) {
            let _=self.monitor().send(crate::actors::monitor::Command::Platform(crate::macos::bridge::PlatformCommand::Login(sender.state()==NSControlStateValueOn)));
        }

        #[unsafe(method(toggleApplication:))]
        fn toggle_application(&self, sender: &NSSwitch) {
            let Some(id) = sender.identifier() else { return; };
            let _ = self.monitor().send(crate::actors::monitor::Command::SetApplicationEnabled {
                id: id.to_string(), enabled: sender.state() == NSControlStateValueOn,
            });
        }

        #[unsafe(method(toggleMenuAnimation:))]
        fn toggle_menu_animation(&self, sender: &NSButton) {
            let _=self.monitor().send(crate::actors::monitor::Command::Platform(crate::macos::bridge::PlatformCommand::Animation(sender.state()==NSControlStateValueOn)));
            self.update_badge(self.monitor().snapshot().pending_count());
        }

        #[unsafe(method(openLoginSettings:))]
        fn open_login_settings(&self, _sender: Option<&AnyObject>) {
            let _=self.monitor().send(crate::actors::monitor::Command::Platform(crate::macos::bridge::PlatformCommand::LoginSettings));
        }

        #[unsafe(method(recordShortcut:))]
        fn record_shortcut(&self, _sender: Option<&AnyObject>) {
            self.begin_shortcut_recording();
        }

        #[unsafe(method(clearShortcut:))]
        fn clear_shortcut(&self, _sender: Option<&AnyObject>) {
            self.clear_menu_shortcut();
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {let _=self.monitor().send(crate::actors::monitor::Command::Shutdown);}
    }

);

fn open_telegram_bot(link: &str) {
    let workspace = NSWorkspace::sharedWorkspace();
    if let Ok(web) = url::Url::parse(link)
        && web.host_str() == Some("t.me")
        && !web.path().trim_matches('/').is_empty()
    {
        let mut native = url::Url::parse("tg://resolve").expect("valid Telegram scheme");
        native
            .query_pairs_mut()
            .append_pair("domain", web.path().trim_matches('/'));
        for (key, value) in web.query_pairs().filter(|(key, _)| key == "start") {
            native.query_pairs_mut().append_pair(&key, &value);
        }
        if let Some(url) = NSURL::URLWithString(&NSString::from_str(native.as_str()))
            && workspace.openURL(&url)
        {
            return;
        }
    }
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(link)) {
        workspace.openURL(&url);
    }
}

fn settings_note(
    view: &NSView,
    text: &str,
    frame: NSRect,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    label.setTextColor(Some(&NSColor::secondaryLabelColor()));
    label.setMaximumNumberOfLines(0);
    label.setFrame(frame);
    if let Some(cell) = label.cell() {
        cell.setUsesSingleLineMode(false);
        cell.setScrollable(false);
        cell.setWraps(true);
        cell.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    }
    view.addSubview(&label);
    label
}

define_class!(
    // SAFETY: Native button subclass; all cursor work stays on the AppKit thread.
    #[unsafe(super(NSButton))]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    struct ActionButton;
    unsafe impl NSObjectProtocol for ActionButton {}
    impl ActionButton {
        #[unsafe(method(resetCursorRects))]
        fn reset_action_cursor(&self) {
            unsafe { let _: () = msg_send![super(self), resetCursorRects]; }
            if self.isEnabled() {
                self.addCursorRect_cursor(self.bounds(), &NSCursor::pointingHandCursor());
            }
        }
        #[unsafe(method(setEnabled:))]
        fn set_action_enabled(&self, enabled: bool) {
            unsafe { let _: () = msg_send![super(self), setEnabled: enabled]; }
            if let Some(window) = self.window() { window.invalidateCursorRectsForView(self); }
        }
    }
);

define_class!(
    // SAFETY: NSView supports subclassing. The view is confined to the main thread.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    struct SessionListView;
    unsafe impl NSObjectProtocol for SessionListView {}
    impl SessionListView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
    }
);

define_class!(
    // SAFETY: This main-thread decoration never handles mouse input.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    struct StatusContentView;
    unsafe impl NSObjectProtocol for StatusContentView {}
    impl StatusContentView {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> { None }
    }
);

define_class!(
    // SAFETY: This NSView is main-thread-only. Its decoration never intercepts
    // clicks; the owning NSStatusBarButton handles the complete hit area.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = StatusContentState]
    #[derive(Debug)]
    struct StatusSnapshotView;
    unsafe impl NSObjectProtocol for StatusSnapshotView {}
    impl StatusSnapshotView {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> { None }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            if let Some(images) = self.ivars().images.borrow().as_ref() {
                let image = &images[usize::from(visual::is_dark(self))];
                image.drawInRect_fromRect_operation_fraction(self.bounds(), NSRect::ZERO, NSCompositingOperation::SourceOver, 1.0);
            }
        }

    }
);

fn owner(view: &NSView) -> Option<Retained<MenuController>> {
    let mut current = Some(unsafe { Retained::retain(view as *const NSView as *mut NSView) }?);
    while let Some(view) = current {
        if let Some(root) = view.downcast_ref::<PanelView>() {
            return root.ivars().owner.get().and_then(|weak| weak.load());
        }
        current = unsafe { view.superview() };
    }
    None
}

fn add_popup_placeholder(menu: &NSMenu, mtm: MainThreadMarker) {
    // A nonempty native menu can enter tracking; menuWillOpen: replaces this
    // inert item before presentation. It owns no custom view or controller.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!(""),
            None,
            ns_string!(""),
        )
    };
    menu.addItem(&item);
}
pub(super) fn route_wheel(view: &NSView, event: &NSEvent) {
    if let Some(controller) = owner(view) {
        controller.route_scroll(event);
    }
}
const RESIZE_EDGE_WIDTH: f64 = 6.0;
const RESIZE_CORNER_INSET: f64 = 18.0;

fn resize_edge(point: NSPoint, bounds: NSRect) -> Option<bool> {
    let x = point.x - bounds.origin.x;
    let y = point.y - bounds.origin.y;
    if bounds.size.height <= 2.0 * RESIZE_CORNER_INSET
        || y <= RESIZE_CORNER_INSET
        || y >= bounds.size.height - RESIZE_CORNER_INSET
    {
        return None;
    }
    if (0.0..=RESIZE_EDGE_WIDTH).contains(&x) {
        Some(true)
    } else if (bounds.size.width - RESIZE_EDGE_WIDTH..=bounds.size.width).contains(&x) {
        Some(false)
    } else {
        None
    }
}

#[derive(Default)]
struct PanelState {
    owner: OnceCell<objc2::rc::Weak<MenuController>>,
    resize_drag: Cell<Option<(f64, NSRect, bool, f64)>>,
}

impl std::fmt::Debug for PanelState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PanelState")
    }
}

define_class!(
    // SAFETY: A main-thread NSView that routes wheel input from the window margins.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    #[ivars=PanelState]
    struct PanelView;
    unsafe impl NSObjectProtocol for PanelView {}
    impl PanelView {
        #[unsafe(method(mouseDown:))]
        fn begin_resize(&self, event: &NSEvent) {
            if let Some(controller) = self.ivars().owner.get().and_then(|o| o.load()) {
                controller.handle_width_mouse(event);
            }
        }
        #[unsafe(method(mouseDragged:))]
        fn drag_resize(&self, _event: &NSEvent) {
            if let Some((x, frame, left, initial_width)) = self.ivars().resize_drag.get()
                && let Some(controller) = self.ivars().owner.get().and_then(|o| o.load()) {
                let delta = NSEvent::mouseLocation().x - x;
                controller.change_width(initial_width + if left { -delta } else { delta });
                if let Some(window) = self.window() {
                    // change_width synchronizes both the item and its native
                    // host. Only anchor the opposite edge here; do not reuse
                    // a window/item delta from an earlier native layout.
                    let mut origin = window.frame().origin;
                    origin.x = if left { frame.origin.x + frame.size.width - window.frame().size.width } else { frame.origin.x };
                    window.setFrameOrigin(origin);
                    controller.constrain_menu_to_work_area();
                }
                controller.paint_resize_cursor(true);
            }
        }
        #[unsafe(method(mouseUp:))]
        fn end_resize(&self, _event: &NSEvent) { self.ivars().resize_drag.set(None); }

        #[unsafe(method(drawRect:))]
        fn draw_background(&self, _dirty: NSRect) {
            // Keep the native menu material underneath, but attenuate wallpaper
            // and window contrast with an appearance-aware translucent surface.
            draw_menu_tint(self.bounds());
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn attached(&self) {
            // Prepare the native menu padding before its first visible frame.
            if let Some(controller) = self.ivars().owner.get().and_then(|owner| owner.load()) {
                controller.constrain_menu_to_work_area();
                controller.sync_header_extension();
            }
        }

        #[unsafe(method(viewWillDraw))]
        fn will_draw(&self) {
            // AppKit can resize the tracking window after our layout and before
            // painting. Extend the fill in that same frame, not on the next tick.
            if let Some(controller) = self.ivars().owner.get().and_then(|owner| owner.load()) {
                controller.sync_window_width();
                controller.constrain_menu_to_work_area();
                controller.sync_backdrop();
                controller.sync_header_extension();
                controller.sync_footer_extension();
            }
            // SAFETY: Preserve NSView's normal pre-draw preparation.
            unsafe { let _: () = msg_send![super(self), viewWillDraw]; }
        }

        #[unsafe(method(scrollWheel:))]
        fn wheel(&self, event: &NSEvent) { route_wheel(self,event); }
    }
);
impl PanelView {
    fn track_resize(&self, left: bool) {
        if let Some(window) = self.window() {
            self.ivars().resize_drag.set(Some((
                NSEvent::mouseLocation().x,
                window.frame(),
                left,
                self.bounds().size.width,
            )));
            if let Some(controller) = self.ivars().owner.get().and_then(|o| o.load()) {
                controller.paint_resize_cursor(true);
            }
            let app = NSApplication::sharedApplication(self.mtm());
            while let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp,
                Some(&NSDate::distantFuture()),
                ns_string!("NSEventTrackingRunLoopMode"),
                true,
            ) {
                self.drag_resize(sel!(mouseDragged:), &event);
                if event.r#type() == NSEventType::LeftMouseUp {
                    break;
                }
            }
            self.ivars().resize_drag.set(None);
            if let Some(controller) = self.ivars().owner.get().and_then(|o| o.load()) {
                NSUserDefaults::standardUserDefaults()
                    .setDouble_forKey(controller.menu_width(), ns_string!("MonitorWindowWidth"));
                controller.sync_resize_cursor();
            }
        }
    }
}
#[derive(Default, Debug)]
struct ScrollOwner {
    controller: RefCell<Option<objc2::rc::Weak<MenuController>>>,
}
define_class!(
    // SAFETY: Standard NSScrollView override, confined to the main thread.
    #[unsafe(super(NSScrollView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ScrollOwner]
    #[derive(Debug)]
    struct SessionScrollView;
    unsafe impl NSObjectProtocol for SessionScrollView {}
    impl SessionScrollView {
        #[unsafe(method(reflectScrolledClipView:))]
        fn reflected(&self, clip: &NSClipView) {
            // SAFETY: Forward AppKit's standard scroll notification.
            unsafe { let _: () = msg_send![super(self), reflectScrolledClipView: clip]; }
            // AppKit also calls this while removing/deallocating a scroll
            // view. Never retain self or walk its dying ancestor hierarchy.
            let controller = self.ivars().controller.borrow().as_ref().and_then(|owner| owner.load());
            if let Some(controller) = controller {
                controller.sync_backdrop();
                controller.update_hover();
            }
        }
    }
);
define_class!(
    // SAFETY: NSClipView clips the reveal animation but never consumes wheel input.
    #[unsafe(super(NSClipView))]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    struct RevealClipView;
    unsafe impl NSObjectProtocol for RevealClipView {}
    impl RevealClipView {
        #[unsafe(method(scrollWheel:))]
        fn wheel(&self, event: &NSEvent) { route_wheel(self,event); }
    }
);

#[derive(Default, Debug)]
struct HoverState {
    background: OnceCell<Retained<NSBox>>,
    active: Cell<bool>,
    focused: Cell<bool>,
    matches: OnceCell<Vec<std::ops::Range<usize>>>,
    title: OnceCell<Retained<NSTextField>>,
    arrow: OnceCell<Retained<NSImageView>>,
    thread: OnceCell<Retained<SessionThread>>,
    destination: OnceCell<Retained<StatusContentView>>,
    destination_arrow: OnceCell<Retained<NSImageView>>,
    destination_icon: OnceCell<Retained<StatusContentView>>,
}
define_class!(
    // SAFETY: This NSView only stores its hover decoration on the main thread.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = HoverState]
    #[derive(Debug)]
    struct SessionRowView;
    unsafe impl NSObjectProtocol for SessionRowView {}
);
impl SessionRowView {
    fn create(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let allocated = Self::alloc(mtm).set_ivars(HoverState::default());
        // SAFETY: NSView's initializer is valid for this subclass and its ivars.
        let row: Retained<Self> = unsafe { msg_send![super(allocated), initWithFrame: frame] };
        if row.respondsToSelector(sel!(setClipsToBounds:)) {
            row.setClipsToBounds(false);
        }
        let background = hover_background(
            mtm,
            rect(
                SESSION_HEADER_HOVER_LEFT,
                frame.size.height - SESSION_HEADER_HEIGHT + 3.0,
                frame.size.width - 2.0 * SESSION_HEADER_HOVER_LEFT,
                SESSION_HEADER_HEIGHT - 6.0,
            ),
            15.0,
        );
        if let Some(surface) = background.downcast_ref::<GlassBox>() {
            surface.ivars().kind.set(6);
        }
        row.addSubview(&background);
        row.ivars()
            .background
            .set(background)
            .expect("background initialized once");
        row
    }
    fn set_highlight(&self, active: bool, focused: bool) {
        let changed = self.ivars().active.replace(active) != active;
        let changed = (self.ivars().focused.replace(focused) != focused) || changed;
        if !changed {
            return;
        }
        if let Some(background) = self.ivars().background.get() {
            if let Some(surface) = background.downcast_ref::<GlassBox>() {
                surface.ivars().focused.set(focused);
                surface.ivars().hovered.set(active);
            }
            let color = if focused {
                NSColor::controlAccentColor()
            } else {
                hover_fill_color()
            };
            background.setFillColor(&color);
            paint_hover(background, if active || focused { 1.0 } else { 0.0 });
        }
        if let Some(icon) = self.ivars().destination_icon.get() {
            icon.setFrameOrigin(NSPoint::new(
                if active || focused { 18.0 } else { 0.0 },
                0.0,
            ));
        }
        if let Some(arrow) = self.ivars().destination_arrow.get() {
            arrow.setHidden(!active && !focused);
            let color = if focused {
                primary_text_color()
            } else {
                secondary_text_color()
            };
            arrow.setContentTintColor(Some(&color));
        }
        self.paint_title(focused);
        if let Some(arrow) = self.ivars().arrow.get() {
            arrow.setContentTintColor(Some(&secondary_text_color()));
        }
    }
    fn paint_title(&self, _focused: bool) {
        let Some(title) = self.ivars().title.get() else {
            return;
        };
        let text = title.stringValue();
        let attributed =
            NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &text);
        let full = NSRange::new(0, text.length());
        let color = primary_text_color();
        // SAFETY: Attribute values have AppKit's documented NSColor/NSFont types;
        // all ranges refer to UTF-16 positions in this exact displayed title.
        unsafe {
            attributed.addAttribute_value_range(NSForegroundColorAttributeName, &color, full);
            if let Some(font) = title.font() {
                attributed.addAttribute_value_range(NSFontAttributeName, &font, full);
            }
            for range in self.ivars().matches.get().into_iter().flatten() {
                let range = NSRange::new(range.start, range.end - range.start);
                attributed.addAttribute_value_range(
                    NSBackgroundColorAttributeName,
                    &NSColor::findHighlightColor(),
                    range,
                );
                attributed.addAttribute_value_range(
                    NSForegroundColorAttributeName,
                    &NSColor::blackColor(),
                    range,
                );
            }
        }
        title.setAttributedStringValue(&attributed);
    }
}
fn hover_background(mtm: MainThreadMarker, frame: NSRect, radius: f64) -> Retained<NSBox> {
    let background = glass_box(mtm, frame, 0);
    background.setBoxType(NSBoxType::Custom);
    background.setTitlePosition(NSTitlePosition::NoTitle);
    background.setBorderWidth(0.0);
    background.setCornerRadius(radius);
    background.setFillColor(&hover_fill_color());
    background.setWantsLayer(true);
    background.setHidden(true);
    background
}

fn paint_hover(background: &NSBox, opacity: f64) {
    // Explicitly invalidate the parent when showing or removing a translucent
    // decoration. Updating alpha alone can leave the last cached drawing intact.
    background.setAlphaValue(if opacity > 0.0 { opacity } else { 1.0 });
    background.setHidden(opacity <= 0.0);
    background.setNeedsDisplay(true);
    // SAFETY: UI hierarchy access is confined to the main thread; the retained
    // decoration and its parent cannot be removed concurrently.
    if let Some(parent) = unsafe { background.superview() } {
        parent.setNeedsDisplayInRect(background.frame());
    }
}
fn allow_status_overflow(view: &NSView) {
    if view.respondsToSelector(sel!(setClipsToBounds:)) {
        view.setClipsToBounds(false);
    }
    // SAFETY: Status-item views and their parents are accessed only on the UI thread.
    let mut ancestor = unsafe { view.superview() };
    while let Some(parent) = ancestor {
        if parent.respondsToSelector(sel!(setClipsToBounds:)) {
            parent.setClipsToBounds(false);
        }
        // SAFETY: The retained parent cannot be removed concurrently on this thread.
        ancestor = unsafe { parent.superview() };
    }
}

struct ButtonHover {
    button: Retained<NSButton>,
    background: Retained<NSBox>,
    base_alpha: f64,
    radio: Option<inline_questions::Radio>,
    caption: Option<Retained<NSTextField>>,
    symbol: Option<Retained<visual::DecisionIcon>>,
    focusable_when_disabled: bool,
    gutter: Option<GutterHover>,
}

struct GutterHover {
    message: NSRect,
    icon: Retained<StatusContentView>,
}

struct LayoutRow {
    view: Retained<NSView>,
    row: Option<Retained<SessionRowView>>,
    id: Option<String>,
    gap: f64,
    base_height: f64,
    detail_height: f64,
    detail: Option<Retained<RevealClipView>>,
    arrow: Option<Retained<NSImageView>>,
    robot: Retained<SessionRobot>,
}

/// Use one row only when the entire set fits; otherwise keep natural widths
/// in a left-aligned column. The final gap separates controls from metadata.
#[cfg(test)]
fn answer_button_layout(sizes: &[NSSize], available: f64) -> (Vec<NSRect>, f64) {
    let halo = 2.0 * BUTTON_HOVER_PADDING;
    let gap = halo + 6.0;
    if sizes.is_empty() {
        return (vec![], 0.0);
    }
    let width = sizes.iter().map(|size| size.width).sum::<f64>()
        + gap * sizes.len().saturating_sub(1) as f64;
    if width <= available {
        let height = sizes.iter().map(|size| size.height).fold(0.0, f64::max);
        let mut x = 0.0;
        let frames = sizes
            .iter()
            .map(|size| {
                let frame = rect(x, (height - size.height) / 2.0, size.width, size.height);
                x += size.width + gap;
                frame
            })
            .collect();
        (frames, height + halo)
    } else {
        let mut y = 0.0;
        let frames = sizes
            .iter()
            .map(|size| {
                let frame = rect(0.0, y, size.width, size.height);
                y += size.height + gap;
                frame
            })
            .collect();
        (frames, y - gap + halo)
    }
}

#[cfg(test)]
fn messages_before_request(session: &Session, history: &MessageHistory) -> String {
    earlier_messages(session, history)
        .iter()
        .map(|(text, _)| *text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn earlier_messages<'a>(
    session: &'a Session,
    history: &'a MessageHistory,
) -> Vec<(&'a str, &'a [crate::domain::media::Image])> {
    let mut messages: Vec<_> = history
        .messages
        .iter()
        .map(|message| (message.text.trim_end(), message.images.as_slice()))
        .collect();
    let latest = session.last_message().trim_end();
    let anchor = history.anchor.trim_end();
    if !history.messages.is_empty() && !anchor.is_empty() && anchor != latest {
        messages.push((anchor, &history.anchor_images));
    }
    if session.approval().is_some() && !latest.is_empty() {
        messages.push((latest, session.images()));
    }
    messages
}

/// Wrap natural-width decisions as whole blocks, including a scope disclosure.
/// Expanded scopes follow all decision rows and never move their buttons.
fn request_action_layout(
    sizes: &[NSSize],
    contexts: &[Option<f64>],
    width: f64,
) -> (Vec<NSRect>, f64) {
    let mut frames = Vec::new();
    let (mut x, mut y, mut row_height) = (0.0, 0.0, 0.0_f64);
    for size in sizes {
        // Width already includes the embedded trailing disclosure segment.
        let block_width = size.width;
        if x > 0.0 && x + block_width > width {
            x = 0.0;
            y += row_height + 10.0;
            row_height = 0.0;
        }
        frames.push(rect(x, y, size.width, size.height));
        row_height = row_height.max(size.height);
        x += block_width + 10.0;
    }
    let mut height = y + row_height;
    for context in contexts.iter().flatten().filter(|height| **height > 0.0) {
        height += 4.0 + context;
    }
    (frames, height)
}

fn request_scope_y(frames: &[NSRect], contexts: &[Option<f64>], index: usize) -> f64 {
    frames
        .iter()
        .map(|frame| frame.origin.y + frame.size.height)
        .fold(0.0, f64::max)
        + 4.0
        + contexts[..index]
            .iter()
            .flatten()
            .filter(|height| **height > 0.0)
            .map(|height| height + 4.0)
            .sum::<f64>()
}

/// Left-to-right status-item order: pending requests, unseen completions, working.
fn status_counter_positions(widths: [Option<f64>; 3]) -> ([f64; 3], f64) {
    let mut positions = [26.0; 3];
    let mut end = 26.0;
    let mut first = true;
    for (index, width) in widths.into_iter().enumerate() {
        if let Some(width) = width {
            end += if first { 2.0 } else { 5.0 };
            positions[index] = end;
            end += width;
            first = false;
        }
    }
    (positions, end - 26.0)
}

fn request_glyph(control: &RequestControl) -> Option<visual::DecisionGlyph> {
    match control.label.as_str() {
        "Allow once" | "Allow" => Some(visual::DecisionGlyph::Check),
        "Deny" => Some(visual::DecisionGlyph::Cross),
        _ => None,
    }
}

fn command_action(control: &RequestControl) -> bool {
    control.detail.trim().is_empty()
        && matches!(control.label.as_str(), "Allow once" | "Allow")
        && matches!(control.kind, ControlKind::Reply(Response::Action(_)))
}

// AppKit may re-enter callbacks while a window is shown or reconciled.
// Keep the existing forms alive without holding a registry borrow.
fn open_owned_form<T: Clone>(
    forms: &RefCell<Vec<T>>,
    create: impl FnOnce(&[T]) -> Option<T>,
    reconcile: impl FnOnce(&T),
) {
    let existing = forms.borrow().clone();
    if let Some(form) = create(&existing) {
        reconcile(&form);
        forms.borrow_mut().push(form);
    }
}

fn shortcut_needs_sync(
    applied: &Option<hotkey::Shortcut>,
    desired: &Option<hotkey::Shortcut>,
    menu_open: bool,
    recording: bool,
) -> bool {
    !menu_open && !recording && applied != desired
}

fn menu_navigation_key(key: u16, modifiers: NSEventModifierFlags) -> bool {
    matches!(key, 125 | 126)
        && !modifiers.intersects(
            NSEventModifierFlags::Command
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Option,
        )
}

// Mutually exclusive presentation states; unseen completion is a subset of
// Idle, never an additional count for a working/waiting session.
fn overview_counts(sessions: &[Session], mut finished: impl FnMut(&Session) -> bool) -> [usize; 5] {
    let mut counts = [0; 5];
    for session in sessions {
        let index = if session.approval().is_some() || session.activity == Activity::Confirmation {
            0
        } else {
            match session.activity {
                Activity::Working => 1,
                Activity::Idle if finished(session) => 2,
                Activity::Idle => 3,
                _ => 4,
            }
        };
        counts[index] += 1;
    }
    counts
}

fn adjacent_session(current: Option<usize>, count: usize, forward: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match current.filter(|index| *index < count) {
        None => {
            if forward {
                0
            } else {
                count - 1
            }
        }
        Some(index) => {
            if forward {
                (index + 1).min(count - 1)
            } else {
                index.saturating_sub(1)
            }
        }
    })
}

fn horizontal_answer(
    current: &str,
    buttons: &[(String, f64, bool)],
    forward: bool,
) -> Option<String> {
    let (_, center, _) = buttons.iter().find(|(id, _, _)| id == current)?;
    if buttons.iter().any(|(_, y, _)| (y - center).abs() > 0.5) {
        return None;
    }
    let enabled: Vec<_> = buttons.iter().filter(|(_, _, enabled)| *enabled).collect();
    let index = enabled.iter().position(|(id, _, _)| id == current)?;
    let next = if forward {
        (index + 1) % enabled.len()
    } else {
        (index + enabled.len() - 1) % enabled.len()
    };
    Some(enabled[next].0.clone())
}

fn keyboard_order(
    sessions: impl IntoIterator<Item = (String, bool)>,
    answers: &[(String, String)],
) -> Vec<String> {
    let mut targets = Vec::new();
    for (session, expanded) in sessions {
        targets.push(session.clone());
        if expanded {
            targets.extend(
                answers
                    .iter()
                    .filter(|(_, owner)| *owner == session)
                    .map(|(id, _)| id.clone()),
            );
        }
    }
    targets
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

pub(super) fn detail_text_color(approval: bool) -> Retained<NSColor> {
    let (dark_rgb, light_rgb) = if approval {
        ([245.0, 242.0, 255.0], [41.0, 36.0, 59.0])
    } else {
        ([186.0, 178.0, 208.0], [101.0, 91.0, 118.0])
    };
    appearance_color(light_rgb, dark_rgb, 1.0)
}

fn footer_gap(bounds: NSRect, footer: NSRect, flipped: bool) -> NSRect {
    let (y, height) = if flipped {
        let edge = footer.origin.y + footer.size.height;
        (edge, bounds.origin.y + bounds.size.height - edge)
    } else {
        (bounds.origin.y, footer.origin.y - bounds.origin.y)
    };
    rect(footer.origin.x, y, footer.size.width, height.max(0.0))
}

fn center_label(label: &NSTextField, center_y: f64) {
    let mut frame = label.frame();
    // A title may contain pasted attachment metadata with newlines. Never size
    // a one-line row using the unconstrained height of its text contents.
    if let Some(font) = label.font() {
        frame.size.height = (font.ascender() - font.descender() + font.leading()).ceil() + 2.0;
    }
    frame.origin.y = center_y - frame.size.height / 2.0;
    label.setFrame(frame);
}

impl MenuController {
    fn create_popup(&self, menu: &NSMenu) -> Retained<Self> {
        let allocated = Self::alloc(self.mtm()).set_ivars(State::default());
        // SAFETY: Same initializer as the resident, with independent UI storage.
        let popup: Retained<Self> = unsafe { msg_send![super(allocated), init] };
        let state = popup.ivars();
        state.popup_owner.set(objc2::rc::Weak::new(self)).ok();
        state.detached_frame.set(self.ivars().detached_frame.get());
        state.monitor.set(self.monitor().clone()).ok();
        state.revision.set(u64::MAX);
        state
            .ui_sender
            .set(self.ivars().ui_sender.get().unwrap().clone())
            .ok();
        state.menu.set(Retained::from(menu)).ok();
        menu.removeAllItems();
        if let Some(status) = self.ivars().status.get() {
            state.status.set(status.clone()).ok();
        }
        *state.applications.borrow_mut() =
            self.monitor().snapshot().preferences.applications.clone();
        *state.telegram_auto_open.borrow_mut() =
            std::mem::take(&mut *self.ivars().telegram_auto_open.borrow_mut());
        *state.expanded.borrow_mut() = std::mem::take(&mut *self.ivars().expanded.borrow_mut());
        *state.keyboard_focus.borrow_mut() = self.ivars().keyboard_focus.borrow_mut().take();
        *state.previous_menu_application.borrow_mut() =
            self.ivars().previous_menu_application.borrow_mut().take();
        *state.composer_drafts.borrow_mut() =
            std::mem::take(&mut *self.ivars().composer_drafts.borrow_mut());
        *state.question_drafts.borrow_mut() =
            std::mem::take(&mut *self.ivars().question_drafts.borrow_mut());
        *state.message_history.borrow_mut() =
            std::mem::take(&mut *self.ivars().message_history.borrow_mut());
        state
            .history_generation
            .set(self.ivars().history_generation.get());
        popup.build_menu();
        popup
    }

    fn save_popup_data(&self, popup: &Self) {
        let source = popup.ivars();
        let target = self.ivars();
        let mut composers = std::mem::take(&mut *source.composer_drafts.borrow_mut());
        composers.extend(
            source
                .composers
                .borrow()
                .iter()
                .map(|(key, composer)| (key.clone(), composer.draft())),
        );
        *target.composer_drafts.borrow_mut() = composers;
        let mut questions = std::mem::take(&mut *source.question_drafts.borrow_mut());
        questions.extend(
            source
                .question_panels
                .borrow()
                .iter()
                .map(|(key, panel)| (key.clone(), panel.draft())),
        );
        *target.question_drafts.borrow_mut() = questions;
        *target.message_history.borrow_mut() =
            std::mem::take(&mut *source.message_history.borrow_mut());
        target
            .history_generation
            .set(source.history_generation.get());
    }

    fn menu_width(&self) -> f64 {
        let width = self.ivars().popup_width.get();
        if width >= MENU_WIDTH {
            width
        } else {
            MENU_WIDTH
        }
    }
    fn list_width(&self) -> f64 {
        self.menu_width() - 22.0
    }
    fn detail_width(&self) -> f64 {
        DETAIL_WIDTH + self.menu_width() - MENU_WIDTH
    }
    fn header_hover_width(&self) -> f64 {
        self.list_width() - 2.0 * SESSION_HEADER_HOVER_LEFT
    }

    fn change_width(&self, width: f64) {
        let maximum = self
            .ivars()
            .root
            .get()
            .and_then(|v| v.window())
            .and_then(|w| w.screen())
            .map_or(1000.0, |s| s.visibleFrame().size.width - 8.0);
        let width = width.clamp(MENU_WIDTH.min(maximum), maximum).round();
        if (self.menu_width() - width).abs() < 1.0 {
            return;
        }
        self.ivars().popup_width.set(width);
        if !self.is_detached() {
            self.ivars().pinned_width.set(width);
        }
        if let Some(menu) = self.ivars().menu.get() {
            menu.setMinimumWidth(width);
        }
        self.render_footer();
        self.render_rows(&self.ivars().filtered_sessions());
        self.resize_content(self.ivars().content_height.get());
    }

    fn sync_window_width(&self) {
        if !self.ivars().menu_open.get() || self.is_detached() {
            return;
        }
        let Some(root) = self.ivars().root.get() else {
            return;
        };
        let Some(window) = root.window() else {
            return;
        };
        let width = self.menu_width();
        // NSMenu caches its widest measured item. Its tracking window can grow
        // back to that cached width after our custom item has already shrunk.
        // The controller owns both widths; never add an old window/item delta.
        root.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        let mut frame = window.frame();
        if (frame.size.width - width).abs() > 0.5 {
            frame.size.width = width;
            window.setFrame_display(frame, false);
        }
        if (root.frame().size.width - width).abs() > 0.5 {
            root.setFrameSize(NSSize::new(width, root.frame().size.height));
        }
    }

    pub fn shutdown(&self) {
        self.remember_detached_frame();
        if let Some(window) = self.ivars().detached_window.borrow_mut().take() {
            window.setDelegate(None);
            window.close();
            self.set_detached_application_policy(false);
        }
        if let Some(popup) = self.ivars().popup.borrow_mut().take() {
            popup.close_panel();
            popup.shutdown();
        }
        if let Some(timer) = self.ivars().snapshot_timer.get() {
            timer.invalidate();
        }
        if let Some(timer) = self.ivars().status_timer.borrow_mut().take() {
            timer.invalidate();
        }
        self.ivars().shortcut_registration.borrow_mut().take();
        self.stop_keyboard_tracking();
        // SAFETY: Both registrations belong to this retained main-thread controller.
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(self);
            NSDistributedNotificationCenter::defaultCenter().removeObserver(self);
        }
    }
    fn monitor(&self) -> &crate::actors::monitor::Handle {
        self.ivars().monitor.get().expect("monitor injected")
    }
    fn show_form(
        &self,
        session: Session,
        draft: Option<std::collections::BTreeMap<String, Vec<String>>>,
    ) {
        if let Some(owner) = self
            .ivars()
            .popup_owner
            .get()
            .and_then(|owner| owner.load())
        {
            owner.show_form(session, draft);
            return;
        }
        open_owned_form(
            &self.ivars().forms,
            |forms| {
                answers::show_with_draft(
                    session,
                    draft,
                    self.monitor().clone(),
                    self.ivars().ui_sender.get().unwrap().clone(),
                    forms,
                )
            },
            |form| form.reconcile(&self.monitor().snapshot()),
        );
    }
    fn drain_ui(&self) {
        use crate::macos::bridge::Event;
        let Some(receiver) = self.ivars().ui_incoming.get() else {
            return;
        };
        loop {
            let event = receiver.borrow_mut().try_recv();
            let Ok(event) = event else {
                break;
            };
            match event {
                Event::Stop => crate::macos::native::stop_application(self.mtm()),
                Event::Toggle => {
                    if self.ivars().menu_open.get() {
                        self.dismiss_menu();
                    } else {
                        self.show();
                    }
                }
                Event::FormClosed(key) => self
                    .ivars()
                    .forms
                    .borrow_mut()
                    .retain(|form| form.key() != Some(key.clone())),
                Event::Platform(command) => self
                    .ivars()
                    .platform
                    .get()
                    .unwrap()
                    .borrow_mut()
                    .apply(command),
                Event::ShowRequest {
                    session,
                    form,
                    draft,
                } => {
                    if form {
                        self.dismiss_menu();
                        self.show_form(*session, draft);
                    } else {
                        let popup = self.ivars().popup.borrow().clone();
                        let controller = popup.as_deref().unwrap_or(self);
                        let key = session.key();
                        controller.ivars().expanded.borrow_mut().insert(key.clone());
                        *controller.ivars().keyboard_focus.borrow_mut() = Some(key);
                        controller.ivars().filter_dirty.set(true);
                        if controller.ivars().menu_open.get() {
                            controller.hide_settings();
                            controller.filter();
                            controller.focus_answer();
                            controller.focus_detached();
                        } else {
                            self.show();
                        }
                    }
                }
            }
        }
    }

    fn clear_menu_shortcut(&self) {
        self.end_shortcut_recording(false);
        self.ivars().shortcut_registration.borrow_mut().take();
        let _ = self
            .monitor()
            .send(crate::actors::monitor::Command::Platform(
                crate::macos::bridge::PlatformCommand::Shortcut(None),
            ));
        self.ivars().shortcut_error.borrow_mut().take();
        self.refresh_settings();
    }

    fn install_shortcut(&self) {
        if self.ivars().popup_owner.get().is_some() {
            self.configure_menu_shortcut();
            return;
        }
        let old = self.ivars().shortcut_registration.borrow_mut().take();
        drop(old);
        self.ivars().shortcut_error.borrow_mut().take();
        self.configure_menu_shortcut();
        let shortcut = self.monitor().snapshot().preferences.shortcut.clone();
        *self.ivars().shortcut_applied.borrow_mut() = shortcut.clone();
        if let Some(shortcut) = shortcut {
            if self.ivars().menu_open.get() {
                return;
            }
            match hotkey::Registration::new(
                &shortcut,
                self.mtm(),
                self.ivars().ui_sender.get().unwrap().clone(),
            ) {
                Ok(registration) => {
                    *self.ivars().shortcut_registration.borrow_mut() = Some(registration)
                }
                Err(error) => *self.ivars().shortcut_error.borrow_mut() = Some(error.to_string()),
            }
        }
    }

    fn configure_menu_shortcut(&self) {
        if let Some(item) = self.ivars().menu_close_shortcut.get() {
            if let Some(shortcut) = self.monitor().snapshot().preferences.shortcut.clone() {
                item.setKeyEquivalent(&NSString::from_str(&shortcut.menu_key_equivalent()));
                item.setKeyEquivalentModifierMask(shortcut.cocoa_modifiers());
                item.setEnabled(true);
            } else {
                item.setKeyEquivalent(ns_string!(""));
                item.setEnabled(false);
            }
        }
    }

    fn begin_shortcut_recording(&self) {
        if self.ivars().shortcut_monitor.borrow().is_some() {
            self.end_shortcut_recording(true);
            return;
        }
        // Temporarily release the existing key so it can be recorded again.
        let old = self.ivars().shortcut_registration.borrow_mut().take();
        drop(old);
        self.ivars().shortcut_error.borrow_mut().take();
        let weak = objc2::rc::Weak::new(self);
        let callback = block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| {
            let handled = weak.load().is_some_and(|controller| {
                controller.record_shortcut_key(unsafe { event.as_ref() })
            });
            if handled {
                std::ptr::null_mut()
            } else {
                event.as_ptr()
            }
        });
        // SAFETY: This retained local monitor only receives events in this app.
        // Its block borrows no controller; removal always uses the returned token.
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &callback)
        };
        if let Some(monitor) = monitor {
            *self.ivars().shortcut_monitor.borrow_mut() = Some(monitor);
        } else {
            self.install_shortcut();
            *self.ivars().shortcut_error.borrow_mut() =
                Some("Could not start shortcut recording.".into());
        }
        self.refresh_settings();
    }

    fn end_shortcut_recording(&self, restore: bool) {
        let monitor = self.ivars().shortcut_monitor.borrow_mut().take();
        if let Some(monitor) = monitor {
            // SAFETY: This is exactly the token returned by our local monitor.
            unsafe {
                NSEvent::removeMonitor(&monitor);
            }
            if restore {
                self.install_shortcut();
            }
            self.refresh_settings();
        }
    }

    fn record_shortcut_key(&self, event: &NSEvent) -> bool {
        if !self.ivars().settings_active.get() || !self.ivars().menu_open.get() {
            self.end_shortcut_recording(true);
            return false;
        }
        if event.isARepeat() {
            return true;
        }
        let flags = event.modifierFlags();
        let modified = flags.intersects(
            NSEventModifierFlags::Command
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Shift,
        );
        if !modified && (event.keyCode() == 53 || event.keyCode() == 48) {
            self.end_shortcut_recording(true);
            return event.keyCode() != 48;
        }
        if !modified && (event.keyCode() == 51 || event.keyCode() == 117) {
            self.clear_menu_shortcut();
            return true;
        }
        let characters = event
            .charactersByApplyingModifiers(NSEventModifierFlags::empty())
            .map(|text| text.to_string())
            .unwrap_or_default();
        let Some(shortcut) = hotkey::Shortcut::from_event(event.keyCode(), flags, &characters)
        else {
            *self.ivars().shortcut_error.borrow_mut() = Some("Use ⌘, ⌥ or ⌃ with a key.".into());
            self.refresh_settings();
            return true;
        };
        match hotkey::Registration::new(
            &shortcut,
            self.mtm(),
            self.ivars().ui_sender.get().unwrap().clone(),
        ) {
            Ok(registration) => {
                *self.ivars().shortcut_applied.borrow_mut() = Some(shortcut.clone());
                *self.ivars().shortcut_registration.borrow_mut() = Some(registration);
                let _ = self
                    .monitor()
                    .send(crate::actors::monitor::Command::Platform(
                        crate::macos::bridge::PlatformCommand::Shortcut(Some(shortcut)),
                    ));
                self.ivars().shortcut_error.borrow_mut().take();
                self.end_shortcut_recording(false);
            }
            Err(error) => {
                *self.ivars().shortcut_error.borrow_mut() = Some(error.to_string());
                self.refresh_settings();
            }
        }
        true
    }

    #[allow(deprecated)] // Explicit user menu actions must also work on older supported macOS.
    fn acquire_menu_cursor_owner(&self) {
        let app = NSApplication::sharedApplication(self.mtm());
        if app.isActive() || self.ivars().previous_menu_application.borrow().is_some() {
            return;
        }
        let own_application = NSRunningApplication::currentApplication();
        *self.ivars().previous_menu_application.borrow_mut() = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .filter(|previous| previous != &own_application);
        // NSCursor.set only changes this process's cursor while another app owns
        // the foreground cursor. Give the open interactive menu that ownership.
        app.activateIgnoringOtherApps(true);
    }

    #[allow(deprecated)]
    fn restore_menu_application(&self) {
        let previous = self.ivars().previous_menu_application.borrow_mut().take();
        let own_application = NSRunningApplication::currentApplication();
        if let Some(previous) = previous
            && !previous.isTerminated()
            && NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .is_some_and(|current| current == own_application)
        {
            // A user switch to another app wins; never switch back over it.
            previous.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        }
    }

    fn dismiss_menu(&self) {
        // Navigation and answers dismiss transient menus, but must leave the
        // deliberately detached window available beside the agent application.
        if self.is_detached() {
            return;
        }
        if self.ivars().menu_open.get()
            && let Some(menu) = self.ivars().menu.get()
        {
            menu.cancelTracking();
        }
    }

    fn show(&self) {
        let state = self.ivars();
        let popup = state.popup.borrow().clone();
        if let Some(popup) = popup
            && popup.focus_detached()
        {
            return;
        }
        if state.menu_open.get() {
            return;
        }
        self.receive_sessions();
        self.refresh();
        if let Some(button) = state.status.get().and_then(|item| item.button(self.mtm()))
            && button.window().is_some()
        {
            self.acquire_menu_cursor_owner();
            // SAFETY: AppKit owns the attached status menu's button action.
            unsafe {
                button.performClick(None);
            }
        }
    }

    fn is_detached(&self) -> bool {
        self.ivars().detached_window.borrow().is_some()
    }

    #[allow(deprecated)]
    fn focus_detached(&self) -> bool {
        let window = self.ivars().detached_window.borrow().clone();
        let Some(window) = window else {
            return false;
        };
        if NSApplication::sharedApplication(self.mtm()).activationPolicy()
            == NSApplicationActivationPolicy::Prohibited
        {
            return true;
        }
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
        NSApplication::sharedApplication(self.mtm()).activateIgnoringOtherApps(true);
        window.makeKeyAndOrderFront(None);
        true
    }

    /// Reparent only after NSMenu has ended tracking. The same views retain
    /// their drafts, expanded rows and scroll position across both hosts.
    fn detach_window(&self) {
        let state = self.ivars();
        let Some(root) = state.root.get() else {
            return;
        };
        let origin = root.window().map(|w| w.frame().origin);
        state.pinned_width.set(self.menu_width());
        self.suspend_host_editors();
        for extension in [state.header_extension.get(), state.footer_extension.get()]
            .into_iter()
            .flatten()
        {
            extension.removeFromSuperview();
        }
        if let Some(item) = state.menu_item.get() {
            item.setView(None);
        }
        root.removeFromSuperview();
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(self.mtm()),
                root.bounds(),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // The resident retains the popup, which retains this native window.
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        window.setTitle(ns_string!("AI Agent Monitor"));
        window.setContentMinSize(NSSize::new(MENU_WIDTH, 200.0));
        *state.detached_window.borrow_mut() = Some(window.clone());
        window.setContentView(Some(root));
        if let Some(origin) = origin {
            window.setFrameOrigin(origin);
        } else {
            window.center();
        }
        if let Some(frame) = state.detached_frame.get() {
            window.setFrame_display(window.constrainFrameRect_toScreen(frame, None), false);
        } else if self.persists_window_geometry() {
            window.setFrameUsingName(ns_string!("MonitorDetachedWindow"));
        }
        root.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        if let Some(button) = state.pin_button.get() {
            button.setHidden(true);
        }
        if let Some(background) = state.pin_background.get() {
            background.setHidden(true);
        }
        let accessory = NSTitlebarAccessoryViewController::new(self.mtm());
        accessory.setLayoutAttribute(NSLayoutAttribute::Left);
        let pin = NSButton::initWithFrame(NSButton::alloc(self.mtm()), rect(0.0, 0.0, 28.0, 24.0));
        pin.setBordered(false);
        pin.setToolTip(Some(ns_string!("Pin to menu bar")));
        pin.setImage(
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("pin.fill"),
                Some(ns_string!("Pin to menu bar")),
            )
            .as_deref(),
        );
        unsafe {
            pin.setTarget(Some(self));
            pin.setAction(Some(sel!(togglePin:)));
        }
        accessory.setView(&pin);
        window.addTitlebarAccessoryViewController(&accessory);
        window.setDelegate(Some(ProtocolObject::from_ref(self)));
        state.menu_open.set(true);
        if let Some(menu) = state.menu.get() {
            menu.removeAllItems();
            add_popup_placeholder(menu, self.mtm());
        }
        self.layout_detached();
        self.remember_detached_frame();
        self.resume_host_editors();
        self.set_detached_application_policy(true);
        // Activation happens after menuDidClose returns, so the old tracking
        // window cannot steal focus back while it completes dismissal.
        if let Some(owner) = state.popup_owner.get().and_then(|o| o.load()) {
            unsafe {
                owner.performSelector_withObject_afterDelay(sel!(showDetached:), None, 0.0);
            }
        } else {
            self.focus_detached();
        }
        self.start_pointer_tracking();
    }

    fn layout_detached(&self) {
        let state = self.ivars();
        if state.detached_layout.replace(true) {
            return;
        }
        if let Some(window) = state.detached_window.borrow().clone()
            && let Some(root) = state.root.get()
        {
            let size = window.contentRectForFrameRect(window.frame()).size;
            root.setFrameSize(size);
            if (state.popup_width.replace(size.width) - size.width).abs() > 0.5 {
                self.render_rows(&state.filtered_sessions());
            }
            self.render_footer();
            self.resize_content(state.content_height.get());
        }
        state.detached_layout.set(false);
    }

    fn reattach_window(&self) {
        let state = self.ivars();
        self.remember_detached_frame();
        let Some(window) = state.detached_window.borrow_mut().take() else {
            return;
        };
        self.suspend_host_editors();
        window.setDelegate(None);
        if let Some(root) = state.root.get() {
            root.removeFromSuperview();
            root.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
            state.menu_item.get().unwrap().setView(Some(root));
        }
        window.setContentView(None);
        window.close();
        self.set_detached_application_policy(false);
        if let Some(button) = state.pin_button.get() {
            button.setHidden(false);
        }
        if let Some(background) = state.pin_background.get() {
            background.setHidden(false);
        }
        if let Some(menu) = state.menu.get() {
            menu.removeAllItems();
            menu.addItem(state.menu_item.get().unwrap());
            menu.addItem(state.menu_close_shortcut.get().unwrap());
        }
        state.menu_open.set(false);
        state.reattaching.set(true);
        state.resume_editors_on_attach.set(true);
        self.stop_pointer_tracking();
        self.stop_keyboard_tracking();
        self.change_width(state.pinned_width.get().max(MENU_WIDTH));
        self.resize_content(state.content_height.get());
        if NSApplication::sharedApplication(self.mtm()).activationPolicy()
            != NSApplicationActivationPolicy::Prohibited
            && let Some(owner) = state.popup_owner.get().and_then(|o| o.load())
        {
            unsafe {
                owner.performSelector_withObject_afterDelay(sel!(showPinned:), None, 0.0);
            }
        }
    }

    fn suspend_host_editors(&self) {
        for composer in self.ivars().composers.borrow().values() {
            composer.suspend();
        }
        for panel in self.ivars().question_panels.borrow().values() {
            panel.suspend_editing();
        }
        if let Some(window) = self.ivars().root.get().and_then(|root| root.window()) {
            unsafe {
                window.endEditingFor(None);
            }
        }
    }

    fn resume_host_editors(&self) {
        for composer in self.ivars().composers.borrow().values() {
            composer.resume();
        }
        for panel in self.ivars().question_panels.borrow().values() {
            panel.resume_editing();
        }
    }

    fn dispose_detached(&self) {
        let state = self.ivars();
        self.remember_detached_frame();
        let window = state.detached_window.borrow_mut().take();
        if let Some(window) = window {
            window.setDelegate(None);
        }
        self.set_detached_application_policy(false);
        state.menu_open.set(false);
        self.close_panel();
        if let Some(owner) = state.popup_owner.get().and_then(|o| o.load()) {
            owner.save_popup_data(self);
            *owner.ivars().telegram_auto_open.borrow_mut() =
                std::mem::take(&mut *state.telegram_auto_open.borrow_mut());
            owner.ivars().popup.borrow_mut().take();
            owner.install_shortcut();
        }
        self.shutdown();
    }

    fn persists_window_geometry(&self) -> bool {
        // Hidden AppKit verification must never change the user's preferences.
        NSApplication::sharedApplication(self.mtm()).activationPolicy()
            != NSApplicationActivationPolicy::Prohibited
    }

    fn set_detached_application_policy(&self, detached: bool) {
        let app = NSApplication::sharedApplication(self.mtm());
        // Keep hidden verification processes out of Dock and the app switcher.
        if app.activationPolicy() == NSApplicationActivationPolicy::Prohibited {
            return;
        }
        let policy = if detached {
            NSApplicationActivationPolicy::Regular
        } else {
            NSApplicationActivationPolicy::Accessory
        };
        if app.activationPolicy() != policy && !app.setActivationPolicy(policy) {
            eprintln!("ai-agent-monitor: could not update detached-window application policy");
        }
    }

    fn remember_detached_frame(&self) {
        let window = self.ivars().detached_window.borrow().clone();
        let Some(window) = window else {
            return;
        };
        let frame = window.frame();
        self.ivars().detached_frame.set(Some(frame));
        if let Some(owner) = self.ivars().popup_owner.get().and_then(|o| o.load()) {
            owner.ivars().detached_frame.set(Some(frame));
        }
        if self.persists_window_geometry() {
            window.saveFrameUsingName(ns_string!("MonitorDetachedWindow"));
        }
    }

    fn create(
        mtm: MainThreadMarker,
        lock: File,
        monitor: crate::actors::monitor::Handle,
        ui: crate::macos::bridge::Sender,
        incoming: tokio::sync::mpsc::Receiver<crate::macos::bridge::Event>,
    ) -> Retained<Self> {
        let allocated = Self::alloc(mtm).set_ivars(State::default());
        // SAFETY: NSObject's initializer is valid for this subclass and its ivars.
        let this: Retained<Self> = unsafe { msg_send![super(allocated), init] };
        this.ivars().lock.set(lock).expect("lock initialized once");
        this.ivars()
            .platform
            .set(RefCell::new(crate::macos::bridge::Platform::new(
                monitor.clone(),
                ui.clone(),
            )))
            .ok();
        this.ivars().monitor.set(monitor).ok();
        this.ivars().ui_sender.set(ui).ok();
        this.ivars().ui_incoming.set(RefCell::new(incoming)).ok();
        let timer = unsafe {
            NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                0.1,
                &this,
                sel!(pollState:),
                None,
                true,
            )
        };
        unsafe {
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        }
        this.ivars().snapshot_timer.set(timer).ok();
        *this.ivars().applications.borrow_mut() =
            this.monitor().snapshot().preferences.applications.clone();
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("AI Agent Monitor"));
        menu.setAutoenablesItems(false);
        menu.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        add_popup_placeholder(&menu, mtm);
        this.ivars()
            .menu
            .set(menu)
            .expect("resident menu initialized once");
        this.build_status_item();
        this.install_shortcut();
        this.refresh();
        // SAFETY: Observer implements showSessions: and is retained for the entire
        // process lifetime. Distributed notifications carry no executable payload.
        unsafe {
            NSDistributedNotificationCenter::defaultCenter()
                .addObserver_selector_name_object_suspensionBehavior(
                    &this,
                    sel!(showSessions:),
                    Some(ns_string!("local.codex.agent-monitor.showSessions")),
                    None,
                    NSNotificationSuspensionBehavior::DeliverImmediately,
                );
        }
        this
    }

    fn show_settings(&self) {
        let state = self.ivars();
        let shell = state.settings_shell.get_or_init(|| {
            let view =
                NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 244.0));
            let note = self.label(
                &view,
                "Open AI Agent Monitor automatically",
                rect(44.0, 190.0, 392.0, 18.0),
                11.0,
                0.0,
                true,
            );
            let separator =
                NSBox::initWithFrame(NSBox::alloc(self.mtm()), rect(24.0, 176.0, 412.0, 1.0));
            separator.setBoxType(NSBoxType::Separator);
            view.addSubview(&separator);
            self.label(
                &view,
                "Animate the eyes while an agent is working",
                rect(44.0, 118.0, 392.0, 18.0),
                11.0,
                0.0,
                true,
            );
            for (y, action, storage, title) in [
                (
                    209.0,
                    sel!(toggleLogin:),
                    &state.login_switch,
                    ns_string!("Launch at login"),
                ),
                (
                    137.0,
                    sel!(toggleMenuAnimation:),
                    &state.animation_switch,
                    ns_string!("Menu bar animation"),
                ),
            ] {
                // SAFETY: The controller implements these selectors and outlives the controls.
                let toggle = unsafe {
                    NSButton::checkboxWithTitle_target_action(title, Some(self), Some(action), self.mtm())
                };
                toggle.setFrame(rect(24.0, y, 412.0, 24.0));
                toggle.setFont(Some(&NSFont::systemFontOfSize(13.0)));
                view.addSubview(&toggle);
                storage
                    .set(toggle)
                    .expect("settings switch initialized once");
            }
            // SAFETY: This target and selector remain valid for the window's lifetime.
            let login_button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    ns_string!("Open Login Items Settings…"),
                    Some(self),
                    Some(sel!(openLoginSettings:)),
                    self.mtm(),
                )
            };
            login_button.setFrame(rect(24.0, 16.0, 240.0, 28.0));
            login_button.setHidden(true);
            view.addSubview(&login_button);
            state
                .login_settings_button
                .set(login_button)
                .expect("login settings button initialized once");
            state
                .login_note
                .set(note)
                .expect("login note initialized once");
            let separator = NSBox::initWithFrame(NSBox::alloc(self.mtm()), rect(24.0, 104.0, 412.0, 1.0));
            separator.setBoxType(NSBoxType::Separator);
            view.addSubview(&separator);
            self.label(&view, "Keyboard shortcut", rect(24.0, 70.0, 220.0, 22.0), 13.0, 0.23, false);
            let note = self.label(&view, "Open the sessions menu from any app", rect(24.0, 48.0, 412.0, 18.0), 11.0, 0.0, true);
            state.shortcut_note.set(note).expect("shortcut note initialized once");
            // SAFETY: Both action selectors are implemented by this retained controller.
            let record = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Record shortcut"), Some(self), Some(sel!(recordShortcut:)), self.mtm()) };
            record.setFrame(rect(264.0, 66.0, 140.0, 28.0));
            record.setToolTip(Some(ns_string!("Click, then press a shortcut. Escape cancels.")));
            view.addSubview(&record);
            state.shortcut_button.set(record).expect("shortcut recorder initialized once");
            // SAFETY: The clear action's target remains valid for the window's lifetime.
            let clear = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("×"), Some(self), Some(sel!(clearShortcut:)), self.mtm()) };
            clear.setFrame(rect(408.0, 66.0, 28.0, 28.0));
            clear.setToolTip(Some(ns_string!("Clear keyboard shortcut")));
            // SAFETY: NSButton supports the accessibility label property.
            unsafe { let _: () = msg_send![&*clear, setAccessibilityLabel: ns_string!("Clear keyboard shortcut")]; }
            view.addSubview(&clear);
            state.shortcut_clear.set(clear).expect("shortcut clear button initialized once");
            let applications_view = NSView::initWithFrame(
                NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 200.0),
            );
            let heading = self.label(&applications_view, "Show sessions in the menu", rect(24.0, 166.0, 412.0, 22.0), 13.0, 0.0, false);
            state.applications_heading.set(heading).expect("applications heading initialized once");
            // SAFETY: Ivar-free flipped NSView, initialized on the UI thread.
            let list: Retained<SessionListView> = unsafe { msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 412.0, 140.0)] };
            let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(self.mtm()), rect(24.0, 16.0, 412.0, 140.0));
            scroll.setDrawsBackground(false);
            scroll.setHasVerticalScroller(true);
            scroll.setAutohidesScrollers(true);
            scroll.setHasHorizontalScroller(false);
            scroll.setDocumentView(Some(&list));
            applications_view.addSubview(&scroll);
            state.applications_scroll.set(scroll).expect("applications scroll initialized once");
            state.applications_list.set(list).expect("application settings initialized once");

            let integrations_view = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 280.0));
            let heading = self.label(&integrations_view, "Detected agents", rect(24.0, 246.0, 412.0, 22.0), 13.0, 0.0, false);
            state.integrations_heading.set(heading).expect("integrations heading initialized once");
            // SAFETY: Flipped, ivar-free document view on the UI thread.
            let integrations: Retained<SessionListView> = unsafe { msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 412.0, 172.0)] };
            let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(self.mtm()), rect(24.0, 64.0, 412.0, 172.0));
            scroll.setDrawsBackground(false);
            scroll.setHasVerticalScroller(true);
            scroll.setAutohidesScrollers(true);
            scroll.setHasHorizontalScroller(false);
            scroll.setDocumentView(Some(&integrations));
            integrations_view.addSubview(&scroll);
            state.integrations_scroll.set(scroll).expect("integrations scroll initialized once");
            state.integrations_list.set(integrations).expect("integrations initialized once");
            let note = settings_note(&integrations_view, "", rect(24.0, 12.0, 228.0, 40.0), self.mtm());
            note.setUsesSingleLineMode(false);
            note.setMaximumNumberOfLines(2);
            state.integrations_note.set(note).expect("integration note initialized once");
            // SAFETY: Implemented selector and retained target.
            let setup = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Check & set up"), Some(self), Some(sel!(setupIntegrations:)), self.mtm()) };
            setup.setFrame(rect(264.0, 16.0, 172.0, 28.0));
            integrations_view.addSubview(&setup);
            state.integrations_button.set(setup).expect("integration setup button initialized once");

            let telegram_view = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 494.0));
            self.label(&telegram_view, "Set up Telegram integration", rect(24.0, 454.0, 412.0, 24.0), 15.0, 0.3, false);
            self.label(&telegram_view, "Set up your personal Telegram bot in a few simple steps.", rect(24.0, 430.0, 412.0, 18.0), 11.0, 0.0, true);
            settings_note(&telegram_view, "We chose personal Telegram bots over shared ones to protect your privacy. Your bot connects directly to the app running on your Mac, so your conversations never pass through our servers — only Telegram's.", rect(24.0, 366.0, 412.0, 56.0), self.mtm());
            self.label(&telegram_view, "Already have a bot? Just paste its token below.", rect(24.0, 342.0, 412.0, 18.0), 11.0, 0.0, true);
            self.label(&telegram_view, "1. Open BotFather and send /newbot.", rect(24.0, 310.0, 336.0, 20.0), 12.0, 0.3, false);
            // SAFETY: Every selector is implemented by this retained settings controller.
            let bot_father = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Open"), Some(self), Some(sel!(openBotFather:)), self.mtm()) };
            bot_father.setFrame(rect(374.0, 306.0, 62.0, 28.0));
            bot_father.setToolTip(Some(ns_string!("Open Telegram with /newbot ready to send")));
            telegram_view.addSubview(&bot_father);
            self.label(&telegram_view, "2. Choose your bot’s name and username", rect(24.0, 276.0, 412.0, 20.0), 12.0, 0.3, false);
            for (tag, title, value, y, width) in [(0, "Bot name:", "AI Agent Monitor".to_owned(), 244.0, 254.0), (1, "Bot username:", crate::macos::telegram_setup::username(false), 212.0, 220.0)] {
                let label = self.label(&telegram_view, title, rect(24.0, y + 2.0, 82.0, 20.0), 11.0, 0.0, false);
                center_label(&label, y + 12.0);
                let field = NSTextField::textFieldWithString(&NSString::from_str(&value), self.mtm());
                field.setEditable(false);
                field.setSelectable(true);
                field.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(11.0, 0.0)));
                field.setToolTip(Some(&NSString::from_str(&value)));
                field.setFrame(rect(112.0, y, width, 24.0));
                telegram_view.addSubview(&field);
                if tag == 1 { state.telegram_username.set(field).expect("suggested username initialized once"); }
                let copy = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Copy"), Some(self), Some(sel!(copyBotValue:)), self.mtm()) };
                copy.setTag(tag);
                copy.setFrame(rect(374.0, y - 1.0, 62.0, 26.0));
                copy.setToolTip(Some(if tag == 0 { ns_string!("Copy bot name") } else { ns_string!("Copy bot username") }));
                telegram_view.addSubview(&copy);
            }
            let regenerate = unsafe { NSButton::buttonWithTitle_target_action(ns_string!(""), Some(self), Some(sel!(regenerateBotUsername:)), self.mtm()) };
            regenerate.setFrame(rect(338.0, 211.0, 28.0, 26.0));
            let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(ns_string!("arrow.clockwise"), Some(ns_string!("Generate another bot username")));
            regenerate.setImage(image.as_deref());
            regenerate.setToolTip(Some(ns_string!("Generate another username if this one is taken")));
            telegram_view.addSubview(&regenerate);
            self.label(&telegram_view, "Must be unique. Regenerate if the username is taken.", rect(24.0, 190.0, 412.0, 18.0), 11.0, 0.0, true);
            self.label(&telegram_view, "3. Copy the token from BotFather and paste it below.", rect(24.0, 162.0, 412.0, 20.0), 12.0, 0.3, false);
            self.label(&telegram_view, "Find it in BotFather’s last message.", rect(24.0, 140.0, 412.0, 20.0), 11.0, 0.0, true);
            self.label(&telegram_view, "Your bot token will be stored securely in macOS Keychain.", rect(24.0, 88.0, 412.0, 18.0), 11.0, 0.0, true);
            let token = NSSecureTextField::initWithFrame(NSSecureTextField::alloc(self.mtm()), rect(24.0, 110.0, 342.0, 26.0));
            token.setPlaceholderString(Some(ns_string!("Paste the entire token from BotFather")));
            token.setEditable(true);
            token.setSelectable(true);
            token.setUsesSingleLineMode(true);
            token.setMaximumNumberOfLines(1);
            if let Some(cell) = token.cell() {
                cell.setWraps(false);
                cell.setScrollable(true);
                cell.setUsesSingleLineMode(true);
            }
            telegram_view.addSubview(&token);
            state.telegram_token.set(token).expect("Telegram token initialized once");
            let connect = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Connect"), Some(self), Some(sel!(connectTelegram:)), self.mtm()) };
            connect.setFrame(rect(24.0, 8.0, 104.0, 28.0));
            telegram_view.addSubview(&connect);
            let paste = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Paste"), Some(self), Some(sel!(pasteTelegramToken:)), self.mtm()) };
            paste.setFrame(rect(374.0, 109.0, 62.0, 28.0));
            telegram_view.addSubview(&paste);
            let status = settings_note(&telegram_view, "", rect(24.0, 42.0, 412.0, 42.0), self.mtm());
            status.setUsesSingleLineMode(false);
            status.setMaximumNumberOfLines(3);
            state.telegram_status.set(status).expect("Telegram status initialized once");
            for (title, action, x, width, storage) in [
                ("Open bot", sel!(pairTelegram:), 136.0, 132.0, &state.telegram_pair),
                ("Cancel", sel!(disconnectTelegram:), 312.0, 124.0, &state.telegram_disconnect),
            ] {
                let button = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(self), Some(action), self.mtm()) };
                button.setFrame(rect(x, 8.0, width, 28.0));
                telegram_view.addSubview(&button);
                storage.set(button).expect("Telegram control initialized once");
            }

            let telegram_root = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 200.0));
            telegram_root.setAutoresizesSubviews(false);
            telegram_view.setAutoresizesSubviews(false);
            telegram_root.addSubview(&telegram_view);
            state.telegram_setup_view.set(telegram_view.clone()).expect("Telegram setup initialized once");
            let connected = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 200.0));
            let connected_title = self.label(&connected, "Telegram is connected", rect(24.0, 152.0, 412.0, 24.0), 15.0, 0.3, false);
            state.telegram_connected_title.set(connected_title).expect("connected title initialized once");
            let bot = self.label(&connected, "", rect(24.0, 124.0, 240.0, 24.0), 13.0, 0.0, false);
            bot.setSelectable(true);
            state.telegram_bot_label.set(bot).expect("connected bot label initialized once");
            let connected_status = settings_note(&connected, "", rect(24.0, 70.0, 412.0, 40.0), self.mtm());
            connected_status.setUsesSingleLineMode(false);
            connected_status.setMaximumNumberOfLines(3);
            state.telegram_connected_status.set(connected_status).expect("Telegram status initialized once");
            let test = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Test"), Some(self), Some(sel!(testTelegram:)), self.mtm()) };
            let open_bot = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Open bot"), Some(self), Some(sel!(pairTelegram:)), self.mtm()) };
            open_bot.setTitle(ns_string!("Open"));
            open_bot.setFrame(rect(366.0, 123.0, 70.0, 26.0));
            connected.addSubview(&open_bot);
            state.telegram_connected_open.set(open_bot).expect("connected open button initialized once");
            test.setFrame(rect(280.0, 123.0, 78.0, 26.0));
            connected.addSubview(&test);
            state.telegram_connected_test.set(test).expect("connected test initialized once");
            let change = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Change bot"), Some(self), Some(sel!(changeTelegram:)), self.mtm()) };
            change.setFrame(rect(24.0, 10.0, 116.0, 28.0));
            change.setToolTip(Some(ns_string!("Set up another bot; keep the current bot until pairing is complete")));
            connected.addSubview(&change);
            let remove = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Unlink bot"), Some(self), Some(sel!(removeTelegram:)), self.mtm()) };
            remove.setFrame(rect(152.0, 10.0, 116.0, 28.0));
            remove.setToolTip(Some(ns_string!("Disconnect and remove this bot from AI Agent Monitor")));
            connected.addSubview(&remove);
            connected.setAutoresizesSubviews(false);
            telegram_root.addSubview(&connected);
            state.telegram_connected_view.set(connected).expect("connected Telegram view initialized once");

            let web=super::web_settings::WebSettings::new(self.mtm());
            state.web_settings.set(web.clone()).expect("web settings initialized once");
            let web_content: Retained<NSView>=web.into_super();
            // Retain native pane controllers/items; the inline shell owns their presentation.
            let tabs = NSTabViewController::new(self.mtm());
            state.settings_tabs.set(tabs.clone()).expect("settings tabs initialized once");
            tabs.setTabStyle(NSTabViewControllerTabStyle::Unspecified);
            tabs.setTitle(Some(ns_string!("AI Agent Monitor Settings")));
            for (title, symbol, content) in [
                ("General", "gearshape", &view),
                ("Applications", "square.stack.3d.up", &applications_view),
                ("Agents", "bot", &integrations_view),
                ("Telegram", "paperplane", &telegram_root),
                ("Web", "network", &web_content),
            ] {
                let controller = NSViewController::new(self.mtm());
                controller.setTitle(Some(&NSString::from_str(title)));
                content.setAutoresizesSubviews(false);
                controller.setPreferredContentSize(content.frame().size);
                controller.setView(content);
                if title == "Applications" { state.applications_controller.set(controller.clone()).expect("applications controller initialized once"); }
                if title == "Agents" { state.integrations_controller.set(controller.clone()).expect("integrations controller initialized once"); }
                if title == "Telegram" { state.telegram_controller.set(controller.clone()).expect("Telegram controller initialized once"); }
                let item = NSTabViewItem::tabViewItemWithViewController(&controller);
                if title == "Telegram" { state.telegram_tab_item.set(item.clone()).expect("Telegram item initialized once"); }
                item.setLabel(&NSString::from_str(title));
                let image = if symbol == "bot" {
                    Some(super::session_robot::settings_icon())
                } else {
                    NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        &NSString::from_str(symbol), Some(&NSString::from_str(title)),
                    )
                };
                item.setImage(image.as_deref());
                tabs.addTabViewItem(&item);
            }
            tabs.setSelectedTabViewItemIndex(0);
            super::inline_settings::Shell::new(self.mtm(), self)
        });
        self.refresh_settings();
        self.refresh_application_settings();
        self.refresh_integration_settings();
        self.refresh_telegram_settings();
        if !state.settings_active.replace(true) {
            if let Some(scroll) = state.scroll.get() {
                state
                    .settings_session_scroll
                    .set(scroll.contentView().bounds().origin);
                scroll.setHidden(true);
            }
            if let Some(footer) = state.footer.get() {
                footer.setHidden(true);
            }
            if let Some(root) = state.root.get() {
                root.addSubview(&shell.scroll);
            }
            for composer in state.composers.borrow().values() {
                composer.stop_editing();
            }
            for panel in state.question_panels.borrow().values() {
                panel.stop_editing();
            }
            self.ivars().keyboard_focus.borrow_mut().take();
            self.clear_hover();
        }
        shell.scroll.setHidden(false);
        self.layout_settings();
        if let Some(window) = shell.scroll.window() {
            // Opening settings must not create keyboard selection on Sessions.
            // The first Tab/arrow establishes focus through handle_settings_key.
            unsafe {
                window.endEditingFor(None);
            }
            window.makeFirstResponder(None);
        }
        if !state.menu_open.get() {
            self.show();
        }
    }

    fn layout_settings(&self) {
        let state = self.ivars();
        if !state.settings_active.get() {
            return;
        }
        let (Some(tabs), Some(shell)) = (state.settings_tabs.get(), state.settings_shell.get())
        else {
            return;
        };
        shell.sync_versions(&self.monitor().snapshot().versions);
        let index = state.settings_selected.get();
        if tabs.selectedTabViewItemIndex() != index as isize {
            tabs.setSelectedTabViewItemIndex(index as isize);
        }
        if let Some(item) = tabs
            .tabViewItems()
            .objectAtIndex(index)
            .viewController(self.mtm())
        {
            let size = if index == 4 {
                let size = item.view().frame().size;
                item.setPreferredContentSize(size);
                size
            } else {
                item.preferredContentSize()
            };
            shell.select(index, &item.view(), size.height);
        }
        self.resize_content(state.content_height.get());
    }

    fn hide_settings(&self) {
        let state = self.ivars();
        if !state.settings_active.replace(false) {
            return;
        }
        self.end_shortcut_recording(false);
        if let Some(window) = state.root.get().and_then(|v| v.window()) {
            unsafe {
                window.endEditingFor(None);
            }
            window.makeFirstResponder(None);
        }
        if let Some(shell) = state.settings_shell.get() {
            shell.scroll.setHidden(true);
        }
        if let Some(scroll) = state.scroll.get() {
            scroll.setHidden(false);
        }
        if let Some(footer) = state.footer.get() {
            footer.setHidden(false);
        }
        self.render_footer();
        self.filter();
        self.resize_content(state.content_height.get());
        if let Some(scroll) = state.scroll.get() {
            scroll
                .contentView()
                .scrollToPoint(state.settings_session_scroll.get());
        }
        self.sync_backdrop();
    }

    fn size_settings_panel(&self, controller: &NSViewController, height: f64) {
        let size = NSSize::new(460.0, height);
        controller.view().setAutoresizesSubviews(false);
        controller.setPreferredContentSize(size);
        self.layout_settings();
    }

    fn refresh_telegram_settings(&self) {
        let status = self.monitor().snapshot().telegram.clone();
        let state = self.ivars();
        let auto_link = state
            .telegram_auto_open
            .borrow_mut()
            .take_link(status.link.as_deref());
        if let Some(link) = auto_link {
            open_telegram_bot(&link);
        }
        let configured = status.connected && status.paired && !status.setting_up;
        if let Some(label) = state.telegram_bot_label.get() {
            label.setStringValue(&NSString::from_str(&format!("@{}", status.bot_username)));
        }
        if let Some(label) = state.telegram_connected_status.get() {
            let text = if status.text.starts_with('@') {
                ""
            } else {
                &status.text
            };
            label.setStringValue(&NSString::from_str(text));
        }
        if let Some(button) = state.telegram_connected_test.get() {
            button.setEnabled(status.enabled);
        }
        let note = if status.text.starts_with('@') {
            ""
        } else {
            &status.text
        };
        let connected_height = if note.is_empty() { 116.0 } else { 160.0 };
        if let Some(view) = state.telegram_connected_view.get() {
            view.setFrame(rect(0.0, 0.0, 460.0, connected_height));
        }
        if let Some(view) = state.telegram_setup_view.get() {
            view.setFrame(rect(0.0, 0.0, 460.0, 494.0));
        }
        if let Some(label) = state.telegram_connected_title.get() {
            label.setFrameOrigin(NSPoint::new(24.0, connected_height - 40.0));
        }
        let center_y = connected_height - 54.0;
        if let Some(label) = state.telegram_bot_label.get() {
            let name_width = label
                .cell()
                .map(|cell| cell.cellSize().width.ceil())
                .unwrap_or(220.0)
                .min(268.0);
            label.setFrame(rect(24.0, center_y - 12.0, name_width, 24.0));
            label.setToolTip(Some(&label.stringValue()));
            center_label(label, center_y);
            let test_x = 24.0 + name_width + 12.0;
            if let Some(button) = state.telegram_connected_test.get() {
                button.setFrame(rect(test_x, center_y - 13.0, 62.0, 26.0));
            }
            if let Some(button) = state.telegram_connected_open.get() {
                button.setFrame(rect(test_x + 70.0, center_y - 13.0, 62.0, 26.0));
            }
        }
        if let Some(label) = state.telegram_connected_status.get() {
            label.setHidden(note.is_empty());
            label.setFrame(rect(24.0, 48.0, 412.0, 36.0));
        }
        if let Some(controller) = state.telegram_controller.get()
            && let Some(item) = state.telegram_tab_item.get()
            && let Some(pane) = if configured {
                state.telegram_connected_view.get()
            } else {
                state.telegram_setup_view.get()
            }
        {
            crate::macos::settings_pane::replace(controller, item, pane, self.mtm());
            self.size_settings_panel(
                controller,
                if configured { connected_height } else { 494.0 },
            );
        }
        if let Some(label) = state.telegram_status.get() {
            label.setStringValue(&NSString::from_str(&status.text));
        }
        if let Some(button) = state.telegram_pair.get() {
            let ready = status.link.is_some();
            button.setHidden(!ready);
            button.setEnabled(ready);
        }
        if let Some(button) = state.telegram_disconnect.get() {
            button.setHidden(!status.connected && !status.setting_up);
            button.setEnabled(status.connected || status.setting_up);
        }
    }

    fn refresh_integration_settings(&self) {
        use crate::macos::onboarding::Phase;
        let state = self.ivars();
        let Some(list) = state.integrations_list.get() else {
            return;
        };
        if let Some(button) = state.integrations_button.get() {
            button.setEnabled(!self.monitor().snapshot().integration_busy);
            button.setTitle(if self.monitor().snapshot().integration_busy {
                ns_string!("Setting up…")
            } else {
                ns_string!("Check & set up")
            });
        }
        if let Some(note) = state.integrations_note.get() {
            let text = self.monitor().snapshot().integration_note.clone();
            note.setStringValue(&NSString::from_str(&text));
            note.setToolTip(
                (!text.is_empty())
                    .then(|| NSString::from_str(&text))
                    .as_deref(),
            );
        }
        let rows = self.monitor().snapshot().integrations.clone();
        if state.integrations_snapshot.borrow().as_ref() == Some(&rows) {
            return;
        }
        list.setAutoresizesSubviews(false);
        let old = list.subviews();
        for index in 0..old.count() {
            old.objectAtIndex(index).removeFromSuperview();
        }
        if rows.is_empty() {
            let text = NSTextField::wrappingLabelWithString(
                ns_string!(
                    "No supported agents found. Install Codex, Claude Code, or Cline, then click Check & set up."
                ),
                self.mtm(),
            );
            text.setFont(Some(&NSFont::systemFontOfSize(12.0)));
            text.setTextColor(Some(&NSColor::secondaryLabelColor()));
            text.setFrame(rect(0.0, 8.0, 396.0, 64.0));
            list.addSubview(&text);
        }
        let mut y = 0.0;
        for row in &rows {
            let provider = self.label(
                list,
                row.agent.name(),
                rect(0.0, y + 4.0, 166.0, 22.0),
                13.0,
                0.23,
                false,
            );
            let (symbol, color) = match row.phase {
                Phase::Installed => ("checkmark.circle.fill", NSColor::systemGreenColor()),
                Phase::Failed => (
                    "exclamationmark.triangle.fill",
                    NSColor::systemOrangeColor(),
                ),
                Phase::Installing => (
                    "arrow.triangle.2.circlepath",
                    NSColor::secondaryLabelColor(),
                ),
                Phase::Detected => ("circle", NSColor::secondaryLabelColor()),
            };
            let status_x = provider
                .cell()
                .map(|cell| cell.cellSize().width.ceil())
                .unwrap_or(90.0)
                + 12.0;
            let status = self.label(
                list,
                row.status(),
                rect(status_x, y + 6.0, 210.0, 18.0),
                11.0,
                0.0,
                false,
            );
            status.setAlignment(NSTextAlignment::Left);
            status.setTextColor(Some(&color));
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                None,
            ) {
                image.setTemplate(true);
                let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                let icon_x = status_x
                    + status
                        .cell()
                        .map(|cell| cell.cellSize().width.ceil())
                        .unwrap_or(140.0)
                    + 6.0;
                icon.setFrame(rect(icon_x, y + 7.0, 14.0, 14.0));
                icon.setContentTintColor(Some(&color));
                list.addSubview(&icon);
            }
            let detail =
                NSTextField::wrappingLabelWithString(&NSString::from_str(&row.detail), self.mtm());
            detail.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            detail.setTextColor(Some(&NSColor::secondaryLabelColor()));
            detail.setMaximumNumberOfLines(0);
            detail.setToolTip(Some(&NSString::from_str(&format!(
                "{}\n{}",
                row.detail, row.directory
            ))));
            let height = detail
                .cell()
                .map(|cell| {
                    cell.cellSizeForBounds(rect(0.0, 0.0, 396.0, 100000.0))
                        .height
                        .ceil()
                })
                .unwrap_or(32.0)
                .max(16.0);
            detail.setFrame(rect(0.0, y + 26.0, 396.0, height));
            list.addSubview(&detail);
            y += height + 32.0;
        }
        let content = if rows.is_empty() {
            72.0
        } else {
            (y - 6.0).max(32.0)
        };
        let viewport = content.min(240.0);
        let height = viewport + 88.0;
        list.setFrameSize(NSSize::new(412.0, content));
        if let Some(scroll) = state.integrations_scroll.get() {
            scroll.setFrame(rect(24.0, 52.0, 412.0, viewport));
        }
        if let Some(heading) = state.integrations_heading.get() {
            heading.setFrameOrigin(NSPoint::new(24.0, height - 32.0));
        }
        if let Some(controller) = state.integrations_controller.get() {
            self.size_settings_panel(controller, height);
        }
        *state.integrations_snapshot.borrow_mut() = Some(rows);
    }

    fn refresh_application_settings(&self) {
        let state = self.ivars();
        let Some(list) = state.applications_list.get() else {
            return;
        };
        list.setAutoresizesSubviews(false);
        let old = list.subviews();
        for index in 0..old.count() {
            old.objectAtIndex(index).removeFromSuperview();
        }
        let applications = state.applications.borrow();
        let apps = applications.list();
        if apps.is_empty() {
            self.label(
                list,
                "Applications appear as sessions are discovered.",
                rect(0.0, 6.0, 396.0, 20.0),
                12.0,
                0.0,
                true,
            );
        }
        for (index, app) in apps.iter().enumerate() {
            let y = index as f64 * 36.0;
            let image = app
                .icon_path
                .as_ref()
                .filter(|path| std::path::Path::new(path).exists())
                .map(|path| NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path)))
                .or_else(|| {
                    NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        ns_string!("app"),
                        None,
                    )
                });
            if let Some(image) = image {
                let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                icon.setFrame(rect(0.0, y + 7.0, 22.0, 22.0));
                list.addSubview(&icon);
            }
            let title = self.label(
                list,
                &app.name,
                rect(32.0, y + 8.0, 312.0, 20.0),
                13.0,
                0.0,
                false,
            );
            center_label(&title, y + 18.0);
            let checkbox = NSSwitch::initWithFrame(
                NSSwitch::alloc(self.mtm()),
                rect(356.0, y + 6.0, 40.0, 24.0),
            );
            // SAFETY: Implemented selector; this controller outlives its weak target.
            unsafe {
                checkbox.setTarget(Some(self));
                checkbox.setAction(Some(sel!(toggleApplication:)));
            }
            checkbox.setIdentifier(Some(&NSString::from_str(&app.id)));
            checkbox.setState(if app.enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            checkbox.setToolTip(Some(&NSString::from_str(&format!(
                "Show sessions from {}",
                app.name
            ))));
            // SAFETY: NSButton supports the accessibility label property.
            unsafe {
                let _: () =
                    msg_send![&*checkbox, setAccessibilityLabel: &*NSString::from_str(&app.name)];
            }
            list.addSubview(&checkbox);
        }
        let content = (apps.len() as f64 * 36.0).max(36.0);
        let viewport = content.min(216.0);
        let height = viewport + 52.0;
        list.setFrameSize(NSSize::new(412.0, content));
        if let Some(scroll) = state.applications_scroll.get() {
            scroll.setFrame(rect(24.0, 12.0, 412.0, viewport));
        }
        if let Some(heading) = state.applications_heading.get() {
            heading.setFrameOrigin(NSPoint::new(24.0, height - 34.0));
        }
        if let Some(controller) = state.applications_controller.get() {
            self.size_settings_panel(controller, height);
        }
    }

    fn refresh_settings(&self) {
        self.configure_menu_shortcut();
        let state = self.ivars();
        let status = self.monitor().snapshot().preferences.login;
        if let Some(toggle) = state.login_switch.get() {
            toggle.setState(
                if matches!(
                    status,
                    settings::LoginStatus::Enabled | settings::LoginStatus::RequiresApproval
                ) {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                },
            );
        }
        if let Some(toggle) = state.animation_switch.get() {
            toggle.setState(if self.monitor().snapshot().preferences.animation {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        if let Some(note) = state.login_note.get() {
            note.setToolTip(None);
            note.setTextColor(Some(&NSColor::secondaryLabelColor()));
            note.setStringValue(ns_string!("Open AI Agent Monitor automatically"));
            if status == settings::LoginStatus::RequiresApproval {
                note.setStringValue(ns_string!("Allow launch at login in macOS settings"));
            } else if status == settings::LoginStatus::Unavailable {
                note.setStringValue(ns_string!("Launch at login is unavailable for this copy"));
            }
        }
        if let Some(button) = state.login_settings_button.get() {
            button.setHidden(status != settings::LoginStatus::RequiresApproval);
        }
        if let Some(error) = self.monitor().snapshot().preferences.error.as_ref()
            && let Some(note) = state.login_note.get()
        {
            note.setStringValue(&NSString::from_str(error));
            note.setTextColor(Some(&NSColor::systemRedColor()));
        }
        let shortcut = self.monitor().snapshot().preferences.shortcut.clone();
        let recording = state.shortcut_monitor.borrow().is_some();
        if let Some(button) = state.shortcut_button.get() {
            let title = if recording {
                "Press shortcut…".into()
            } else {
                shortcut
                    .as_ref()
                    .map(|shortcut| shortcut.label())
                    .unwrap_or_else(|| "Record shortcut".into())
            };
            button.setTitle(&NSString::from_str(&title));
        }
        if let Some(button) = state.shortcut_clear.get() {
            button.setEnabled(shortcut.is_some() || recording);
        }
        if let Some(note) = state.shortcut_note.get() {
            let error = state.shortcut_error.borrow();
            let message = error.as_deref().unwrap_or(if recording {
                "Press a shortcut. Escape cancels."
            } else {
                "Open the sessions menu from any app"
            });
            note.setStringValue(&NSString::from_str(message));
            note.setToolTip(Some(&NSString::from_str(message)));
            let color = if error.is_some() {
                NSColor::systemRedColor()
            } else {
                NSColor::secondaryLabelColor()
            };
            note.setTextColor(Some(&color));
        }
    }

    fn build_status_item(&self) {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(20.0);
        if let Some(button) = item.button(self.mtm()) {
            self.build_status_artwork(&button);
        }
        // Let AppKit own status-item selection and switching between menu-bar
        // items. The custom view confines the menu to the work area before draw.
        item.setMenu(self.ivars().menu.get().map(|menu| &**menu));
        self.ivars()
            .status
            .set(item)
            .expect("status initialized once");
        self.update_badge(self.monitor().snapshot().pending_count());
    }

    fn build_status_artwork(&self, button: &NSView) {
        let height = button.bounds().size.height;
        // SAFETY: NSView's designated initializer is valid for this subclass.
        let content =
            NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, 0.0, 26.0, height));
        let data = NSData::with_bytes(include_bytes!("../../assets/menubar/robots-template.pdf"));
        let image = NSImage::initWithData(NSImage::alloc(), &data).or_else(|| {
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("rectangle.stack"),
                Some(ns_string!("Agent Sessions")),
            )
        });
        if let Some(image) = image {
            image.setAccessibilityDescription(Some(ns_string!("Agent Sessions")));
            image.setTemplate(true);
            let icon = NSImageView::imageViewWithImage(&image, self.mtm());
            image.setSize(NSSize::new(24.0, 17.6));
            icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            icon.setFrame(rect(1.0, height / 2.0 - 8.8, 24.0, 17.6));
            icon.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewMinYMargin
                    | NSAutoresizingMaskOptions::ViewMaxYMargin,
            );
            icon.setContentTintColor(Some(&NSColor::labelColor()));
            content.addSubview(&icon);
            let active_data =
                NSData::with_bytes(include_bytes!("../../assets/menubar/robots-working.pdf"));
            let active_image = NSImage::initWithData(NSImage::alloc(), &active_data)
                .expect("embedded working icon is a valid PDF");
            active_image.setTemplate(true);
            active_image.setSize(NSSize::new(24.0, 17.6));
            self.ivars()
                .status_base_images
                .set([image, active_image])
                .expect("base icons initialized once");
            self.ivars()
                .status_icon
                .set(icon)
                .expect("status icon initialized once");
        }
        let masks = [
            include_bytes!("../../assets/menubar/eyes-left.pdf").as_slice(),
            include_bytes!("../../assets/menubar/eyes-right.pdf").as_slice(),
        ]
        .map(|bytes| {
            let data = NSData::with_bytes(bytes);
            let image = NSImage::initWithData(NSImage::alloc(), &data)
                .expect("embedded eye mask is a valid PDF");
            image.setTemplate(true);
            image.setSize(NSSize::new(24.0, 17.6));
            image
        });
        let eyes = masks.map(|image| {
            let eyes = NSImageView::imageViewWithImage(&image, self.mtm());
            eyes.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            eyes.setFrame(rect(1.0, height / 2.0 - 8.8, 24.0, 17.6));
            eyes.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewMinYMargin
                    | NSAutoresizingMaskOptions::ViewMaxYMargin,
            );
            eyes.setContentTintColor(Some(&NSColor::systemGreenColor()));
            eyes.setHidden(true);
            content.addSubview(&eyes);
            eyes
        });
        self.ivars()
            .status_eyes
            .set(eyes)
            .expect("eye overlays initialized once");
        let badge = NSBox::initWithFrame(
            NSBox::alloc(self.mtm()),
            rect(14.0, (height / 2.0 - 10.0).max(1.0), 12.0, 12.0),
        );
        badge.setBoxType(NSBoxType::Custom);
        badge.setTitlePosition(NSTitlePosition::NoTitle);
        badge.setBorderWidth(0.0);
        badge.setCornerRadius(7.0);
        badge.setFillColor(&appearance_color(
            [203.0, 119.0, 12.0],
            [255.0, 159.0, 10.0],
            1.0,
        ));
        content.addSubview(&badge);
        let count = self.label(&content, "", rect(28.0, 0.0, 14.0, 14.0), 10.0, 0.3, false);
        count.setFont(Some(&NSFont::systemFontOfSize(10.0)));
        count.setAlignment(NSTextAlignment::Center);
        count.setTextColor(Some(&appearance_color([255.0; 3], [0.0; 3], 1.0)));
        center_label(&count, height / 2.0);
        let working_count =
            self.label(&content, "", rect(28.0, 0.0, 14.0, 18.0), 12.0, 0.23, false);
        working_count.setTextColor(Some(&NSColor::systemGreenColor()));
        working_count.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        working_count.setHidden(true);
        center_label(&working_count, height / 2.0);
        self.ivars()
            .status_working_count
            .set(working_count)
            .expect("working count initialized once");
        let finished_badge =
            NSBox::initWithFrame(NSBox::alloc(self.mtm()), rect(28.0, 0.0, 14.0, 14.0));
        finished_badge.setBoxType(NSBoxType::Custom);
        finished_badge.setTitlePosition(NSTitlePosition::NoTitle);
        finished_badge.setBorderWidth(0.0);
        finished_badge.setCornerRadius(7.0);
        finished_badge.setFillColor(&appearance_color(
            [62.0, 109.0, 185.0],
            [114.75, 198.9, 255.0],
            1.0,
        ));
        finished_badge.setHidden(true);
        content.addSubview(&finished_badge);
        self.ivars()
            .status_finished_badge
            .set(finished_badge)
            .expect("finished badge initialized once");
        let finished_count =
            self.label(&content, "", rect(28.0, 0.0, 14.0, 14.0), 10.0, 0.4, false);
        finished_count.setTextColor(Some(&appearance_color(
            [255.0; 3],
            [17.85, 38.25, 53.55],
            1.0,
        )));
        finished_count.setFont(Some(&NSFont::systemFontOfSize(10.0)));
        finished_count.setAlignment(NSTextAlignment::Center);
        finished_count.setHidden(true);
        self.ivars()
            .status_finished_count
            .set(finished_count)
            .expect("finished count initialized once");
        let presentation: Retained<StatusSnapshotView> = unsafe {
            msg_send![super(StatusSnapshotView::alloc(self.mtm()).set_ivars(StatusContentState::default())), initWithFrame: rect(-3.0, 0.0, 26.0, height)]
        };
        self.ivars()
            .status_artwork
            .set(content)
            .expect("status artwork initialized once");
        button.addSubview(&presentation);
        self.ivars()
            .status_content
            .set(presentation)
            .expect("status content initialized once");
        self.ivars()
            .status_badge
            .set(badge)
            .expect("badge initialized once");
        self.ivars()
            .status_count
            .set(count)
            .expect("count initialized once");
    }

    fn paint_status_eyes(&self) {
        let state = self.ivars();
        let (levels, active) = {
            let animation = state.status_eye_animation.borrow();
            (animation.levels(), animation.active())
        };
        let previous = state.status_painted_eyes.replace(Some((active, levels)));
        if previous == Some((active, levels)) {
            return;
        }
        let key = [
            u8::from(active),
            (levels[0] * 2.0) as u8,
            (levels[1] * 2.0) as u8,
        ];
        if let Some(image) = state.status_frames.borrow().get(&key).cloned() {
            self.present_status_frame(image);
            return;
        }
        if let Some(icon) = state.status_icon.get()
            && let Some(images) = state.status_base_images.get()
        {
            icon.setImage(Some(&images[usize::from(active)]));
        }
        if let Some(eyes) = state.status_eyes.get() {
            for (eye, level) in eyes.iter().zip(levels) {
                eye.setAlphaValue(level);
                eye.setHidden(level == 0.0);
            }
        }
        let (Some(artwork), Some(presentation)) =
            (state.status_artwork.get(), state.status_content.get())
        else {
            return;
        };
        let size = artwork.bounds().size;
        let scale = presentation
            .window()
            .map_or(2.0, |window| window.backingScaleFactor());
        // AppKit temporarily switches the source view's appearance while taking
        // light/dark status-item snapshots. Those callbacks must never invalidate
        // the cache or schedule another redraw. Prepare both variants here, away
        // from the snapshot/drawing path; drawRect: only selects an existing one.
        let mut variants = Vec::with_capacity(2);
        for name in unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] } {
            artwork.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
            // SAFETY: A null planes pointer asks AppKit to allocate owned RGBA
            // storage. Dimensions and logical size preserve Retina resolution.
            let Some(bitmap) = (unsafe {
                NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                    NSBitmapImageRep::alloc(), std::ptr::null_mut(), (size.width * scale).ceil() as isize,
                    (size.height * scale).ceil() as isize, 8, 4, true, false, NSDeviceRGBColorSpace, 0, 0,
                )
            }) else {
                return;
            };
            bitmap.setSize(size);
            artwork.cacheDisplayInRect_toBitmapImageRep(artwork.bounds(), &bitmap);
            let image = NSImage::initWithSize(NSImage::alloc(), size);
            image.addRepresentation(&bitmap);
            variants.push(image);
        }
        let images: [Retained<NSImage>; 2] = variants
            .try_into()
            .expect("both status appearances prepared");
        state.status_frames.borrow_mut().insert(key, images.clone());
        self.present_status_frame(images);
    }

    fn present_status_frame(&self, images: [Retained<NSImage>; 2]) {
        if let Some(content) = self.ivars().status_content.get() {
            *content.ivars().images.borrow_mut() = Some(images);
            content.setNeedsDisplay(true);
        }
        if let Some(button) = self
            .ivars()
            .status
            .get()
            .and_then(|item| item.button(self.mtm()))
        {
            NSView::setNeedsDisplay(&button, true);
        }
    }

    fn update_badge(&self, count: usize) {
        let state = self.ivars();
        if state.popup_owner.get().is_some() {
            // Popup tracking can receive a revision before the resident's
            // snapshot timer. The resident alone owns status-item artwork, but
            // it must observe the same revision while the popup stays open.
            if let Some(owner) = state.popup_owner.get().and_then(|owner| owner.load()) {
                owner.receive_sessions();
            }
            return;
        }
        let working_count = state.working_count();
        let working = count == 0 && working_count > 0;
        let finished_count = state.finished_count();
        let animate = working
            && self.monitor().snapshot().preferences.animation
            && !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        // Agent progress can change many times per second. It must not relayout
        // the status item or invalidate AppKit's replicant snapshot unless its
        // visible counters or animation mode actually change.
        let badge_snapshot = (count, working_count, finished_count, animate);
        if state.status_badge_snapshot.get() == Some(badge_snapshot) {
            return;
        }
        let number_width = if let Some(label) = state.status_working_count.get() {
            label.setHidden(working_count == 0);
            label.setStringValue(&NSString::from_str(&working_count.to_string()));
            if working_count == 0 {
                0.0
            } else {
                label
                    .cell()
                    .map(|cell| {
                        cell.cellSizeForBounds(rect(0.0, 0.0, 1000.0, 24.0))
                            .width
                            .ceil()
                            + 1.0
                    })
                    .unwrap_or(14.0)
            }
        } else {
            0.0
        };
        let finished_number_width = if finished_count > 99 {
            26.0
        } else if finished_count > 9 {
            20.0
        } else {
            14.0
        };
        if let Some(label) = state.status_finished_count.get() {
            label.setHidden(finished_count == 0);
            label.setStringValue(&NSString::from_str(&if finished_count > 99 {
                "99+".into()
            } else {
                finished_count.to_string()
            }));
        }
        let badge_width = if count > 99 {
            26.0
        } else if count > 9 {
            20.0
        } else {
            14.0
        };
        let (counter_x, extra_width) = status_counter_positions([
            (count > 0).then_some(badge_width),
            (finished_count > 0).then_some(finished_number_width),
            (working_count > 0).then_some(number_width),
        ]);
        let canvas_width = 26.0 + extra_width;
        let Some(item) = state.status.get() else {
            return;
        };
        let Some(button) = item.button(self.mtm()) else {
            return;
        };
        state.status_badge_snapshot.set(Some(badge_snapshot));
        state.status_frames.borrow_mut().clear();
        state.status_painted_eyes.set(None);
        // macOS adds margins outside the public button's content rectangle.
        // Target the real capsule width, rather than treating length as that width.
        let margins = button
            .window()
            .map(|window| (window.frame().size.width - button.frame().size.width).max(0.0))
            .unwrap_or(16.0);
        let width = (36.0 + extra_width - margins).max(1.0);
        if item.length() != width {
            item.setLength(width);
        }
        // Keep the complete 26-point image canvas centered, including the part
        // inside macOS's built-in status-item margins. No glyph scaling is applied.
        allow_status_overflow(&button);
        let height = button.bounds().size.height;
        if let Some(content) = state.status_content.get() {
            content.setFrame(rect(
                (button.bounds().size.width - canvas_width) / 2.0,
                0.0,
                canvas_width,
                height,
            ));
            allow_status_overflow(content);
        }
        if let Some(artwork) = state.status_artwork.get() {
            artwork.setFrameSize(NSSize::new(canvas_width, height));
        }
        if let Some(label) = state.status_working_count.get() {
            label.setFrame(rect(counter_x[2], 0.0, number_width, 18.0));
            center_label(label, height / 2.0);
        }
        let finished_frame = rect(
            counter_x[1],
            height / 2.0 - 7.0,
            finished_number_width,
            14.0,
        );
        if let Some(badge) = state.status_finished_badge.get() {
            badge.setHidden(finished_count == 0);
            badge.setFrame(finished_frame);
        }
        if let Some(label) = state.status_finished_count.get() {
            label.setFrame(finished_frame);
            center_label(label, height / 2.0);
        }
        let badge_x = counter_x[0];
        let badge_y = height / 2.0 - 7.0;
        if let Some(badge) = state.status_badge.get() {
            badge.setHidden(count == 0);
            badge.setFrame(rect(badge_x, badge_y, badge_width, 14.0));
        }
        state
            .status_eye_animation
            .borrow_mut()
            .update(working, animate);
        if animate && state.status_timer.borrow().is_none() {
            // SAFETY: The resident controller implements animateStatus: and
            // outlives the timer; common modes include native menu tracking.
            let timer = unsafe {
                NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    0.15,
                    self,
                    sel!(animateStatus:),
                    None,
                    true,
                )
            };
            // SAFETY: The timer and its main-thread target are retained for its
            // lifetime; the Foundation common-mode constant is valid.
            unsafe {
                NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
            }
            *state.status_timer.borrow_mut() = Some(timer);
        } else if !animate && let Some(timer) = state.status_timer.borrow_mut().take() {
            timer.invalidate();
        }
        if let Some(label) = state.status_count.get() {
            label.setHidden(count == 0);
            let text = if count > 99 {
                "99+".to_owned()
            } else {
                count.to_string()
            };
            label.setStringValue(&NSString::from_str(&text));
            label.setFrame(rect(badge_x, badge_y, badge_width, 14.0));
            center_label(label, height / 2.0);
        }
        let mut tooltip = "Agent Sessions".to_owned();
        if count > 0 {
            tooltip.push_str(&format!(" · Pending requests: {count}"));
        }
        if finished_count > 0 {
            tooltip.push_str(&format!(" · Unseen finished: {finished_count}"));
        }
        if working_count > 0 {
            tooltip.push_str(&format!(" · Working: {working_count}"));
        }
        button.setToolTip(Some(&NSString::from_str(&tooltip)));
        self.paint_status_eyes();
    }

    fn label(
        &self,
        view: &NSView,
        text: &str,
        frame: NSRect,
        size: f64,
        weight: f64,
        secondary: bool,
    ) -> Retained<NSTextField> {
        let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let label = NSTextField::labelWithString(&NSString::from_str(&single_line), self.mtm());
        label.setFrame(frame);
        label.setFont(Some(&NSFont::systemFontOfSize_weight(size, weight)));
        label.setMaximumNumberOfLines(1);
        if let Some(cell) = label.cell() {
            cell.setWraps(false);
            cell.setScrollable(false);
            cell.setUsesSingleLineMode(true);
            cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        }
        if secondary {
            label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        }
        view.addSubview(&label);
        label
    }

    fn message_with_images(
        &self,
        text: &str,
        images: &[crate::domain::media::Image],
        width: f64,
        request: bool,
    ) -> Retained<NSView> {
        let mut previews = Vec::new();
        let mut pixels = 0_u64;
        for image in images.iter().take(10) {
            // Reject oversized/unknown raster dimensions before asking AppKit
            // to decode; transcript data is untrusted and must stay bounded.
            let Some((w, h)) = image.dimensions() else {
                continue;
            };
            let size = u64::from(w) * u64::from(h);
            if w == 0 || h == 0 || w > 8192 || h > 8192 || pixels + size > 16_000_000 {
                continue;
            }
            let Some(native) =
                NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&image.data))
            else {
                continue;
            };
            let scale = (width / f64::from(w)).min(240.0 / f64::from(h)).min(1.0);
            let preview = NSImageView::initWithFrame(
                NSImageView::alloc(self.mtm()),
                rect(0.0, 0.0, f64::from(w) * scale, f64::from(h) * scale),
            );
            preview.setImage(Some(&native));
            preview.setImageScaling(NSImageScaling::ScaleProportionallyDown);
            // Retain only the display-sized raster in the view tree, not a
            // decoded full-resolution image for every expanded history entry.
            let Some(bitmap) = preview.bitmapImageRepForCachingDisplayInRect(preview.bounds())
            else {
                continue;
            };
            preview.cacheDisplayInRect_toBitmapImageRep(preview.bounds(), &bitmap);
            let thumbnail = NSImage::initWithSize(NSImage::alloc(), preview.frame().size);
            thumbnail.addRepresentation(&bitmap);
            preview.setImage(Some(&thumbnail));
            preview.setToolTip(Some(&NSString::from_str("Agent image")));
            pixels += size;
            previews.push(preview);
        }
        let text = if text.trim() == "[Image]" && !previews.is_empty() {
            ""
        } else {
            text
        };
        let label = crate::macos::markdown::message(
            text,
            width,
            &detail_text_color(request),
            request,
            self.mtm(),
        );
        if previews.is_empty() {
            return label;
        }
        let mut y = if text.is_empty() {
            0.0
        } else {
            label.frame().size.height + 8.0
        };
        // SAFETY: Same flipped, main-thread document view as session content.
        let content: Retained<SessionListView> = unsafe {
            msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, width, 1.0)]
        };
        if !text.is_empty() {
            content.addSubview(&label);
        }
        let last = previews.len() - 1;
        for (index, preview) in previews.into_iter().enumerate() {
            preview.setFrameOrigin(NSPoint::new(0.0, y));
            y += preview.frame().size.height + if index == last { 0.0 } else { 8.0 };
            content.addSubview(&preview);
        }
        content.setFrameSize(NSSize::new(width, y));
        content.into_super()
    }

    fn build_menu(&self) {
        let mtm = self.mtm();
        #[cfg(not(test))]
        {
            let saved = NSUserDefaults::standardUserDefaults()
                .doubleForKey(ns_string!("MonitorWindowWidth"));
            if saved.is_finite() && saved >= MENU_WIDTH {
                let maximum =
                    NSScreen::mainScreen(mtm).map_or(1000.0, |s| s.visibleFrame().size.width - 8.0);
                self.ivars()
                    .popup_width
                    .set(saved.min(maximum).max(MENU_WIDTH));
            }
        }
        // NSMenu supplies the system background and owns presentation/dismissal.
        // SAFETY: NSView's initializer is valid for this ivar-free subclass.
        let allocated = PanelView::alloc(mtm).set_ivars(PanelState::default());
        let view: Retained<PanelView> = unsafe {
            msg_send![super(allocated),initWithFrame:rect(0.0,0.0,self.menu_width(),476.0)]
        };
        view.ivars()
            .owner
            .set(objc2::rc::Weak::new(self))
            .expect("root owner initialized once");
        view.setWantsLayer(true);
        view.setClipsToBounds(true);
        let backdrop = visual::MeshView::new(mtm, view.bounds());
        view.addSubview(&backdrop);
        self.ivars()
            .backdrop
            .set(backdrop)
            .expect("backdrop initialized once");
        // SAFETY: OverlayView has no ivars or custom initialization.
        let toolbar = NSView::initWithFrame(
            NSView::alloc(mtm),
            rect(0.0, 0.0, self.list_width(), BRAND_HEIGHT),
        );
        let hero = SessionRobot::new(mtm, rect(8.0, 31.0, 40.0, 35.0), Activity::Idle);
        toolbar.addSubview(&hero);
        let title = self.label(
            &toolbar,
            "AI Agent Monitor",
            rect(61.0, 48.0, 158.0, 25.0),
            18.0,
            0.35,
            false,
        );
        title.setTextColor(Some(&primary_text_color()));
        title.setIdentifier(Some(ns_string!("brand-title")));
        let title_width = title
            .cell()
            .map(|cell| cell.cellSize().width.ceil())
            .unwrap_or(158.0);
        title.setFrameSize(NSSize::new(title_width, 25.0));
        let version_x = title.frame().origin.x + title_width + 8.0;
        let versions = self.monitor().snapshot().versions.clone();
        let release = super::release_link::ReleaseLink::new(
            mtm,
            rect(version_x, 53.5, 332.0 - version_x, 14.0),
        );
        release.sync(&versions);
        toolbar.addSubview(&release);
        self.ivars().release.set(release).unwrap();
        let overview = self.label(
            &toolbar,
            "",
            rect(61.0, 28.0, self.list_width() - 73.0, 18.0),
            11.0,
            0.0,
            true,
        );
        overview.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        self.ivars()
            .overview
            .set(overview)
            .expect("overview initialized once");
        let caption = self.label(&view, "", rect(20.0, 445.0, 286.0, 18.0), 10.0, 0.0, true);
        caption.setHidden(true);
        for (symbol, action, x, tooltip) in [
            (
                "github",
                sel!(openProject:),
                333.0,
                "Open project on GitHub",
            ),
            ("gearshape", sel!(settings:), 366.0, "Settings"),
            ("power", sel!(quit:), 399.0, "Quit AI Agent Monitor"),
        ] {
            let button = self.action_button(
                &toolbar,
                "",
                action,
                "",
                rect(x + self.menu_width() - MENU_WIDTH, 44.0, 28.0, 28.0),
                false,
            );
            button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
            if let Some(control) = self.ivars().buttons.borrow().last() {
                control
                    .background
                    .setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
            }
            let image = if symbol == "github" {
                Some(
                    NSImage::initWithData(
                        NSImage::alloc(),
                        &NSData::with_bytes(include_bytes!("../../assets/github.svg")),
                    )
                    .expect("valid GitHub vector icon"),
                )
            } else {
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(symbol),
                    Some(&NSString::from_str(tooltip)),
                )
            };
            if let Some(image) = image {
                image.setTemplate(true);
                image.setSize(NSSize::new(15.0, 15.0));
                button.setImage(Some(&image));
            }
            button.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
            button.setToolTip(Some(&NSString::from_str(tooltip)));
            // SAFETY: Every icon-only header control has a public native accessibility label.
            unsafe {
                let _: () =
                    msg_send![&*button, setAccessibilityLabel: &*NSString::from_str(tooltip)];
            }
        }
        // SAFETY: NSView's designated initializer is valid for this subclass.
        let list: Retained<SessionListView> = unsafe {
            msg_send![SessionListView::alloc(mtm), initWithFrame: rect(0.0, 0.0, self.list_width(), 400.0)]
        };
        // SAFETY: NSScrollView's initializer is valid for this ivar-free subclass.
        let scroll: Retained<SessionScrollView> = unsafe {
            msg_send![super(SessionScrollView::alloc(mtm).set_ivars(ScrollOwner::default())), initWithFrame: rect(11.0, 4.0, self.list_width(), 460.0)]
        };
        *scroll.ivars().controller.borrow_mut() = Some(objc2::rc::Weak::new(self));
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setHasHorizontalScroller(false);
        // A menu has pinned decorations and live layout updates. Elastic
        // overscroll fights their edge constraints and makes the gutter jump.
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
        scroll.setDocumentView(Some(&list));
        view.addSubview(&scroll);
        let footer =
            NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, self.menu_width(), 36.0));
        view.addSubview(&footer);
        self.ivars()
            .footer
            .set(footer)
            .expect("footer initialized once");
        list.addSubview(&toolbar);
        view.addSubview(&caption);
        let pin = self.action_button(
            &view,
            "",
            sel!(togglePin:),
            "window-pin",
            rect(
                1.0,
                view.bounds().size.height - ICON_BUTTON_SIZE - 1.0,
                ICON_BUTTON_SIZE,
                ICON_BUTTON_SIZE,
            ),
            false,
        );
        pin.setImage(
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("pin"),
                Some(ns_string!("Detach window")),
            )
            .as_deref(),
        );
        pin.setContentTintColor(Some(&secondary_text_color()));
        pin.setToolTip(Some(ns_string!("Detach window")));
        self.ivars().pin_button.set(pin).unwrap();
        self.ivars()
            .pin_background
            .set(
                self.ivars()
                    .buttons
                    .borrow()
                    .last()
                    .unwrap()
                    .background
                    .clone(),
            )
            .unwrap();
        *self.ivars().toolbar_buttons.borrow_mut() =
            std::mem::take(&mut *self.ivars().buttons.borrow_mut());
        view.setPostsFrameChangedNotifications(true);
        let menu = self.ivars().menu.get().cloned().unwrap_or_else(|| {
            let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("AI Agent Monitor"));
            menu.setDelegate(Some(ProtocolObject::from_ref(self)));
            menu
        });
        menu.setAutoenablesItems(false);
        menu.setMinimumWidth(self.menu_width());
        // SAFETY: nil action and an empty key equivalent create an inert container item.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                ns_string!(""),
                None,
                ns_string!(""),
            )
        };
        item.setEnabled(true);
        item.setView(Some(&view));
        menu.addItem(&item);
        // A hidden native command receives the shortcut during menu tracking;
        // the custom session view continues to own the visible menu contents.
        // SAFETY: closeMenu: is implemented by the retained controller.
        let close = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                ns_string!("Close"),
                Some(sel!(closeMenu:)),
                ns_string!(""),
            )
        };
        // SAFETY: The target lives as long as the resident controller and menu.
        unsafe {
            close.setTarget(Some(self));
        }
        close.setHidden(true);
        close.setAllowsKeyEquivalentWhenHidden(true);
        close.setEnabled(false);
        menu.addItem(&close);
        self.ivars()
            .menu_close_shortcut
            .set(close)
            .expect("menu shortcut initialized once");
        self.ivars()
            .menu_item
            .set(item)
            .expect("menu item initialized once");
        if self.ivars().menu.get().is_none() {
            self.ivars().menu.set(menu).expect("menu initialized once");
        }
        self.ivars().list.set(list).expect("list initialized once");
        self.ivars().root.set(view).expect("root initialized once");
        self.ivars()
            .toolbar
            .set(toolbar)
            .expect("toolbar initialized once");
        self.ivars()
            .scroll
            .set(scroll)
            .expect("scroll initialized once");
        self.ivars()
            .caption
            .set(caption)
            .expect("caption initialized once");
        self.render_footer();
        self.render_rows(&[]);
        self.refresh();
    }

    fn action_button(
        &self,
        view: &NSView,
        title: &str,
        action: objc2::runtime::Sel,
        id: &str,
        frame: NSRect,
        transparent: bool,
    ) -> Retained<NSButton> {
        // SAFETY: All callers use implemented selectors and the retained controller as target.
        let native: Retained<ActionButton> =
            unsafe { msg_send![ActionButton::alloc(self.mtm()), initWithFrame: frame] };
        let button = native.into_super();
        button.setTitle(&NSString::from_str(title));
        button.setButtonType(NSButtonType::MomentaryPushIn);
        button.setBezelStyle(NSBezelStyle::Push);
        unsafe {
            button.setTarget(Some(self));
            button.setAction(Some(action));
        }
        let frame = if !transparent && title.is_empty() {
            rect(
                frame.origin.x + (frame.size.width - ICON_BUTTON_SIZE) / 2.0,
                frame.origin.y + (frame.size.height - ICON_BUTTON_SIZE) / 2.0,
                ICON_BUTTON_SIZE,
                ICON_BUTTON_SIZE,
            )
        } else if transparent {
            frame
        } else {
            rect(
                frame.origin.x - BUTTON_HOVER_PADDING,
                frame.origin.y - BUTTON_HOVER_PADDING,
                frame.size.width + 2.0 * BUTTON_HOVER_PADDING,
                frame.size.height + 2.0 * BUTTON_HOVER_PADDING,
            )
        };
        button.setFrame(frame);
        button.setBordered(false);
        button.setTransparent(transparent);
        button.setIdentifier(Some(&NSString::from_str(id)));
        if !transparent {
            let background = hover_background(
                self.mtm(),
                frame,
                frame.size.width.min(frame.size.height) / 2.0,
            );
            let base_alpha = if action == sel!(respondToRequest:) {
                0.5
            } else {
                0.0
            };
            paint_hover(&background, base_alpha);
            view.addSubview(&background);
            self.ivars().buttons.borrow_mut().push(ButtonHover {
                button: button.clone(),
                background,
                base_alpha,
                radio: None,
                caption: None,
                symbol: None,
                focusable_when_disabled: false,
                gutter: None,
            });
        }
        view.addSubview(&button);
        button
    }

    fn client_icon(&self, client: Client) -> Retained<StatusContentView> {
        // Keep the decoration click-through while using the app's own full-color icon.
        // SAFETY: This decoration-only NSView subclass has no ivars.
        let view: Retained<StatusContentView> = unsafe {
            msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: rect(session_layout::ICON_LEFT, 0.0, session_layout::ICON_SIZE, session_layout::ICON_SIZE)]
        };
        let workspace = NSWorkspace::sharedWorkspace();
        let bundle_id = match client {
            Client::VSCode => Some("com.microsoft.VSCode"),
            Client::Terminal => Some("com.apple.Terminal"),
            _ => None,
        };
        let path = bundle_id
            .and_then(|id| workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(id)))
            .and_then(|url| url.path())
            .map(|path| path.to_string())
            .or_else(|| match client {
                Client::Desktop { app_path, .. } => Some(app_path.to_owned()),
                Client::VSCode => Some("/Applications/Visual Studio Code.app".to_owned()),
                Client::Terminal => Some("/System/Applications/Utilities/Terminal.app".to_owned()),
                Client::Unknown => None,
            });
        let image = path
            .filter(|path| std::path::Path::new(path).exists())
            .map(|path| workspace.iconForFile(&NSString::from_str(&path)))
            .or_else(|| {
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    ns_string!("app"),
                    Some(&NSString::from_str(client.label())),
                )
            });
        if let Some(image) = image {
            let icon = NSImageView::imageViewWithImage(&image, self.mtm());
            icon.setFrame(rect(
                0.0,
                0.0,
                session_layout::ICON_SIZE,
                session_layout::ICON_SIZE,
            ));
            icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            icon.setToolTip(Some(&NSString::from_str(client.label())));
            view.addSubview(&icon);
        }
        view
    }

    fn status_icon(&self, view: &NSView, activity: Activity, finished: bool, y: f64) {
        if activity == Activity::Confirmation || (finished && activity == Activity::Idle) {
            let is_finished = activity == Activity::Idle;
            let color = if is_finished {
                appearance_color([62.0, 109.0, 185.0], [140.0, 216.0, 255.0], 1.0)
            } else {
                request_color()
            };
            let ink = if is_finished {
                appearance_color([255.0; 3], [21.0, 49.0, 73.0], 1.0)
            } else {
                visual::request_ink_color()
            };
            // Match the preview's 18-point symbol viewport and 15-point disc.
            let disc: Retained<StatusContentView> = unsafe {
                msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: rect(self.list_width() - 61.0, y - 1.0, 18.0, 18.0)]
            };
            let circle = NSBox::initWithFrame(NSBox::alloc(self.mtm()), rect(1.5, 1.5, 15.0, 15.0));
            circle.setBoxType(NSBoxType::Custom);
            circle.setTitlePosition(NSTitlePosition::NoTitle);
            circle.setBorderWidth(0.0);
            circle.setCornerRadius(7.5);
            circle.setFillColor(&color);
            disc.addSubview(&circle);
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                if is_finished {
                    ns_string!("checkmark")
                } else {
                    ns_string!("exclamationmark")
                },
                Some(if is_finished {
                    ns_string!("Finished (unseen)")
                } else {
                    ns_string!("Needs response")
                }),
            ) {
                let configuration =
                    NSImageSymbolConfiguration::configurationWithPointSize_weight(10.0, 0.4);
                let image = image
                    .imageWithSymbolConfiguration(&configuration)
                    .unwrap_or(image);
                image.setTemplate(true);
                let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                icon.setFrame(rect(4.0, 4.0, 10.0, 10.0));
                icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                icon.setContentTintColor(Some(&ink));
                disc.addSubview(&icon);
            }
            disc.setToolTip(Some(if is_finished {
                ns_string!("Finished (unseen)")
            } else {
                ns_string!("Needs response")
            }));
            view.addSubview(&disc);
            return;
        }
        let (symbol, color) = match activity {
            Activity::Idle | Activity::Confirmation | Activity::Working => return,
            Activity::Unknown => ("questionmark.circle", NSColor::tertiaryLabelColor()),
        };
        if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol),
            Some(&NSString::from_str(activity.label())),
        ) {
            image.setTemplate(true);
            let frame = rect(self.list_width() - 59.0, y + 1.0, 14.0, 14.0);
            let icon = NSImageView::imageViewWithImage(&image, self.mtm());
            icon.setFrame(frame);
            icon.setContentTintColor(Some(&color));
            icon.setToolTip(Some(&NSString::from_str(activity.label())));
            view.addSubview(&icon);
        }
    }

    fn advance_robot_eyes(&self) {
        let state = self.ivars();
        let seconds = state
            .working_epoch
            .get_or_init(Instant::now)
            .elapsed()
            .as_secs_f64();
        let animate = !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        for item in state.layout.borrow().iter() {
            item.robot.advance(seconds, animate);
        }
        for caption in state.work_glows.borrow().iter() {
            caption.advance(seconds, animate && state.menu_open.get());
        }
    }

    fn close_panel(&self) {
        if let Some(scroll) = self.ivars().scroll.get() {
            scroll.ivars().controller.borrow_mut().take();
        }
        self.end_shortcut_recording(false);
        for caption in self.ivars().work_glows.borrow().iter() {
            caption.release_sprite();
        }
        for composer in self.ivars().composers.borrow().values() {
            composer.stop_editing();
        }
        let panels: Vec<_> = self
            .ivars()
            .question_panels
            .borrow()
            .values()
            .cloned()
            .collect();
        for panel in panels {
            panel.stop_editing();
        }
        if let Some(extension) = self.ivars().footer_extension.get() {
            extension.removeFromSuperview();
        }
        if let Some(extension) = self.ivars().header_extension.get() {
            extension.removeFromSuperview();
        }
        self.stop_keyboard_tracking();
        self.ivars().focus_answer_on_open.set(false);
        self.ivars().keyboard_focus.borrow_mut().take();
        self.ivars().filter_query.borrow_mut().clear();
        self.ivars().filter_dirty.set(true);
        self.stop_pointer_tracking();
        let state = self.ivars();
        state.reset_disclosures();
    }

    fn start_keyboard_tracking(&self) {
        let state = self.ivars();
        if state.root.get().and_then(|root| root.window()).is_some()
            && state.resume_editors_on_attach.replace(false)
        {
            self.resume_host_editors();
        }
        // Install after NSMenu has attached the view and entered its tracking loop.
        if state.menu_open.get()
            && state.keyboard_handler.borrow().is_none()
            && state.root.get().and_then(|root| root.window()).is_some()
        {
            match hotkey::MenuKeyboard::new(self.mtm(), objc2::rc::Weak::new(self)) {
                Ok(handler) => *state.keyboard_handler.borrow_mut() = Some(handler),
                Err(error) => eprintln!("ai-agent-monitor: {error:#}"),
            }
        }
        // Wait until AppKit has attached and sized the tracking window so that
        // revealing an answer uses the actual menu viewport, not its old frame.
        if state.menu_open.get()
            && state.root.get().and_then(|root| root.window()).is_some()
            && state.focus_answer_on_open.replace(false)
            && !state.settings_active.get()
        {
            self.focus_answer();
        }
    }

    fn focus_answer(&self) {
        let state = self.ivars();
        let targets = self.keyboard_targets(&state.layout.borrow());
        let requested = state.keyboard_focus.borrow().clone();
        let first = {
            let controls = state.request_controls.borrow();
            targets
                .iter()
                .find(|id| {
                    controls.get(*id).is_some_and(|control| {
                        !matches!(
                            control.kind,
                            ControlKind::ContextToggle(_) | ControlKind::Composer
                        ) && Some(control.session.key()) == requested
                    })
                })
                .or_else(|| {
                    targets.iter().find(|id| {
                        controls.get(*id).is_some_and(|control| {
                            !matches!(
                                control.kind,
                                ControlKind::ContextToggle(_) | ControlKind::Composer
                            )
                        })
                    })
                })
                .cloned()
        };
        if let Some(id) = first {
            *state.keyboard_focus.borrow_mut() = Some(id);
        }
        self.reveal_keyboard_focus();
        self.apply_hover(state.hovered.get());
    }

    fn stop_keyboard_tracking(&self) {
        self.ivars().keyboard_handler.borrow_mut().take();
    }

    pub fn handle_session_key(&self, event: &NSEvent) -> bool {
        // NSApplication retains the resident controller, while the actual
        // controls and focus belong to the current disposable popup.
        // Clone before dispatch: Enter/Escape can close and release that popup.
        let popup = self.ivars().popup.borrow().clone();
        if let Some(popup) = popup {
            return popup.handle_session_key(event);
        }
        let state = self.ivars();
        if let Some(window) = state.detached_window.borrow().as_ref()
            && !window.isKeyWindow()
        {
            return false;
        }
        if self.is_detached()
            && event.keyCode() == 13
            && event
                .modifierFlags()
                .contains(NSEventModifierFlags::Command)
            && !event
                .modifierFlags()
                .intersects(NSEventModifierFlags::Control | NSEventModifierFlags::Option)
        {
            self.close_menu(sel!(closeMenu:), None);
            return true;
        }
        if state.settings_active.get() {
            return self.handle_settings_key(event);
        }
        if state.menu_open.get() {
            let composers: Vec<_> = state.composers.borrow().values().cloned().collect();
            if menu_navigation_key(event.keyCode(), event.modifierFlags()) {
                for composer in &composers {
                    if composer.editing() {
                        *state.keyboard_focus.borrow_mut() = Some(composer.id());
                        composer.stop_editing();
                    }
                }
            } else {
                let focus = state.keyboard_focus.borrow().clone();
                if composers
                    .iter()
                    .any(|composer| composer.handle_key(event, focus.as_deref()))
                {
                    return true;
                }
            }
            let panels: Vec<_> = state.question_panels.borrow().values().cloned().collect();
            if menu_navigation_key(event.keyCode(), event.modifierFlags()) {
                // Commit text before navigation, preserving the active input's
                // place in the menu order instead of letting its editor eat arrows.
                for panel in &panels {
                    if let Some(id) = panel.editing_target() {
                        *state.keyboard_focus.borrow_mut() = Some(id);
                        panel.stop_editing();
                    }
                }
            } else {
                let focus = state.keyboard_focus.borrow().clone();
                if panels
                    .iter()
                    .any(|panel| panel.handle_key(event, focus.as_deref()))
                {
                    return true;
                }
            }
        }
        if !state.menu_open.get()
            || event.modifierFlags().intersects(
                NSEventModifierFlags::Command
                    | NSEventModifierFlags::Control
                    | NSEventModifierFlags::Option,
            )
        {
            return false;
        }
        let key = event.keyCode();
        if key == 53 && state.usage_resets_expanded.get() {
            state.usage_resets_expanded.set(false);
            self.render_footer();
            return true;
        }
        if matches!(key, 36 | 76 | 49)
            && state.keyboard_focus.borrow().as_deref() == Some("usage-resets")
        {
            if !event.isARepeat() {
                self.toggle_usage_resets(sel!(toggleUsageResets:), None);
            }
            return true;
        }
        if key == 49 {
            let focus = state.keyboard_focus.borrow().clone();
            let button = focus.as_ref().and_then(|id| {
                state
                    .question_panels
                    .borrow()
                    .values()
                    .find_map(|panel| panel.preset_button(id))
            });
            if let Some(button) = button {
                if !event.isARepeat() {
                    // SAFETY: Activate the live preset's configured target/action,
                    // exactly as Enter does; text editors handled Space above.
                    unsafe {
                        button.performClick(None);
                    }
                }
                return true;
            }
        }
        if key == 51 {
            let text = state.filter_query.borrow().clone();
            let text = NSString::from_str(&text);
            if text.length() > 0 {
                let range = text.rangeOfComposedCharacterSequenceAtIndex(text.length() - 1);
                *state.filter_query.borrow_mut() =
                    text.substringToIndex(range.location).to_string();
                self.filter_edited();
            }
            return true;
        }
        if !matches!(key, 123 | 124 | 125 | 126 | 36 | 76) {
            let text = event
                .characters()
                .map(|text| text.to_string())
                .unwrap_or_default();
            if !search::printable(&text) {
                return false;
            }
            state.filter_query.borrow_mut().push_str(&text);
            self.filter_edited();
            return true;
        }
        if matches!(key, 125 | 126) {
            let ids = self.keyboard_targets(&state.layout.borrow());
            let current = state
                .keyboard_focus
                .borrow()
                .as_ref()
                .and_then(|id| ids.iter().position(|key| key == id));
            if let Some(index) = adjacent_session(current, ids.len(), key == 125) {
                *state.keyboard_focus.borrow_mut() = Some(ids[index].clone());
                self.reveal_keyboard_focus();
                self.apply_hover(state.hovered.get());
            }
            return true;
        }
        let Some(id) = state.keyboard_focus.borrow().clone() else {
            // With no target inside the panel, let the native menu bar handle
            // horizontal navigation to neighboring status items.
            return !matches!(key, 123 | 124);
        };
        if state
            .composers
            .borrow()
            .values()
            .any(|composer| composer.id() == id)
        {
            // A field target never falls through to session disclosure/open.
            return true;
        }
        let control = state.request_controls.borrow().get(&id).cloned();
        let session_id = control
            .as_ref()
            .map(|control| control.session.key())
            .unwrap_or_else(|| id.clone());
        if matches!(key, 123 | 124) {
            if control.is_some() {
                let controls = state.request_controls.borrow();
                let buttons: Vec<_> = state
                    .buttons
                    .borrow()
                    .iter()
                    .filter_map(|item| {
                        let id = item.button.identifier()?.to_string();
                        controls.get(&id).filter(|control| {
                            control.session.key() == session_id
                                && !matches!(control.kind, ControlKind::ContextToggle(_))
                        })?;
                        let frame = item.button.frame();
                        Some((
                            id,
                            frame.origin.y + frame.size.height / 2.0,
                            item.button.isEnabled(),
                        ))
                    })
                    .collect();
                drop(controls);
                if let Some(next) = horizontal_answer(&id, &buttons, key == 124) {
                    *state.keyboard_focus.borrow_mut() = Some(next);
                    self.reveal_keyboard_focus();
                    self.apply_hover(state.hovered.get());
                    return true;
                }
                // A response control may use horizontal navigation, but must
                // never fall through to the owning session's disclosure.
                return true;
            }
            let expanded = state.expanded.borrow().contains(&session_id);
            if !event.isARepeat() && expanded != (key == 124) {
                *state.keyboard_focus.borrow_mut() = Some(session_id.clone());
                self.begin_disclosure(&session_id);
            }
        } else if control.is_some() && !event.isARepeat() {
            let button = state
                .buttons
                .borrow()
                .iter()
                .find(|item| {
                    item.button.isEnabled()
                        && item
                            .button
                            .identifier()
                            .is_some_and(|key| key.to_string() == id)
                })
                .map(|item| item.button.clone());
            if let Some(button) = button {
                // SAFETY: The live button retains its configured target/action
                // contract, including inline radios and the shared submit button.
                unsafe {
                    button.performClick(None);
                }
            }
        } else if !event.isARepeat()
            && let Some(session) = state.all.borrow().iter().find(|s| s.key() == id).cloned()
        {
            state.acknowledge_finished(&session.key());
            self.dismiss_menu();
            self.monitor().open(&session);
        }
        true
    }

    fn handle_settings_key(&self, event: &NSEvent) -> bool {
        let state = self.ivars();
        if state.shortcut_monitor.borrow().is_some() {
            return self.record_shortcut_key(event);
        }
        if event.keyCode() == 53 {
            self.hide_settings();
            return true;
        }
        let Some(shell) = state.settings_shell.get() else {
            return true;
        };
        let Some(window) = shell.scroll.window() else {
            return true;
        };
        let controls = shell.controls();
        let responder = window.firstResponder();
        let at = controls.iter().position(|control| {
            control
                .downcast_ref::<NSTextField>()
                .is_some_and(|field| field.currentEditor().is_some())
                || responder.as_ref().is_some_and(|r| {
                    std::ptr::eq::<AnyObject>(
                        (&**r as *const NSResponder).cast(),
                        (&**control as *const NSControl).cast(),
                    )
                })
        });
        let editor = at
            .and_then(|i| controls[i].downcast_ref::<NSTextField>())
            .and_then(|f| f.currentEditor());
        let flags = event.modifierFlags();
        if let Some(editor) = editor.as_ref() {
            if flags.contains(NSEventModifierFlags::Command) {
                // SAFETY: Standard edit commands on the active native field editor.
                unsafe {
                    match event.keyCode() {
                        0 => editor.selectAll(None),
                        7 => editor.cut(None),
                        8 => editor.copy(None),
                        9 => editor.paste(None),
                        _ => return false,
                    }
                }
                return true;
            }
            if !matches!(event.keyCode(), 48 | 125 | 126) {
                editor.keyDown(event);
                return true;
            }
        }
        if matches!(event.keyCode(), 48 | 125 | 126) && !controls.is_empty() {
            let backwards = event.keyCode() == 126
                || (event.keyCode() == 48 && flags.contains(NSEventModifierFlags::Shift));
            let next = match at {
                Some(i) => (i + if backwards { controls.len() - 1 } else { 1 }) % controls.len(),
                None => {
                    if backwards {
                        controls.len() - 1
                    } else {
                        0
                    }
                }
            };
            // SAFETY: Commit the current field's editor before changing its owner.
            unsafe {
                window.endEditingFor(None);
            }
            if let Some(field) = controls[next].downcast_ref::<NSTextField>() {
                inline_questions::begin_editing(field, None);
            } else {
                window.makeFirstResponder(Some(&controls[next]));
            }
            controls[next].scrollRectToVisible(controls[next].bounds());
            shell.paint();
            return true;
        }
        if matches!(event.keyCode(), 123 | 124)
            && at.is_some_and(|i| {
                shell.tabs.iter().any(|tab| {
                    std::ptr::eq::<AnyObject>(
                        (&**tab as *const NSButton).cast(),
                        (&*controls[i] as *const NSControl).cast(),
                    )
                })
            })
        {
            let index =
                (state.settings_selected.get() + if event.keyCode() == 124 { 1 } else { 4 }) % 5;
            state.settings_selected.set(index);
            self.layout_settings();
            window.makeFirstResponder(Some(&shell.tabs[index]));
            return true;
        }
        if matches!(event.keyCode(), 36 | 76 | 49)
            && !flags.intersects(
                NSEventModifierFlags::Command
                    | NSEventModifierFlags::Control
                    | NSEventModifierFlags::Option,
            )
        {
            if let Some(i) = at {
                unsafe {
                    controls[i].performClick(None);
                }
            }
            return true;
        }
        !flags.intersects(
            NSEventModifierFlags::Command
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Option,
        )
    }

    fn keyboard_targets(&self, layout: &[LayoutRow]) -> Vec<String> {
        let state = self.ivars();
        let expanded = state.expanded.borrow();
        let controls = state.request_controls.borrow();
        let buttons = state.buttons.borrow();
        let mut answers: Vec<_> = buttons
            .iter()
            .filter(|item| item.button.isEnabled() || item.focusable_when_disabled)
            .filter(|item| !item.button.isHiddenOrHasHiddenAncestor())
            .filter_map(|item| {
                let key = item.button.identifier()?.to_string();
                let control = controls.get(&key)?;
                if matches!(control.kind, ControlKind::ContextToggle(_)) {
                    return None;
                }
                let owner = control.session.key();
                Some((key, owner))
            })
            .collect();
        for composer in state
            .composers
            .borrow()
            .values()
            .filter(|composer| composer.attached() && composer.field().isEnabled())
        {
            let before = answers
                .iter()
                .position(|(id, _)| id == &format!("compose-send:{}", composer.session()))
                .unwrap_or(answers.len());
            answers.insert(before, (composer.id(), composer.session().to_owned()));
        }
        let mut order = keyboard_order(
            layout.iter().filter_map(|row| {
                row.id
                    .as_ref()
                    .map(|id| (id.clone(), expanded.contains(id)))
            }),
            &answers,
        );
        if !state.settings_active.get() {
            order.push("usage-resets".into());
        }
        order
    }

    fn filter_edited(&self) {
        self.ivars().filter_dirty.set(true);
        self.filter();
        // Filtering may select its first match, but background state updates
        // must not recreate keyboard focus after an answer has resolved.
        if !self.ivars().filter_query.borrow().is_empty()
            && self.ivars().keyboard_focus.borrow().is_none()
        {
            *self.ivars().keyboard_focus.borrow_mut() = self
                .ivars()
                .layout
                .borrow()
                .iter()
                .find_map(|row| row.id.clone());
            self.apply_hover(self.ivars().hovered.get());
        }
        self.render_footer();
        self.reveal_keyboard_focus();
    }

    fn reveal_keyboard_focus(&self) {
        let state = self.ivars();
        let focus = state.keyboard_focus.borrow().clone();
        let button = state
            .buttons
            .borrow()
            .iter()
            .find(|item| {
                focus
                    .as_ref()
                    .is_some_and(|id| state.request_controls.borrow().contains_key(id))
                    && item
                        .button
                        .identifier()
                        .is_some_and(|id| Some(id.to_string()) == focus)
            })
            .map(|item| item.button.clone());
        let header = button
            .and_then(|button| {
                state
                    .list
                    .get()
                    .map(|list| list.convertRect_fromView(button.bounds(), Some(&button)))
            })
            .or_else(|| {
                let composers = state.composers.borrow();
                let composer = composers.values().find(|composer| {
                    focus.as_deref() == Some(composer.id().as_str()) && composer.attached()
                })?;
                state.list.get().map(|list| {
                    list.convertRect_fromView(composer.view().bounds(), Some(composer.view()))
                })
            })
            .or_else(|| {
                state
                    .layout
                    .borrow()
                    .iter()
                    .find(|row| row.id.is_some() && row.id == focus)
                    .map(|row| {
                        let frame = row.view.frame();
                        rect(
                            frame.origin.x,
                            frame.origin.y - SESSION_HOVER_PADDING,
                            frame.size.width,
                            SESSION_HEADER_HEIGHT + 2.0 * SESSION_HOVER_PADDING,
                        )
                    })
            });
        // Release layout borrows: scrolling synchronously updates hover hit testing.
        if let Some(header) = header
            && let Some(list) = state.list.get()
        {
            // Reserve the overlay's height below the focused control.
            let visible_target = rect(
                header.origin.x,
                header.origin.y,
                header.size.width,
                header.size.height + state.footer_height.get(),
            );
            list.scrollRectToVisible(visible_target);
        }
        if let Some(id) = focus {
            for composer in state.composers.borrow().values() {
                composer.focus(&id);
            }
            let panels: Vec<_> = state.question_panels.borrow().values().cloned().collect();
            for panel in panels {
                if panel.focus_field(&id) {
                    break;
                }
            }
        }
    }

    fn start_pointer_tracking(&self) {
        let state = self.ivars();
        state.last_pointer.set(None);
        if state.pointer_timer.borrow().is_none() {
            // Sample the actual screen pointer independently of first responder,
            // field editors and AppKit's mouse-moved delivery. Common modes keep
            // this alive during scrolling and control tracking. Only runs while shown.
            // SAFETY: Target is the retained controller and implements pollPointer:.
            let timer = unsafe {
                NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    1.0 / 60.0,
                    self,
                    sel!(pollPointer:),
                    None,
                    true,
                )
            };
            // SAFETY: Valid timer scheduled on the UI thread's common run-loop modes.
            unsafe {
                NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
            }
            *state.pointer_timer.borrow_mut() = Some(timer);
        }
        self.sample_pointer();
    }

    fn stop_pointer_tracking(&self) {
        self.paint_resize_cursor(false);
        self.paint_hand_cursor(false);
        if let Some(timer) = self.ivars().pointer_timer.borrow_mut().take() {
            timer.invalidate();
        }
        self.ivars().last_pointer.set(None);
        self.clear_hover();
    }

    #[allow(deprecated)] // Keep the horizontal resize cursor on older supported macOS versions.
    fn paint_resize_cursor(&self, active: bool) {
        let was_active = self.ivars().resize_cursor_active.replace(active);
        if !active && !was_active {
            return;
        }
        let cursor = NSCursor::resizeLeftRightCursor();
        if active {
            // Native menu tracking does not reliably honor NSView cursor rects.
            cursor.set();
        } else if was_active && NSCursor::currentCursor() == cursor {
            // Only release the cursor we own; leave native text/link cursors alone.
            NSCursor::arrowCursor().set();
        }
    }

    /// Handle the tracking window's real edges before NSMenu consumes its gutters.
    pub(super) fn handle_width_mouse(&self, event: &NSEvent) -> bool {
        let popup = self.ivars().popup.borrow().clone();
        if let Some(popup) = popup {
            return popup.handle_width_mouse(event);
        }
        if self.is_detached()
            || !self.ivars().menu_open.get()
            || event.r#type() != NSEventType::LeftMouseDown
        {
            return false;
        }
        let Some(root) = self.ivars().root.get() else {
            return false;
        };
        let Some(window) = root.window() else {
            return false;
        };
        if event
            .window(self.mtm())
            .is_some_and(|other| other.windowNumber() != window.windowNumber())
        {
            return false;
        }
        let Some(left) = resize_edge(NSEvent::mouseLocation(), window.frame()) else {
            return false;
        };
        root.track_resize(left);
        true
    }

    pub(super) fn sync_resize_cursor(&self) {
        let popup = self.ivars().popup.borrow().clone();
        if let Some(popup) = popup {
            popup.sync_resize_cursor();
            return;
        }
        let active = !self.is_detached()
            && self.ivars().menu_open.get()
            && self.ivars().root.get().is_some_and(|root| {
                root.ivars().resize_drag.get().is_some()
                    || root.window().is_some_and(|window| {
                        resize_edge(NSEvent::mouseLocation(), window.frame()).is_some()
                    })
            });
        self.paint_resize_cursor(active);
        self.sync_action_cursor();
    }

    fn sample_pointer(&self) {
        self.sync_resize_cursor();
        let state = self.ivars();
        if state.settings_active.get() {
            if let Some(shell) = state.settings_shell.get() {
                shell.paint();
            }
            self.sync_backdrop();
            self.sync_action_cursor();
            return;
        }
        let Some(window) = state.root.get().and_then(|view| view.window()) else {
            return;
        };
        // The custom view is attached to AppKit's menu window while tracking.
        if !window.acceptsMouseMovedEvents() {
            window.setAcceptsMouseMovedEvents(true);
        }
        let point = window.convertPointFromScreen(NSEvent::mouseLocation());
        if state.last_pointer.replace(Some(point)) != Some(point) {
            self.update_hover_at(point);
        }
        self.sync_action_cursor();
    }

    fn paint_hand_cursor(&self, active: bool) {
        let was_active = self.ivars().hand_cursor_active.replace(active);
        let cursor = NSCursor::pointingHandCursor();
        if active {
            cursor.set();
        } else if was_active && NSCursor::currentCursor() == cursor {
            // Never replace native text, link or resize cursors on exit.
            NSCursor::arrowCursor().set();
        }
    }

    fn sync_action_cursor(&self) {
        let state = self.ivars();
        if NSApplication::sharedApplication(self.mtm()).activationPolicy()
            == NSApplicationActivationPolicy::Prohibited
        {
            return;
        }
        let active = !state.resize_cursor_active.get()
            && state.menu_open.get()
            && state
                .root
                .get()
                .and_then(|root| root.window())
                .is_some_and(|window| {
                    let screen_point = NSEvent::mouseLocation();
                    window.isVisible()
                        && NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(
                            screen_point,
                            0,
                            self.mtm(),
                        ) == window.windowNumber()
                        && self.action_cursor_at(window.convertPointFromScreen(screen_point))
                });
        self.paint_hand_cursor(active);
    }

    fn action_cursor_at(&self, point: NSPoint) -> bool {
        let state = self.ivars();
        let Some(root) = state.root.get() else {
            return false;
        };
        // hitTest expects the receiver's parent coordinates, not document ones.
        let parent = unsafe { root.superview() };
        let point = parent.map_or(point, |parent| parent.convertPoint_fromView(point, None));
        let mut hit = root.hitTest(point);
        while let Some(view) = hit {
            if view.downcast_ref::<NSTextView>().is_some() {
                return false;
            }
            if let Some(field) = view.downcast_ref::<NSTextField>()
                && (field.isEditable() || field.isSelectable())
            {
                return false;
            }
            if let Some(control) = view.downcast_ref::<NSControl>()
                && control.action().is_some()
            {
                return control.isEnabled();
            }
            if std::ptr::eq(&*view, &***root) {
                break;
            }
            hit = unsafe { view.superview() };
        }
        // Custom session titles and radio surfaces share the actual hover hit
        // regions; empty row space and hidden/disabled buttons have no target.
        !state.settings_active.get() && state.hovered.get().is_some()
    }

    fn apply_hover(&self, target: Option<pointer::Target>) {
        for composer in self.ivars().composers.borrow().values() {
            composer.paint(self.ivars().keyboard_focus.borrow().as_deref());
        }
        let state = self.ivars();
        state.hovered.set(target);
        // A native field editor is the keyboard target even before the next timer tick.
        let editing = state
            .question_panels
            .borrow()
            .values()
            .find_map(|panel| panel.editing_target());
        if let Some(id) = editing {
            *state.keyboard_focus.borrow_mut() = Some(id);
        }
        let focus = state.keyboard_focus.borrow();
        for (index, item) in state
            .layout
            .borrow()
            .iter()
            .filter(|item| item.row.is_some())
            .enumerate()
        {
            if let Some(row) = &item.row {
                row.set_highlight(
                    target == Some(pointer::Target::Row(index)),
                    item.id
                        .as_ref()
                        .is_some_and(|id| Some(id) == focus.as_ref()),
                );
            }
        }
        for (index, item) in state
            .toolbar_buttons
            .borrow()
            .iter()
            .chain(state.buttons.borrow().iter())
            .enumerate()
        {
            if item.button.isHiddenOrHasHiddenAncestor() {
                paint_hover(&item.background, 0.0);
                continue;
            }
            let focused = item.button.identifier().is_some_and(|id| {
                let id = id.to_string();
                focus.as_ref() == Some(&id) && state.request_controls.borrow().contains_key(&id)
            });
            if let Some(radio) = &item.radio {
                radio.paint(focused, target == Some(pointer::Target::Button(index)));
                paint_hover(&item.background, 0.0);
                continue;
            }
            let hovered = target == Some(pointer::Target::Button(index));
            if item
                .button
                .identifier()
                .is_some_and(|id| id.to_string() == "usage-resets")
            {
                item.background.setFillColor(&appearance_color(
                    [196.0, 184.0, 214.0],
                    [33.0, 26.0, 46.0],
                    1.0,
                ));
                let focused = focus.as_deref() == Some("usage-resets");
                item.background
                    .setBorderWidth(if focused { 2.0 } else { 0.0 });
                item.background
                    .setBorderColor(&NSColor::controlAccentColor());
                paint_hover(
                    &item.background,
                    if hovered || focused || state.usage_resets_expanded.get() {
                        1.0
                    } else {
                        0.0
                    },
                );
                continue;
            }
            if index < state.toolbar_buttons.borrow().len() {
                let color = if focused {
                    primary_text_color()
                } else if hovered {
                    secondary_text_color()
                } else {
                    visual::tertiary_text_color()
                };
                item.button.setContentTintColor(Some(&color));
            }
            if let Some(surface) = item.background.downcast_ref::<GlassBox>() {
                surface.ivars().focused.set(focused);
                surface.ivars().hovered.set(hovered);
                surface.ivars().enabled.set(item.button.isEnabled());
            }
            let background = if focused {
                NSColor::controlAccentColor()
            } else if item.caption.is_some() && item.base_alpha > 0.0 {
                appearance_color(
                    if hovered {
                        [222.0, 208.0, 237.0]
                    } else {
                        [225.0, 213.0, 237.0]
                    },
                    if hovered {
                        [87.0, 73.0, 104.0]
                    } else {
                        [69.0, 59.0, 84.0]
                    },
                    1.0,
                )
            } else {
                hover_fill_color()
            };
            item.background.setFillColor(&background);
            if let Some(caption) = &item.caption {
                let color = if item.button.isEnabled() {
                    if item
                        .background
                        .downcast_ref::<GlassBox>()
                        .is_some_and(|surface| surface.ivars().kind.get() == 4)
                    {
                        NSColor::whiteColor()
                    } else {
                        primary_text_color()
                    }
                } else {
                    NSColor::disabledControlTextColor()
                };
                caption.setTextColor(Some(&color));
                if let Some(symbol) = &item.symbol {
                    symbol.setNeedsDisplay(true);
                }
                paint_hover(
                    &item.background,
                    if focused || hovered || item.base_alpha > 0.0 {
                        1.0
                    } else {
                        item.base_alpha
                    },
                );
                continue;
            }
            let title = item.button.title();
            let attributed = NSMutableAttributedString::initWithString(
                NSMutableAttributedString::alloc(),
                &title,
            );
            let color = if focused {
                primary_text_color()
            } else {
                NSColor::labelColor()
            };
            // SAFETY: The range covers this exact title and the attribute is an NSColor.
            unsafe {
                attributed.addAttribute_value_range(
                    NSForegroundColorAttributeName,
                    &color,
                    NSRange::new(0, title.length()),
                );
                if let Some(font) = item.button.font() {
                    attributed.addAttribute_value_range(
                        NSFontAttributeName,
                        &font,
                        NSRange::new(0, title.length()),
                    );
                }
            }
            item.button.setAttributedTitle(&attributed);
            paint_hover(
                &item.background,
                if focused || target == Some(pointer::Target::Button(index)) {
                    1.0
                } else {
                    item.base_alpha
                },
            );
        }
        drop(focus);
        // Flush the changed decorations even when no control redraw or click
        // occurs: pointer sampling can run while NSApplication waits for events.
        if let Some(root) = state.root.get() {
            root.setNeedsDisplay(true);
            if let Some(window) = root.window() {
                window.displayIfNeeded();
            }
        }
    }

    fn clear_hover(&self) {
        for item in self.ivars().buttons.borrow().iter() {
            if let Some(gutter) = &item.gutter {
                gutter.icon.setHidden(true);
            }
        }
        self.apply_hover(None);
    }

    fn update_hover(&self) {
        if let Some(window) = self.ivars().root.get().and_then(|view| view.window()) {
            self.update_hover_at(window.convertPointFromScreen(NSEvent::mouseLocation()));
        } else {
            self.clear_hover();
        }
    }

    fn update_hover_at(&self, point: NSPoint) {
        let state = self.ivars();
        if !state.menu_open.get() {
            self.clear_hover();
            return;
        }
        let (Some(root), Some(list), Some(scroll)) =
            (state.root.get(), state.list.get(), state.scroll.get())
        else {
            return;
        };
        let area = |rect: NSRect| pointer::Area {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        };
        let window = area(list.convertRect_fromView(root.bounds(), Some(root)));
        let clip = scroll.contentView();
        let mut viewport = area(list.convertRect_fromView(clip.bounds(), Some(&clip)));
        // The opaque quota overlay must not activate controls hidden behind it.
        viewport.height = (viewport.height - state.footer_height.get()).max(0.0);
        let p = list.convertPoint_fromView(point, None);
        let button_area = |item: &ButtonHover| {
            if item.button.isEnabled() && !item.button.isHiddenOrHasHiddenAncestor() {
                // visibleRect describes drawing visibility and may exceed the
                // view's bounds when clipping is disabled. Never let it enlarge
                // a hover target; use it only to clip the button's actual bounds.
                area(list.convertRect_fromView(item.button.bounds(), Some(&item.button)))
                    .intersected(area(
                        list.convertRect_fromView(item.button.visibleRect(), Some(&item.button)),
                    ))
            } else {
                pointer::Area {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                }
            }
        };
        for item in state.buttons.borrow().iter() {
            if let Some(gutter) = &item.gutter {
                // SAFETY: All views are retained and accessed on the main thread.
                let message = unsafe { item.button.superview() }.map(|parent| {
                    area(list.convertRect_fromView(gutter.message, Some(&parent))).intersected(
                        area(list.convertRect_fromView(parent.visibleRect(), Some(&parent))),
                    )
                });
                let visible = !item.button.isHiddenOrHasHiddenAncestor()
                    && message.is_some_and(|message| {
                        pointer::gutter_visible(
                            (p.x, p.y),
                            window,
                            viewport,
                            message,
                            area(
                                list.convertRect_fromView(item.button.bounds(), Some(&item.button)),
                            )
                            .intersected(area(
                                list.convertRect_fromView(
                                    item.button.visibleRect(),
                                    Some(&item.button),
                                ),
                            )),
                        )
                    });
                gutter.icon.setHidden(!visible);
            }
        }
        let toolbar: Vec<_> = state
            .toolbar_buttons
            .borrow()
            .iter()
            .map(&button_area)
            .collect();
        let buttons: Vec<_> = state.buttons.borrow().iter().map(&button_area).collect();
        let headers: Vec<_> = state
            .layout
            .borrow()
            .iter()
            .filter(|item| item.row.is_some())
            .map(|item| {
                let frame = item.view.frame();
                pointer::Area {
                    x: frame.origin.x + SESSION_HEADER_HOVER_LEFT,
                    y: frame.origin.y - SESSION_HOVER_PADDING,
                    width: item
                        .row
                        .as_ref()
                        .and_then(|row| row.ivars().background.get())
                        .map(|background| background.frame().size.width)
                        .unwrap_or(self.header_hover_width()),
                    height: SESSION_HEADER_HEIGHT + SESSION_HOVER_PADDING * 2.0,
                }
            })
            .collect();
        if let Some(index) = state.buttons.borrow().iter().position(|item| {
            item.button
                .identifier()
                .is_some_and(|id| id.to_string() == "usage-resets")
                && button_area(item).contains((p.x, p.y))
        }) {
            self.apply_hover(Some(pointer::Target::Button(
                state.toolbar_buttons.borrow().len() + index,
            )));
            return;
        }
        self.apply_hover(pointer::target_at(
            (p.x, p.y),
            window,
            viewport,
            &toolbar,
            &buttons,
            &headers,
        ));
    }

    fn route_scroll(&self, event: &NSEvent) {
        let state = self.ivars();
        // Prevent edge propagation from the main scroller from routing back to itself.
        if state.routing_scroll.replace(true) {
            return;
        }
        let resets_scroll = state.usage_resets_scroll.borrow().clone();
        if let Some(scroll) = resets_scroll.filter(|scroll| {
            let point = scroll.convertPoint_fromView(event.locationInWindow(), None);
            let size = scroll.bounds().size;
            point.x >= 0.0 && point.y >= 0.0 && point.x < size.width && point.y < size.height
        }) {
            scroll.scrollWheel(event);
        } else if state.settings_active.get() {
            if let Some(shell) = state.settings_shell.get() {
                shell.scroll.scrollWheel(event);
            }
        } else if let Some(scroll) = state.scroll.get() {
            scroll.scrollWheel(event);
        }
        state.routing_scroll.set(false);
        self.update_hover();
    }

    fn begin_disclosure(&self, id: &str) {
        let state = self.ivars();
        if !state.all.borrow().iter().any(|session| session.key() == id) {
            return;
        }
        state.acknowledge_finished(id);
        self.advance_disclosure();
        let mut expanded = state.expanded.borrow_mut();
        let mut from = state.reveal.borrow().clone();
        for key in expanded.iter() {
            from.entry(key.clone()).or_insert(1.0);
        }
        let was_expanded = expanded.contains(id);
        from.entry(id.to_owned())
            .or_insert(if was_expanded { 1.0 } else { 0.0 });
        if was_expanded {
            expanded.remove(id);
        } else {
            expanded.insert(id.to_owned());
        }
        let to = from
            .keys()
            .map(|key| (key.clone(), if expanded.contains(key) { 1.0 } else { 0.0 }))
            .collect();
        drop(expanded);
        *state.reveal.borrow_mut() = from.clone();
        *state.motion.borrow_mut() = Some(motion::Disclosure::new(from, to, Instant::now()));
        state
            .animation_epoch
            .set(state.animation_epoch.get().wrapping_add(1));
        self.render_rows(&state.filtered_sessions());
        // pollPointer: advances the same timeline in common run-loop modes,
        // including NSMenu's tracking loop and nested NSButton tracking.
    }

    fn advance_disclosure(&self) {
        let state = self.ivars();
        let now = if state.menu_open.get() {
            Instant::now()
        } else {
            Instant::now() + motion::DURATION
        };
        let sample = state
            .motion
            .borrow()
            .as_ref()
            .map(|motion| motion.sample(now));
        if let Some((values, finished)) = sample {
            *state.reveal.borrow_mut() = values;
            if finished {
                state.motion.borrow_mut().take();
            }
            self.layout_rows();
        }
    }

    fn layout_rows(&self) {
        let state = self.ivars();
        let Some(list) = state.list.get() else {
            return;
        };
        let epoch = state.animation_epoch.get();
        let origin = list.visibleRect().origin;
        let reveal = state.reveal.borrow();
        let expanded = state.expanded.borrow();
        let layout = state.layout.borrow();
        let mut y = session_layout::TOP_INSET;
        for item in layout.iter() {
            y += item.gap;
            let progress = item
                .id
                .as_ref()
                .map(|id| {
                    reveal
                        .get(id)
                        .copied()
                        .unwrap_or(if expanded.contains(id) { 1.0 } else { 0.0 })
                })
                .unwrap_or(0.0);
            let detail_height = item.detail_height * progress;
            let height = item.base_height + detail_height;
            let frame = item.view.frame();
            item.view
                .setFrame(rect(frame.origin.x, y, frame.size.width, height));
            if let Some(thread) = item.row.as_ref().and_then(|row| row.ivars().thread.get()) {
                thread.setFrame(rect(0.0, 0.0, frame.size.width, height));
                thread.setHidden(progress <= 0.0);
                thread.setAlphaValue(progress);
                thread.setNeedsDisplay(true);
            }
            if let Some(detail) = &item.detail {
                detail.setFrame(rect(0.0, 0.0, self.list_width(), detail_height));
                detail.setAlphaValue(progress);
                detail.setHidden(progress <= 0.0);
                detail.scrollToPoint(NSPoint::new(0.0, 0.0));
            }
            if let Some(arrow) = &item.arrow {
                arrow.setFrameCenterRotation(-90.0 * progress);
            }
            y += height;
        }
        // Headers have a hover halo outside their 24-pt layout bounds.
        // Reserve room below the final row so the scroll clip cannot cut it off.
        y += LIST_BOTTOM_PADDING;
        // Menu layout may synchronously deliver menuDidClose:. Release state
        // borrows before AppKit can reset the disclosure views from that callback.
        drop(layout);
        drop(expanded);
        drop(reveal);
        list.setFrameSize(NSSize::new(self.list_width(), y.max(52.0)));
        let viewport = self.resize_content(y);
        if state.animation_epoch.get() != epoch {
            return;
        }
        let document_height = y + state.footer_height.get();
        list.setFrameSize(NSSize::new(
            self.list_width(),
            document_height.max(viewport),
        ));
        list.scrollPoint(NSPoint::new(
            origin.x,
            origin.y.min((document_height - viewport).max(0.0)),
        ));
        self.sync_backdrop();
        self.update_hover();
    }

    fn request_context(
        &self,
        session: &Session,
        width: f64,
        action: Option<(usize, &RequestControl)>,
    ) -> Option<(Retained<SessionListView>, f64)> {
        let scoped = action.is_some();
        let sections = if let Some((index, action)) = action {
            if command_action(action) {
                let key = format!(
                    "context:{}:0",
                    session.approval_key().unwrap_or_else(|| session.key())
                );
                if !self.ivars().expanded_contexts.borrow().contains(&key) {
                    return None;
                }
                vec![(
                    0,
                    "Command".to_owned(),
                    session
                        .interaction()
                        .command
                        .filter(|text| !text.trim().is_empty())?,
                )]
            } else {
                if action.detail.trim().is_empty()
                    || !matches!(action.kind, ControlKind::Reply(Response::Action(_)))
                {
                    return None;
                }
                let key = format!(
                    "context:{}:{}",
                    session.approval_key().unwrap_or_else(|| session.key()),
                    index + 1
                );
                if !self.ivars().expanded_contexts.borrow().contains(&key) {
                    return None;
                }
                vec![(index + 1, action.label.clone(), action.detail.clone())]
            }
        } else {
            session
                .interaction()
                .command
                .filter(|value| !value.trim().is_empty())
                .map(|command| vec![(0, "Command".to_owned(), command)])
                .unwrap_or_default()
        };
        if sections.is_empty() {
            return None;
        }
        // This container shares the flipped coordinates of the surrounding document.
        let view: Retained<SessionListView> = unsafe {
            msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, width, 1.0)]
        };
        let mut y = 0.0;
        for (index, title, text) in sections {
            if y > 0.0 {
                y += 8.0;
            }
            if !scoped {
                let caption = NSTextField::labelWithString(&NSString::from_str(&title), self.mtm());
                caption.setFont(Some(&NSFont::systemFontOfSize(11.0)));
                caption.setTextColor(Some(&NSColor::secondaryLabelColor()));
                caption.setFrame(rect(18.0, y, width - 48.0, 16.0));
                caption.setTextColor(Some(&secondary_text_color()));
                if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    if index == 0 {
                        ns_string!("terminal")
                    } else {
                        ns_string!("sparkles")
                    },
                    None,
                ) {
                    let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                    icon.setFrame(rect(0.0, y + 1.0, 13.0, 13.0));
                    icon.setContentTintColor(Some(&secondary_text_color()));
                    view.addSubview(&icon);
                }
                view.addSubview(&caption);
                y += 20.0;
            }
            let code = NSTextField::wrappingLabelWithString(
                &NSString::from_str(text.trim_end()),
                self.mtm(),
            );
            code.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(10.0, 0.0)));
            code.setTextColor(Some(&secondary_text_color()));
            code.setSelectable(true);
            command_text(&code, text.trim_end());
            code.setMaximumNumberOfLines(0);
            if let Some(cell) = code.cell() {
                cell.setLineBreakMode(NSLineBreakMode::ByCharWrapping);
            }
            let code_width = width - 16.0;
            let full_height = code
                .cell()
                .map(|cell| {
                    cell.cellSizeForBounds(rect(0.0, 0.0, code_width, 1000000.0))
                        .height
                        .ceil()
                })
                .unwrap_or(18.0)
                .max(18.0);
            // Measure two actual line fragments in the same native field so
            // the limit includes both explicit newlines and soft wrapping.
            let original = code.stringValue();
            command_text(&code, "M\nM");
            let two_lines = code
                .cell()
                .map(|cell| {
                    cell.cellSizeForBounds(rect(0.0, 0.0, code_width, 1000000.0))
                        .height
                        .ceil()
                })
                .unwrap_or(32.0);
            command_text(&code, &original.to_string());
            let key = format!(
                "context:{}:{index}",
                session.approval_key().unwrap_or_else(|| session.key())
            );
            let expanded = scoped || self.ivars().expanded_contexts.borrow().contains(&key);
            let overflowing = full_height > two_lines;
            let height = if expanded {
                full_height
            } else {
                full_height.min(two_lines)
            };
            code.setMaximumNumberOfLines(if expanded { 0 } else { 2 });
            let panel_height = height + 12.0;
            let background: Retained<StatusContentView> = unsafe {
                msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: rect(0.0, y, width, panel_height)]
            };
            let box_view = glass_box(self.mtm(), rect(0.0, 0.0, width, panel_height), 1);
            box_view.setBoxType(NSBoxType::Custom);
            box_view.setTitlePosition(NSTitlePosition::NoTitle);
            box_view.setCornerRadius(11.0);
            box_view.setBorderWidth(0.0);
            box_view.setFillColor(&appearance_color([218.0; 3], [42.0; 3], 1.0));
            background.addSubview(&box_view);
            view.addSubview(&background);
            code.setFrame(rect(8.0, y + 6.0, code_width, height));
            view.addSubview(&code);
            if overflowing && !scoped {
                let button = self.action_button(
                    &view,
                    "",
                    sel!(respondToRequest:),
                    &key,
                    rect(width - 28.0, y - 26.0, 28.0, 28.0),
                    false,
                );
                button.setRefusesFirstResponder(true);
                button.setImage(
                    NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        if expanded {
                            ns_string!("chevron.up")
                        } else {
                            ns_string!("chevron.down")
                        },
                        Some(if expanded {
                            ns_string!("Collapse details")
                        } else {
                            ns_string!("Expand details")
                        }),
                    )
                    .as_deref(),
                );
                button.setToolTip(Some(if expanded {
                    ns_string!("Collapse details")
                } else {
                    ns_string!("Expand details")
                }));
                if let Some(hover) = self.ivars().buttons.borrow_mut().last_mut() {
                    hover.base_alpha = 0.0;
                    paint_hover(&hover.background, 0.0);
                }
                self.ivars().request_controls.borrow_mut().insert(
                    key.clone(),
                    RequestControl {
                        session: session.clone(),
                        label: String::new(),
                        detail: String::new(),
                        kind: ControlKind::ContextToggle(key),
                    },
                );
            }
            y += panel_height;
        }
        view.setFrameSize(NSSize::new(width, y));
        Some((view, y))
    }

    fn action_disclosure(
        &self,
        view: &NSView,
        session: &Session,
        frame: NSRect,
        section: usize,
        primary: bool,
    ) {
        let key = format!(
            "context:{}:{section}",
            session.approval_key().unwrap_or_else(|| session.key())
        );
        let expanded = self.ivars().expanded_contexts.borrow().contains(&key);
        let arrow_frame = rect(
            frame.origin.x + frame.size.width - 28.0,
            frame.origin.y,
            28.0,
            frame.size.height,
        );
        // This trailing segment is above the transparent approval hit target.
        // Its parent approval surface supplies the continuous fill and hover.
        let arrow: Retained<ActionButton> =
            unsafe { msg_send![ActionButton::alloc(self.mtm()), initWithFrame: arrow_frame] };
        arrow.setBordered(false);
        arrow.setButtonType(NSButtonType::MomentaryPushIn);
        arrow.setTitle(ns_string!(""));
        arrow.setIdentifier(Some(&NSString::from_str(&key)));
        arrow.setRefusesFirstResponder(true);
        let description = match (section == 0, expanded) {
            (true, false) => ns_string!("Show command"),
            (true, true) => ns_string!("Hide command"),
            (false, false) => ns_string!("Show command scope"),
            (false, true) => ns_string!("Hide command scope"),
        };
        arrow.setToolTip(Some(description));
        arrow.setImage(
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                if expanded {
                    ns_string!("chevron.up")
                } else {
                    ns_string!("chevron.down")
                },
                Some(description),
            )
            .as_deref(),
        );
        arrow.setImagePosition(NSCellImagePosition::ImageOnly);
        let tint = if primary {
            NSColor::whiteColor()
        } else {
            primary_text_color()
        };
        arrow.setContentTintColor(Some(&tint));
        unsafe {
            arrow.setTarget(Some(self));
            arrow.setAction(Some(sel!(respondToRequest:)));
            let _: () = msg_send![&*arrow, setAccessibilityLabel: description];
        }
        let decoration: Retained<StatusContentView> = unsafe {
            msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: arrow.bounds()]
        };
        let divider = NSBox::initWithFrame(
            NSBox::alloc(self.mtm()),
            rect(0.0, 7.0, 1.0, (frame.size.height - 14.0).max(1.0)),
        );
        divider.setBoxType(NSBoxType::Custom);
        divider.setTitlePosition(NSTitlePosition::NoTitle);
        divider.setBorderWidth(0.0);
        divider.setFillColor(&tint.colorWithAlphaComponent(0.3));
        decoration.addSubview(&divider);
        arrow.addSubview(&decoration);
        view.addSubview(&arrow);
        self.ivars().request_controls.borrow_mut().insert(
            key.clone(),
            RequestControl {
                session: session.clone(),
                label: String::new(),
                detail: String::new(),
                kind: ControlKind::ContextToggle(key),
            },
        );
    }

    fn session_metadata(&self, session: &Session) -> (Retained<SessionListView>, f64) {
        // Use the same flipped coordinate system as the expanded detail document.
        // SAFETY: This ivar-free view uses NSView's initializer.
        let view: Retained<SessionListView> = unsafe {
            msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, self.detail_width(), 0.0)]
        };
        let directory = session.working_directory();
        let full_directory = directory.display().to_string();
        let short_directory = match directory.strip_prefix(core::home()) {
            Ok(relative) if relative.as_os_str().is_empty() => "~".into(),
            Ok(relative) => format!("~/{}", relative.display()),
            Err(_) => full_directory.clone(),
        };
        let mut rows = vec![(
            "folder",
            "Working directory",
            short_directory,
            full_directory,
        )];
        if let Some(branch) = &session.git_branch {
            rows.push(("git-branch", "Git branch", branch.clone(), branch.clone()));
        }
        if let Some(label) = session.model_label() {
            rows.push((
                "cpu",
                "Model",
                label,
                format!("Model: {}", session.model.as_deref().unwrap_or_default()),
            ));
        }
        let (mut x, mut y) = (0.0, 0.0);
        let height = 15.4;
        for (symbol, description, value, tooltip) in rows {
            let measure = NSTextField::labelWithString(&NSString::from_str(&value), self.mtm());
            measure.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            measure.sizeToFit();
            let width = (18.0 + measure.frame().size.width.ceil()).min(self.detail_width());
            if x > 0.0 && x + width > self.detail_width() {
                x = 0.0;
                y += height + 2.0;
            }
            let label = self.label(
                &view,
                &value,
                rect(x + 18.0, y, width - 18.0, height),
                11.0,
                0.0,
                false,
            );
            label.setSelectable(true);
            label.setTextColor(Some(&visual::tertiary_text_color()));
            label.setToolTip(Some(&NSString::from_str(&tooltip)));
            let icon = visual::MetadataIcon::new(
                self.mtm(),
                rect(x, y + (height - 12.0) / 2.0, 12.0, 12.0),
                symbol,
            );
            icon.setToolTip(Some(&NSString::from_str(description)));
            view.addSubview(&icon);
            x += width + 16.0;
        }
        let total_height = y + height;
        view.setFrameSize(NSSize::new(self.detail_width(), total_height));
        (view, total_height)
    }

    fn render_rows(&self, sessions: &[Session]) {
        self.ivars().work_glows.borrow_mut().clear();
        let snapshot = self.monitor().snapshot();
        let composers: Vec<_> = self.ivars().composers.borrow().values().cloned().collect();
        for composer in &composers {
            composer.suspend();
            if let Some(session) = snapshot
                .sessions
                .iter()
                .find(|session| session.key() == composer.session())
            {
                composer.set_width(self.detail_width());
                composer.sync(&snapshot, session);
            }
        }
        let Some(list) = self.ivars().list.get() else {
            return;
        };
        let panels: Vec<_> = self
            .ivars()
            .question_panels
            .borrow()
            .values()
            .cloned()
            .collect();
        for panel in &panels {
            panel.suspend_editing();
        }
        self.clear_hover();
        self.ivars().buttons.borrow_mut().retain(|item| {
            item.button
                .identifier()
                .is_some_and(|id| id.to_string() == "usage-resets")
        });
        self.ivars().request_controls.borrow_mut().clear();
        let children = list.subviews();
        for index in 0..children.count() {
            children.objectAtIndex(index).removeFromSuperview();
        }
        if let Some(toolbar) = self.ivars().toolbar.get() {
            list.addSubview(toolbar);
        }
        let expanded = self.ivars().expanded.borrow();
        let reveal = self.ivars().reveal.borrow();
        let mut layout = Vec::new();
        let mut y = session_layout::TOP_INSET;
        let mut ordered: Vec<_> = sessions.iter().collect();
        ordered.sort_by(|a, b| crate::platform::session_order(a, b));
        for session in ordered {
            let inline = if !session.interaction().questions.is_empty()
                && session.interaction().actions.is_empty()
            {
                session.approval_key().map(|key| {
                    let existing = self
                        .ivars()
                        .question_panels
                        .borrow()
                        .get(&key)
                        .filter(|panel| panel.matches_request(session))
                        .cloned();
                    existing
                        .map(|panel| panel.resized(self.detail_width() - 26.0))
                        .unwrap_or_else(|| {
                            let panel = inline_questions::Panel::create(
                                session.clone(),
                                self.monitor().clone(),
                                self.detail_width() - 26.0,
                                &detail_text_color(true),
                            );
                            if let Some(draft) =
                                self.ivars().question_drafts.borrow_mut().remove(&key)
                            {
                                panel.restore_draft(draft);
                            }
                            self.ivars()
                                .question_panels
                                .borrow_mut()
                                .insert(key, panel.clone());
                            panel
                        })
                })
            } else {
                None
            };
            if let Some(panel) = &inline {
                if let Some(key) = session.approval_key() {
                    self.ivars()
                        .question_panels
                        .borrow_mut()
                        .insert(key, panel.clone());
                }
                let enabled = session.authoritative()
                    && session
                        .approval_key()
                        .is_some_and(|key| !self.ivars().responding.borrow().contains(&key));
                let snapshot = self.monitor().snapshot();
                let phase = session
                    .approval_key()
                    .and_then(|key| snapshot.submissions.get(&key));
                let status = match phase {
                    Some(crate::actors::monitor::Submission::Sending) => "Sending…",
                    Some(crate::actors::monitor::Submission::Reconciling) => "Checking delivery…",
                    Some(crate::actors::monitor::Submission::Failed(error)) => error,
                    None => "",
                };
                panel.reconcile(enabled, status);
            }
            let mut controls = if inline.is_some() {
                vec![]
            } else {
                request_controls(session)
            };
            // Keep scope-specific alternatives after the immediate decisions.
            // The bound response value stays attached to its control.
            controls.sort_by_key(|control| !control.detail.trim().is_empty());
            let is_question = session.approval().is_some();
            let question_padding = if is_question { 13.0 } else { 0.0 };
            let metadata_gap = if is_question { 8.0 } else { 0.0 };
            let content_width = self.detail_width() - 2.0 * question_padding;
            let content_x = SESSION_TEXT_LEFT + question_padding;
            // Include the expanded button's visible bounds in the card padding.
            let control_inset = if is_question {
                BUTTON_HOVER_PADDING
            } else {
                0.0
            };
            let control_width = content_width - 2.0 * control_inset;
            let command_control = session
                .interaction()
                .command
                .as_ref()
                .filter(|text| !text.trim().is_empty())
                .and_then(|_| controls.iter().position(command_action));
            let context = if is_question && inline.is_none() && command_control.is_none() {
                self.request_context(session, content_width, None)
            } else {
                None
            };
            let context_height = context
                .as_ref()
                .map(|(_, height)| height + 8.0)
                .unwrap_or(0.0);
            let control_sizes: Vec<_> = controls
                .iter()
                .enumerate()
                .map(|(index, control)| {
                    let disclosure_width =
                        if command_control == Some(index) || !control.detail.trim().is_empty() {
                            28.0
                        } else {
                            0.0
                        };
                    let natural = NSTextField::wrappingLabelWithString(
                        &NSString::from_str(&control.label),
                        self.mtm(),
                    );
                    natural.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, 0.23)));
                    natural.sizeToFit();
                    let icon_width = if request_glyph(control).is_some() {
                        21.0
                    } else {
                        0.0
                    };
                    let width = (natural.frame().size.width.ceil()
                        + 4.0
                        + 2.0 * BUTTON_HORIZONTAL_INSET
                        + icon_width
                        + disclosure_width)
                        .clamp(52.0, control_width);
                    let measure = NSTextField::wrappingLabelWithString(
                        &NSString::from_str(&control.label),
                        self.mtm(),
                    );
                    measure.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, 0.23)));
                    let height = measure
                        .cell()
                        .map(|cell| {
                            cell.cellSizeForBounds(rect(
                                0.0,
                                0.0,
                                width
                                    - 2.0 * BUTTON_HORIZONTAL_INSET
                                    - icon_width
                                    - disclosure_width,
                                100000.0,
                            ))
                            .height
                            .ceil()
                                + 2.0 * BUTTON_VERTICAL_INSET
                        })
                        .unwrap_or(BUTTON_MIN_HEIGHT)
                        .max(BUTTON_MIN_HEIGHT);
                    NSSize::new(width, height)
                })
                .collect();
            let action_contexts: Vec<_> = controls
                .iter()
                .enumerate()
                .map(|(index, control)| {
                    self.request_context(session, content_width, Some((index, control)))
                })
                .collect();
            let context_heights: Vec<_> = action_contexts
                .iter()
                .zip(&controls)
                .enumerate()
                .map(|(index, (context, control))| {
                    if control.detail.trim().is_empty() && command_control != Some(index) {
                        None
                    } else {
                        Some(context.as_ref().map_or(0.0, |(_, height)| *height))
                    }
                })
                .collect();
            let (control_frames, controls_height) =
                request_action_layout(&control_sizes, &context_heights, control_width);
            let is_expanded = expanded.contains(&session.key());
            let composer = if super::session_composer::available(session)
                && (is_expanded
                    || reveal
                        .get(&session.key())
                        .is_some_and(|progress| *progress > 0.0))
            {
                let composer = self
                    .ivars()
                    .composers
                    .borrow_mut()
                    .entry(session.key())
                    .or_insert_with(|| {
                        super::session_composer::Composer::new(
                            self.mtm(),
                            self.monitor().clone(),
                            session,
                            self.detail_width(),
                        )
                    })
                    .clone();
                composer.set_width(self.detail_width());
                if let Some(draft) = self
                    .ivars()
                    .composer_drafts
                    .borrow_mut()
                    .remove(&session.key())
                {
                    composer.restore_draft(draft);
                }
                composer.sync(&snapshot, session);
                Some(composer)
            } else {
                None
            };
            let progress = session
                .work_caption()
                .filter(|_| {
                    is_expanded
                        || reveal
                            .get(&session.key())
                            .is_some_and(|progress| *progress > 0.0)
                })
                .map(|text| {
                    let (view, glow) =
                        super::work_caption::new(self.mtm(), text, self.detail_width());
                    self.ivars().work_glows.borrow_mut().push(glow);
                    view
                });
            let history = self
                .ivars()
                .message_history
                .borrow()
                .get(&session.key())
                .cloned()
                .unwrap_or_default();
            let earlier = earlier_messages(session, &history);
            let mut earlier_labels = Vec::new();
            let mut earlier_height = 0.0;
            if !earlier.is_empty()
                && (is_expanded
                    || reveal
                        .get(&session.key())
                        .is_some_and(|progress| *progress > 0.0))
            {
                for (text, images) in &earlier {
                    let label = self.message_with_images(text, images, self.detail_width(), false);
                    let height = label.frame().size.height;
                    label.setFrame(rect(
                        SESSION_TEXT_LEFT,
                        MESSAGE_TOP_GAP + earlier_height,
                        self.detail_width(),
                        height,
                    ));
                    earlier_height += height + 8.0;
                    earlier_labels.push(label);
                }
            }
            let mut first_message_height = 22.0;
            let detail = if is_expanded
                || reveal
                    .get(&session.key())
                    .is_some_and(|progress| *progress > 0.0)
            {
                let text = session
                    .approval()
                    .as_ref()
                    .map(|request| request.body.as_str())
                    .unwrap_or(session.last_message())
                    .trim_end();
                let text = if text.is_empty() {
                    "No messages yet"
                } else {
                    text
                };
                let first_text = history
                    .messages
                    .first()
                    .map(|message| message.text.as_str())
                    .or_else(|| {
                        (is_question && !session.last_message().trim().is_empty())
                            .then_some(session.last_message())
                    })
                    .unwrap_or(text);
                let first_width = if earlier_height > 0.0 || !history.messages.is_empty() {
                    self.detail_width()
                } else {
                    content_width
                };
                let first_images = history
                    .messages
                    .first()
                    .map(|message| message.images.as_slice())
                    .unwrap_or_else(|| {
                        if session.approval().is_none()
                            || (is_question && !session.last_message().trim().is_empty())
                        {
                            session.images()
                        } else {
                            &[]
                        }
                    });
                first_message_height = self
                    .message_with_images(first_text.trim_end(), first_images, first_width, false)
                    .frame()
                    .size
                    .height;
                let images = if session.approval().is_none() {
                    session.images()
                } else {
                    &[]
                };
                let label = self.message_with_images(
                    text,
                    images,
                    content_width,
                    session.approval().is_some(),
                );
                let height = inline
                    .as_ref()
                    .map(|panel| panel.view().frame().size.height)
                    .unwrap_or(label.frame().size.height);
                label.setFrame(rect(0.0, 0.0, content_width, height));
                let (metadata, metadata_height) = self.session_metadata(session);
                Some((label, height, metadata, metadata_height))
            } else {
                None
            };
            let detail_height = detail
                .as_ref()
                .map(|(_, height, _, metadata_height)| {
                    height
                        + metadata_height
                        + MESSAGE_TOP_GAP
                        + 20.0
                        + controls_height
                        + context_height
                        + 2.0 * question_padding
                        + metadata_gap
                        + earlier_height
                        + composer
                            .as_ref()
                            .map(|composer| composer.view().frame().size.height + 8.0)
                            .unwrap_or(0.0)
                        + progress
                            .as_ref()
                            .map(|view| view.frame().size.height + 8.0)
                            .unwrap_or(0.0)
                })
                .unwrap_or(0.0);
            let height = SESSION_HEADER_HEIGHT + detail_height;
            let row = SessionRowView::create(self.mtm(), rect(0.0, y, self.list_width(), height));
            if !layout.is_empty() {
                let separator = NSBox::initWithFrame(
                    NSBox::alloc(self.mtm()),
                    rect(0.0, height - 0.5, self.list_width(), 0.5),
                );
                separator.setBoxType(NSBoxType::Custom);
                separator.setTitlePosition(NSTitlePosition::NoTitle);
                separator.setBorderWidth(0.0);
                separator.setFillColor(&appearance_color(
                    [115.0, 86.0, 156.0],
                    [178.0, 153.0, 236.0],
                    0.13,
                ));
                row.addSubview(&separator);
            }
            // Reserve the status and disclosure controls independently of the
            // title's shared hover, keyboard highlight and click target.
            let finished = self
                .monitor()
                .snapshot()
                .unseen_finished
                .contains_key(&session.key());
            let title_right =
                if matches!(session.activity, Activity::Idle | Activity::Working) && !finished {
                    self.list_width() - 36.0
                } else {
                    self.list_width() - 68.0
                };
            let text_width = title_right - SESSION_TEXT_LEFT - 12.0 - 42.0;
            let robot_button = self.action_button(
                &row,
                "",
                sel!(expandSession:),
                &session.key(),
                rect(
                    session_layout::ICON_LEFT - 4.0,
                    height - SESSION_HEADER_HEIGHT / 2.0 - 14.0,
                    32.0,
                    28.0,
                ),
                false,
            );
            robot_button.setToolTip(Some(if is_expanded {
                ns_string!("Collapse")
            } else {
                ns_string!("Show details")
            }));
            let (robot_width, robot_height) = if session.activity == Activity::Working {
                (20.0, 16.0)
            } else {
                (14.0, 11.0)
            };
            let robot = SessionRobot::new(
                self.mtm(),
                rect(
                    (robot_button.bounds().size.width - robot_width) / 2.0,
                    (robot_button.bounds().size.height - robot_height) / 2.0,
                    robot_width,
                    robot_height,
                ),
                session.activity,
            );
            robot_button.addSubview(&robot);
            let title = self.label(
                &row,
                &session.title,
                rect(
                    SESSION_TEXT_LEFT,
                    height - SESSION_HEADER_HEIGHT / 2.0 - 11.0,
                    text_width,
                    22.0,
                ),
                14.0,
                0.3,
                false,
            );
            title.setTextColor(Some(&primary_text_color()));
            center_label(&title, height - SESSION_HEADER_HEIGHT / 2.0);
            let natural_width = title
                .cell()
                .map(|cell| cell.cellSize().width.ceil())
                .unwrap_or(text_width);
            let highlight_width = SESSION_TEXT_LEFT + natural_width.min(text_width) + 42.0 + 12.0
                - SESSION_HEADER_HOVER_LEFT;
            if let Some(background) = row.ivars().background.get() {
                background.setFrameSize(NSSize::new(highlight_width, SESSION_HEADER_HEIGHT - 6.0));
            }
            row.ivars()
                .matches
                .set(search::highlights(
                    &title.stringValue().to_string(),
                    &self.ivars().filter_query.borrow(),
                ))
                .expect("matches initialized once");
            row.ivars()
                .title
                .set(title)
                .expect("title initialized once");
            row.paint_title(false);
            if finished && session.activity == Activity::Idle {
                let button = self.action_button(
                    &row,
                    "",
                    sel!(markFinishedSeen:),
                    &session.key(),
                    rect(
                        self.list_width() - 63.0,
                        height - SESSION_HEADER_HEIGHT / 2.0 - 12.0,
                        22.0,
                        24.0,
                    ),
                    false,
                );
                button.setToolTip(Some(ns_string!("Mark as seen")));
            }
            self.status_icon(
                &row,
                session.activity,
                finished,
                height - SESSION_HEADER_HEIGHT / 2.0 - 8.0,
            );
            let open = self.action_button(
                &row,
                &session.title,
                sel!(openSession:),
                &session.key(),
                rect(
                    SESSION_HEADER_HOVER_LEFT,
                    height - SESSION_HEADER_HEIGHT,
                    highlight_width,
                    SESSION_HEADER_HEIGHT,
                ),
                true,
            );
            let destination: Retained<StatusContentView> = unsafe {
                msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: rect(SESSION_TEXT_LEFT + natural_width.min(text_width) + 8.0, height - SESSION_HEADER_HEIGHT / 2.0 - 8.0, 34.0, 16.0)]
            };
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("arrow.right"),
                None,
            ) {
                let arrow = NSImageView::imageViewWithImage(&image, self.mtm());
                arrow.setFrame(rect(0.0, 1.0, 14.0, 14.0));
                arrow.setHidden(true);
                arrow.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
                destination.addSubview(&arrow);
                row.ivars()
                    .destination_arrow
                    .set(arrow)
                    .expect("destination arrow initialized once");
            }
            let app_icon = self.client_icon(session.client);
            app_icon.setFrame(rect(0.0, 0.0, 16.0, 16.0));
            let children = app_icon.subviews();
            for index in 0..children.count() {
                children
                    .objectAtIndex(index)
                    .setFrame(rect(0.0, 0.0, 16.0, 16.0));
            }
            destination.addSubview(&app_icon);
            row.ivars()
                .destination_icon
                .set(app_icon)
                .expect("destination icon initialized once");
            row.addSubview(&destination);
            row.ivars()
                .destination
                .set(destination)
                .expect("destination initialized once");
            open.setToolTip(Some(&NSString::from_str(&format!(
                "Switch to {}",
                session.client.label()
            ))));
            let arrow = self.action_button(
                &row,
                "",
                sel!(expandSession:),
                &session.key(),
                rect(
                    self.list_width() - 33.0,
                    height - SESSION_HEADER_HEIGHT / 2.0 - 12.0,
                    26.0,
                    24.0,
                ),
                false,
            );
            let mut arrow_icon = None;
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("chevron.right"),
                Some(ns_string!("Session details")),
            ) {
                image.setTemplate(true);
                // SAFETY: This NSView subclass has no ivars or custom initialization.
                let decoration: Retained<StatusContentView> = unsafe {
                    msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, ICON_BUTTON_SIZE, ICON_BUTTON_SIZE)]
                };
                let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                icon.setFrame(rect(7.0, 7.0, 14.0, 14.0));
                icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
                decoration.addSubview(&icon);
                arrow.addSubview(&decoration);
                row.ivars()
                    .arrow
                    .set(icon.clone())
                    .expect("arrow initialized once");
                arrow_icon = Some(icon);
            }
            arrow.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
            arrow.setToolTip(Some(if is_expanded {
                ns_string!("Collapse")
            } else {
                ns_string!("Show details")
            }));
            // Header controls remain pinned to the top while the detail grows below.
            let header_children = row.subviews();
            for index in 0..header_children.count() {
                header_children
                    .objectAtIndex(index)
                    .setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
            }
            let mut detail_clip = None;
            if let Some((label, text_height, metadata, _metadata_height)) = detail {
                // The clip only reveals content; the main list is the sole scroller.
                // SAFETY: NSClipView initializes this ivar-free subclass.
                let clip: Retained<RevealClipView> = unsafe {
                    msg_send![RevealClipView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, self.list_width(), detail_height)]
                };
                clip.setDrawsBackground(false);
                // SAFETY: NSView's initializer is valid for this ivar-free subclass.
                let document: Retained<SessionListView> = unsafe {
                    msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, self.list_width(), detail_height)]
                };
                let question_top = MESSAGE_TOP_GAP + earlier_height;
                let message_top = question_top + question_padding;
                let text_end = message_top + text_height;
                let message_end = text_end + context_height;
                let controls_gap = if inline.is_some() { 0.0 } else { 12.0 };
                let footer_y =
                    message_end + controls_gap + controls_height + question_padding + metadata_gap;
                if is_question {
                    let frame = rect(
                        SESSION_TEXT_LEFT,
                        question_top,
                        self.detail_width(),
                        footer_y - question_top - metadata_gap,
                    );
                    // SAFETY: Decorative view ignores hit testing, preserving text selection,
                    // answer buttons and scrolling across the entire question card.
                    let decoration: Retained<StatusContentView> = unsafe {
                        msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: frame]
                    };
                    let card = glass_box(
                        self.mtm(),
                        rect(0.0, 0.0, frame.size.width, frame.size.height),
                        2,
                    );
                    card.setBoxType(NSBoxType::Custom);
                    card.setTitlePosition(NSTitlePosition::NoTitle);
                    card.setContentViewMargins(NSSize::new(0.0, 0.0));
                    card.setCornerRadius(16.0);
                    card.setBorderWidth(1.0);
                    card.setBorderColor(&NSColor::separatorColor());
                    card.setFillColor(&appearance_color([231.0; 3], [55.0; 3], 1.0));
                    decoration.addSubview(&card);
                    document.addSubview(&decoration);
                }
                if let Some((context, _)) = context {
                    context.setFrameOrigin(NSPoint::new(content_x, text_end + 8.0));
                    document.addSubview(&context);
                }
                let composer_height = composer
                    .as_ref()
                    .map(|composer| composer.view().frame().size.height + 8.0)
                    .unwrap_or(0.0);
                let progress_height = progress
                    .as_ref()
                    .map(|view| view.frame().size.height + 8.0)
                    .unwrap_or(0.0);
                metadata.setFrameOrigin(NSPoint::new(
                    SESSION_TEXT_LEFT,
                    footer_y + composer_height + progress_height,
                ));
                document.addSubview(&metadata);
                if let Some(progress) = &progress {
                    progress.setFrameOrigin(NSPoint::new(SESSION_TEXT_LEFT, text_end + 8.0));
                    document.addSubview(progress);
                }
                if let Some(composer) = &composer {
                    composer
                        .view()
                        .setFrameOrigin(NSPoint::new(SESSION_TEXT_LEFT, text_end + 8.0));
                    document.addSubview(composer.view());
                    let button = composer.button();
                    let id = button.identifier().unwrap().to_string();
                    self.ivars().request_controls.borrow_mut().insert(
                        id,
                        RequestControl {
                            session: session.clone(),
                            label: "Send".into(),
                            detail: String::new(),
                            kind: ControlKind::Composer,
                        },
                    );
                    self.ivars().buttons.borrow_mut().push(ButtonHover {
                        button,
                        background: composer.background(),
                        base_alpha: 1.0,
                        radio: None,
                        caption: Some(composer.caption()),
                        symbol: Some(composer.symbol()),
                        focusable_when_disabled: false,
                        gutter: None,
                    });
                }
                label.setFrameOrigin(NSPoint::new(content_x, message_top));
                if let Some(panel) = &inline {
                    panel
                        .view()
                        .setFrameOrigin(NSPoint::new(content_x, message_top));
                    document.addSubview(panel.view());
                    for item in panel.buttons() {
                        let id = item.button.identifier().unwrap().to_string();
                        self.ivars().request_controls.borrow_mut().insert(
                            id,
                            RequestControl {
                                session: session.clone(),
                                label: item.button.title().to_string(),
                                detail: String::new(),
                                kind: ControlKind::Native,
                            },
                        );
                        self.ivars().buttons.borrow_mut().push(ButtonHover {
                            button: item.button,
                            background: item.background,
                            base_alpha: item.base_alpha,
                            radio: item.radio,
                            caption: item.caption,
                            symbol: None,
                            focusable_when_disabled: item.submit,
                            gutter: None,
                        });
                    }
                } else {
                    document.addSubview(&label);
                }
                let mut message_centers: Vec<_> = earlier_labels
                    .iter()
                    .skip(1)
                    .map(|label| SESSION_HEADER_HEIGHT + label.frame().origin.y + 9.0)
                    .collect();
                if !is_question && !earlier_labels.is_empty() {
                    message_centers.push(SESSION_HEADER_HEIGHT + message_top + 9.0);
                }
                for label in earlier_labels {
                    document.addSubview(&label);
                }
                let request_center =
                    is_question.then_some(SESSION_HEADER_HEIGHT + message_top + 2.0 + 6.5);
                let contour = robot.bottom_contour_in(&row);
                let start = row.bounds().size.height - contour.y;
                let thread = SessionThread::new(
                    self.mtm(),
                    row.bounds(),
                    start,
                    request_center,
                    message_centers,
                    session.activity == Activity::Working,
                );
                row.addSubview_positioned_relativeTo(&thread, NSWindowOrderingMode::Below, None);
                row.ivars()
                    .thread
                    .set(thread)
                    .expect("thread initialized once");
                let first_top = if earlier_height > 0.0 {
                    MESSAGE_TOP_GAP
                } else {
                    message_top
                };
                let gutter_frame = rect(
                    SESSION_TEXT_LEFT - 17.0,
                    first_top,
                    16.0,
                    first_message_height,
                );
                let background = hover_background(self.mtm(), gutter_frame, 8.0);
                document.addSubview(&background);
                let previous = self.action_button(
                    &document,
                    "",
                    sel!(previousMessage:),
                    &session.key(),
                    gutter_frame,
                    true,
                );
                // Keep the full gutter clickable while its decoration is hidden.
                // SAFETY: This decoration-only subclass uses NSView's initializer.
                let decoration: Retained<StatusContentView> = unsafe {
                    msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 16.0, first_message_height)]
                };
                if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    ns_string!("chevron.up"),
                    Some(ns_string!("Previous message")),
                ) {
                    image.setTemplate(true);
                    let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                    icon.setFrame(rect(3.0, (first_message_height - 12.0) / 2.0, 10.0, 12.0));
                    icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
                    decoration.addSubview(&icon);
                }
                decoration.setHidden(true);
                previous.addSubview(&decoration);
                self.ivars().buttons.borrow_mut().push(ButtonHover {
                    button: previous.clone(),
                    background,
                    base_alpha: 0.0,
                    radio: None,
                    caption: None,
                    symbol: None,
                    focusable_when_disabled: false,
                    gutter: Some(GutterHover {
                        message: rect(
                            if earlier_height > 0.0 {
                                SESSION_TEXT_LEFT
                            } else {
                                content_x
                            },
                            first_top,
                            if earlier_height > 0.0 {
                                self.detail_width()
                            } else {
                                content_width
                            },
                            first_message_height,
                        ),
                        icon: decoration,
                    }),
                });
                previous.setEnabled(!history.loading && !history.exhausted);
                let tooltip = if let Some(error) = &history.error {
                    format!("Could not load history: {error}. Click to retry.")
                } else if history.loading {
                    "Loading previous message…".into()
                } else if history.exhausted {
                    "No earlier agent messages".into()
                } else {
                    "Show previous agent message".into()
                };
                previous.setToolTip(Some(&NSString::from_str(&tooltip)));
                if let Some(key) = session.approval_key() {
                    for (index, control) in controls.into_iter().enumerate() {
                        if let Some((context, _)) = &action_contexts[index] {
                            context.setFrameOrigin(NSPoint::new(
                                content_x,
                                message_end
                                    + 8.0
                                    + control_inset
                                    + request_scope_y(&control_frames, &context_heights, index),
                            ));
                            document.addSubview(context);
                        }
                        let id = format!("response:{key}:{index}");
                        let button = self.action_button(
                            &document,
                            &control.label,
                            sel!(respondToRequest:),
                            &id,
                            rect(
                                content_x + control_inset + control_frames[index].origin.x,
                                message_end + 8.0 + control_inset + control_frames[index].origin.y,
                                control_frames[index].size.width,
                                control_frames[index].size.height,
                            ),
                            false,
                        );
                        button.setEnabled(
                            matches!(&control.kind, ControlKind::Open)
                                || (session.authoritative()
                                    && !self.ivars().responding.borrow().contains(&key)
                                    && !matches!(
                                        self.monitor().snapshot().submissions.get(&key),
                                        Some(
                                            crate::actors::monitor::Submission::Sending
                                                | crate::actors::monitor::Submission::Reconciling
                                        )
                                    )),
                        );
                        button.setBezelStyle(NSBezelStyle::Push);
                        button.setFont(Some(&NSFont::systemFontOfSize(12.0)));
                        if let Some(cell) = button.cell() {
                            cell.setWraps(true);
                            cell.setUsesSingleLineMode(false);
                            cell.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
                        }
                        let caption = NSTextField::wrappingLabelWithString(
                            &NSString::from_str(&control.label),
                            self.mtm(),
                        );
                        caption.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, 0.23)));
                        caption.setAlignment(NSTextAlignment::Center);
                        let frame = button.frame();
                        let disclosure_width = if command_control == Some(index)
                            || !control.detail.trim().is_empty()
                        {
                            28.0
                        } else {
                            0.0
                        };
                        let glyph = request_glyph(&control);
                        let icon_width = if glyph.is_some() { 21.0 } else { 0.0 };
                        let available = frame.size.width
                            - 2.0 * BUTTON_HORIZONTAL_INSET
                            - icon_width
                            - disclosure_width;
                        let caption_width = caption
                            .cell()
                            .map(|cell| cell.cellSize().width.ceil())
                            .unwrap_or(available)
                            .min(available)
                            .max(1.0);
                        let caption_height = caption
                            .cell()
                            .map(|cell| {
                                cell.cellSizeForBounds(rect(0.0, 0.0, caption_width, 100000.0))
                                    .height
                                    .ceil()
                            })
                            .unwrap_or(16.0);
                        let group_x = frame.origin.x
                            + (frame.size.width - disclosure_width - caption_width - icon_width)
                                / 2.0;
                        caption.setFrame(rect(
                            group_x + icon_width,
                            frame.origin.y + (frame.size.height - caption_height) / 2.0,
                            caption_width,
                            caption_height,
                        ));
                        let symbol = glyph.map(|glyph| {
                            visual::DecisionIcon::new(
                                self.mtm(),
                                rect(
                                    group_x,
                                    frame.origin.y + (frame.size.height - 14.0) / 2.0,
                                    14.0,
                                    14.0,
                                ),
                                caption.clone(),
                                glyph,
                            )
                        });
                        if let Some(symbol) = &symbol {
                            document.addSubview(symbol);
                        }
                        button.setTransparent(true);
                        document.addSubview_positioned_relativeTo(
                            &caption,
                            NSWindowOrderingMode::Below,
                            Some(&button),
                        );
                        if let Some(hover) = self.ivars().buttons.borrow_mut().last_mut() {
                            hover.caption = Some(caption);
                            hover.symbol = symbol;
                            if let Some(surface) = hover.background.downcast_ref::<GlassBox>() {
                                surface.ivars().kind.set(5);
                            }
                            hover.base_alpha = 1.0;
                            hover
                                .background
                                .setCornerRadius(if control.detail.is_empty() {
                                    12.0
                                } else {
                                    16.0
                                });
                            if index == 0
                                && matches!(control.label.as_str(), "Allow once" | "Allow")
                                && let Some(surface) = hover.background.downcast_ref::<GlassBox>()
                            {
                                surface.ivars().kind.set(4);
                                let bounds = surface.frame();
                                surface.setFrame(rect(
                                    bounds.origin.x - 4.0,
                                    bounds.origin.y - 4.0,
                                    bounds.size.width + 8.0,
                                    bounds.size.height + 8.0,
                                ));
                                surface.setCornerRadius(12.0);
                            }
                        }
                        if command_control == Some(index) || !control.detail.trim().is_empty() {
                            let section = if command_control == Some(index) {
                                0
                            } else {
                                index + 1
                            };
                            let primary = index == 0
                                && matches!(control.label.as_str(), "Allow once" | "Allow");
                            self.action_disclosure(&document, session, frame, section, primary);
                        }
                        button.setToolTip(Some(&NSString::from_str(
                            &if control.detail.is_empty() {
                                control.label.clone()
                            } else {
                                format!("{}\n{}", control.label, control.detail)
                            },
                        )));
                        self.ivars()
                            .request_controls
                            .borrow_mut()
                            .insert(id, control);
                    }
                }
                clip.setDocumentView(Some(&document));
                row.addSubview(&clip);
                detail_clip = Some(clip);
            }
            list.addSubview(&row);
            layout.push(LayoutRow {
                view: row.clone().into_super(),
                row: Some(row),
                id: Some(session.key()),
                gap: 0.0,
                base_height: SESSION_HEADER_HEIGHT,
                detail_height,
                detail: detail_clip,
                arrow: arrow_icon,
                robot,
            });
            y += height;
        }
        drop(reveal);
        drop(expanded);
        if self
            .ivars()
            .keyboard_focus
            .borrow()
            .as_ref()
            .is_some_and(|id| !self.keyboard_targets(&layout).contains(id))
        {
            // A removed answer control must not transfer focus to its session.
            self.ivars().keyboard_focus.borrow_mut().take();
        }
        *self.ivars().layout.borrow_mut() = layout;
        self.layout_rows();
        if self.ivars().menu_open.get() {
            for panel in self.ivars().question_panels.borrow().values() {
                panel.resume_editing();
            }
            for composer in &composers {
                composer.resume();
            }
        }
        let active: HashSet<_> = self
            .ivars()
            .all
            .borrow()
            .iter()
            .filter_map(Session::approval_key)
            .collect();
        self.ivars()
            .question_panels
            .borrow_mut()
            .retain(|key, _| active.contains(key));
        let live: HashSet<_> = self.ivars().all.borrow().iter().map(Session::key).collect();
        self.ivars()
            .composers
            .borrow_mut()
            .retain(|key, _| live.contains(key));
        self.advance_robot_eyes();
    }

    fn sync_header_extension(&self) {
        let state = self.ivars();
        if !state.menu_open.get() || self.is_detached() {
            return;
        }
        let Some(root) = state.root.get() else {
            return;
        };
        let Some(content) = root.window().and_then(|window| window.contentView()) else {
            return;
        };
        let frame = content.convertRect_fromView(root.bounds(), Some(root));
        let bounds = content.bounds();
        let gap = if content.isFlipped() {
            rect(
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width,
                (frame.origin.y - bounds.origin.y).max(0.0),
            )
        } else {
            let top = frame.origin.y + frame.size.height;
            rect(
                bounds.origin.x,
                top,
                bounds.size.width,
                (bounds.origin.y + bounds.size.height - top).max(0.0),
            )
        };
        let Some(backdrop) = state.backdrop.get() else {
            return;
        };
        let extension = state
            .header_extension
            .get_or_init(|| backdrop.sibling(self.mtm(), gap));
        // The native tracking window's content view can retain Aqua while its
        // menu item uses Dark Aqua. Match the item, not that unrelated parent.
        let appearance = root.effectiveAppearance();
        if extension.effectiveAppearance() != appearance {
            extension.setAppearance(Some(&appearance));
            extension.setNeedsDisplay(true);
        }
        extension.setHidden(gap.size.height <= 0.0);
        // SAFETY: All view hierarchy access is on the main thread.
        if !unsafe { extension.superview() }.is_some_and(|parent| std::ptr::eq(&*parent, &*content))
        {
            content.addSubview(extension);
        }
        if extension.frame() != gap {
            extension.setFrame(gap);
        }
        self.sync_mesh_slice(extension);
    }

    fn sync_footer_extension(&self) {
        let state = self.ivars();
        if !state.menu_open.get() || self.is_detached() {
            return;
        }
        // Settings has no quota footer, but native NSMenu padding still needs
        // the same continuous mesh below the custom item.
        let anchor: &NSView = if state.settings_active.get() {
            let Some(root) = state.root.get() else {
                return;
            };
            root
        } else {
            let Some(footer) = state.footer.get() else {
                return;
            };
            footer
        };
        let Some(content) = anchor.window().and_then(|window| window.contentView()) else {
            return;
        };
        let frame = content.convertRect_fromView(anchor.bounds(), Some(anchor));
        let gap = footer_gap(content.bounds(), frame, content.isFlipped());
        let Some(backdrop) = state.backdrop.get() else {
            return;
        };
        let extension = state
            .footer_extension
            .get_or_init(|| backdrop.sibling(self.mtm(), gap));
        let appearance = anchor.effectiveAppearance();
        if extension.effectiveAppearance() != appearance {
            extension.setAppearance(Some(&appearance));
            extension.setNeedsDisplay(true);
        }
        extension.setHidden(gap.size.height <= 0.0);
        // SAFETY: All hierarchy access is confined to the main thread.
        if !unsafe { extension.superview() }.is_some_and(|parent| std::ptr::eq(&*parent, &*content))
        {
            content.addSubview(extension);
        }
        if extension.frame() != gap {
            extension.setFrame(gap);
        }
        self.sync_mesh_slice(extension);
    }

    fn sync_mesh_slice(&self, view: &visual::MeshView) {
        let state = self.ivars();
        let (Some(root), Some(scroll)) = (state.root.get(), state.scroll.get()) else {
            return;
        };
        // SAFETY: The slice is already attached to the same native menu window.
        if unsafe { view.superview() }.is_none() {
            return;
        }
        let source = root.convertRect_fromView(view.bounds(), Some(view));
        let settings = state
            .settings_active
            .get()
            .then(|| state.settings_shell.get())
            .flatten();
        let scroll: &NSScrollView = settings.map_or(&**scroll, |s| &*s.scroll);
        let offset = scroll.contentView().bounds().origin.y + root.bounds().size.height
            - source.origin.y
            - source.size.height;
        let document = settings.map_or(
            state.content_height.get() + state.footer_height.get(),
            |s| s.natural_height(),
        );
        let document = document.max(scroll.contentView().bounds().size.height);
        view.set_brand_wave(!state.settings_active.get());
        view.sync(source.origin.x, offset, document, self.menu_width());
    }

    fn sync_backdrop(&self) {
        self.sync_pin_position();
        let state = self.ivars();
        let (Some(root), Some(scroll), Some(backdrop)) =
            (state.root.get(), state.scroll.get(), state.backdrop.get())
        else {
            return;
        };
        if backdrop.frame() != root.bounds() {
            backdrop.setFrame(root.bounds());
        }
        let settings = state
            .settings_active
            .get()
            .then(|| state.settings_shell.get())
            .flatten();
        let active_scroll: &NSScrollView = settings.map_or(&**scroll, |s| &*s.scroll);
        let document = settings.map_or(
            state.content_height.get() + state.footer_height.get(),
            |s| s.natural_height(),
        );
        let document = document.max(active_scroll.contentView().bounds().size.height);
        backdrop.set_brand_wave(!state.settings_active.get());
        backdrop.sync(
            0.0,
            active_scroll.contentView().bounds().origin.y,
            document,
            self.menu_width(),
        );
        if let Some(slice) = state.header_extension.get() {
            self.sync_mesh_slice(slice);
        }
        if let Some(slice) = state.footer_extension.get() {
            self.sync_mesh_slice(slice);
        }
    }

    fn sync_pin_position(&self) {
        let state = self.ivars();
        let (Some(root), Some(scroll), Some(pin), Some(background)) = (
            state.root.get(),
            state.scroll.get(),
            state.pin_button.get(),
            state.pin_background.get(),
        ) else {
            return;
        };
        // The scroll view has an 11-point horizontal gutter. Keep the pin in
        // the root so that gutter cannot offset or clip its circular hover;
        // move it with the header's scroll offset rather than fixing it onscreen.
        let offset = scroll.contentView().bounds().origin.y;
        let frame = rect(
            1.0,
            root.bounds().size.height - ICON_BUTTON_SIZE - 1.0 + offset,
            ICON_BUTTON_SIZE,
            ICON_BUTTON_SIZE,
        );
        let hidden =
            self.is_detached() || state.settings_active.get() || offset >= ICON_BUTTON_SIZE + 1.0;
        if pin.frame() != frame || pin.isHidden() != hidden {
            pin.setFrame(frame);
            background.setFrame(frame);
            pin.setHidden(hidden);
            if hidden {
                background.setHidden(true);
            }
            state.last_pointer.set(None);
        }
    }

    fn render_footer(&self) {
        let state = self.ivars();
        if state.settings_active.get() {
            return;
        }
        let mut resets = self.monitor().snapshot().usage_resets.clone();
        if let Some(generation) = state.usage_resets_pending.get() {
            if resets.generation >= generation {
                state.usage_resets_pending.set(None);
            } else {
                resets.phase = crate::domain::usage::ResetPhase::Loading;
                resets.credits = None;
                resets.available_count = None;
                resets.error = None;
            }
        }
        let expanded = state.usage_resets_expanded.get();
        let local_error = state.usage_resets_error.borrow().clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let query = state.filter_query.borrow().clone();
        let windows = state
            .usage
            .borrow()
            .as_ref()
            .map(|usage| usage.windows.clone())
            .unwrap_or_default();
        let mut visible_resets = resets.clone();
        visible_resets.generation = 0;
        let key = FooterRenderKey {
            remaining: windows.iter().map(|window| window.remaining(now)).collect(),
            windows,
            resets: visible_resets,
            expanded,
            error: local_error.clone(),
            query: query.clone(),
            width: self.menu_width(),
        };
        if state.footer_render_key.borrow().as_ref() == Some(&key) {
            return;
        }
        *state.footer_render_key.borrow_mut() = Some(key);
        state.buttons.borrow_mut().retain(|item| {
            item.button
                .identifier()
                .is_none_or(|id| id.to_string() != "usage-resets")
        });
        let loading = expanded
            && local_error.is_none()
            && matches!(
                resets.phase,
                crate::domain::usage::ResetPhase::Idle | crate::domain::usage::ResetPhase::Loading
            );
        let reset_rows = resets.credits.as_ref().map(Vec::len).unwrap_or(0);
        let reset_height = if !expanded {
            0.0
        } else if loading {
            78.0
        } else if local_error.is_some() || resets.phase == crate::domain::usage::ResetPhase::Error {
            76.0
        } else {
            34.0 + reset_rows.clamp(1, 6) as f64 * 24.0
        };
        let reset_scroll_origin = state
            .usage_resets_scroll
            .borrow()
            .as_ref()
            .map(|scroll| scroll.contentView().bounds().origin);
        state.usage_resets_scroll.borrow_mut().take();
        let usage = state.usage.borrow();
        let filter_rows = usize::from(!query.is_empty());
        let Some(footer) = state.footer.get() else {
            return;
        };
        let windows = usage
            .as_ref()
            .map(|usage| usage.windows.as_slice())
            .unwrap_or_default();
        let height =
            windows.len().max(1) as f64 * 40.0 + 20.0 + filter_rows as f64 * 20.0 + reset_height;
        let changed = state.footer_height.replace(height) != height;
        let children = footer.subviews();
        for index in 0..children.count() {
            children.objectAtIndex(index).removeFromSuperview();
        }
        footer.setFrame(rect(0.0, 0.0, self.menu_width(), height));
        if !query.is_empty() {
            self.label(
                footer,
                &format!("Filter: {query}"),
                rect(26.0, height - 22.0, self.menu_width() - 52.0, 18.0),
                11.0,
                0.0,
                true,
            );
        }
        let quota_width = self.menu_width().min(MENU_WIDTH);
        let quota = NSView::initWithFrame(
            NSView::alloc(self.mtm()),
            rect(
                (self.menu_width() - quota_width) / 2.0,
                0.0,
                quota_width,
                height,
            ),
        );
        footer.addSubview(&quota);
        for index in 0..windows.len().max(1) {
            let window = windows.get(index);
            let remaining = window.and_then(|window| window.remaining(now));
            let percentage = remaining
                .map(|value| format!("{value}%"))
                .unwrap_or_else(|| "—".to_owned());
            let reset = window
                .and_then(|window| window.resets_at)
                .map(|reset| {
                    let formatter = NSDateFormatter::new();
                    formatter.setDateFormat(Some(
                        if window.is_some_and(|window| window.minutes >= 1440) {
                            ns_string!("MMM d")
                        } else {
                            ns_string!("HH:mm")
                        },
                    ));
                    formatter
                        .stringFromDate(&NSDate::dateWithTimeIntervalSince1970(reset as f64))
                        .to_string()
                })
                .unwrap_or_else(|| "—".to_owned());
            let name_text = window
                .map(|window| format!("Codex {}", window.label()))
                .unwrap_or_else(|| "Codex usage".to_owned());
            let y = height
                - 16.0
                - filter_rows as f64 * 20.0
                - 36.0
                - index as f64 * 40.0
                - reset_height;
            let panel_height = 36.0 + if index == 0 { reset_height } else { 0.0 };
            let panel = glass_box(
                self.mtm(),
                rect(16.0, y, quota_width - 32.0, panel_height),
                3,
            );
            panel.setBoxType(NSBoxType::Custom);
            panel.setTitlePosition(NSTitlePosition::NoTitle);
            panel.setCornerRadius(17.0);
            quota.addSubview(&panel);
            let name = self.label(
                &quota,
                &name_text,
                rect(29.0, y + 8.0, 150.0, 20.0),
                11.0,
                0.0,
                false,
            );
            name.setTextColor(Some(&primary_text_color()));
            name.sizeToFit();
            let name_width = name.frame().size.width.ceil();
            name.setFrame(rect(29.0, y + 8.0, name_width, 20.0));
            let date = self.label(
                &quota,
                &reset,
                rect(0.0, y + 8.0, 80.0, 20.0),
                11.0,
                0.0,
                true,
            );
            date.sizeToFit();
            let date_width = date.frame().size.width.ceil().max(15.0);
            let date_x = quota_width - 29.0 - date_width - if index == 0 { 22.0 } else { 0.0 };
            date.setFrame(rect(date_x, y + 8.0, date_width, 20.0));
            let percent = self.label(
                &quota,
                &percentage,
                rect(0.0, y + 8.0, 40.0, 20.0),
                11.0,
                0.0,
                true,
            );
            if remaining.is_some_and(|value| value <= 40) {
                percent.setFont(Some(&NSFont::boldSystemFontOfSize(11.0)));
            }
            percent.sizeToFit();
            let percent_width = percent.frame().size.width.ceil().max(15.0);
            let percent_x = date_x - 24.0 - percent_width;
            percent.setFrame(rect(percent_x, y + 8.0, percent_width, 20.0));
            let track_x = 29.0 + name_width + 12.0;
            let track_width = (percent_x - 12.0 - track_x).max(0.0);
            let track: Retained<UsageTrack> = unsafe {
                msg_send![super(UsageTrack::alloc(self.mtm()).set_ivars(UsageTrackState { remaining: remaining.map(f64::from) })),initWithFrame:rect(track_x,y+14.0,track_width,8.0)]
            };
            quota.addSubview(&track);
            let tooltip = NSString::from_str(&if let (Some(window), Some(usage)) =
                (window, usage.as_ref())
            {
                format!(
                    "Codex · {} quota remaining\nReset: {}\nLast reported: {}{}",
                    window.label(),
                    window
                        .resets_at
                        .map(|reset| {
                            let formatter = NSDateFormatter::new();
                            formatter.setDateFormat(Some(ns_string!("yyyy-MM-dd HH:mm")));
                            formatter
                                .stringFromDate(&NSDate::dateWithTimeIntervalSince1970(
                                    reset as f64,
                                ))
                                .to_string()
                        })
                        .unwrap_or_else(|| "Unknown".to_owned()),
                    usage.observed_at,
                    if remaining.is_none() {
                        "\nWaiting for Codex to report the new balance."
                    } else {
                        ""
                    }
                )
            } else {
                "Usage will appear after Codex reports its account limits.".to_owned()
            });
            for label in [&name, &percent, &date] {
                center_label(label, y + 18.0);
                label.setTextColor(Some(&visual::quota_value_color()));
                label.setToolTip(Some(&tooltip));
            }
            if let Some(value) = remaining.filter(|value| *value <= 50) {
                let rgb = crate::domain::layout::quota_rgb(f64::from(value)).map(f64::from);
                percent.setTextColor(Some(&appearance_color(rgb, rgb, 1.0)));
            }
            name.setTextColor(Some(&primary_text_color()));
            track.setToolTip(Some(&tooltip));
            if index == 0 {
                let frame = rect(quota_width - 45.0, y + 6.0, 24.0, 24.0);
                let background = NSBox::initWithFrame(NSBox::alloc(self.mtm()), frame);
                background.setBoxType(NSBoxType::Custom);
                background.setTitlePosition(NSTitlePosition::NoTitle);
                background.setBorderWidth(0.0);
                background.setCornerRadius(12.0);
                background.setFillColor(&appearance_color(
                    [196.0, 184.0, 214.0],
                    [33.0, 26.0, 46.0],
                    1.0,
                ));
                paint_hover(&background, if expanded { 1.0 } else { 0.0 });
                quota.addSubview(&background);
                let button = self.action_button(
                    &quota,
                    "",
                    sel!(toggleUsageResets:),
                    "usage-resets",
                    frame,
                    true,
                );
                button.setTransparent(false);
                button.setImage(
                    NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        ns_string!("ellipsis"),
                        Some(ns_string!("Usage limit resets")),
                    )
                    .as_deref(),
                );
                button.setContentTintColor(Some(&secondary_text_color()));
                button.setToolTip(Some(ns_string!("Show / hide usage limit resets")));
                self.ivars().buttons.borrow_mut().push(ButtonHover {
                    button,
                    background,
                    base_alpha: if expanded { 1.0 } else { 0.0 },
                    radio: None,
                    caption: None,
                    symbol: None,
                    focusable_when_disabled: false,
                    gutter: None,
                });
                if expanded {
                    let top = y + 36.0 + reset_height;
                    if loading {
                        self.label(
                            &quota,
                            "Loading usage limit resets…",
                            rect(78.0, top - 46.0, quota_width - 115.0, 18.0),
                            11.0,
                            0.0,
                            true,
                        );
                        let spinner = state.usage_resets_spinner.get_or_init(|| {
                            let spinner = NSProgressIndicator::initWithFrame(
                                NSProgressIndicator::alloc(self.mtm()),
                                rect(0.0, 0.0, 16.0, 16.0),
                            );
                            spinner.setStyle(NSProgressIndicatorStyle::Spinning);
                            spinner.setIndeterminate(true);
                            spinner
                        });
                        spinner.removeFromSuperview();
                        spinner.setFrame(rect(53.0, top - 44.0, 16.0, 16.0));
                        quota.addSubview(spinner);
                        if !state.usage_resets_spinning.replace(true) {
                            // SAFETY: Main-thread AppKit animation, no sender is required.
                            unsafe {
                                spinner.startAnimation(None);
                            }
                        }
                    } else if let Some(error) = local_error.as_deref().or(resets.error.as_deref()) {
                        let label = self.label(
                            &quota,
                            error,
                            rect(29.0, top - 38.0, quota_width - 58.0, 30.0),
                            10.0,
                            0.0,
                            true,
                        );
                        label.setMaximumNumberOfLines(2);
                        if let Some(cell) = label.cell() {
                            cell.setWraps(true);
                            cell.setScrollable(false);
                            cell.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
                        }
                        label.setToolTip(Some(&NSString::from_str(error)));
                    } else {
                        let heading = self.label(
                            &quota,
                            "Usage limit resets",
                            rect(29.0, top - 27.0, 150.0, 16.0),
                            11.0,
                            0.3,
                            false,
                        );
                        heading.sizeToFit();
                        if let Some(count) = resets.available_count {
                            self.label(
                                &quota,
                                &format!("{count} available"),
                                rect(35.0 + heading.frame().size.width, top - 27.0, 90.0, 16.0),
                                10.0,
                                0.0,
                                true,
                            );
                        }
                        let expires = self.label(
                            &quota,
                            "Expires",
                            rect(quota_width - 103.0, top - 27.0, 74.0, 16.0),
                            11.0,
                            0.3,
                            false,
                        );
                        expires.setAlignment(NSTextAlignment::Right);
                        if let Some(credits) =
                            resets.credits.as_ref().filter(|rows| !rows.is_empty())
                        {
                            let formatter = NSDateFormatter::new();
                            formatter.setDateFormat(Some(ns_string!("d MMM yyyy 'at' HH:mm")));
                            let document_height = credits.len() as f64 * 24.0;
                            let viewport_height = credits.len().min(6) as f64 * 24.0;
                            let scroll = NSScrollView::initWithFrame(
                                NSScrollView::alloc(self.mtm()),
                                rect(29.0, y + 40.0, quota_width - 58.0, viewport_height),
                            );
                            scroll.setDrawsBackground(false);
                            scroll.setHasVerticalScroller(credits.len() > 6);
                            scroll.setAutohidesScrollers(true);
                            scroll.setBorderType(NSBorderType::NoBorder);
                            let document = NSView::initWithFrame(
                                NSView::alloc(self.mtm()),
                                rect(0.0, 0.0, quota_width - 58.0, document_height),
                            );
                            for (row, credit) in credits.iter().enumerate() {
                                let ry = document_height - 18.0 - row as f64 * 24.0;
                                self.label(
                                    &document,
                                    &credit.title,
                                    rect(0.0, ry, 130.0, 16.0),
                                    11.0,
                                    0.0,
                                    false,
                                );
                                let text = credit
                                    .expires_at
                                    .map(|value| {
                                        formatter
                                            .stringFromDate(&NSDate::dateWithTimeIntervalSince1970(
                                                value as f64,
                                            ))
                                            .to_string()
                                    })
                                    .unwrap_or_else(|| "No expiration".into());
                                let date = self.label(
                                    &document,
                                    &text,
                                    rect(quota_width - 268.0, ry, 210.0, 16.0),
                                    10.0,
                                    0.0,
                                    true,
                                );
                                date.setAlignment(NSTextAlignment::Right);
                            }
                            scroll.setDocumentView(Some(&document));
                            let origin = reset_scroll_origin.unwrap_or_else(|| {
                                NSPoint::new(0.0, (document_height - viewport_height).max(0.0))
                            });
                            scroll.contentView().scrollToPoint(origin);
                            scroll.reflectScrolledClipView(&scroll.contentView());
                            quota.addSubview(&scroll);
                            *state.usage_resets_scroll.borrow_mut() = Some(scroll);
                        } else {
                            self.label(
                                &quota,
                                if resets.available_count == Some(0) {
                                    "No resets available."
                                } else {
                                    "Expiration details unavailable."
                                },
                                rect(29.0, top - 51.0, quota_width - 58.0, 16.0),
                                10.0,
                                0.0,
                                true,
                            );
                        }
                    }
                }
            }
        }
        if !loading
            && state.usage_resets_spinning.replace(false)
            && let Some(spinner) = state.usage_resets_spinner.get()
        {
            // SAFETY: Main-thread AppKit animation, no sender is required.
            unsafe {
                spinner.stopAnimation(None);
            }
        }
        if changed {
            self.layout_rows();
        }
    }

    fn constrain_menu_to_work_area(&self) {
        if !self.ivars().menu_open.get() || self.is_detached() {
            return;
        }
        let Some(window) = self.ivars().root.get().and_then(|root| root.window()) else {
            return;
        };
        let Some(screen) = window.screen() else {
            return;
        };
        // Native status menus stay on the screen but may overlap a side Dock.
        // Constrain the actual tracking window after AppKit attaches/resizes it;
        // retain NSMenu's normal tracking, dismissal and non-activating behavior.
        let frame = window.frame();
        let visible = screen.visibleFrame();
        let mut origin = menu_work_area_origin(frame, visible);
        // Popup placement can add a native edge offset. Align the window's
        // actual top to the menu bar, not just the custom item's content edge.
        origin.y = visible.origin.y + visible.size.height - frame.size.height;
        if frame.origin != origin {
            window.setFrameOrigin(origin);
        }
    }

    fn resize_content(&self, content_height: f64) -> f64 {
        let state = self.ivars();
        let settings = state.settings_active.get();
        if !settings {
            state.content_height.set(content_height);
        }
        let content_height = if settings {
            state
                .settings_shell
                .get()
                .map_or(content_height, |s| s.natural_height())
        } else {
            content_height
        };
        let screen = state
            .status
            .get()
            .and_then(|item| item.button(self.mtm()))
            .and_then(|button| button.window())
            .and_then(|window| window.screen())
            .or_else(|| NSScreen::mainScreen(self.mtm()));
        let maximum = screen
            .map(|screen| {
                (screen.frame().size.height * 0.8 - 20.0)
                    .min(screen.visibleFrame().size.height - 24.0)
            })
            .unwrap_or(640.0)
            .max(100.0);
        let footer_height = if settings {
            0.0
        } else {
            state.footer_height.get()
        };
        let natural_height = (content_height + footer_height)
            .max(64.0 + footer_height)
            .min(maximum);
        let height = state
            .detached_window
            .borrow()
            .as_ref()
            .map_or(natural_height, |w| {
                w.contentRectForFrameRect(w.frame()).size.height
            });
        let viewport = height;
        NSAnimationContext::beginGrouping();
        let context = NSAnimationContext::currentContext();
        context.setDuration(0.0);
        context.setAllowsImplicitAnimation(false);
        let mut resized = false;
        if let Some(root) = state.root.get() {
            let size = NSSize::new(self.menu_width(), height);
            if root.frame().size != size {
                root.setFrameSize(size);
                resized = true;
            }
        }
        if let Some(scroll) = state.scroll.get() {
            scroll.setFrame(rect(11.0, 0.0, self.list_width(), viewport));
        }
        if let Some(toolbar) = state.toolbar.get() {
            toolbar.setFrame(rect(0.0, 0.0, self.list_width(), BRAND_HEIGHT));
        }
        if let Some(caption) = state.caption.get() {
            caption.setFrame(rect(49.0, height - BRAND_HEIGHT - 40.0, 360.0, 18.0));
        }
        if settings && let Some(shell) = state.settings_shell.get() {
            shell.resize(NSSize::new(self.menu_width(), height));
        }
        self.sync_backdrop();
        NSAnimationContext::endGrouping();
        if resized
            && state.menu_open.get()
            && !self.is_detached()
            && let Some(menu) = state.menu.get()
        {
            menu.update();
            self.sync_header_extension();
            self.sync_footer_extension();
        }
        self.sync_window_width();
        viewport
    }

    fn refresh(&self) {
        let _ = self
            .monitor()
            .send(crate::actors::monitor::Command::Refresh);
    }
    fn receive_sessions(&self) {
        self.receive_sessions_with_mouse(self.mouse_tracking_active());
    }

    fn mouse_tracking_active(&self) -> bool {
        NSApplication::sharedApplication(self.mtm()).activationPolicy()
            != NSApplicationActivationPolicy::Prohibited
            && NSEvent::pressedMouseButtons() & 1 != 0
    }

    fn receive_sessions_with_mouse(&self, mouse_pressed: bool) {
        let state = self.ivars();
        if mouse_pressed && state.root.get().is_some() && state.revision.get() != u64::MAX {
            if let Some(owner) = state.popup_owner.get().and_then(|owner| owner.load()) {
                owner.receive_sessions_with_mouse(true);
            }
            // Leave the revision unread: the first tick after release applies
            // the newest snapshot, coalescing all intermediate revisions.
            return;
        }
        let snapshot = self.monitor().snapshot();
        if state.revision.replace(snapshot.revision) == snapshot.revision {
            return;
        }
        if state.root.get().is_none() {
            // No popup UI exists while closed. Services, native notifications,
            // standalone forms and the status item remain resident.
            *state.all.borrow_mut() = snapshot.sessions.clone();
            *state.applications.borrow_mut() = snapshot.preferences.applications.clone();
            state.composer_drafts.borrow_mut().retain(|key, _| {
                snapshot
                    .sessions
                    .iter()
                    .any(|session| session.key() == *key)
            });
            state.question_drafts.borrow_mut().retain(|key, _| {
                snapshot
                    .sessions
                    .iter()
                    .any(|session| session.approval_key().as_ref() == Some(key))
            });
            state.message_history.borrow_mut().retain(|key, _| {
                snapshot
                    .sessions
                    .iter()
                    .any(|session| session.key() == *key)
            });
            for form in state.forms.borrow().iter() {
                form.reconcile(&snapshot);
            }
            if shortcut_needs_sync(
                &state.shortcut_applied.borrow(),
                &snapshot.preferences.shortcut,
                state.menu_open.get(),
                state.shortcut_monitor.borrow().is_some(),
            ) {
                self.install_shortcut();
            }
            let link = state
                .telegram_auto_open
                .borrow_mut()
                .take_link(snapshot.telegram.link.as_deref());
            if let Some(link) = link {
                open_telegram_bot(&link);
            }
            self.update_badge(snapshot.pending_count());
            if snapshot.stopping {
                crate::macos::native::stop_application(self.mtm());
            }
            return;
        }
        if let Some(release) = state.release.get() {
            release.sync(&snapshot.versions);
        }
        if let Some(overview) = state.overview.get() {
            let mut frame = overview.frame();
            frame.origin.y = 28.0;
            overview.setFrame(frame);
        }
        if let Some(shell) = state.settings_shell.get() {
            shell.sync_versions(&snapshot.versions);
        }
        *state.usage.borrow_mut() = snapshot.usage.clone();
        self.render_footer();
        for form in state.forms.borrow().iter() {
            form.reconcile(&snapshot);
        }
        {
            let mut histories = state.message_history.borrow_mut();
            for (key, history) in histories.iter_mut() {
                if let Some(value) = snapshot
                    .history
                    .get(&crate::actors::monitor::HistoryKey::window(key.clone()))
                    && value.generation == history.ticket
                {
                    if history.messages != value.messages
                        || history.cursor != value.cursor
                        || history.loading != value.loading
                        || history.exhausted != value.exhausted
                        || history.error != value.error
                    {
                        state.filter_dirty.set(true);
                    }
                    history.messages = value.messages.clone();
                    history.cursor = value.cursor.clone();
                    history.loading = value.loading;
                    history.exhausted = value.exhausted;
                    history.error = value.error.clone();
                }
            }
        }
        state.responding.borrow_mut().retain(|key| {
            matches!(
                snapshot.submissions.get(key),
                Some(
                    crate::actors::monitor::Submission::Sending
                        | crate::actors::monitor::Submission::Reconciling
                )
            )
        });
        let status = RowStatus {
            submissions: snapshot.submissions.clone(),
            messages: snapshot.message_sends.clone(),
            finished: snapshot.unseen_finished.keys().cloned().collect(),
        };
        if state.row_status.borrow().as_ref() != Some(&status) {
            *state.row_status.borrow_mut() = Some(status);
            state.filter_dirty.set(true);
        }
        state.responding.borrow_mut().extend(
            snapshot
                .submissions
                .iter()
                .filter(|(_, phase)| {
                    matches!(
                        phase,
                        crate::actors::monitor::Submission::Sending
                            | crate::actors::monitor::Submission::Reconciling
                    )
                })
                .map(|(key, _)| key.clone()),
        );
        let sessions = snapshot.sessions.clone();
        let preferences = &snapshot.preferences;
        let applications_changed = *state.applications.borrow() != preferences.applications;
        *state.applications.borrow_mut() = preferences.applications.clone();
        if applications_changed {
            self.refresh_application_settings();
        }
        if shortcut_needs_sync(
            &state.shortcut_applied.borrow(),
            &preferences.shortcut,
            state.menu_open.get(),
            state.shortcut_monitor.borrow().is_some(),
        ) {
            self.install_shortcut();
        }
        self.refresh_settings();
        self.refresh_telegram_settings();
        self.refresh_integration_settings();
        *state.all.borrow_mut() = sessions;
        // Keep the latest state and dirty flag while closed. menuWillOpen:
        // applies them before the menu becomes visible. Native message and
        // approval controls need not be rebuilt for an invisible popup.
        if state.menu_open.get() {
            self.filter();
        }
        if snapshot.stopping {
            crate::macos::native::stop_application(self.mtm());
        }
        self.update_badge(snapshot.pending_count());
        self.sync_action_cursor();
    }

    fn filter(&self) {
        let state = self.ivars();
        if state.settings_active.get() {
            return;
        }
        let visible = state.filtered_sessions();
        let count = visible.len();
        let previous = state.visible.borrow();
        let expanded = state.expand_active_sessions(&visible);
        let changed = state.filter_dirty.replace(false)
            || expanded
            || previous.len() != visible.len()
            || previous.iter().zip(&visible).any(|(a, b)| {
                a.key() != b.key()
                    || a.title != b.title
                    || a.cwd != b.cwd
                    || a.workspace != b.workspace
                    || a.git_branch != b.git_branch
                    || a.model != b.model
                    || a.model_display_name != b.model_display_name
                    || a.reasoning_effort != b.reasoning_effort
                    || a.client != b.client
                    || a.connection != b.connection
                    || a.target != b.target
                    || a.activity != b.activity
                    || a.last_message() != b.last_message()
                    || a.latest_message().map(|message| &message.id)
                        != b.latest_message().map(|message| &message.id)
                    || a.images() != b.images()
                    || a.progress() != b.progress()
                    || a.approval() != b.approval()
                    || a.interaction() != b.interaction()
            });
        drop(previous);
        if changed {
            self.render_rows(&visible);
            *state.visible.borrow_mut() = visible;
        }
        if let Some(overview) = state.overview.get() {
            let snapshot = self.monitor().snapshot();
            let counts = overview_counts(&state.visible.borrow(), |session| {
                snapshot.unseen_finished.contains_key(&session.key())
            });
            let segments: Vec<_> = ["waiting for you", "working", "finished", "idle"]
                .iter()
                .enumerate()
                .filter(|(index, _)| counts[*index] > 0)
                .map(|(index, label)| (index, format!("{} {label}", counts[index])))
                .collect();
            let summary = segments
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join(" · ");
            let value = NSMutableAttributedString::initWithString(
                NSMutableAttributedString::alloc(),
                &NSString::from_str(&summary),
            );
            let colors = [
                request_color(),
                working_status_color(),
                appearance_color([62.0, 109.0, 185.0], [140.0, 216.0, 255.0], 1.0),
                visual::neutral_robot_color(),
            ];
            // SAFETY: Each range is measured in UTF-16 in this exact summary.
            unsafe {
                value.addAttribute_value_range(
                    NSForegroundColorAttributeName,
                    &secondary_text_color(),
                    NSRange::new(0, summary.encode_utf16().count()),
                );
                let mut offset = 0;
                for (index, segment) in &segments {
                    let length = segment.encode_utf16().count();
                    let color = &colors[*index];
                    value.addAttribute_value_range(
                        NSForegroundColorAttributeName,
                        color,
                        NSRange::new(offset, length),
                    );
                    offset += length + 3;
                }
            }
            if overview.stringValue().to_string() != summary {
                overview.setAttributedStringValue(&value);
            }
            let hint = if counts[4] == 0 {
                summary
            } else {
                format!("{summary}\nStatus unavailable: {}", counts[4])
            };
            overview.setToolTip(Some(&NSString::from_str(&hint)));
        }
        if let Some(caption) = state.caption.get() {
            let text = if count == 0 {
                if state
                    .filter_query
                    .borrow()
                    .split_whitespace()
                    .next()
                    .is_some()
                {
                    "No matching sessions"
                } else if !state.all.borrow().is_empty() {
                    "No visible sessions"
                } else {
                    "No open sessions"
                }
            } else {
                ""
            };
            caption.setStringValue(&NSString::from_str(text));
            caption.setHidden(text.is_empty());
        }
    }
}

pub struct Resident {
    pub monitor: crate::actors::monitor::Handle,
    pub ui: crate::macos::bridge::Sender,
    pub controller: Retained<MenuController>,
    pub notifications: tokio::sync::mpsc::Sender<native::NotificationInput>,
    _actors: crate::app::actor_system::ActorSystem,
    _ipc: tokio::task::JoinHandle<()>,
}
impl Drop for Resident {
    fn drop(&mut self) {
        self.controller.shutdown();
        self._ipc.abort();
    }
}
pub fn start(mtm: MainThreadMarker, show: bool) -> Result<Option<Resident>> {
    let directory = core::home().join("Library/Application Support/AI Agent Monitor");
    fs::create_dir_all(&directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(directory.join("menu.lock"))?;
    // SAFETY: flock acts on this owned file descriptor. The file remains open for
    // the lifetime of the resident controller; the OS releases the lock on exit.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::WouldBlock {
            return Err(error).context("session menu lock");
        }
        if show {
            // SAFETY: This notification has no userInfo and only asks our resident
            // process to show its existing window.
            unsafe {
                NSDistributedNotificationCenter::defaultCenter()
                    .postNotificationName_object_userInfo_deliverImmediately(
                        ns_string!("local.codex.agent-monitor.showSessions"),
                        None,
                        None,
                        true,
                    );
            }
        }
        return Ok(None);
    }
    let (ui, incoming) = tokio::sync::mpsc::channel(128);
    // Standard edit key equivalents dispatch to the field editor through the
    // responder chain, including secure text fields in the settings pane.
    let app = NSApplication::sharedApplication(mtm);
    let main_menu = app.mainMenu().unwrap_or_else(|| NSMenu::new(mtm));
    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Edit"));
    for (title, action, key) in [
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        // SAFETY: Standard AppKit actions, with nil target for responder routing.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        edit.addItem(&item);
    }
    let item = NSMenuItem::new(mtm);
    item.setTitle(ns_string!("Edit"));
    item.setSubmenu(Some(&edit));
    main_menu.addItem(&item);
    app.setMainMenu(Some(&main_menu));
    let actors = crate::app::actor_system::ActorSystem::start(ui.clone())?;
    let notifications = actors.start_notifications(ui.clone());
    let ipc = actors.start_ipc()?;
    let controller =
        MenuController::create(mtm, lock, actors.monitor.clone(), ui.clone(), incoming);
    // Wait for launch completion before an explicit CLI request opens the menu.
    // The regular app launch keeps only its status icon, with no popup window.
    if show {
        // SAFETY: The controller implements showSessions: and is retained below.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &controller,
                sel!(showSessions:),
                Some(NSApplicationDidFinishLaunchingNotification),
                None,
            );
        }
    }
    let _ = actors
        .monitor
        .send(crate::actors::monitor::Command::Platform(
            crate::macos::bridge::PlatformCommand::Setup(false),
        ));
    Ok(Some(Resident {
        monitor: actors.monitor.clone(),
        ui,
        controller,
        notifications,
        _actors: actors,
        _ipc: ipc,
    }))
}

fn menu_work_area_origin(frame: NSRect, visible: NSRect) -> NSPoint {
    const GAP: f64 = 4.0;
    let left = visible.origin.x + GAP;
    let bottom = visible.origin.y + GAP;
    let right = (visible.origin.x + visible.size.width - frame.size.width - GAP).max(left);
    // The visible area's top already ends at the menu bar: no extra gap there.
    let top = (visible.origin.y + visible.size.height - frame.size.height).max(bottom);
    NSPoint::new(
        frame.origin.x.clamp(left, right),
        frame.origin.y.clamp(bottom, top),
    )
}

#[cfg(test)]
#[allow(dead_code)]
#[path = "menu_visual_test.rs"]
pub(crate) mod visual_test;

#[cfg(test)]
mod close_tests {
    #[test]
    fn resize_edges_cover_native_padding_and_exclude_corners() {
        use super::{rect, resize_edge};
        use objc2_foundation::NSPoint;
        let window = rect(-800.0, 200.0, 480.0, 600.0);
        let item = rect(-780.0, 210.0, 460.0, 580.0);
        let left = NSPoint::new(-798.0, 500.0);
        assert_eq!(resize_edge(left, window), Some(true));
        assert_eq!(
            resize_edge(left, item),
            None,
            "native left padding is outside the custom item"
        );
        assert_eq!(
            resize_edge(NSPoint::new(-322.0, 500.0), window),
            Some(false)
        );
        assert_eq!(
            resize_edge(NSPoint::new(-778.0, 500.0), window),
            None,
            "internal item edge is not a window edge"
        );
        for point in [
            NSPoint::new(-798.0, 202.0),
            NSPoint::new(-798.0, 798.0),
            NSPoint::new(-322.0, 202.0),
            NSPoint::new(-322.0, 798.0),
            NSPoint::new(-560.0, 202.0),
        ] {
            assert_eq!(resize_edge(point, window), None);
        }
    }

    #[test]
    fn menu_avoids_side_and_bottom_docks_on_offset_screens() {
        use super::{menu_work_area_origin, rect};
        use objc2_foundation::NSPoint;

        // Right Dock: move left without changing the menu's vertical anchor.
        assert_eq!(
            menu_work_area_origin(
                rect(1000.0, 500.0, 392.0, 350.0),
                rect(0.0, 0.0, 1320.0, 880.0)
            ),
            NSPoint::new(924.0, 500.0),
        );
        // Left Dock on a display left of the primary screen.
        assert_eq!(
            menu_work_area_origin(
                rect(-1440.0, 500.0, 392.0, 350.0),
                rect(-1360.0, 0.0, 1360.0, 880.0)
            ),
            NSPoint::new(-1356.0, 500.0),
        );
        // Bottom Dock: keep the entire menu above its reserved area.
        assert_eq!(
            menu_work_area_origin(
                rect(500.0, 20.0, 392.0, 600.0),
                rect(0.0, 80.0, 1440.0, 800.0)
            ),
            NSPoint::new(500.0, 84.0),
        );
        // An already safe position must remain stable across tracking ticks.
        assert_eq!(
            menu_work_area_origin(
                rect(500.0, 100.0, 392.0, 600.0),
                rect(0.0, 80.0, 1440.0, 800.0)
            ),
            NSPoint::new(500.0, 100.0),
        );
        // Keep the menu flush with the menu bar, while retaining the Dock gap.
        assert_eq!(
            menu_work_area_origin(
                rect(500.0, 534.0, 392.0, 350.0),
                rect(0.0, 0.0, 1440.0, 880.0)
            ),
            NSPoint::new(500.0, 530.0),
        );
    }

    #[test]
    fn opening_an_answer_form_releases_registry_borrows_before_callbacks_and_retention() {
        let forms = std::cell::RefCell::new(vec![1]);
        super::open_owned_form(
            &forms,
            |existing| {
                assert_eq!(existing, &[1]);
                forms.borrow_mut().push(2);
                Some(3)
            },
            |form| {
                assert_eq!(*form, 3);
                forms.borrow_mut().push(4);
            },
        );
        assert_eq!(*forms.borrow(), vec![1, 2, 4, 3]);
        super::open_owned_form(
            &forms,
            |_| None,
            |_| panic!("existing forms are not added twice"),
        );
        assert_eq!(forms.borrow().len(), 4);
    }

    #[test]
    fn delayed_preferences_install_shortcut_and_recording_defers_changes() {
        let desired = Some(crate::domain::shortcut::Shortcut {
            key_code: 1,
            modifiers: 256,
            key_name: "S".into(),
        });
        assert!(super::shortcut_needs_sync(&None, &desired, false, false));
        assert!(!super::shortcut_needs_sync(&None, &desired, true, false));
        assert!(!super::shortcut_needs_sync(&None, &desired, false, true));
        assert!(!super::shortcut_needs_sync(
            &desired, &desired, false, false
        ));
    }
    #[test]
    fn full_monitor_queue_does_not_leave_history_loading() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let monitor = crate::actors::monitor::Handle {
            tx,
            state: tokio::sync::watch::channel(std::sync::Arc::new(
                crate::actors::monitor::State::default(),
            ))
            .1,
        };
        monitor
            .send(crate::actors::monitor::Command::Refresh)
            .unwrap();
        let mut history = super::MessageHistory {
            loading: true,
            ..Default::default()
        };
        history.enqueued(monitor.send(crate::actors::monitor::Command::History {
            key: crate::actors::monitor::HistoryKey::window("one".into()),
            cursor: None,
            generation: 1,
        }));
        assert!(!history.loading);
        assert!(history.error.is_some());
        assert!(!history.exhausted);
    }

    use super::*;

    #[test]
    fn pending_questions_keep_latest_message_and_history_above_the_card() {
        let mut session = session("chat", Some("question"));
        session.set_last_message("Latest message\n\n".into());
        let mut history = MessageHistory::default();
        assert_eq!(
            messages_before_request(&session, &history),
            "Latest message"
        );
        history.messages.push(crate::domain::history::Message {
            images: vec![],
            cursor: "earlier".into(),
            text: "Previous message\n".into(),
        });
        history.anchor = "Latest message\n".into();
        assert_eq!(
            messages_before_request(&session, &history),
            "Previous message\n\nLatest message"
        );
        session.set_last_message("New message".into());
        assert_eq!(
            messages_before_request(&session, &history),
            "Previous message\n\nLatest message\n\nNew message"
        );
        session.set_approval(None);
        assert_eq!(
            messages_before_request(&session, &history),
            "Previous message\n\nLatest message"
        );
        session.set_last_message(String::new());
        history = MessageHistory::default();
        assert!(messages_before_request(&session, &history).is_empty());
    }

    #[test]
    fn working_count_respects_application_visibility_without_following_title_search() {
        let state = State::default();
        let mut editor = session("editor", None);
        editor.activity = Activity::Working;
        let mut terminal = session("terminal", None);
        terminal.client = Client::Terminal;
        terminal.activity = Activity::Working;
        let idle = session("idle", None);
        state
            .applications
            .borrow_mut()
            .observe([Client::VSCode, Client::Terminal]);
        *state.all.borrow_mut() = vec![editor, terminal, idle];
        assert_eq!(state.working_count(), 2);
        state
            .applications
            .borrow_mut()
            .set_enabled(&Client::Terminal.application_id(), false);
        assert_eq!(state.working_count(), 1);
        *state.filter_query.borrow_mut() = "no matching titles".into();
        assert!(state.filtered_sessions().is_empty());
        assert_eq!(state.working_count(), 1);
        state
            .applications
            .borrow_mut()
            .set_enabled(&Client::VSCode.application_id(), false);
        assert_eq!(state.working_count(), 0);
    }

    #[test]
    fn finished_count_follows_application_visibility_but_not_title_filter() {
        let state = State::default();
        let editor = session("editor", None);
        let mut terminal = session("terminal", None);
        terminal.client = Client::Terminal;
        let snapshot = crate::actors::monitor::State {
            sessions: vec![editor.clone(), terminal.clone()],
            unseen_finished: HashMap::from([(editor.key(), 1), (terminal.key(), 2)]),
            ..Default::default()
        };
        let handle = crate::actors::monitor::Handle {
            tx: tokio::sync::mpsc::channel(1).0,
            state: tokio::sync::watch::channel(std::sync::Arc::new(snapshot)).1,
        };
        assert!(state.monitor.set(handle).is_ok());
        state
            .applications
            .borrow_mut()
            .observe([Client::VSCode, Client::Terminal]);
        assert_eq!(state.finished_count(), 2);
        state
            .applications
            .borrow_mut()
            .set_enabled(&Client::Terminal.application_id(), false);
        assert_eq!(state.finished_count(), 1);
        *state.filter_query.borrow_mut() = "no match".into();
        assert_eq!(state.finished_count(), 1);
    }

    #[test]
    fn session_actions_acknowledge_only_the_current_finished_generation() {
        let state = State::default();
        let idle = session("finished", None);
        let mut working = session("working", None);
        working.activity = Activity::Working;
        let idle_key = idle.key();
        let working_key = working.key();
        let snapshot = crate::actors::monitor::State {
            sessions: vec![idle, working],
            unseen_finished: HashMap::from([(idle_key.clone(), 7), (working_key.clone(), 8)]),
            ..Default::default()
        };
        let (commands, mut received) = tokio::sync::mpsc::channel(8);
        let (published, snapshots) =
            tokio::sync::watch::channel(std::sync::Arc::new(snapshot.clone()));
        assert!(
            state
                .monitor
                .set(crate::actors::monitor::Handle {
                    tx: commands,
                    state: snapshots
                })
                .is_ok()
        );
        state.acknowledge_finished("codex:missing");
        state.acknowledge_finished(&working_key);
        assert!(received.try_recv().is_err());
        state.acknowledge_finished(&idle_key);
        let crate::actors::monitor::Command::FinishedSeen { completions } =
            received.try_recv().unwrap()
        else {
            panic!("expected completion acknowledgement");
        };
        assert_eq!(completions, HashMap::from([(idle_key.clone(), 7)]));
        let mut restarted = snapshot;
        restarted.sessions[0].activity = Activity::Working;
        published.send_replace(std::sync::Arc::new(restarted));
        state.acknowledge_finished(&idle_key);
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn application_checkboxes_filter_the_window_without_losing_discovery_or_text_search() {
        let state = State::default();
        let vscode = session("editor", None);
        let mut terminal = session("terminal", None);
        terminal.client = Client::Terminal;
        state
            .applications
            .borrow_mut()
            .observe([Client::VSCode, Client::Terminal]);
        *state.all.borrow_mut() = vec![vscode, terminal];
        *state.filter_query.borrow_mut() = "EDITOR".into();
        assert_eq!(state.filtered_sessions().len(), 1);
        state
            .applications
            .borrow_mut()
            .set_enabled(&Client::VSCode.application_id(), false);
        assert!(state.filtered_sessions().is_empty());
        state.filter_query.borrow_mut().clear();
        assert_eq!(state.filtered_sessions()[0].client, Client::Terminal);
        assert_eq!(state.all.borrow().len(), 2);
        state
            .applications
            .borrow_mut()
            .set_enabled(&Client::VSCode.application_id(), true);
        assert_eq!(state.filtered_sessions().len(), 2);
    }

    #[test]
    fn footer_fill_reaches_the_menu_bottom_in_both_coordinate_systems() {
        let bounds = rect(0.0, 0.0, 392.0, 400.0);
        assert_eq!(
            footer_gap(bounds, rect(0.0, 4.0, 392.0, 68.0), false),
            rect(0.0, 0.0, 392.0, 4.0)
        );
        assert_eq!(
            footer_gap(bounds, rect(0.0, 328.0, 392.0, 68.0), true),
            rect(0.0, 396.0, 392.0, 4.0)
        );
        assert_eq!(
            footer_gap(bounds, rect(0.0, 0.0, 392.0, 68.0), false)
                .size
                .height,
            0.0
        );
    }

    #[test]
    fn status_counters_follow_priority_without_gaps_for_hidden_counters() {
        for mask in 0..8 {
            let widths = std::array::from_fn(|index| (mask & (1 << index) != 0).then_some(14.0));
            let (positions, extra) = status_counter_positions(widths);
            let mut end = 26.0;
            for index in 0..3 {
                if widths[index].is_some() {
                    assert_eq!(positions[index], end + if end == 26.0 { 2.0 } else { 5.0 });
                    end = positions[index] + 14.0;
                }
            }
            assert_eq!(extra, end - 26.0);
        }
        let (positions, extra) = status_counter_positions([Some(26.0), Some(20.0), Some(9.0)]);
        assert_eq!(positions, [28.0, 59.0, 84.0]);
        assert_eq!(extra, 67.0);
    }

    #[test]
    fn approval_decisions_wrap_naturally_without_moving_on_scope_expansion() {
        let sizes = [
            NSSize::new(120.0, 32.0),
            NSSize::new(80.0, 32.0),
            NSSize::new(140.0, 32.0),
        ];
        let closed = [None, None, Some(0.0)];
        let open = [None, None, Some(60.0)];
        let (frames, height) = request_action_layout(&sizes, &closed, 420.0);
        assert_eq!(
            frames,
            vec![
                rect(0.0, 0.0, 120.0, 32.0),
                rect(130.0, 0.0, 80.0, 32.0),
                rect(220.0, 0.0, 140.0, 32.0)
            ]
        );
        assert_eq!(height, 32.0);
        let joined = [Some(0.0), None, Some(0.0)];
        assert_eq!(
            request_action_layout(&sizes, &joined, 420.0).0,
            frames,
            "an embedded disclosure must not reserve a second outside button"
        );
        let (expanded, height) = request_action_layout(&sizes, &open, 420.0);
        assert_eq!(expanded, frames);
        assert_eq!(request_scope_y(&frames, &open, 2), 36.0);
        assert_eq!(height, 96.0);
        let (wrapped, height) = request_action_layout(&sizes, &closed, 340.0);
        assert_eq!(wrapped[2], rect(0.0, 42.0, 140.0, 32.0));
        assert_eq!(height, 74.0);
        for frame in &wrapped {
            assert!(frame.origin.x + frame.size.width <= 340.0);
        }
        assert_eq!(request_action_layout(&[], &[], 362.0), (vec![], 0.0));
    }

    #[test]
    fn answer_buttons_share_one_row_only_when_all_fit_and_never_stretch() {
        let sizes = [
            NSSize::new(60.0, 28.0),
            NSSize::new(80.0, 28.0),
            NSSize::new(100.0, 28.0),
        ];
        let (row, height) = answer_button_layout(&sizes, 318.0);
        assert_eq!(
            row,
            vec![
                rect(0.0, 0.0, 60.0, 28.0),
                rect(66.0, 0.0, 80.0, 28.0),
                rect(152.0, 0.0, 100.0, 28.0)
            ]
        );
        assert_eq!(height, 28.0);
        let (column, height) = answer_button_layout(&sizes, 240.0);
        assert_eq!(
            column,
            vec![
                rect(0.0, 0.0, 60.0, 28.0),
                rect(0.0, 34.0, 80.0, 28.0),
                rect(0.0, 68.0, 100.0, 28.0)
            ]
        );
        assert_eq!(height, 96.0);
        for pair in row.windows(2) {
            assert_eq!(
                pair[1].origin.x
                    - pair[0].origin.x
                    - pair[0].size.width
                    - 2.0 * BUTTON_HOVER_PADDING,
                6.0
            );
        }
        for pair in column.windows(2) {
            assert_eq!(
                pair[1].origin.y
                    - pair[0].origin.y
                    - pair[0].size.height
                    - 2.0 * BUTTON_HOVER_PADDING,
                6.0
            );
        }
        assert_eq!(answer_button_layout(&[], 318.0), (vec![], 0.0));
    }

    #[test]
    fn horizontal_answers_wrap_skip_disabled_and_preserve_vertical_disclosure_keys() {
        let row = vec![
            ("allow".into(), 20.0, true),
            ("disabled".into(), 20.0, false),
            ("deny".into(), 20.0, true),
        ];
        assert_eq!(
            horizontal_answer("allow", &row, true).as_deref(),
            Some("deny")
        );
        assert_eq!(
            horizontal_answer("deny", &row, true).as_deref(),
            Some("allow")
        );
        assert_eq!(
            horizontal_answer("allow", &row, false).as_deref(),
            Some("deny")
        );
        assert_eq!(
            horizontal_answer("deny", &row, false).as_deref(),
            Some("allow")
        );
        assert!(horizontal_answer("disabled", &row, true).is_none());
        let mut column = row.clone();
        column[2].1 = 60.0;
        assert!(horizontal_answer("allow", &column, true).is_none());
        assert!(horizontal_answer("allow", &[], true).is_none());
        assert_eq!(
            horizontal_answer("allow", &row[..1], false).as_deref(),
            Some("allow")
        );
    }

    #[test]
    fn keyboard_order_includes_answers_only_under_their_visible_expanded_session() {
        let answers = vec![
            ("allow".into(), "a".into()),
            ("deny".into(), "a".into()),
            ("answer".into(), "b".into()),
            ("hidden".into(), "filtered".into()),
        ];
        let order = keyboard_order(
            [("a".into(), true), ("b".into(), false), ("c".into(), true)],
            &answers,
        );
        assert_eq!(order, ["a", "allow", "deny", "b", "c"]);
        let down = adjacent_session(Some(0), order.len(), true).unwrap();
        assert_eq!(order[down], "allow");
        assert_eq!(
            order[adjacent_session(Some(3), order.len(), false).unwrap()],
            "deny"
        );
        assert_eq!(
            keyboard_order([("a".into(), false), ("b".into(), true)], &answers),
            ["a", "b", "answer"]
        );
        assert!(keyboard_order([], &answers).is_empty());
    }

    #[test]
    fn vertical_navigation_escapes_text_editing_but_cursor_and_modified_keys_do_not() {
        for key in [125, 126] {
            assert!(menu_navigation_key(key, NSEventModifierFlags::empty()));
            assert!(menu_navigation_key(key, NSEventModifierFlags::Shift));
            for modifier in [
                NSEventModifierFlags::Command,
                NSEventModifierFlags::Control,
                NSEventModifierFlags::Option,
            ] {
                assert!(!menu_navigation_key(key, modifier));
            }
        }
        for key in [123, 124, 36, 76, 49, 51, 0] {
            assert!(!menu_navigation_key(key, NSEventModifierFlags::empty()));
        }
    }

    #[test]
    fn keyboard_navigation_starts_at_an_edge_and_stays_within_the_sessions() {
        assert_eq!(adjacent_session(None, 0, true), None);
        assert_eq!(adjacent_session(None, 3, true), Some(0));
        assert_eq!(adjacent_session(None, 3, false), Some(2));
        assert_eq!(adjacent_session(Some(0), 3, false), Some(0));
        assert_eq!(adjacent_session(Some(1), 3, true), Some(2));
        assert_eq!(adjacent_session(Some(2), 3, true), Some(2));
        assert_eq!(adjacent_session(Some(1), 3, false), Some(0));
    }

    fn session(id: &str, approval: Option<&str>) -> Session {
        {
            let mut __session = Session {
                target: None,
                agent: "codex",
                id: id.into(),
                title: id.into(),
                cwd: "/project".into(),
                workspace: None,
                git_branch: None,
                model: None,
                model_display_name: None,
                reasoning_effort: None,
                updated_at: 0,
                client: Client::VSCode,
                activity: if approval.is_some() {
                    Activity::Confirmation
                } else {
                    Activity::Idle
                },
                connection: if true {
                    crate::domain::session::ConnectionState::live(true)
                } else {
                    Default::default()
                },
                activities: Default::default(),
            };
            __session.seed_snapshot(
                String::new(),
                vec![],
                None,
                approval.map(|path| crate::agents::sessions::Approval {
                    id: path.into(),
                    body: "Confirmation".into(),
                    expires: u64::MAX,
                    can_allow: true,
                }),
                Default::default(),
            );
            __session
        }
    }

    #[test]
    fn active_sessions_expand_on_open_and_after_async_discovery_but_respect_manual_collapse() {
        let state = State::default();
        let mut working = session("working", None);
        working.activity = Activity::Working;
        let mut unknown = session("unknown", None);
        unknown.activity = Activity::Unknown;
        let mut sessions = vec![
            session("waiting", Some("/tmp/request-one/response")),
            session("idle", None),
            working,
            unknown,
        ];
        assert!(!state.expand_active_sessions(&sessions));
        state.menu_open.set(true);
        assert!(state.expand_active_sessions(&sessions));
        assert_eq!(
            *state.expanded.borrow(),
            HashSet::from(["codex:waiting".into(), "codex:working".into()])
        );
        state.expanded.borrow_mut().remove("codex:waiting");
        state.expanded.borrow_mut().remove("codex:working");
        state
            .reveal
            .borrow_mut()
            .insert("codex:waiting".into(), 0.0);
        state
            .reveal
            .borrow_mut()
            .insert("codex:working".into(), 0.0);
        assert!(!state.expand_active_sessions(&sessions));
        assert!(!state.expanded.borrow().contains("codex:waiting"));
        assert!(!state.expanded.borrow().contains("codex:working"));
        sessions.push(session("late", Some("/tmp/request-two/response")));
        assert!(state.expand_active_sessions(&sessions));
        assert!(state.expanded.borrow().contains("codex:late"));
        assert!(!state.expanded.borrow().contains("codex:waiting"));
        state.menu_open.set(false);
        state.reset_disclosures();
        state.menu_open.set(true);
        assert!(state.expand_active_sessions(&sessions));
        assert!(state.expanded.borrow().contains("codex:waiting"));
        assert!(state.expanded.borrow().contains("codex:working"));
        assert!(!state.expanded.borrow().contains("codex:idle"));
        assert!(!state.expanded.borrow().contains("codex:unknown"));
    }

    #[test]
    fn a_new_request_in_the_same_session_stays_expanded_during_an_existing_animation() {
        let state = State::default();
        state.menu_open.set(true);
        state.expand_active_sessions(&[session("chat", Some("/tmp/first/response"))]);
        state.expanded.borrow_mut().remove("codex:chat");
        let now = Instant::now();
        *state.motion.borrow_mut() = Some(motion::Disclosure::new(
            HashMap::from([("codex:chat".into(), 1.0), ("other".into(), 0.0)]),
            HashMap::from([("codex:chat".into(), 0.0), ("other".into(), 1.0)]),
            now,
        ));
        assert!(state.expand_active_sessions(&[session("chat", Some("/tmp/second/response"))]));
        let motion = state.motion.borrow();
        let (sample, _) = motion.as_ref().unwrap().sample(now + motion::DURATION / 2);
        assert_eq!(sample["codex:chat"], 1.0);
        assert!(sample["other"] > 0.0 && sample["other"] < 1.0);
    }

    #[test]
    fn closing_mid_animation_collapses_every_session_and_invalidates_queued_frames() {
        let state = State::default();
        state
            .expanded
            .borrow_mut()
            .extend(["one".into(), "two".into()]);
        state
            .reveal
            .borrow_mut()
            .extend([("one".into(), 1.0), ("two".into(), 0.4)]);
        *state.motion.borrow_mut() = Some(motion::Disclosure::new(
            state.reveal.borrow().clone(),
            HashMap::from([("two".into(), 1.0)]),
            Instant::now(),
        ));
        let queued_epoch = state.animation_epoch.get();
        state.reset_disclosures();
        assert_ne!(state.animation_epoch.get(), queued_epoch);
        assert!(state.expanded.borrow().is_empty());
        assert!(state.reveal.borrow().is_empty());
        assert!(state.motion.borrow().is_none());
        assert!(state.opened_approvals.borrow().is_empty());
        assert!(state.opened_working.borrow().is_empty());
    }
}
