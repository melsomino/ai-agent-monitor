//! Resident menu bar item and native sessions menu.
use crate::{
    core, motion, native, pending, pointer,
    sessions::{self, Activity, Client, Session},
    settings,
};
use anyhow::{Context, Result};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
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
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static RESIDENT: AtomicBool = AtomicBool::new(false);
thread_local! { static CONTROLLER: RefCell<Option<Retained<MenuController>>> = const { RefCell::new(None) }; }

pub fn is_resident() -> bool {
    RESIDENT.load(Ordering::Relaxed)
}

#[derive(Default)]
struct State {
    status: OnceCell<Retained<NSStatusItem>>,
    status_content: OnceCell<Retained<StatusContentView>>,
    status_badge: OnceCell<Retained<NSBox>>,
    status_count: OnceCell<Retained<NSTextField>>,
    status_working: OnceCell<Retained<WorkingIconView>>,
    status_timer: RefCell<Option<Retained<NSTimer>>>,
    settings_window: OnceCell<Retained<NSWindow>>,
    login_switch: OnceCell<Retained<NSSwitch>>,
    animation_switch: OnceCell<Retained<NSSwitch>>,
    login_note: OnceCell<Retained<NSTextField>>,
    login_settings_button: OnceCell<Retained<NSButton>>,
    menu: OnceCell<Retained<NSMenu>>,
    menu_item: OnceCell<Retained<NSMenuItem>>,
    menu_open: Cell<bool>,
    incoming: RefCell<Option<std::sync::mpsc::Receiver<Result<Vec<Session>>>>>,
    last_refresh: Cell<Option<Instant>>,
    list: OnceCell<Retained<SessionListView>>,
    root: OnceCell<Retained<PanelView>>,
    toolbar: OnceCell<Retained<NSView>>,
    scroll: OnceCell<Retained<SessionScrollView>>,
    content_height: Cell<f64>,
    expanded: RefCell<HashSet<String>>,
    reveal: RefCell<HashMap<String, f64>>,
    motion: RefCell<Option<motion::Disclosure>>,
    animation_epoch: Cell<u64>,
    working_epoch: OnceCell<Instant>,
    layout: RefCell<Vec<LayoutRow>>,
    caption: OnceCell<Retained<NSTextField>>,
    all: RefCell<Vec<Session>>,
    visible: RefCell<Vec<Session>>,
    lock: OnceCell<File>,
    refreshing: Cell<bool>,
    routing_scroll: Cell<bool>,
    buttons: RefCell<Vec<ButtonHover>>,
    toolbar_buttons: RefCell<Vec<ButtonHover>>,
    hovered: Cell<Option<pointer::Target>>,
    pointer_timer: RefCell<Option<Retained<NSTimer>>>,
    last_pointer: Cell<Option<NSPoint>>,
}

impl State {
    fn reset_disclosures(&self) {
        self.animation_epoch
            .set(self.animation_epoch.get().wrapping_add(1));
        self.motion.borrow_mut().take();
        self.expanded.borrow_mut().clear();
        self.reveal.borrow_mut().clear();
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. UI and all state are
    // confined to the main thread; retained controls outlive their weak targets.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    struct MenuController;

    unsafe impl NSObjectProtocol for MenuController {}
    // SAFETY: The retained controller handles the window's main-thread delegate callback.
    unsafe impl NSWindowDelegate for MenuController {
        #[unsafe(method(windowDidBecomeKey:))]
        fn settings_became_key(&self, _notification: &NSNotification) {
            self.refresh_settings();
        }
    }
    // SAFETY: The retained main-thread controller implements NSMenuDelegate.
    unsafe impl NSMenuDelegate for MenuController {
        #[unsafe(method(menuWillOpen:))]
        fn will_open(&self, _menu: &NSMenu) {
            self.ivars().menu_open.set(true);
            self.receive_sessions();
            self.refresh();
            self.start_pointer_tracking();
        }
        #[unsafe(method(menuDidClose:))]
        fn did_close(&self, _menu: &NSMenu) {
            self.ivars().menu_open.set(false);
            self.close_panel();
        }
    }

    impl MenuController {
        #[unsafe(method(animateStatus:))]
        fn animate_status(&self, _timer: &NSTimer) {
            if let Some(icon) = self.ivars().status_working.get() {
                icon.ivars().angle.set(self.working_angle());
                NSView::setNeedsDisplay(icon, true);
            }
        }

        #[unsafe(method(pollPointer:))]
        fn poll_pointer(&self, _timer: &NSTimer) {
            self.receive_sessions();
            self.advance_disclosure();
            self.advance_working_icons();
            if self.ivars().last_refresh.get().is_none_or(|last| last.elapsed() >= Duration::from_secs(2)) { self.refresh(); }
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

        #[unsafe(method(allowSession:))]
        fn allow_session(&self, sender: Option<&NSButton>) {
            let Some(id) = sender.and_then(|button| button.identifier()).map(|id| id.to_string()) else { return; };
            let request = self.ivars().all.borrow().iter().filter_map(|session| session.approval.as_ref()).find(|request| request.response.to_string_lossy() == id).cloned();
            if let Some(request) = request {
                let allowed = pending::allow(&request);
                if allowed { native::dismiss_approval(&request.response); }
                if let Some(button) = sender { button.setEnabled(false); button.setTitle(if allowed { ns_string!("Разрешено") } else { ns_string!("Запрос завершён") }); }
                self.update_badge(pending::list().len());
                self.refresh();
            }
        }

        #[unsafe(method(openSession:))]
        fn open_session(&self, sender: Option<&NSButton>) {
            let Some(id) = sender.and_then(|button| button.identifier()) else { return; };
            let data = self.ivars().all.borrow();
            if let Some(session) = data.iter().find(|session| session.id == id.to_string()) {
                let target = core::vscode_target(&serde_json::json!({"cwd":session.cwd}),
                    &core::home().join("Library/Application Support/Code/User/workspaceStorage"));
                self.dismiss_menu();
                native::open_target(&target);
            }
        }

        #[unsafe(method(settings:))]
        fn settings(&self, _sender: Option<&AnyObject>) {
            self.dismiss_menu();
            self.show_settings();
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, sender: &NSSwitch) {
            let result = settings::set_launch_at_login(sender.state() == NSControlStateValueOn);
            self.refresh_settings();
            if let Err(error) = result && let Some(note) = self.ivars().login_note.get() {
                let message = NSString::from_str(&format!("Не удалось изменить автозапуск: {error}"));
                note.setStringValue(&message);
                note.setToolTip(Some(&message));
                note.setTextColor(Some(&NSColor::systemRedColor()));
            }
        }

        #[unsafe(method(toggleMenuAnimation:))]
        fn toggle_menu_animation(&self, sender: &NSSwitch) {
            settings::set_menu_animation(sender.state() == NSControlStateValueOn);
            self.update_badge(pending::list().len());
        }

        #[unsafe(method(openLoginSettings:))]
        fn open_login_settings(&self, _sender: Option<&AnyObject>) {
            settings::open_login_settings();
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) { NSApplication::sharedApplication(self.mtm()).terminate(None); }
    }

);

#[derive(Debug)]
struct WorkingIconState {
    angle: Cell<f64>,
    badge: bool,
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
            if self.ivars().badge {
                NSColor::colorWithSRGBRed_green_blue_alpha(0.04, 0.25, 0.13, 1.0).setFill();
                NSBezierPath::bezierPathWithOvalInRect(bounds).fill();
            }
            let transform = NSAffineTransform::transform();
            transform.translateXBy_yBy(x, y);
            transform.rotateByDegrees(self.ivars().angle.get());
            transform.translateXBy_yBy(-x, -y);
            transform.concat();
            if self.ivars().badge {
                // Draw both opposed arrows around the exact same center as the
                // background, avoiding independent symbol padding and view
                // geometry for this small badge.
                let color = NSColor::colorWithSRGBRed_green_blue_alpha(0.65, 1.0, 0.72, 1.0);
                color.setStroke();
                color.setFill();
                let center = NSPoint::new(x, y);
                for start in [-20.0_f64, 160.0] {
                    let end = start + 130.0;
                    let arc = NSBezierPath::bezierPath();
                    arc.setLineWidth(1.2);
                    arc.setLineCapStyle(NSLineCapStyle::Round);
                    arc.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(
                        center, 3.6, start, end, false,
                    );
                    arc.stroke();
                    let (sin, cos) = end.to_radians().sin_cos();
                    let point = NSPoint::new(x + 3.6 * cos, y + 3.6 * sin);
                    let head = NSBezierPath::bezierPath();
                    head.moveToPoint(NSPoint::new(point.x - sin * 1.8, point.y + cos * 1.8));
                    head.lineToPoint(NSPoint::new(point.x + sin * 0.5 + cos * 1.2, point.y - cos * 0.5 + sin * 1.2));
                    head.lineToPoint(NSPoint::new(point.x + sin * 0.5 - cos * 1.2, point.y - cos * 0.5 - sin * 1.2));
                    head.closePath();
                    head.fill();
                }
            } else {
                // SAFETY: Invoke NSImageView's drawing within the saved graphics state.
                unsafe { let _: () = msg_send![super(self), drawRect: dirty]; }
            }
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

fn pointer_changed() {
    CONTROLLER.with(|slot| {
        if let Ok(slot) = slot.try_borrow()
            && let Some(controller) = slot.as_ref()
        {
            controller.update_hover();
        }
    });
}
fn route_wheel(event: &NSEvent) {
    CONTROLLER.with(|slot| {
        if let Ok(slot) = slot.try_borrow()
            && let Some(controller) = slot.as_ref()
        {
            controller.route_scroll(event);
        }
    });
}

define_class!(
    // SAFETY: A main-thread NSView that routes wheel input from the window margins.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    struct PanelView;
    unsafe impl NSObjectProtocol for PanelView {}
    impl PanelView {
        #[unsafe(method(scrollWheel:))]
        fn wheel(&self, event: &NSEvent) { route_wheel(event); }
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
            pointer_changed();
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
        fn wheel(&self, event: &NSEvent) { route_wheel(event); }
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
        let background =
            hover_background(mtm, rect(4.0, frame.size.height - 46.0, 364.0, 44.0), 8.0);
        row.addSubview(&background);
        row.ivars()
            .background
            .set(background)
            .expect("background initialized once");
        row
    }
    fn set_hovered(&self, active: bool) {
        if self.ivars().active.replace(active) != active
            && let Some(background) = self.ivars().background.get()
        {
            paint_hover(background, if active { 1.0 } else { 0.0 });
        }
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

struct ButtonHover {
    button: Retained<NSButton>,
    background: Retained<NSBox>,
    base_alpha: f64,
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

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
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

    fn create(mtm: MainThreadMarker, lock: File) -> Retained<Self> {
        let allocated = Self::alloc(mtm).set_ivars(State::default());
        // SAFETY: NSObject's initializer is valid for this subclass and its ivars.
        let this: Retained<Self> = unsafe { msg_send![super(allocated), init] };
        this.ivars().lock.set(lock).expect("lock initialized once");
        this.build_menu();
        this.build_status_item();
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
                    rect(0.0, 0.0, 460.0, 216.0),
                    NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                    NSBackingStoreType::Buffered,
                    false,
                );
                window.setReleasedWhenClosed(false);
                window
            };
            window.setTitle(ns_string!("Настройки Codex Notify"));
            window.setDelegate(Some(ProtocolObject::from_ref(self)));
            let view =
                NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, 0.0, 460.0, 216.0));
            self.label(
                &view,
                "Запускать при входе в macOS",
                rect(24.0, 166.0, 350.0, 22.0),
                13.0,
                0.23,
                false,
            );
            let note = self.label(
                &view,
                "Открывать Codex Notify автоматически",
                rect(24.0, 144.0, 370.0, 18.0),
                11.0,
                0.0,
                true,
            );
            let separator =
                NSBox::initWithFrame(NSBox::alloc(self.mtm()), rect(24.0, 126.0, 412.0, 1.0));
            separator.setBoxType(NSBoxType::Separator);
            view.addSubview(&separator);
            self.label(
                &view,
                "Анимация в менюбаре",
                rect(24.0, 88.0, 350.0, 22.0),
                13.0,
                0.23,
                false,
            );
            self.label(
                &view,
                "Вращать стрелки, пока агент работает",
                rect(24.0, 66.0, 350.0, 18.0),
                11.0,
                0.0,
                true,
            );
            for (y, action, storage, title) in [
                (
                    163.0,
                    sel!(toggleLogin:),
                    &state.login_switch,
                    ns_string!("Запускать при входе в macOS"),
                ),
                (
                    85.0,
                    sel!(toggleMenuAnimation:),
                    &state.animation_switch,
                    ns_string!("Анимация в менюбаре"),
                ),
            ] {
                let toggle = NSSwitch::initWithFrame(
                    NSSwitch::alloc(self.mtm()),
                    rect(396.0, y, 40.0, 24.0),
                );
                // SAFETY: The controller implements these selectors and outlives the controls.
                unsafe {
                    toggle.setTarget(Some(self));
                    toggle.setAction(Some(action));
                    let _: () = msg_send![&*toggle, setAccessibilityLabel: title];
                }
                view.addSubview(&toggle);
                storage
                    .set(toggle)
                    .expect("settings switch initialized once");
            }
            // SAFETY: This target and selector remain valid for the window's lifetime.
            let login_button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    ns_string!("Открыть настройки входа…"),
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
            window.setContentView(Some(&view));
            window.center();
            window
        });
        self.refresh_settings();
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

    fn refresh_settings(&self) {
        let state = self.ivars();
        let status = settings::login_status();
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
            toggle.setState(if settings::menu_animation() {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        if let Some(note) = state.login_note.get() {
            note.setToolTip(None);
            note.setTextColor(Some(&NSColor::secondaryLabelColor()));
            note.setStringValue(ns_string!("Открывать Codex Notify автоматически"));
            if status == settings::LoginStatus::RequiresApproval {
                note.setStringValue(ns_string!("Разрешите автозапуск в настройках macOS"));
            } else if status == settings::LoginStatus::Unavailable {
                note.setStringValue(ns_string!(
                    "Автозапуск недоступен для этой копии приложения"
                ));
            }
        }
        if let Some(button) = state.login_settings_button.get() {
            button.setHidden(status != settings::LoginStatus::RequiresApproval);
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
            let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("sparkles.rectangle.stack"),
                Some(ns_string!("Сессии Codex")),
            )
            .or_else(|| {
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    ns_string!("rectangle.stack"),
                    Some(ns_string!("Сессии Codex")),
                )
            });
            if let Some(image) = image {
                let configuration =
                    NSImageSymbolConfiguration::configurationWithPointSize_weight(21.0, 0.0);
                let image = image
                    .imageWithSymbolConfiguration(&configuration)
                    .unwrap_or(image);
                image.setTemplate(true);
                let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                image.setSize(NSSize::new(26.0, 22.0));
                icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                icon.setFrame(rect(0.0, height / 2.0 - 11.0, 26.0, 22.0));
                icon.setAutoresizingMask(
                    NSAutoresizingMaskOptions::ViewMinYMargin
                        | NSAutoresizingMaskOptions::ViewMaxYMargin,
                );
                icon.setContentTintColor(Some(&NSColor::labelColor()));
                content.addSubview(&icon);
            }
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
            let allocated = WorkingIconView::alloc(self.mtm()).set_ivars(WorkingIconState {
                angle: Cell::new(0.0),
                badge: true,
            });
            // SAFETY: The image view's ivars are initialized before its designated initializer.
            let icon: Retained<WorkingIconView> =
                unsafe { msg_send![super(allocated), initWithFrame: rect(12.0, 1.0, 14.0, 14.0)] };
            icon.setHidden(true);
            content.addSubview(&icon);
            self.ivars()
                .status_working
                .set(icon)
                .expect("working badge initialized once");
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
        self.update_badge(pending::list().len());
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
            && settings::menu_animation()
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
        if let Some(icon) = state.status_working.get() {
            icon.setHidden(!working);
            icon.setFrame(rect(12.0, badge_y, 14.0, 14.0));
            icon.ivars()
                .angle
                .set(if animate { self.working_angle() } else { 0.0 });
            NSView::setNeedsDisplay(icon, true);
        }
        if animate && state.status_timer.borrow().is_none() {
            // SAFETY: The resident controller implements animateStatus: and
            // outlives the timer; common modes include native menu tracking.
            let timer = unsafe {
                NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    1.0 / 60.0,
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
            "Сессии Codex · Работает".to_owned()
        } else if count == 0 {
            "Сессии Codex".to_owned()
        } else {
            format!("Сессии Codex · Ожидают подтверждения: {count}")
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
        let line = NSBox::initWithFrame(NSBox::alloc(self.mtm()), rect(20.0, y, width, 1.0));
        line.setBoxType(NSBoxType::Separator);
        view.addSubview(&line);
    }

    fn build_menu(&self) {
        let mtm = self.mtm();
        // NSMenu supplies the system background and owns presentation/dismissal.
        // SAFETY: NSView's initializer is valid for this ivar-free subclass.
        let view: Retained<PanelView> = unsafe {
            msg_send![PanelView::alloc(mtm), initWithFrame: rect(0.0, 0.0, 392.0, 476.0)]
        };
        view.setWantsLayer(true);
        // SAFETY: OverlayView has no ivars or custom initialization.
        let toolbar: Retained<OverlayView> = unsafe {
            msg_send![OverlayView::alloc(mtm), initWithFrame: rect(0.0, 438.0, 392.0, 32.0)]
        };
        let caption = self.label(&toolbar, "", rect(20.0, 7.0, 286.0, 18.0), 10.0, 0.0, true);
        caption.setHidden(true);
        for (symbol, action, x, tooltip) in [
            ("gearshape", sel!(settings:), 324.0, "Настройки"),
            ("power", sel!(quit:), 356.0, "Завершить Codex Notify"),
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
        view.addSubview(&toolbar);
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
        button.setFrame(frame);
        button.setBordered(false);
        button.setTransparent(transparent);
        button.setIdentifier(Some(&NSString::from_str(id)));
        if !transparent {
            let background = hover_background(self.mtm(), frame, 6.0);
            let base_alpha = if action == sel!(allowSession:) {
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
            });
        }
        view.addSubview(&button);
        button
    }

    fn client_icon(&self, row: &NSView, client: Client, y: f64) {
        let app = match client {
            Client::VSCode => Some("/Applications/Visual Studio Code.app"),
            Client::Desktop => Some("/Applications/ChatGPT.app"),
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
            icon.setFrame(rect(6.0, y, 26.0, 26.0));
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
            let frame = rect(311.0, y + 1.0, 14.0, 14.0);
            if activity == Activity::Working {
                let allocated = WorkingIconView::alloc(self.mtm()).set_ivars(WorkingIconState {
                    angle: Cell::new(0.0),
                    badge: false,
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
        self.stop_pointer_tracking();
        let state = self.ivars();
        state.reset_disclosures();
        self.render_rows(&state.all.borrow());
        if let Some(scroll) = state.scroll.get() {
            scroll.contentView().scrollToPoint(NSPoint::new(0.0, 0.0));
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
        if state.hovered.replace(target) == target {
            return;
        }
        for (index, item) in state
            .layout
            .borrow()
            .iter()
            .filter(|item| item.row.is_some())
            .enumerate()
        {
            if let Some(row) = &item.row {
                row.set_hovered(target == Some(pointer::Target::Row(index)));
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
            if item.button.isEnabled() {
                area(list.convertRect_fromView(item.button.bounds(), Some(&item.button)))
            } else {
                pointer::Area {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                }
            }
        };
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
                    x: frame.origin.x + 4.0,
                    y: frame.origin.y + 2.0,
                    width: 364.0,
                    height: 44.0,
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
        if !state.all.borrow().iter().any(|session| session.id == id) {
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
        self.render_rows(&state.all.borrow());
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
                detail.setFrame(rect(0.0, 0.0, 372.0, detail_height));
                detail.setAlphaValue(progress);
                detail.setHidden(progress <= 0.0);
                detail.scrollToPoint(NSPoint::new(0.0, 0.0));
            }
            if let Some(arrow) = &item.arrow {
                arrow.setFrameCenterRotation(-90.0 * progress);
            }
            y += height;
        }
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

    fn render_rows(&self, sessions: &[Session]) {
        let Some(list) = self.ivars().list.get() else {
            return;
        };
        self.clear_hover();
        self.ivars().buttons.borrow_mut().clear();
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
            Client::Desktop => 2,
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
                        _ => a.id.cmp(&b.id),
                    }
                })
        });
        let mut group = None;
        for session in ordered {
            if group != Some(session.client) {
                let gap = if group.is_some() { 6.0 } else { 0.0 };
                y += gap;
                let name = if session.client == Client::Unknown {
                    "Другие приложения"
                } else {
                    session.client.label()
                };
                let header =
                    NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0.0, y, 384.0, 28.0));
                self.client_icon(&header, session.client, 1.0);
                let name = self.label(&header, name, rect(38.0, 0.0, 336.0, 22.0), 12.0, 0.3, true);
                center_label(&name, 14.0);
                list.addSubview(&header);
                layout.push(LayoutRow {
                    view: header,
                    row: None,
                    id: None,
                    gap,
                    base_height: 28.0,
                    detail_height: 0.0,
                    detail: None,
                    arrow: None,
                    working_icon: None,
                });
                y += 28.0;
                group = Some(session.client);
            }
            let is_expanded = expanded.contains(&session.id);
            let detail = if is_expanded
                || reveal
                    .get(&session.id)
                    .is_some_and(|progress| *progress > 0.0)
            {
                let text = session
                    .approval
                    .as_ref()
                    .map(|request| request.body.as_str())
                    .unwrap_or(&session.last_message);
                let text = if text.is_empty() {
                    "Сообщений пока нет"
                } else {
                    text
                };
                let label =
                    NSTextField::wrappingLabelWithString(&NSString::from_str(text), self.mtm());
                label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
                label.setSelectable(true);
                label.setMaximumNumberOfLines(0);
                let height = label
                    .cell()
                    .map(|cell| {
                        cell.cellSizeForBounds(rect(0.0, 0.0, 318.0, 1000000.0))
                            .height
                            .ceil()
                    })
                    .unwrap_or(60.0)
                    .max(22.0);
                label.setFrame(rect(0.0, 0.0, 318.0, height));
                Some((label, height))
            } else {
                None
            };
            let detail_height = detail
                .as_ref()
                .map(|(_, height)| height + 16.0)
                .unwrap_or(0.0);
            let height = 48.0 + detail_height;
            let row = SessionRowView::create(self.mtm(), rect(12.0, y, 372.0, height));
            let text_width = if session.approval.is_some() {
                196.0
            } else {
                274.0
            };
            let title = self.label(
                &row,
                &session.title,
                rect(26.0, height - 26.0, text_width, 22.0),
                14.0,
                0.23,
                false,
            );
            center_label(&title, height - 15.0);
            let project = session
                .cwd
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.label(
                &row,
                &project,
                rect(26.0, height - 40.0, text_width, 16.0),
                12.0,
                0.0,
                true,
            );
            let working_icon = self.status_icon(&row, session.activity, height - 23.0);
            let open = self.action_button(
                &row,
                &session.title,
                sel!(openSession:),
                &session.id,
                rect(22.0, height - 48.0, text_width + 8.0, 48.0),
                true,
            );
            open.setToolTip(Some(&NSString::from_str(&format!(
                "{} · {}\n{}\nОткрыть проект в VS Code",
                session.title,
                session.client.label(),
                session.cwd.display()
            ))));
            let arrow = self.action_button(
                &row,
                "",
                sel!(expandSession:),
                &session.id,
                rect(336.0, height - 27.0, 26.0, 24.0),
                false,
            );
            let mut arrow_icon = None;
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("chevron.right"),
                Some(ns_string!("Подробности сессии")),
            ) {
                image.setTemplate(true);
                // SAFETY: This NSView subclass has no ivars or custom initialization.
                let decoration: Retained<StatusContentView> = unsafe {
                    msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 26.0, 24.0)]
                };
                let icon = NSImageView::imageViewWithImage(&image, self.mtm());
                icon.setFrame(rect(6.0, 5.0, 14.0, 14.0));
                icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
                decoration.addSubview(&icon);
                arrow.addSubview(&decoration);
                arrow_icon = Some(icon);
            }
            arrow.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
            arrow.setToolTip(Some(if is_expanded {
                ns_string!("Свернуть")
            } else {
                ns_string!("Показать подробности")
            }));
            if let Some(request) = &session.approval {
                let allow = self.action_button(
                    &row,
                    "Allow",
                    sel!(allowSession:),
                    &request.response.to_string_lossy(),
                    rect(232.0, height - 29.0, 70.0, 28.0),
                    false,
                );
                allow.setBordered(false);
                allow.setBezelStyle(NSBezelStyle::Push);
            }
            // Header controls remain pinned to the top while the detail grows below.
            let header_children = row.subviews();
            for index in 0..header_children.count() {
                header_children
                    .objectAtIndex(index)
                    .setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
            }
            let mut detail_clip = None;
            if let Some((label, _)) = detail {
                // The clip only reveals content; the main list is the sole scroller.
                // SAFETY: NSClipView initializes this ivar-free subclass.
                let clip: Retained<RevealClipView> = unsafe {
                    msg_send![RevealClipView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 372.0, detail_height)]
                };
                clip.setDrawsBackground(false);
                // SAFETY: NSView's initializer is valid for this ivar-free subclass.
                let document: Retained<SessionListView> = unsafe {
                    msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 372.0, detail_height)]
                };
                label.setFrameOrigin(NSPoint::new(26.0, 8.0));
                document.addSubview(&label);
                self.separator(&document, detail_height - 1.0, 332.0);
                clip.setDocumentView(Some(&document));
                row.addSubview(&clip);
                detail_clip = Some(clip);
            }
            list.addSubview(&row);
            layout.push(LayoutRow {
                view: row.clone().into_super(),
                row: Some(row),
                id: Some(session.id.clone()),
                gap: 0.0,
                base_height: 48.0,
                detail_height,
                detail: detail_clip,
                arrow: arrow_icon,
                working_icon,
            });
            y += height;
        }
        drop(reveal);
        drop(expanded);
        *self.ivars().layout.borrow_mut() = layout;
        self.layout_rows();
        self.advance_working_icons();
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
        let height = (content_height + 8.0).max(64.0).min(maximum);
        let viewport = height - 8.0;
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
            scroll.setFrame(rect(4.0, 4.0, 384.0, viewport));
        }
        if let Some(toolbar) = state.toolbar.get() {
            toolbar.setFrame(rect(0.0, height - 34.0, 392.0, 32.0));
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
        let state = self.ivars();
        if state.refreshing.replace(true) {
            return;
        }
        state.last_refresh.set(Some(Instant::now()));
        let (sender, receiver) = std::sync::mpsc::channel();
        *state.incoming.borrow_mut() = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(sessions::open(&core::codex_home()));
        });
    }

    fn receive_sessions(&self) {
        let state = self.ivars();
        let result =
            state
                .incoming
                .borrow()
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        Some(Err(anyhow::anyhow!("session worker disconnected")))
                    }
                });
        let Some(result) = result else {
            return;
        };
        state.incoming.borrow_mut().take();
        state.refreshing.set(false);
        match result {
            Ok(sessions) => {
                *state.all.borrow_mut() = sessions;
                self.filter();
            }
            Err(error) => {
                eprintln!("codex-notify: cannot read sessions: {error:#}");
                state.all.borrow_mut().clear();
                self.filter();
                if let Some(caption) = state.caption.get() {
                    caption.setStringValue(ns_string!("Не удалось обновить сессии"));
                    caption.setHidden(false);
                }
            }
        }
        self.update_badge(pending::list().len());
    }

    fn filter(&self) {
        let state = self.ivars();
        let visible = state.all.borrow().clone();
        let count = visible.len();
        let previous = state.visible.borrow();
        let changed = previous.len() != visible.len()
            || previous.iter().zip(&visible).any(|(a, b)| {
                a.id != b.id
                    || a.title != b.title
                    || a.cwd != b.cwd
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
                "Нет открытых сессий"
            } else {
                ""
            };
            caption.setStringValue(&NSString::from_str(text));
            caption.setHidden(text.is_empty());
        }
    }
}

pub fn start(mtm: MainThreadMarker, show: bool) -> Result<bool> {
    let directory = core::home().join("Library/Application Support/Codex Notify");
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
        return Ok(false);
    }
    RESIDENT.store(true, Ordering::Relaxed);
    let controller = MenuController::create(mtm, lock);
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
    CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller));
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let pending_count = pending::list().len();
            dispatch2::DispatchQueue::main().exec_async(move || {
                CONTROLLER.with(|slot| {
                    if let Some(controller) = slot.borrow().as_ref() {
                        controller.update_badge(pending_count);
                        controller.receive_sessions();
                        if controller
                            .ivars()
                            .last_refresh
                            .get()
                            .is_none_or(|last| last.elapsed() >= Duration::from_secs(2))
                        {
                            controller.refresh();
                        }
                    }
                })
            });
        }
    });
    Ok(true)
}

#[cfg(test)]
mod close_tests {
    use super::*;
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
    }
}
