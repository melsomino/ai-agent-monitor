//! Resident menu bar item and native sessions menu.
use crate::{
    agents::{
        codex::usage,
        interaction::{Action, Response},
        sessions::{Activity, Client, Session},
    },
    app::core,
    macos::{answers, hotkey, motion, native, pointer, search, settings},
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

const SESSION_HEADER_HEIGHT: f64 = 24.0;
const APPLICATION_HEADER_HEIGHT: f64 = 34.0;
const SESSION_SPACING: f64 = 12.0;
const LIST_BOTTOM_PADDING: f64 = 8.0;
const SESSION_HOVER_PADDING: f64 = 4.0;
const SESSION_HOVER_LEFT: f64 = 6.0;
const SESSION_HOVER_WIDTH: f64 = 374.0;
const MESSAGE_TOP_GAP: f64 = 4.0;
const BUTTON_HOVER_PADDING: f64 = 3.0;
const QUESTION_CARD_WIDTH: f64 = 348.0;

#[derive(Clone, Default)]
struct MessageHistory {
    messages: Vec<crate::agents::history::Message>,
    cursor: Option<String>,
    anchor: String,
    loading: bool,
    exhausted: bool,
    error: Option<String>,
    ticket: u64,
}

#[derive(Clone)]
enum ControlKind {
    Reply(Response),
    Form,
    Open,
}
#[derive(Clone)]
struct RequestControl {
    session: Session,
    label: String,
    detail: String,
    kind: ControlKind,
}
fn request_controls(session: &Session) -> Vec<RequestControl> {
    if session.approval.is_none() {
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
    monitor: OnceCell<crate::actors::monitor::Handle>,
    revision: Cell<u64>,
    ui_sender: OnceCell<crate::macos::bridge::Sender>,
    ui_incoming: OnceCell<RefCell<tokio::sync::mpsc::Receiver<crate::macos::bridge::Event>>>,
    forms: RefCell<Vec<Retained<answers::AnswerForm>>>,
    platform: OnceCell<RefCell<crate::macos::bridge::Platform>>,
    snapshot_timer: OnceCell<Retained<NSTimer>>,
    status: OnceCell<Retained<NSStatusItem>>,
    status_content: OnceCell<Retained<StatusContentView>>,
    status_badge: OnceCell<Retained<NSBox>>,
    status_count: OnceCell<Retained<NSTextField>>,
    status_icon: OnceCell<Retained<NSImageView>>,
    status_base_images: OnceCell<[Retained<NSImage>; 2]>,
    status_eyes: OnceCell<[Retained<NSImageView>; 2]>,
    status_eye_animation: RefCell<motion::WorkingEyes>,
    status_timer: RefCell<Option<Retained<NSTimer>>>,
    settings_window: OnceCell<Retained<NSWindow>>,
    applications: RefCell<settings::Applications>,
    applications_list: OnceCell<Retained<SessionListView>>,
    integrations_list: OnceCell<Retained<SessionListView>>,
    integrations_button: OnceCell<Retained<NSButton>>,
    integrations_note: OnceCell<Retained<NSTextField>>,
    integrations_snapshot: RefCell<Option<Vec<crate::macos::onboarding::Row>>>,
    telegram_token: OnceCell<Retained<NSSecureTextField>>,
    telegram_enabled: OnceCell<Retained<NSButton>>,
    telegram_status: OnceCell<Retained<NSTextField>>,
    telegram_pair: OnceCell<Retained<NSButton>>,
    telegram_test: OnceCell<Retained<NSButton>>,
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
    menu: OnceCell<Retained<NSMenu>>,
    menu_item: OnceCell<Retained<NSMenuItem>>,
    menu_close_shortcut: OnceCell<Retained<NSMenuItem>>,
    menu_open: Cell<bool>,
    list: OnceCell<Retained<SessionListView>>,
    root: OnceCell<Retained<PanelView>>,
    toolbar: OnceCell<Retained<NSView>>,
    footer: OnceCell<Retained<NSView>>,
    footer_height: Cell<f64>,
    footer_extension: OnceCell<FooterExtension>,
    footer_fill: OnceCell<Retained<NSColor>>,
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
    all: RefCell<Vec<Session>>,
    visible: RefCell<Vec<Session>>,
    filter_query: RefCell<String>,
    filter_dirty: Cell<bool>,
    usage: RefCell<Option<usage::Usage>>,
    lock: OnceCell<File>,
    routing_scroll: Cell<bool>,
    buttons: RefCell<Vec<ButtonHover>>,
    request_controls: RefCell<HashMap<String, RequestControl>>,
    responding: RefCell<HashSet<String>>,
    message_history: RefCell<HashMap<String, MessageHistory>>,
    history_generation: Cell<u64>,
    toolbar_buttons: RefCell<Vec<ButtonHover>>,
    hovered: Cell<Option<pointer::Target>>,
    keyboard_focus: RefCell<Option<String>>,
    keyboard_handler: RefCell<Option<hotkey::MenuKeyboard>>,
    pointer_timer: RefCell<Option<Retained<NSTimer>>>,
    last_pointer: Cell<Option<NSPoint>>,
}

impl State {
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

define_class!(
    // SAFETY: NSObject has no subclassing requirements. UI and all state are
    // confined to the main thread; retained controls outlive their weak targets.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    pub struct MenuController;

    unsafe impl NSObjectProtocol for MenuController {}
    // SAFETY: The retained controller handles the window's main-thread delegate callback.
    unsafe impl NSWindowDelegate for MenuController {
        #[unsafe(method(windowDidBecomeKey:))]
        fn settings_became_key(&self, _notification: &NSNotification) {
            self.refresh_settings();
        }
        #[unsafe(method(windowDidResignKey:))]
        fn settings_resigned_key(&self, _notification: &NSNotification) {
            self.end_shortcut_recording(true);
        }
        #[unsafe(method(windowWillClose:))]
        fn settings_will_close(&self, _notification: &NSNotification) {
            self.end_shortcut_recording(true);
        }
    }
    // SAFETY: The retained main-thread controller implements NSMenuDelegate.
    unsafe impl NSMenuDelegate for MenuController {
        #[unsafe(method(menuWillOpen:))]
        fn will_open(&self, _menu: &NSMenu) {
            self.ivars().menu_open.set(true);
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
            self.close_panel();
            self.install_shortcut();
        }
    }

    impl MenuController {
        #[unsafe(method(toggleMenuFromHotkey:))]
        fn toggle_menu_from_hotkey(&self, _timer: &NSTimer) {
            if self.ivars().menu_open.get() { self.dismiss_menu(); } else { self.show(); }
        }

        #[unsafe(method(closeMenu:))]
        fn close_menu(&self, _sender: Option<&AnyObject>) {
            self.dismiss_menu();
        }
        #[unsafe(method(animateStatus:))]
        fn animate_status(&self, _timer: &NSTimer) {
            self.ivars().status_eye_animation.borrow_mut().tick();
            self.paint_status_eyes();
        }

        #[unsafe(method(pollState:))]
        fn poll_state(&self,_timer:&NSTimer){self.drain_ui();self.receive_sessions();}
        #[unsafe(method(pollPointer:))]
        fn poll_pointer(&self, _timer: &NSTimer) {
            self.sync_footer_extension();
            self.start_keyboard_tracking();
            self.receive_sessions();
            self.advance_disclosure();
            self.advance_working_icons();
            self.sample_pointer();
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

        #[unsafe(method(previousMessage:))]
        fn previous_message(&self, sender: &NSButton) {
            let Some(key) = sender.identifier().map(|id| id.to_string()) else { return; };
            let state = self.ivars();
            let Some(session) = state.all.borrow().iter().find(|session| session.key() == key).cloned() else { return; };
            let ticket = state.history_generation.get().wrapping_add(1);
            state.history_generation.set(ticket);
            let before = {
                let mut histories = state.message_history.borrow_mut();
                let history = histories.entry(key.clone()).or_default();
                if history.loading || history.exhausted { return; }
                if history.cursor.is_none() && session.approval.is_none() { history.anchor = session.last_message.clone(); }
                history.loading = true;
                history.error = None;
                history.ticket = ticket;
                history.cursor.clone()
            };
            self.render_rows(&state.filtered_sessions());
            let _ = self.monitor().send(crate::actors::monitor::Command::History{session:key,cursor:before,generation:ticket});
        }

        #[unsafe(method(respondToRequest:))]
        fn respond_to_request(&self, sender: Option<&NSButton>) {
            let Some(id) = sender.and_then(|button| button.identifier()).map(|id| id.to_string()) else { return; };
            let control = self.ivars().request_controls.borrow().get(&id).cloned();
            let Some(control) = control else { return; };
            match control.kind {
                ControlKind::Form => { self.dismiss_menu(); self.show_form(control.session,None); }
                ControlKind::Open => {
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
                self.dismiss_menu();
                self.monitor().open(session);
            }
        }

        #[unsafe(method(settings:))]
        fn settings(&self, _sender: Option<&AnyObject>) {
            self.dismiss_menu();
            self.show_settings();
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
                let _=self.monitor().send(crate::actors::monitor::Command::Telegram(crate::actors::human::telegram::Event::Connect(token)));
                self.refresh_telegram_settings();
            }
        }
        #[unsafe(method(disconnectTelegram:))]
        fn disconnect_telegram(&self, _sender: Option<&AnyObject>) {
            let _=self.monitor().send(crate::actors::monitor::Command::Telegram(crate::actors::human::telegram::Event::Disconnect));
        }
        #[unsafe(method(toggleTelegram:))]
        fn toggle_telegram(&self, sender: &NSButton) {
            let _=self.monitor().send(crate::actors::monitor::Command::Telegram(crate::actors::human::telegram::Event::Enable(sender.state() == NSControlStateValueOn)));
        }
        #[unsafe(method(pairTelegram:))]
        fn pair_telegram(&self, _sender: Option<&AnyObject>) {
            if let Some(link) = self.monitor().snapshot().telegram.clone().link
                && let Some(url) = NSURL::URLWithString(&NSString::from_str(&link)) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
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
            let state = self.ivars();
            let mut apps=state.applications.borrow().clone();
            if apps.set_enabled(&id.to_string(),sender.state()==NSControlStateValueOn){let _=self.monitor().send(crate::actors::monitor::Command::Platform(crate::macos::bridge::PlatformCommand::Applications(apps)));}
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

#[derive(Debug)]
struct WorkingIconState {
    angle: Cell<f64>,
}

define_class!(
    // SAFETY: NSImageView supports subclassing; angle and drawing stay on the main thread.
    #[unsafe(super(NSImageView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = WorkingIconState]
    #[derive(Debug)]
    struct WorkingIconView;
    unsafe impl NSObjectProtocol for WorkingIconView {}
    impl WorkingIconView {
        #[unsafe(method(drawRect:))]
        fn draw_rotated(&self, dirty: NSRect) {
            let bounds = self.bounds();
            let x = bounds.origin.x + bounds.size.width / 2.0;
            let y = bounds.origin.y + bounds.size.height / 2.0;
            NSGraphicsContext::saveGraphicsState_class();
            let transform = NSAffineTransform::transform();
            transform.translateXBy_yBy(x, y);
            transform.rotateByDegrees(self.ivars().angle.get());
            transform.translateXBy_yBy(-x, -y);
            transform.concat();
            // SAFETY: Invoke NSImageView's drawing within the saved graphics state.
            unsafe { let _: () = msg_send![super(self), drawRect: dirty]; }
            NSGraphicsContext::restoreGraphicsState_class();
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
    // SAFETY: This NSView is main-thread-only. Its decoration never intercepts
    // clicks; the owning NSStatusBarButton handles the complete hit area.
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
fn pointer_changed(view: &NSView) {
    if let Some(controller) = owner(view) {
        controller.update_hover();
    }
}
fn route_wheel(view: &NSView, event: &NSEvent) {
    if let Some(controller) = owner(view) {
        controller.route_scroll(event);
    }
}
#[derive(Default)]
struct PanelState {
    owner: OnceCell<objc2::rc::Weak<MenuController>>,
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
        #[unsafe(method(scrollWheel:))]
        fn wheel(&self, event: &NSEvent) { route_wheel(self,event); }
    }
);
define_class!(
    // SAFETY: Standard NSScrollView override, confined to the main thread.
    #[unsafe(super(NSScrollView))]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    struct SessionScrollView;
    unsafe impl NSObjectProtocol for SessionScrollView {}
    impl SessionScrollView {
        #[unsafe(method(reflectScrolledClipView:))]
        fn reflected(&self, clip: &NSClipView) {
            // SAFETY: Forward AppKit's standard scroll notification.
            unsafe { let _: () = msg_send![super(self), reflectScrolledClipView: clip]; }
            pointer_changed(self);
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
define_class!(
    // SAFETY: Transparent overlay delegates hit testing to its actual controls.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    struct OverlayView;
    unsafe impl NSObjectProtocol for OverlayView {}
    impl OverlayView {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            // SAFETY: Standard NSView hit test with a valid point.
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            hit.filter(|view| !std::ptr::eq(std::ptr::from_ref(&**view).cast::<()>(), std::ptr::from_ref(self).cast::<()>()))
        }
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
                SESSION_HOVER_LEFT,
                frame.size.height - SESSION_HEADER_HEIGHT - SESSION_HOVER_PADDING,
                SESSION_HOVER_WIDTH,
                SESSION_HEADER_HEIGHT + SESSION_HOVER_PADDING * 2.0,
            ),
            16.0,
        );
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
            let color = if focused {
                NSColor::selectedContentBackgroundColor()
            } else {
                NSColor::labelColor().colorWithAlphaComponent(0.12)
            };
            background.setFillColor(&color);
            paint_hover(background, if active || focused { 1.0 } else { 0.0 });
        }
        self.paint_title(focused);
        if let Some(arrow) = self.ivars().arrow.get() {
            let color = if focused {
                NSColor::selectedMenuItemTextColor()
            } else {
                NSColor::secondaryLabelColor()
            };
            arrow.setContentTintColor(Some(&color));
        }
    }
    fn paint_title(&self, focused: bool) {
        let Some(title) = self.ivars().title.get() else {
            return;
        };
        let text = title.stringValue();
        let attributed =
            NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &text);
        let full = NSRange::new(0, text.length());
        let color = if focused {
            NSColor::selectedMenuItemTextColor()
        } else {
            NSColor::labelColor()
        };
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
    let background = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    background.setBoxType(NSBoxType::Custom);
    background.setTitlePosition(NSTitlePosition::NoTitle);
    background.setBorderWidth(0.0);
    background.setCornerRadius(radius);
    background.setFillColor(&NSColor::labelColor().colorWithAlphaComponent(0.12));
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

struct FooterExtension {
    view: Retained<StatusContentView>,
    fill: Retained<NSBox>,
}

struct ButtonHover {
    button: Retained<NSButton>,
    background: Retained<NSBox>,
    base_alpha: f64,
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
    working_icon: Option<Retained<WorkingIconView>>,
}

/// Use one row only when the entire set fits; otherwise keep natural widths
/// in a left-aligned column. The final gap separates controls from metadata.
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

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn detail_text_color(approval: bool) -> Retained<NSColor> {
    let (dark_rgb, light_rgb) = if approval {
        ([232.0, 197.0, 138.0], [128.0, 84.0, 22.0])
    } else {
        ([168.0, 199.0, 232.0], [54.0, 90.0, 124.0])
    };
    let color = |rgb: [f64; 3]| {
        NSColor::colorWithSRGBRed_green_blue_alpha(
            rgb[0] / 255.0,
            rgb[1] / 255.0,
            rgb[2] / 255.0,
            1.0,
        )
    };
    let dark = color(dark_rgb);
    let light = color(light_rgb);
    let provider = block2::RcBlock::new(move |appearance: std::ptr::NonNull<NSAppearance>| {
        // SAFETY: AppKit supplies a live appearance; framework name constants
        // are immutable, valid NSStrings for the lifetime of the process.
        let is_dark = unsafe {
            appearance
                .as_ref()
                .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[
                    NSAppearanceNameDarkAqua,
                    NSAppearanceNameAqua,
                ]))
                .is_some_and(|name| &*name == NSAppearanceNameDarkAqua)
        };
        std::ptr::NonNull::from(if is_dark { &*dark } else { &*light })
    });
    // SAFETY: Both returned colors are retained by the provider's captures.
    // NSColor copies and retains the provider for subsequent appearance changes.
    unsafe { NSColor::colorWithName_dynamicProvider(None, &provider) }
}

fn footer_fill_color() -> Retained<NSColor> {
    let dark = NSColor::colorWithSRGBRed_green_blue_alpha(0.21, 0.21, 0.21, 0.92);
    let light = NSColor::colorWithSRGBRed_green_blue_alpha(0.94, 0.94, 0.94, 0.92);
    let provider = block2::RcBlock::new(move |appearance: std::ptr::NonNull<NSAppearance>| {
        // SAFETY: AppKit supplies a live appearance and these immutable names.
        let dark_mode = unsafe {
            appearance
                .as_ref()
                .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[
                    NSAppearanceNameDarkAqua,
                    NSAppearanceNameAqua,
                ]))
                .is_some_and(|name| &*name == NSAppearanceNameDarkAqua)
        };
        std::ptr::NonNull::from(if dark_mode { &*dark } else { &*light })
    });
    // SAFETY: Both color objects are retained by the copied provider.
    unsafe { NSColor::colorWithName_dynamicProvider(None, &provider) }
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
    pub fn shutdown(&self) {
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
        if let Some(form) = answers::show_with_draft(
            session,
            draft,
            self.monitor().clone(),
            self.ivars().ui_sender.get().unwrap().clone(),
            &self.ivars().forms.borrow(),
        ) {
            form.reconcile(&self.monitor().snapshot());
            self.ivars().forms.borrow_mut().push(form);
        }
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
                    self.dismiss_menu();
                    if form {
                        self.show_form(*session, draft);
                    } else {
                        let key = session.key();
                        self.ivars().expanded.borrow_mut().insert(key.clone());
                        *self.ivars().keyboard_focus.borrow_mut() = Some(key);
                        self.filter();
                        self.show();
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
        let old = self.ivars().shortcut_registration.borrow_mut().take();
        drop(old);
        self.ivars().shortcut_error.borrow_mut().take();
        self.configure_menu_shortcut();
        if let Some(shortcut) = self.monitor().snapshot().preferences.shortcut.clone() {
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
        if !self
            .ivars()
            .settings_window
            .get()
            .is_some_and(|window| window.isKeyWindow())
        {
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

    fn dismiss_menu(&self) {
        if self.ivars().menu_open.get()
            && let Some(menu) = self.ivars().menu.get()
        {
            menu.cancelTracking();
        }
    }

    fn show(&self) {
        let state = self.ivars();
        if state.menu_open.get() {
            return;
        }
        self.receive_sessions();
        self.resize_content(state.content_height.get());
        self.refresh();
        if let Some(button) = state.status.get().and_then(|item| item.button(self.mtm())) {
            // Use the same native status-item path as an actual menu-bar click.
            // Manual popup coordinates can anchor at the screen origin during launch.
            if button.window().is_some() {
                // SAFETY: nil is a valid sender; AppKit owns this button's menu action.
                unsafe {
                    button.performClick(None);
                }
            }
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
        this.build_menu();
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
        let window = state.settings_window.get_or_init(|| {
            // SAFETY: Standard window initialization on the main thread. State
            // retains the window, so closing it must not release that ownership.
            let window = unsafe {
                let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(self.mtm()),
                    rect(0.0, 0.0, 460.0, 304.0),
                    NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                    NSBackingStoreType::Buffered,
                    false,
                );
                window.setReleasedWhenClosed(false);
                window
            };
            window.setTitle(ns_string!("AI Agent Monitor Settings"));
            window.setLevel(NSFloatingWindowLevel);
            window.setHidesOnDeactivate(false);
            window.setDelegate(Some(ProtocolObject::from_ref(self)));
            let view =
                NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 304.0));
            let note = self.label(
                &view,
                "Open AI Agent Monitor automatically",
                rect(44.0, 232.0, 392.0, 18.0),
                11.0,
                0.0,
                true,
            );
            let separator =
                NSBox::initWithFrame(NSBox::alloc(self.mtm()), rect(24.0, 214.0, 412.0, 1.0));
            separator.setBoxType(NSBoxType::Separator);
            view.addSubview(&separator);
            self.label(
                &view,
                "Animate the eyes while an agent is working",
                rect(44.0, 154.0, 392.0, 18.0),
                11.0,
                0.0,
                true,
            );
            for (y, action, storage, title) in [
                (
                    251.0,
                    sel!(toggleLogin:),
                    &state.login_switch,
                    ns_string!("Launch at login"),
                ),
                (
                    173.0,
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
            let separator = NSBox::initWithFrame(NSBox::alloc(self.mtm()), rect(24.0, 136.0, 412.0, 1.0));
            separator.setBoxType(NSBoxType::Separator);
            view.addSubview(&separator);
            self.label(&view, "Keyboard shortcut", rect(24.0, 98.0, 220.0, 22.0), 13.0, 0.23, false);
            let note = self.label(&view, "Open the sessions menu from any app", rect(24.0, 76.0, 412.0, 18.0), 11.0, 0.0, true);
            state.shortcut_note.set(note).expect("shortcut note initialized once");
            // SAFETY: Both action selectors are implemented by this retained controller.
            let record = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Record shortcut"), Some(self), Some(sel!(recordShortcut:)), self.mtm()) };
            record.setFrame(rect(264.0, 94.0, 140.0, 28.0));
            record.setToolTip(Some(ns_string!("Click, then press a shortcut. Escape cancels.")));
            view.addSubview(&record);
            state.shortcut_button.set(record).expect("shortcut recorder initialized once");
            // SAFETY: The clear action's target remains valid for the window's lifetime.
            let clear = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("×"), Some(self), Some(sel!(clearShortcut:)), self.mtm()) };
            clear.setFrame(rect(408.0, 94.0, 28.0, 28.0));
            clear.setToolTip(Some(ns_string!("Clear keyboard shortcut")));
            // SAFETY: NSButton supports the accessibility label property.
            unsafe { let _: () = msg_send![&*clear, setAccessibilityLabel: ns_string!("Clear keyboard shortcut")]; }
            view.addSubview(&clear);
            state.shortcut_clear.set(clear).expect("shortcut clear button initialized once");
            let applications_view = NSView::initWithFrame(
                NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 304.0),
            );
            self.label(&applications_view, "Show sessions in the menu", rect(24.0, 264.0, 412.0, 22.0), 13.0, 0.0, false);
            // SAFETY: Ivar-free flipped NSView, initialized on the UI thread.
            let list: Retained<SessionListView> = unsafe { msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 412.0, 224.0)] };
            let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(self.mtm()), rect(24.0, 20.0, 412.0, 224.0));
            scroll.setDrawsBackground(false);
            scroll.setHasVerticalScroller(true);
            scroll.setAutohidesScrollers(true);
            scroll.setHasHorizontalScroller(false);
            scroll.setDocumentView(Some(&list));
            applications_view.addSubview(&scroll);
            state.applications_list.set(list).expect("application settings initialized once");

            let integrations_view = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 304.0));
            self.label(&integrations_view, "Detected agents and hook setup", rect(24.0, 264.0, 412.0, 22.0), 13.0, 0.0, false);
            // SAFETY: Flipped, ivar-free document view on the UI thread.
            let integrations: Retained<SessionListView> = unsafe { msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 412.0, 184.0)] };
            let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(self.mtm()), rect(24.0, 64.0, 412.0, 184.0));
            scroll.setDrawsBackground(false);
            scroll.setHasVerticalScroller(true);
            scroll.setAutohidesScrollers(true);
            scroll.setHasHorizontalScroller(false);
            scroll.setDocumentView(Some(&integrations));
            integrations_view.addSubview(&scroll);
            state.integrations_list.set(integrations).expect("integrations initialized once");
            let note = self.label(&integrations_view, "", rect(24.0, 12.0, 228.0, 40.0), 10.0, 0.0, true);
            note.setUsesSingleLineMode(false);
            note.setMaximumNumberOfLines(2);
            state.integrations_note.set(note).expect("integration note initialized once");
            // SAFETY: Implemented selector and retained target.
            let setup = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Check & set up"), Some(self), Some(sel!(setupIntegrations:)), self.mtm()) };
            setup.setFrame(rect(264.0, 16.0, 172.0, 28.0));
            integrations_view.addSubview(&setup);
            state.integrations_button.set(setup).expect("integration setup button initialized once");

            let telegram_view = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 304.0));
            let instructions = self.label(&telegram_view,
                "Create your own bot with @BotFather using /newbot.\nPaste its token, connect, then open the pairing link and press Start.\nQuestions are sent directly from this Mac while the monitor is running.",
                rect(24.0, 234.0, 412.0, 54.0), 11.0, 0.0, true);
            instructions.setUsesSingleLineMode(false);
            instructions.setMaximumNumberOfLines(3);
            self.label(&telegram_view, "Bot token · stored in macOS Keychain", rect(24.0, 207.0, 412.0, 20.0), 12.0, 0.0, false);
            let token = NSSecureTextField::initWithFrame(NSSecureTextField::alloc(self.mtm()), rect(24.0, 176.0, 300.0, 26.0));
            token.setPlaceholderString(Some(ns_string!("Token from @BotFather")));
            token.setEditable(true);
            token.setSelectable(true);
            telegram_view.addSubview(&token);
            state.telegram_token.set(token).expect("Telegram token initialized once");
            // SAFETY: Selectors are implemented by this retained controller.
            let connect = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Connect"), Some(self), Some(sel!(connectTelegram:)), self.mtm()) };
            connect.setFrame(rect(332.0, 175.0, 104.0, 28.0));
            telegram_view.addSubview(&connect);
            // SAFETY: Selector and retained target are valid for the settings lifetime.
            let enabled = unsafe { NSButton::checkboxWithTitle_target_action(ns_string!("Send questions to Telegram"), Some(self), Some(sel!(toggleTelegram:)), self.mtm()) };
            enabled.setFrame(rect(24.0, 136.0, 412.0, 24.0));
            telegram_view.addSubview(&enabled);
            state.telegram_enabled.set(enabled).expect("Telegram toggle initialized once");
            let status = self.label(&telegram_view, "Not connected", rect(24.0, 70.0, 412.0, 54.0), 11.0, 0.0, true);
            status.setUsesSingleLineMode(false);
            status.setMaximumNumberOfLines(3);
            state.telegram_status.set(status).expect("Telegram status initialized once");
            for (title, action, x, width, storage) in [
                ("Open pairing link", sel!(pairTelegram:), 24.0, 132.0, &state.telegram_pair),
                ("Send test", sel!(testTelegram:), 164.0, 116.0, &state.telegram_test),
                ("Disconnect", sel!(disconnectTelegram:), 312.0, 124.0, &state.telegram_disconnect),
            ] {
                // SAFETY: Every selector is implemented by this retained controller.
                let button = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(self), Some(action), self.mtm()) };
                button.setFrame(rect(x, 24.0, width, 28.0));
                telegram_view.addSubview(&button);
                storage.set(button).expect("Telegram control initialized once");
            }

            // AppKit owns the preferences toolbar, selection, and tab transitions.
            let tabs = NSTabViewController::new(self.mtm());
            tabs.setTabStyle(NSTabViewControllerTabStyle::Toolbar);
            tabs.setTitle(Some(ns_string!("AI Agent Monitor Settings")));
            for (title, symbol, content) in [
                ("General", "gearshape", &view),
                ("Applications", "square.stack.3d.up", &applications_view),
                ("Integrations", "puzzlepiece.extension", &integrations_view),
                ("Telegram", "paperplane", &telegram_view),
            ] {
                let controller = NSViewController::new(self.mtm());
                controller.setTitle(Some(&NSString::from_str(title)));
                controller.setView(content);
                let item = NSTabViewItem::tabViewItemWithViewController(&controller);
                item.setLabel(&NSString::from_str(title));
                let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(symbol), Some(&NSString::from_str(title)),
                );
                item.setImage(image.as_deref());
                tabs.addTabViewItem(&item);
            }
            tabs.setSelectedTabViewItemIndex(0);
            window.setToolbarStyle(NSWindowToolbarStyle::Preference);
            window.setContentViewController(Some(&tabs));
            window.setContentSize(NSSize::new(460.0, 304.0));
            if let Some(toolbar) = window.toolbar() {
                toolbar.setDisplayMode(NSToolbarDisplayMode::IconAndLabel);
                toolbar.setAllowsUserCustomization(false);
            }
            window.center();
            window
        });
        self.refresh_settings();
        self.refresh_application_settings();
        self.refresh_integration_settings();
        self.refresh_telegram_settings();
        let app = NSApplication::sharedApplication(self.mtm());
        if app.respondsToSelector(sel!(activate)) {
            app.activate();
        } else {
            // Compatibility with macOS 13, before activate was introduced.
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }
        window.makeKeyAndOrderFront(None);
    }

    fn refresh_telegram_settings(&self) {
        let status = self.monitor().snapshot().telegram.clone();
        let state = self.ivars();
        if let Some(label) = state.telegram_status.get() {
            label.setStringValue(&NSString::from_str(&status.text));
        }
        if let Some(button) = state.telegram_enabled.get() {
            button.setState(if status.enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            button.setEnabled(status.connected && status.paired);
        }
        if let Some(button) = state.telegram_pair.get() {
            button.setEnabled(status.link.is_some());
        }
        if let Some(button) = state.telegram_test.get() {
            button.setEnabled(status.enabled && status.paired);
        }
        if let Some(button) = state.telegram_disconnect.get() {
            button.setEnabled(status.connected);
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
            self.label(
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
            let status = self.label(
                list,
                row.status(),
                rect(170.0, y + 6.0, 202.0, 18.0),
                11.0,
                0.0,
                false,
            );
            status.setAlignment(NSTextAlignment::Right);
            status.setTextColor(Some(&color));
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                None,
            ) {
                image.setTemplate(true);
                let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                icon.setFrame(rect(380.0, y + 7.0, 14.0, 14.0));
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
            detail.setFrame(rect(0.0, y + 30.0, 396.0, height));
            list.addSubview(&detail);
            y += height + 46.0;
        }
        list.setFrameSize(NSSize::new(412.0, y.max(184.0)));
        *state.integrations_snapshot.borrow_mut() = Some(rows);
    }

    fn refresh_application_settings(&self) {
        let state = self.ivars();
        let Some(list) = state.applications_list.get() else {
            return;
        };
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
            let y = index as f64 * 32.0;
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
                icon.setFrame(rect(0.0, y + 5.0, 22.0, 22.0));
                list.addSubview(&icon);
            }
            let title = self.label(
                list,
                &app.name,
                rect(32.0, y + 6.0, 312.0, 20.0),
                13.0,
                0.0,
                false,
            );
            center_label(&title, y + 16.0);
            let checkbox = NSSwitch::initWithFrame(
                NSSwitch::alloc(self.mtm()),
                rect(356.0, y + 4.0, 40.0, 24.0),
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
        list.setFrameSize(NSSize::new(412.0, (apps.len() as f64 * 32.0).max(224.0)));
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
            let height = button.bounds().size.height;
            // SAFETY: NSView's designated initializer is valid for this subclass.
            let content: Retained<StatusContentView> = unsafe {
                msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: rect(-3.0, 0.0, 26.0, height)]
            };
            let data =
                NSData::with_bytes(include_bytes!("../../assets/menubar/robots-template.pdf"));
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
            badge.setCornerRadius(6.0);
            badge.setFillColor(&NSColor::systemRedColor());
            content.addSubview(&badge);
            let count = self.label(&content, "", rect(14.0, 0.0, 12.0, 12.0), 8.0, 0.5, false);
            count.setAlignment(NSTextAlignment::Center);
            count.setTextColor(Some(&NSColor::whiteColor()));
            center_label(&count, height / 2.0);
            button.addSubview(&content);
            self.ivars()
                .status_content
                .set(content)
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
        item.setMenu(self.ivars().menu.get().map(|menu| &**menu));
        self.ivars()
            .status
            .set(item)
            .expect("status initialized once");
        self.update_badge(self.monitor().snapshot().pending_count());
    }

    fn paint_status_eyes(&self) {
        let state = self.ivars();
        let (levels, active) = {
            let animation = state.status_eye_animation.borrow();
            (animation.levels(), animation.active())
        };
        if let Some(icon) = state.status_icon.get()
            && let Some(images) = state.status_base_images.get()
        {
            icon.setImage(Some(&images[usize::from(active)]));
            NSView::setNeedsDisplay(icon, true);
        }
        if let Some(eyes) = state.status_eyes.get() {
            for (eye, level) in eyes.iter().zip(levels) {
                eye.setAlphaValue(level);
                eye.setHidden(level == 0.0);
                NSView::setNeedsDisplay(eye, true);
            }
        }
    }

    fn update_badge(&self, count: usize) {
        let state = self.ivars();
        let working = count == 0
            && state
                .all
                .borrow()
                .iter()
                .any(|session| session.activity == Activity::Working);
        let animate = working
            && self.monitor().snapshot().preferences.animation
            && !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        let Some(item) = state.status.get() else {
            return;
        };
        let badge_width = if count > 99 {
            22.0
        } else if count > 9 {
            16.0
        } else {
            12.0
        };
        let Some(button) = item.button(self.mtm()) else {
            return;
        };
        // macOS adds margins outside the public button's content rectangle.
        // Target the real capsule width, rather than treating length as that width.
        let margins = button
            .window()
            .map(|window| (window.frame().size.width - button.frame().size.width).max(0.0))
            .unwrap_or(16.0);
        let width = (36.0 - margins).max(1.0);
        if item.length() != width {
            item.setLength(width);
        }
        // Keep the complete 26-point image canvas centered, including the part
        // inside macOS's built-in status-item margins. No glyph scaling is applied.
        allow_status_overflow(&button);
        let height = button.bounds().size.height;
        if let Some(content) = state.status_content.get() {
            content.setFrame(rect(
                (button.bounds().size.width - 26.0) / 2.0,
                0.0,
                26.0,
                height,
            ));
            allow_status_overflow(content);
        }
        let badge_x = 26.0 - badge_width;
        let badge_y = (height / 2.0 - 10.0).max(1.0);
        if let Some(badge) = state.status_badge.get() {
            badge.setHidden(count == 0);
            badge.setFrame(rect(badge_x, badge_y, badge_width, 12.0));
        }
        state
            .status_eye_animation
            .borrow_mut()
            .update(working, animate);
        self.paint_status_eyes();
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
            label.setFrame(rect(badge_x, badge_y, badge_width, 12.0));
            center_label(label, badge_y + 6.0);
        }
        let tooltip = if working {
            "Agent Sessions · Working".to_owned()
        } else if count == 0 {
            "Agent Sessions".to_owned()
        } else {
            format!("Agent Sessions · Pending requests: {count}")
        };
        button.setToolTip(Some(&NSString::from_str(&tooltip)));
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

    fn separator(&self, view: &NSView, y: f64, width: f64) {
        let line = NSBox::initWithFrame(NSBox::alloc(self.mtm()), rect(18.0, y, width, 1.0));
        line.setBoxType(NSBoxType::Separator);
        view.addSubview(&line);
    }

    fn build_menu(&self) {
        let mtm = self.mtm();
        // NSMenu supplies the system background and owns presentation/dismissal.
        // SAFETY: NSView's initializer is valid for this ivar-free subclass.
        let allocated = PanelView::alloc(mtm).set_ivars(PanelState::default());
        let view: Retained<PanelView> =
            unsafe { msg_send![super(allocated),initWithFrame:rect(0.0,0.0,392.0,476.0)] };
        view.ivars()
            .owner
            .set(objc2::rc::Weak::new(self))
            .expect("root owner initialized once");
        view.setWantsLayer(true);
        // SAFETY: OverlayView has no ivars or custom initialization.
        let toolbar: Retained<OverlayView> = unsafe {
            msg_send![OverlayView::alloc(mtm), initWithFrame: rect(0.0, 0.0, 392.0, 32.0)]
        };
        let caption = self.label(&view, "", rect(20.0, 445.0, 286.0, 18.0), 10.0, 0.0, true);
        caption.setHidden(true);
        for (symbol, action, x, tooltip) in [
            ("gearshape", sel!(settings:), 316.0, "Settings"),
            ("power", sel!(quit:), 348.0, "Quit AI Agent Monitor"),
        ] {
            let button =
                self.action_button(&toolbar, "", action, "", rect(x, 4.0, 24.0, 24.0), false);
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                Some(&NSString::from_str(tooltip)),
            ) {
                image.setTemplate(true);
                image.setSize(NSSize::new(15.0, 15.0));
                button.setImage(Some(&image));
            }
            button.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
            button.setToolTip(Some(&NSString::from_str(tooltip)));
        }
        *self.ivars().toolbar_buttons.borrow_mut() =
            std::mem::take(&mut *self.ivars().buttons.borrow_mut());
        // SAFETY: NSView's designated initializer is valid for this subclass.
        let list: Retained<SessionListView> = unsafe {
            msg_send![SessionListView::alloc(mtm), initWithFrame: rect(0.0, 0.0, 384.0, 400.0)]
        };
        // SAFETY: NSScrollView's initializer is valid for this ivar-free subclass.
        let scroll: Retained<SessionScrollView> = unsafe {
            msg_send![SessionScrollView::alloc(mtm), initWithFrame: rect(4.0, 4.0, 384.0, 460.0)]
        };
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setHasHorizontalScroller(false);
        scroll.setDocumentView(Some(&list));
        view.addSubview(&scroll);
        let footer = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 392.0, 36.0));
        view.addSubview(&footer);
        self.ivars()
            .footer
            .set(footer)
            .expect("footer initialized once");
        view.addSubview(&toolbar);
        view.addSubview(&caption);
        view.setPostsFrameChangedNotifications(true);
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Codex"));
        menu.setAutoenablesItems(false);
        menu.setMinimumWidth(392.0);
        menu.setDelegate(Some(ProtocolObject::from_ref(self)));
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
        self.ivars().menu.set(menu).expect("menu initialized once");
        self.ivars().list.set(list).expect("list initialized once");
        self.ivars().root.set(view).expect("root initialized once");
        self.ivars()
            .toolbar
            .set(toolbar.into_super())
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
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(self),
                Some(action),
                self.mtm(),
            )
        };
        let frame = if transparent {
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
                gutter: None,
            });
        }
        view.addSubview(&button);
        button
    }

    fn client_icon(&self, row: &NSView, client: Client, y: f64) {
        let app = match client {
            Client::VSCode => Some("/Applications/Visual Studio Code.app"),
            Client::Desktop { app_path, .. } => Some(app_path),
            _ => None,
        };
        let image = if let Some(path) = app.filter(|path| std::path::Path::new(path).exists()) {
            Some(NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path)))
        } else {
            let symbol = match client {
                Client::Terminal => "terminal",
                Client::VSCode => "chevron.left.forwardslash.chevron.right",
                _ => "app",
            };
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                Some(&NSString::from_str(client.label())),
            )
        };
        if let Some(image) = image {
            let icon = NSImageView::imageViewWithImage(&image, self.mtm());
            icon.setFrame(rect(18.0, y, 26.0, 26.0));
            // SF Symbols otherwise stay at their intrinsic size and are centered,
            // shifting narrow symbols to the right of the common content edge.
            icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            icon.setImageAlignment(NSImageAlignment::AlignLeft);
            icon.setToolTip(Some(&NSString::from_str(client.label())));
            row.addSubview(&icon);
        }
    }

    fn status_icon(
        &self,
        view: &NSView,
        activity: Activity,
        y: f64,
    ) -> Option<Retained<WorkingIconView>> {
        let (symbol, color) = match activity {
            Activity::Working => ("arrow.triangle.2.circlepath", NSColor::systemGreenColor()),
            Activity::Confirmation => ("exclamationmark.circle.fill", NSColor::systemOrangeColor()),
            Activity::Idle => return None,
            Activity::Unknown => ("questionmark.circle", NSColor::tertiaryLabelColor()),
        };
        if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol),
            Some(&NSString::from_str(activity.label())),
        ) {
            image.setTemplate(true);
            let frame = rect(323.0, y + 1.0, 14.0, 14.0);
            if activity == Activity::Working {
                let allocated = WorkingIconView::alloc(self.mtm()).set_ivars(WorkingIconState {
                    angle: Cell::new(0.0),
                });
                // SAFETY: NSImageView's initializer is valid after initializing our angle ivar.
                let icon: Retained<WorkingIconView> =
                    unsafe { msg_send![super(allocated), initWithFrame: frame] };
                icon.setImage(Some(&image));
                icon.setContentTintColor(Some(&color));
                icon.setToolTip(Some(&NSString::from_str(activity.label())));
                view.addSubview(&icon);
                return Some(icon);
            }
            let icon = NSImageView::imageViewWithImage(&image, self.mtm());
            icon.setFrame(frame);
            icon.setContentTintColor(Some(&color));
            icon.setToolTip(Some(&NSString::from_str(activity.label())));
            view.addSubview(&icon);
        }
        None
    }

    fn advance_working_icons(&self) {
        let state = self.ivars();
        let angle = self.working_angle();
        for item in state.layout.borrow().iter() {
            if let Some(icon) = &item.working_icon {
                icon.ivars().angle.set(angle);
                NSView::setNeedsDisplay(icon, true);
            }
        }
    }

    fn working_angle(&self) -> f64 {
        let elapsed = self
            .ivars()
            .working_epoch
            .get_or_init(Instant::now)
            .elapsed();
        -(elapsed.as_secs_f64() * 240.0).rem_euclid(360.0)
    }

    fn close_panel(&self) {
        if let Some(extension) = self.ivars().footer_extension.get() {
            extension.view.removeFromSuperview();
        }
        self.stop_keyboard_tracking();
        self.ivars().keyboard_focus.borrow_mut().take();
        self.ivars().filter_query.borrow_mut().clear();
        self.ivars().filter_dirty.set(true);
        self.stop_pointer_tracking();
        let state = self.ivars();
        state.reset_disclosures();
        self.filter();
        self.render_footer();
        if let Some(scroll) = state.scroll.get() {
            scroll.contentView().scrollToPoint(NSPoint::new(0.0, 0.0));
        }
    }

    fn start_keyboard_tracking(&self) {
        let state = self.ivars();
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
    }

    fn stop_keyboard_tracking(&self) {
        self.ivars().keyboard_handler.borrow_mut().take();
    }

    pub fn handle_session_key(&self, event: &NSEvent) -> bool {
        let state = self.ivars();
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
            let ids: Vec<_> = state
                .layout
                .borrow()
                .iter()
                .filter_map(|row| row.id.clone())
                .collect();
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
            return true;
        };
        if matches!(key, 123 | 124) {
            let expanded = state.expanded.borrow().contains(&id);
            if !event.isARepeat() && expanded != (key == 124) {
                self.begin_disclosure(&id);
            }
        } else if !event.isARepeat()
            && let Some(session) = state.all.borrow().iter().find(|s| s.key() == id).cloned()
        {
            self.dismiss_menu();
            self.monitor().open(&session);
        }
        true
    }

    fn filter_edited(&self) {
        self.ivars().filter_dirty.set(true);
        self.filter();
        self.render_footer();
        self.reveal_keyboard_focus();
    }

    fn reveal_keyboard_focus(&self) {
        let state = self.ivars();
        let focus = state.keyboard_focus.borrow().clone();
        let header = state
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
            });
        // Release layout borrows: scrolling synchronously updates hover hit testing.
        if let Some(header) = header
            && let Some(list) = state.list.get()
        {
            list.scrollRectToVisible(header);
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
        if let Some(timer) = self.ivars().pointer_timer.borrow_mut().take() {
            timer.invalidate();
        }
        self.ivars().last_pointer.set(None);
        self.clear_hover();
    }

    fn sample_pointer(&self) {
        let state = self.ivars();
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
    }

    fn apply_hover(&self, target: Option<pointer::Target>) {
        let state = self.ivars();
        state.hovered.set(target);
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
            paint_hover(
                &item.background,
                if target == Some(pointer::Target::Button(index)) {
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
        let viewport = area(list.convertRect_fromView(clip.bounds(), Some(&clip)));
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
                    x: frame.origin.x + SESSION_HOVER_LEFT,
                    y: frame.origin.y - SESSION_HOVER_PADDING,
                    width: SESSION_HOVER_WIDTH,
                    height: SESSION_HEADER_HEIGHT + SESSION_HOVER_PADDING * 2.0,
                }
            })
            .collect();
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
        if let Some(scroll) = state.scroll.get() {
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
        let mut y = 0.0;
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
            if let Some(detail) = &item.detail {
                detail.setFrame(rect(0.0, 0.0, 384.0, detail_height));
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
        list.setFrameSize(NSSize::new(384.0, y.max(52.0)));
        let viewport = self.resize_content(y);
        if state.animation_epoch.get() != epoch {
            return;
        }
        list.setFrameSize(NSSize::new(384.0, y.max(viewport)));
        list.scrollPoint(NSPoint::new(
            origin.x,
            origin.y.min((y - viewport).max(0.0)),
        ));
        self.update_hover();
    }

    fn session_metadata(&self, session: &Session) -> (Retained<SessionListView>, f64) {
        // Use the same flipped coordinate system as the expanded detail document.
        // SAFETY: This ivar-free view uses NSView's initializer.
        let view: Retained<SessionListView> = unsafe {
            msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(6.0, 8.0, 338.0, 0.0)]
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
            rows.push((
                "arrow.triangle.branch",
                "Git branch",
                branch.clone(),
                branch.clone(),
            ));
        }
        let mut y = 0.0;
        for (symbol, description, value, tooltip) in rows {
            if y > 0.0 {
                y += 4.0;
            }
            let height = 16.0;
            let label = self.label(&view, &value, rect(20.0, y, 318.0, height), 11.0, 0.0, true);
            label.setSelectable(true);
            label.setToolTip(Some(&NSString::from_str(&tooltip)));
            // Align the symbol with the visible letter height, not the text field's
            // frame, whose baseline and descender leave extra space below the text.
            let text_bounds = label
                .cell()
                .map(|cell| cell.drawingRectForBounds(label.bounds()))
                .unwrap_or(label.bounds());
            let text_center = label
                .font()
                .map(|font| y + text_bounds.origin.y + font.ascender() - font.capHeight() / 2.0)
                .unwrap_or(y + height / 2.0);
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                Some(&NSString::from_str(description)),
            ) {
                image.setTemplate(true);
                let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                icon.setFrame(rect(0.0, text_center - 6.5, 13.0, 13.0));
                icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
                icon.setToolTip(Some(&NSString::from_str(description)));
                view.addSubview(&icon);
            }
            y += height;
        }
        view.setFrameSize(NSSize::new(338.0, y));
        (view, y)
    }

    fn render_rows(&self, sessions: &[Session]) {
        let Some(list) = self.ivars().list.get() else {
            return;
        };
        self.clear_hover();
        self.ivars().buttons.borrow_mut().clear();
        self.ivars().request_controls.borrow_mut().clear();
        let children = list.subviews();
        for index in 0..children.count() {
            children.objectAtIndex(index).removeFromSuperview();
        }
        let expanded = self.ivars().expanded.borrow();
        let reveal = self.ivars().reveal.borrow();
        let mut layout = Vec::new();
        let mut y = 0.0;
        let mut ordered: Vec<_> = sessions.iter().collect();
        let client_order = |client| match client {
            Client::VSCode => 0,
            Client::Terminal => 1,
            Client::Desktop { .. } => 2,
            Client::Unknown => 3,
        };
        ordered.sort_by(|a, b| {
            client_order(a.client)
                .cmp(&client_order(b.client))
                .then_with(|| {
                    match NSString::from_str(&a.title)
                        .localizedStandardCompare(&NSString::from_str(&b.title))
                    {
                        NSComparisonResult::Ascending => std::cmp::Ordering::Less,
                        NSComparisonResult::Descending => std::cmp::Ordering::Greater,
                        _ => a.key().cmp(&b.key()),
                    }
                })
        });
        let mut group = None;
        for session in ordered {
            let session_gap = if group == Some(session.client) {
                SESSION_SPACING
            } else {
                0.0
            };
            if group != Some(session.client) {
                let gap = if group.is_some() { 6.0 } else { 0.0 };
                y += gap;
                let name = if session.client == Client::Unknown {
                    "Other Apps"
                } else {
                    session.client.label()
                };
                let header = NSView::initWithFrame(
                    NSView::alloc(self.mtm()),
                    rect(0.0, y, 384.0, APPLICATION_HEADER_HEIGHT),
                );
                // This header uses bottom-up coordinates. Keep its content at
                // the top so the extra height is a gap below the application.
                self.client_icon(&header, session.client, APPLICATION_HEADER_HEIGHT - 27.0);
                let name = self.label(&header, name, rect(50.0, 0.0, 324.0, 22.0), 12.0, 0.3, true);
                center_label(&name, APPLICATION_HEADER_HEIGHT - 14.0);
                list.addSubview(&header);
                layout.push(LayoutRow {
                    view: header,
                    row: None,
                    id: None,
                    gap,
                    base_height: APPLICATION_HEADER_HEIGHT,
                    detail_height: 0.0,
                    detail: None,
                    arrow: None,
                    working_icon: None,
                });
                y += APPLICATION_HEADER_HEIGHT;
                group = Some(session.client);
            }
            y += session_gap;
            let controls = request_controls(session);
            let is_question = session.approval.is_some();
            let question_padding = if is_question { 10.0 } else { 0.0 };
            let metadata_gap = if is_question { 8.0 } else { 0.0 };
            let content_width = (if is_question {
                QUESTION_CARD_WIDTH
            } else {
                338.0
            }) - 2.0 * question_padding;
            let content_x = 18.0 + question_padding;
            // Include the expanded button's visible bounds in the card padding.
            let control_inset = if is_question {
                BUTTON_HOVER_PADDING
            } else {
                0.0
            };
            let control_width = content_width - 2.0 * control_inset;
            let control_sizes: Vec<_> = controls
                .iter()
                .map(|control| {
                    let natural = NSTextField::labelWithString(
                        &NSString::from_str(&control.label),
                        self.mtm(),
                    );
                    natural.setFont(Some(&NSFont::systemFontOfSize(12.0)));
                    natural.sizeToFit();
                    let width =
                        (natural.frame().size.width.ceil() + 24.0).clamp(52.0, control_width);
                    let measure = NSTextField::wrappingLabelWithString(
                        &NSString::from_str(&control.label),
                        self.mtm(),
                    );
                    measure.setFont(Some(&NSFont::systemFontOfSize(12.0)));
                    let height = measure
                        .cell()
                        .map(|cell| {
                            cell.cellSizeForBounds(rect(0.0, 0.0, width - 24.0, 100000.0))
                                .height
                                .ceil()
                                + 12.0
                        })
                        .unwrap_or(28.0)
                        .max(28.0);
                    NSSize::new(width, height)
                })
                .collect();
            let (control_frames, controls_height) =
                answer_button_layout(&control_sizes, control_width);
            let is_expanded = expanded.contains(&session.key());
            let history = self
                .ivars()
                .message_history
                .borrow()
                .get(&session.key())
                .cloned()
                .unwrap_or_default();
            let mut earlier = history
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            if !history.anchor.is_empty()
                && (is_question || history.anchor.trim_end() != session.last_message.trim_end())
                && !history.messages.is_empty()
            {
                earlier.push_str("\n\n");
                earlier.push_str(history.anchor.trim_end());
            }
            let mut earlier_label = None;
            let mut earlier_height = 0.0;
            if is_question
                && !earlier.is_empty()
                && (is_expanded
                    || reveal
                        .get(&session.key())
                        .is_some_and(|progress| *progress > 0.0))
            {
                let label =
                    NSTextField::wrappingLabelWithString(&NSString::from_str(&earlier), self.mtm());
                label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
                label.setTextColor(Some(&detail_text_color(false)));
                label.setSelectable(true);
                label.setMaximumNumberOfLines(0);
                let height = label
                    .cell()
                    .map(|cell| {
                        cell.cellSizeForBounds(rect(0.0, 0.0, 338.0, 1000000.0))
                            .height
                            .ceil()
                    })
                    .unwrap_or(22.0)
                    .max(22.0);
                label.setFrame(rect(18.0, MESSAGE_TOP_GAP, 338.0, height));
                earlier_height = height + 8.0;
                earlier_label = Some(label);
            }
            let mut first_message_height = 22.0;
            let detail = if is_expanded
                || reveal
                    .get(&session.key())
                    .is_some_and(|progress| *progress > 0.0)
            {
                let text = session
                    .approval
                    .as_ref()
                    .map(|request| request.body.as_str())
                    .unwrap_or(&session.last_message)
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
                    .unwrap_or(text);
                let first_label = NSTextField::wrappingLabelWithString(
                    &NSString::from_str(first_text.trim_end()),
                    self.mtm(),
                );
                first_label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
                first_label.setMaximumNumberOfLines(0);
                let first_width = if history.messages.is_empty() {
                    content_width
                } else {
                    338.0
                };
                first_message_height = first_label
                    .cell()
                    .map(|cell| {
                        cell.cellSizeForBounds(rect(0.0, 0.0, first_width, 1000000.0))
                            .height
                            .ceil()
                    })
                    .unwrap_or(22.0)
                    .max(22.0);
                let combined = if !is_question && !earlier.is_empty() {
                    if session.last_message.trim().is_empty() {
                        earlier.clone()
                    } else {
                        format!("{earlier}\n\n{text}")
                    }
                } else {
                    text.to_owned()
                };
                let label = NSTextField::wrappingLabelWithString(
                    &NSString::from_str(&combined),
                    self.mtm(),
                );
                label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
                label.setTextColor(Some(&detail_text_color(session.approval.is_some())));
                label.setSelectable(true);
                label.setMaximumNumberOfLines(0);
                let height = label
                    .cell()
                    .map(|cell| {
                        cell.cellSizeForBounds(rect(0.0, 0.0, content_width, 1000000.0))
                            .height
                            .ceil()
                    })
                    .unwrap_or(60.0)
                    .max(22.0);
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
                        + 16.0
                        + controls_height
                        + 2.0 * question_padding
                        + metadata_gap
                        + earlier_height
                })
                .unwrap_or(0.0);
            let height = SESSION_HEADER_HEIGHT + detail_height;
            let row = SessionRowView::create(self.mtm(), rect(0.0, y, 384.0, height));
            if session_gap > 0.0 {
                // Draw in the existing gap. Pinning to the row's top below keeps
                // the divider aligned while either session expands or collapses.
                self.separator(&row, height + session_gap / 2.0 - 0.5, 346.0);
            }
            let text_width = 294.0;
            let title = self.label(
                &row,
                &session.title,
                rect(
                    18.0,
                    height - SESSION_HEADER_HEIGHT / 2.0 - 11.0,
                    text_width,
                    22.0,
                ),
                14.0,
                0.23,
                false,
            );
            center_label(&title, height - SESSION_HEADER_HEIGHT / 2.0);
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
            let working_icon = self.status_icon(
                &row,
                session.activity,
                height - SESSION_HEADER_HEIGHT / 2.0 - 8.0,
            );
            let open = self.action_button(
                &row,
                &session.title,
                sel!(openSession:),
                &session.key(),
                rect(
                    14.0,
                    height - SESSION_HEADER_HEIGHT,
                    text_width + 8.0,
                    SESSION_HEADER_HEIGHT,
                ),
                true,
            );
            open.setToolTip(Some(&NSString::from_str(&format!(
                "{} · {} · {}\n{}\n{}",
                session.title,
                session.agent_label(),
                session.client.label(),
                session.project_path().display(),
                if session.project_path().exists() {
                    "Open project in VS Code"
                } else {
                    "Folder no longer exists. Switch to VS Code."
                }
            ))));
            let arrow = self.action_button(
                &row,
                "",
                sel!(expandSession:),
                &session.key(),
                rect(
                    348.0,
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
                    msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: rect(BUTTON_HOVER_PADDING, BUTTON_HOVER_PADDING, 26.0, 24.0)]
                };
                let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                icon.setFrame(rect(6.0, 5.0, 14.0, 14.0));
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
            if let Some((label, text_height, metadata, _)) = detail {
                // The clip only reveals content; the main list is the sole scroller.
                // SAFETY: NSClipView initializes this ivar-free subclass.
                let clip: Retained<RevealClipView> = unsafe {
                    msg_send![RevealClipView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 384.0, detail_height)]
                };
                clip.setDrawsBackground(false);
                // SAFETY: NSView's initializer is valid for this ivar-free subclass.
                let document: Retained<SessionListView> = unsafe {
                    msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 384.0, detail_height)]
                };
                let question_top = MESSAGE_TOP_GAP + earlier_height;
                let message_top = question_top + question_padding;
                let message_end = message_top + text_height;
                let footer_y =
                    message_end + 8.0 + controls_height + question_padding + metadata_gap;
                if is_question {
                    let frame = rect(
                        18.0,
                        question_top,
                        QUESTION_CARD_WIDTH,
                        footer_y - question_top - metadata_gap,
                    );
                    // SAFETY: Decorative view ignores hit testing, preserving text selection,
                    // answer buttons and scrolling across the entire question card.
                    let decoration: Retained<StatusContentView> = unsafe {
                        msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: frame]
                    };
                    let card = NSBox::initWithFrame(
                        NSBox::alloc(self.mtm()),
                        rect(0.0, 0.0, frame.size.width, frame.size.height),
                    );
                    card.setBoxType(NSBoxType::Custom);
                    card.setTitlePosition(NSTitlePosition::NoTitle);
                    card.setContentViewMargins(NSSize::new(0.0, 0.0));
                    card.setCornerRadius(17.0);
                    card.setBorderWidth(1.0);
                    let tint = detail_text_color(true);
                    card.setBorderColor(&tint.colorWithAlphaComponent(0.24));
                    card.setFillColor(&footer_fill_color());
                    decoration.addSubview(&card);
                    document.addSubview(&decoration);
                }
                metadata.setFrameOrigin(NSPoint::new(18.0, footer_y));
                document.addSubview(&metadata);
                label.setFrameOrigin(NSPoint::new(content_x, message_top));
                document.addSubview(&label);
                if let Some(label) = earlier_label {
                    document.addSubview(&label);
                }
                let first_top = if earlier_height > 0.0 {
                    MESSAGE_TOP_GAP
                } else {
                    message_top
                };
                let gutter_frame = rect(1.0, first_top, 16.0, first_message_height);
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
                    gutter: Some(GutterHover {
                        message: rect(
                            if earlier_height > 0.0 {
                                18.0
                            } else {
                                content_x
                            },
                            first_top,
                            if earlier_height > 0.0 {
                                338.0
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
                            session.authoritative
                                && !matches!(
                                    self.monitor().snapshot().submissions.get(&key),
                                    Some(
                                        crate::actors::monitor::Submission::Sending
                                            | crate::actors::monitor::Submission::Reconciling
                                    )
                                ),
                        );
                        button.setBezelStyle(NSBezelStyle::Push);
                        button.setFont(Some(&NSFont::systemFontOfSize(12.0)));
                        if let Some(cell) = button.cell() {
                            cell.setWraps(true);
                            cell.setUsesSingleLineMode(false);
                            cell.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
                        }
                        button.setToolTip(Some(&NSString::from_str(
                            &if control.detail.is_empty() {
                                control.label.clone()
                            } else {
                                format!("{}\n{}", control.label, control.detail)
                            },
                        )));
                        button.setEnabled(
                            session.authoritative
                                && !self.ivars().responding.borrow().contains(&key),
                        );
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
                gap: session_gap,
                base_height: SESSION_HEADER_HEIGHT,
                detail_height,
                detail: detail_clip,
                arrow: arrow_icon,
                working_icon,
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
            .is_some_and(|id| !layout.iter().any(|row| row.id.as_ref() == Some(id)))
        {
            self.ivars().keyboard_focus.borrow_mut().take();
        }
        if !self.ivars().filter_query.borrow().is_empty()
            && self.ivars().keyboard_focus.borrow().is_none()
        {
            *self.ivars().keyboard_focus.borrow_mut() =
                layout.iter().find_map(|row| row.id.clone());
        }
        *self.ivars().layout.borrow_mut() = layout;
        self.layout_rows();
        self.advance_working_icons();
    }

    fn sync_footer_extension(&self) {
        let state = self.ivars();
        if !state.menu_open.get() {
            return;
        }
        let Some(footer) = state.footer.get() else {
            return;
        };
        let Some(content) = footer.window().and_then(|window| window.contentView()) else {
            return;
        };
        let frame = content.convertRect_fromView(footer.bounds(), Some(footer));
        let gap = footer_gap(content.bounds(), frame, content.isFlipped());
        let extension = state.footer_extension.get_or_init(|| {
            // SAFETY: Decorative ivar-free NSView; hitTest returns nil so the
            // native menu still owns mouse input in its bottom padding.
            let view: Retained<StatusContentView> =
                unsafe { msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: gap] };
            let fill = NSBox::initWithFrame(
                NSBox::alloc(self.mtm()),
                rect(0.0, 0.0, gap.size.width, gap.size.height),
            );
            fill.setBoxType(NSBoxType::Custom);
            fill.setTitlePosition(NSTitlePosition::NoTitle);
            fill.setBorderWidth(0.0);
            fill.setCornerRadius(0.0);
            fill.setContentViewMargins(NSSize::new(0.0, 0.0));
            fill.setFillColor(state.footer_fill.get_or_init(footer_fill_color));
            view.addSubview(&fill);
            FooterExtension { view, fill }
        });
        extension.view.setHidden(gap.size.height <= 0.0);
        // SAFETY: View hierarchy access is confined to the main thread.
        if !unsafe { extension.view.superview() }
            .is_some_and(|parent| std::ptr::eq(&*parent, &*content))
        {
            content.addSubview(&extension.view);
        }
        if extension.view.frame() != gap {
            extension.view.setFrame(gap);
            extension
                .fill
                .setFrame(rect(0.0, 0.0, gap.size.width, gap.size.height));
        }
    }

    fn render_footer(&self) {
        let state = self.ivars();
        let usage = state.usage.borrow();
        let usage = usage.as_ref();
        let query = state.filter_query.borrow().clone();
        let filter_rows = usize::from(!query.is_empty());
        let Some(footer) = state.footer.get() else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        const VERSION_HEIGHT: f64 = 32.0;
        const LEFT: f64 = 20.0;
        let rows = usage.map(|usage| usage.windows.len()).unwrap_or(1);
        let height = (rows + filter_rows) as f64 * 24.0 + 12.0 + VERSION_HEIGHT;
        let resets: Vec<_> = usage
            .into_iter()
            .flat_map(|usage| usage.windows.iter())
            .map(|window| {
                window
                    .resets_at
                    .map(|reset| {
                        let formatter = NSDateFormatter::new();
                        formatter.setDateFormat(Some(if window.minutes >= 1440 {
                            ns_string!("MMM d")
                        } else {
                            ns_string!("HH:mm")
                        }));
                        formatter
                            .stringFromDate(&NSDate::dateWithTimeIntervalSince1970(reset as f64))
                            .to_string()
                    })
                    .unwrap_or_else(|| "—".into())
            })
            .collect();
        let remaining: Vec<_> = usage
            .into_iter()
            .flat_map(|usage| usage.windows.iter())
            .map(|window| {
                window
                    .remaining(now)
                    .map(|value| format!("{value}%"))
                    .unwrap_or_else(|| "—".into())
            })
            .collect();
        let text_width = |text: &str| {
            let label = NSTextField::labelWithString(&NSString::from_str(text), self.mtm());
            label.setFont(Some(&NSFont::systemFontOfSize(12.0)));
            label.sizeToFit();
            label.frame().size.width.ceil()
        };
        let date_width = resets
            .iter()
            .map(|text| text_width(text))
            .fold(text_width("—"), f64::max);
        let percent_width = remaining
            .iter()
            .map(|text| text_width(text))
            .fold(text_width("—"), f64::max);
        let date_x = 372.0 - date_width;
        let percent_x = date_x - 12.0 - percent_width;
        let changed = state.footer_height.replace(height) != height;
        let children = footer.subviews();
        for index in 0..children.count() {
            children.objectAtIndex(index).removeFromSuperview();
        }
        footer.setFrame(rect(0.0, 0.0, 392.0, height));
        // A denser system-color wash reduces the glass/background influence
        // in the footer while leaving AppKit in charge of the menu material.
        let background = NSBox::initWithFrame(NSBox::alloc(self.mtm()), footer.bounds());
        background.setBoxType(NSBoxType::Custom);
        background.setTitlePosition(NSTitlePosition::NoTitle);
        background.setBorderWidth(0.0);
        background.setCornerRadius(0.0);
        background.setContentViewMargins(NSSize::new(0.0, 0.0));
        background.setFillColor(state.footer_fill.get_or_init(footer_fill_color));
        footer.addSubview(&background);
        let separator = NSBox::initWithFrame(
            NSBox::alloc(self.mtm()),
            rect(0.0, height - 1.0, 392.0, 1.0),
        );
        separator.setBoxType(NSBoxType::Separator);
        footer.addSubview(&separator);
        if filter_rows > 0 {
            let label = self.label(
                footer,
                "",
                rect(LEFT, height - 32.0, 330.0, 20.0),
                12.0,
                0.0,
                false,
            );
            label.setStringValue(&NSString::from_str(&format!("Filter: {query}")));
            label.setToolTip(Some(&NSString::from_str(&query)));
        }
        if let Some(usage) = usage {
            for (index, window) in usage.windows.iter().enumerate() {
                let y = height - 8.0 - 24.0 * (index + 1 + filter_rows) as f64;
                let name = self.label(
                    footer,
                    &format!("Codex {}", window.label()),
                    rect(LEFT, y, (percent_x - LEFT - 12.0).max(0.0), 20.0),
                    12.0,
                    0.0,
                    true,
                );
                let percent = self.label(
                    footer,
                    &remaining[index],
                    rect(percent_x, y, percent_width, 20.0),
                    12.0,
                    0.0,
                    true,
                );
                percent.setAlignment(NSTextAlignment::Right);
                let date = self.label(
                    footer,
                    &resets[index],
                    rect(date_x, y, date_width, 20.0),
                    12.0,
                    0.0,
                    true,
                );
                date.setAlignment(NSTextAlignment::Right);
                let tooltip = NSString::from_str(&format!(
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
                        .unwrap_or_else(|| "Unknown".into()),
                    usage.observed_at,
                    if window.remaining(now).is_none() {
                        "\nWaiting for Codex to report the new balance."
                    } else {
                        ""
                    },
                ));
                for label in [name, percent, date] {
                    center_label(&label, y + 10.0);
                    label.setToolTip(Some(&tooltip));
                }
            }
        } else {
            let label = self.label(
                footer,
                "Codex usage",
                rect(LEFT, VERSION_HEIGHT + 4.0, 220.0, 20.0),
                12.0,
                0.0,
                true,
            );
            label.setToolTip(Some(ns_string!(
                "Usage will appear after Codex reports its account limits."
            )));
            let value = self.label(
                footer,
                "—",
                rect(date_x, VERSION_HEIGHT + 4.0, date_width, 20.0),
                12.0,
                0.0,
                true,
            );
            value.setAlignment(NSTextAlignment::Right);
        }
        let version = self.label(
            footer,
            &format!("v{}", core::VERSION),
            rect(20.0, 8.0, 180.0, 16.0),
            10.0,
            0.0,
            true,
        );
        version.setTextColor(Some(&NSColor::tertiaryLabelColor()));
        version.setToolTip(Some(&NSString::from_str(&format!(
            "AI Agent Monitor {}",
            core::VERSION
        ))));
        center_label(&version, 16.0);
        if changed {
            self.layout_rows();
        }
    }

    fn resize_content(&self, content_height: f64) -> f64 {
        let state = self.ivars();
        state.content_height.set(content_height);
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
        let footer_height = state.footer_height.get();
        let height = (content_height + footer_height + 8.0)
            .max(64.0 + footer_height)
            .min(maximum);
        let viewport = height - footer_height - 8.0;
        NSAnimationContext::beginGrouping();
        let context = NSAnimationContext::currentContext();
        context.setDuration(0.0);
        context.setAllowsImplicitAnimation(false);
        let mut resized = false;
        if let Some(root) = state.root.get() {
            let size = NSSize::new(392.0, height);
            if root.frame().size != size {
                root.setFrameSize(size);
                resized = true;
            }
        }
        if let Some(scroll) = state.scroll.get() {
            scroll.setFrame(rect(4.0, footer_height + 4.0, 384.0, viewport));
        }
        if let Some(toolbar) = state.toolbar.get() {
            toolbar.setFrame(rect(0.0, 0.0, 392.0, 32.0));
        }
        if let Some(caption) = state.caption.get() {
            caption.setFrame(rect(20.0, height - 27.0, 286.0, 18.0));
        }
        NSAnimationContext::endGrouping();
        if resized
            && state.menu_open.get()
            && let Some(menu) = state.menu.get()
        {
            menu.update();
        }
        viewport
    }

    fn refresh(&self) {
        let _ = self
            .monitor()
            .send(crate::actors::monitor::Command::Refresh);
    }
    fn receive_sessions(&self) {
        let state = self.ivars();
        let snapshot = self.monitor().snapshot();
        if state.revision.replace(snapshot.revision) == snapshot.revision {
            return;
        }
        *state.usage.borrow_mut() = snapshot.usage.clone();
        self.render_footer();
        for form in state.forms.borrow().iter() {
            form.reconcile(&snapshot);
        }
        {
            let mut histories = state.message_history.borrow_mut();
            for (key, history) in histories.iter_mut() {
                if let Some(value) = snapshot.history.get(key)
                    && value.generation == history.ticket
                {
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
        state.filter_dirty.set(true);
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
        self.refresh_settings();
        self.refresh_telegram_settings();
        self.refresh_integration_settings();
        *state.all.borrow_mut() = sessions;
        self.filter();
        if snapshot.stopping {
            crate::macos::native::stop_application(self.mtm());
        }
        self.update_badge(snapshot.pending_count());
    }

    fn filter(&self) {
        let state = self.ivars();
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
                    || a.client != b.client
                    || a.activity != b.activity
                    || a.last_message != b.last_message
                    || a.approval != b.approval
            });
        drop(previous);
        if changed {
            self.render_rows(&visible);
            *state.visible.borrow_mut() = visible;
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

#[cfg(test)]
mod close_tests {
    use super::*;

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
                rect(72.0, 0.0, 80.0, 28.0),
                rect(164.0, 0.0, 100.0, 28.0)
            ]
        );
        assert_eq!(height, 34.0);
        let (column, height) = answer_button_layout(&sizes, 240.0);
        assert_eq!(
            column,
            vec![
                rect(0.0, 0.0, 60.0, 28.0),
                rect(0.0, 40.0, 80.0, 28.0),
                rect(0.0, 80.0, 100.0, 28.0)
            ]
        );
        assert_eq!(height, 114.0);
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
        Session {
            authoritative: true,
            interaction_data: Default::default(),
            target: None,
            agent: "codex",
            id: id.into(),
            title: id.into(),
            cwd: "/project".into(),
            workspace: None,
            git_branch: None,
            updated_at: 0,
            client: Client::VSCode,
            activity: if approval.is_some() {
                Activity::Confirmation
            } else {
                Activity::Idle
            },
            last_message: String::new(),
            approval: approval.map(|path| crate::agents::sessions::Approval {
                id: path.into(),
                body: "Подтверждение".into(),
                expires: u64::MAX,
                can_allow: true,
            }),
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
