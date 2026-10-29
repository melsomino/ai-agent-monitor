//! Resident menu bar item and native sessions menu.
use crate::{
    agents::{
        codex::usage,
        sessions::{self, Activity, Client, Session},
    },
    app::core,
    macos::{hotkey, motion, native, pointer, search, settings},
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
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const SESSION_HEADER_HEIGHT: f64 = 24.0;
const SESSION_SPACING: f64 = 12.0;
const SESSION_HOVER_PADDING: f64 = 4.0;
const SESSION_HOVER_LEFT: f64 = 14.0;
const SESSION_HOVER_WIDTH: f64 = 354.0;
const MESSAGE_TOP_GAP: f64 = 4.0;

static RESIDENT: AtomicBool = AtomicBool::new(false);
thread_local! { static CONTROLLER: RefCell<Option<Retained<MenuController>>> = const { RefCell::new(None) }; }

pub fn toggle_from_hotkey() {
    CONTROLLER.with(|slot| {
        if let Some(controller) = slot.borrow().as_ref() {
            // Return from Carbon's callback before opening a nested menu loop
            // or suspending the registration that delivered this callback.
            // SAFETY: This retained controller implements toggleMenuFromHotkey:;
            // common run-loop modes also run during native menu tracking.
            unsafe {
                let timer = NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    0.0,
                    controller,
                    sel!(toggleMenuFromHotkey:),
                    None,
                    false,
                );
                NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
            }
        }
    });
}

/// Shared by the native menu dispatcher and AppKit's event-fetching loop.
/// Return false for everything outside our open sessions menu.
pub fn handle_menu_key(event: &NSEvent) -> bool {
    CONTROLLER.with(|slot| {
        slot.try_borrow()
            .ok()
            .and_then(|slot| {
                slot.as_ref()
                    .map(|controller| controller.handle_session_key(event))
            })
            .unwrap_or(false)
    })
}

pub fn is_resident() -> bool {
    RESIDENT.load(Ordering::Relaxed)
}

struct Discovery {
    sessions: Result<Vec<Session>>,
    usage: Option<usage::Usage>,
}

#[derive(Default)]
struct State {
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
    login_switch: OnceCell<Retained<NSSwitch>>,
    animation_switch: OnceCell<Retained<NSSwitch>>,
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
    incoming: RefCell<Option<std::sync::mpsc::Receiver<Discovery>>>,
    last_refresh: Cell<Option<Instant>>,
    list: OnceCell<Retained<SessionListView>>,
    root: OnceCell<Retained<PanelView>>,
    toolbar: OnceCell<Retained<NSView>>,
    footer: OnceCell<Retained<NSView>>,
    footer_height: Cell<f64>,
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
    session_notifications: RefCell<HashSet<String>>,
    all: RefCell<Vec<Session>>,
    visible: RefCell<Vec<Session>>,
    filter_query: RefCell<String>,
    filter_dirty: Cell<bool>,
    usage: RefCell<Option<usage::Usage>>,
    lock: OnceCell<File>,
    refreshing: Cell<bool>,
    routing_scroll: Cell<bool>,
    buttons: RefCell<Vec<ButtonHover>>,
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
        self.all
            .borrow()
            .iter()
            .filter(|session| search::matches(&session.title, &query))
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
    struct MenuController;

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

        #[unsafe(method(pollPointer:))]
        fn poll_pointer(&self, _timer: &NSTimer) {
            self.start_keyboard_tracking();
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
            let session = self.ivars().all.borrow().iter().find(|session| session.approval_key().is_some_and(|key| key == id)).cloned();
            if let Some(session) = session {
                if let Some(button) = sender { button.setEnabled(false); button.setTitle(ns_string!("Allowing…")); }
                std::thread::spawn(move || {
                    let allowed = session.allow();
                    dispatch2::DispatchQueue::main().exec_async(move || {
                        CONTROLLER.with(|slot| {
                            if let Some(controller) = slot.borrow().as_ref() {
                                for item in controller.ivars().buttons.borrow().iter() {
                                    if item.button.identifier().is_some_and(|key| key.to_string() == id) {
                                        item.button.setTitle(if allowed { ns_string!("Allowed") } else { ns_string!("Allow") });
                                        item.button.setEnabled(!allowed);
                                    }
                                }
                                controller.refresh();
                            }
                        });
                    });
                });
            }
        }

        #[unsafe(method(openSession:))]
        fn open_session(&self, sender: Option<&NSButton>) {
            let Some(id) = sender.and_then(|button| button.identifier()) else { return; };
            let data = self.ivars().all.borrow();
            if let Some(session) = data.iter().find(|session| session.key() == id.to_string()) {
                let Some(target) = session.open_target() else { return; };
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
                let message = NSString::from_str(&format!("Could not change launch at login: {error}"));
                note.setStringValue(&message);
                note.setToolTip(Some(&message));
                note.setTextColor(Some(&NSColor::systemRedColor()));
            }
        }

        #[unsafe(method(toggleMenuAnimation:))]
        fn toggle_menu_animation(&self, sender: &NSSwitch) {
            settings::set_menu_animation(sender.state() == NSControlStateValueOn);
            self.update_badge(sessions::pending_count());
        }

        #[unsafe(method(openLoginSettings:))]
        fn open_login_settings(&self, _sender: Option<&AnyObject>) {
            settings::open_login_settings();
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
        fn quit(&self, _sender: Option<&AnyObject>) { NSApplication::sharedApplication(self.mtm()).terminate(None); }
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
    fn clear_menu_shortcut(&self) {
        self.end_shortcut_recording(false);
        self.ivars().shortcut_registration.borrow_mut().take();
        settings::set_menu_shortcut(None);
        self.ivars().shortcut_error.borrow_mut().take();
        self.refresh_settings();
    }

    fn install_shortcut(&self) {
        let old = self.ivars().shortcut_registration.borrow_mut().take();
        drop(old);
        self.ivars().shortcut_error.borrow_mut().take();
        self.configure_menu_shortcut();
        if let Some(shortcut) = settings::menu_shortcut() {
            if self.ivars().menu_open.get() {
                return;
            }
            match hotkey::Registration::new(&shortcut, self.mtm()) {
                Ok(registration) => {
                    *self.ivars().shortcut_registration.borrow_mut() = Some(registration)
                }
                Err(error) => *self.ivars().shortcut_error.borrow_mut() = Some(error.to_string()),
            }
        }
    }

    fn configure_menu_shortcut(&self) {
        if let Some(item) = self.ivars().menu_close_shortcut.get() {
            if let Some(shortcut) = settings::menu_shortcut() {
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
        let callback = block2::RcBlock::new(|event: std::ptr::NonNull<NSEvent>| {
            let handled = CONTROLLER.with(|slot| {
                slot.borrow().as_ref().is_some_and(|controller| {
                    // SAFETY: The local monitor supplies a live event for this callback.
                    controller.record_shortcut_key(unsafe { event.as_ref() })
                })
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
        match hotkey::Registration::new(&shortcut, self.mtm()) {
            Ok(registration) => {
                *self.ivars().shortcut_registration.borrow_mut() = Some(registration);
                settings::set_menu_shortcut(Some(&shortcut));
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

    fn create(mtm: MainThreadMarker, lock: File) -> Retained<Self> {
        let allocated = Self::alloc(mtm).set_ivars(State::default());
        // SAFETY: NSObject's initializer is valid for this subclass and its ivars.
        let this: Retained<Self> = unsafe { msg_send![super(allocated), init] };
        this.ivars().lock.set(lock).expect("lock initialized once");
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
            self.label(
                &view,
                "Launch at login",
                rect(24.0, 254.0, 350.0, 22.0),
                13.0,
                0.23,
                false,
            );
            let note = self.label(
                &view,
                "Open AI Agent Monitor automatically",
                rect(24.0, 232.0, 370.0, 18.0),
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
                "Menu bar animation",
                rect(24.0, 176.0, 350.0, 22.0),
                13.0,
                0.23,
                false,
            );
            self.label(
                &view,
                "Rotate the indicator while an agent is working",
                rect(24.0, 154.0, 350.0, 18.0),
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
        self.configure_menu_shortcut();
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
        let shortcut = settings::menu_shortcut();
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
        self.update_badge(sessions::pending_count());
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
            format!("Agent Sessions · Pending approvals: {count}")
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
            ("gearshape", sel!(settings:), 324.0, "Settings"),
            ("power", sel!(quit:), 356.0, "Quit AI Agent Monitor"),
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
            match hotkey::MenuKeyboard::new(self.mtm()) {
                Ok(handler) => *state.keyboard_handler.borrow_mut() = Some(handler),
                Err(error) => eprintln!("ai-agent-monitor: {error:#}"),
            }
        }
    }

    fn stop_keyboard_tracking(&self) {
        self.ivars().keyboard_handler.borrow_mut().take();
    }

    fn handle_session_key(&self, event: &NSEvent) -> bool {
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
        } else if !event.isARepeat() {
            let target = state
                .all
                .borrow()
                .iter()
                .find(|session| session.key() == id)
                .and_then(Session::open_target);
            if let Some(target) = target {
                self.dismiss_menu();
                native::open_target(&target);
            }
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

    fn session_metadata(&self, session: &Session) -> (Retained<SessionListView>, f64) {
        // Use the same flipped coordinate system as the expanded detail document.
        // SAFETY: This ivar-free view uses NSView's initializer.
        let view: Retained<SessionListView> = unsafe {
            msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(26.0, 8.0, 318.0, 0.0)]
        };
        let mut rows = vec![(
            "folder",
            "Working directory",
            session.working_directory().display().to_string(),
        )];
        if let Some(branch) = &session.git_branch {
            rows.push(("arrow.triangle.branch", "Git branch", branch.clone()));
        }
        let mut y = 0.0;
        for (symbol, description, value) in rows {
            if y > 0.0 {
                y += 4.0;
            }
            let height = 16.0;
            let label = self.label(&view, &value, rect(20.0, y, 298.0, height), 11.0, 0.0, true);
            label.setSelectable(true);
            label.setToolTip(Some(&NSString::from_str(&value)));
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
        view.setFrameSize(NSSize::new(318.0, y));
        (view, y)
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
            y += session_gap;
            let is_expanded = expanded.contains(&session.key());
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
                let label =
                    NSTextField::wrappingLabelWithString(&NSString::from_str(text), self.mtm());
                label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
                label.setTextColor(Some(&detail_text_color(session.approval.is_some())));
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
                        + if session
                            .approval
                            .as_ref()
                            .is_some_and(|request| request.can_allow)
                        {
                            36.0
                        } else {
                            0.0
                        }
                })
                .unwrap_or(0.0);
            let height = SESSION_HEADER_HEIGHT + detail_height;
            let row = SessionRowView::create(self.mtm(), rect(12.0, y, 372.0, height));
            if session_gap > 0.0 {
                // Draw in the existing gap. Pinning to the row's top below keeps
                // the divider aligned while either session expands or collapses.
                self.separator(&row, height + session_gap / 2.0 - 0.5, 332.0);
            }
            let text_width = 274.0;
            let title = self.label(
                &row,
                &session.title,
                rect(
                    26.0,
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
                    22.0,
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
                    336.0,
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
                    msg_send![StatusContentView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 26.0, 24.0)]
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
                    msg_send![RevealClipView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 372.0, detail_height)]
                };
                clip.setDrawsBackground(false);
                // SAFETY: NSView's initializer is valid for this ivar-free subclass.
                let document: Retained<SessionListView> = unsafe {
                    msg_send![SessionListView::alloc(self.mtm()), initWithFrame: rect(0.0, 0.0, 372.0, detail_height)]
                };
                let message_end = MESSAGE_TOP_GAP + text_height;
                let footer_y = message_end
                    + 8.0
                    + if session
                        .approval
                        .as_ref()
                        .is_some_and(|request| request.can_allow)
                    {
                        36.0
                    } else {
                        0.0
                    };
                metadata.setFrameOrigin(NSPoint::new(26.0, footer_y));
                document.addSubview(&metadata);
                label.setFrameOrigin(NSPoint::new(26.0, MESSAGE_TOP_GAP));
                document.addSubview(&label);
                if let Some(key) = session.approval_key().filter(|_| {
                    session
                        .approval
                        .as_ref()
                        .is_some_and(|request| request.can_allow)
                }) {
                    let allow = self.action_button(
                        &document,
                        "Allow",
                        sel!(allowSession:),
                        &key,
                        rect(274.0, message_end + 8.0, 70.0, 28.0),
                        false,
                    );
                    allow.setBezelStyle(NSBezelStyle::Push);
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
        let rows = usage.map(|usage| usage.windows.len()).unwrap_or(1);
        let height = (rows + filter_rows) as f64 * 24.0 + 12.0;
        let changed = state.footer_height.replace(height) != height;
        let children = footer.subviews();
        for index in 0..children.count() {
            children.objectAtIndex(index).removeFromSuperview();
        }
        footer.setFrame(rect(0.0, 0.0, 392.0, height));
        self.separator(footer, height - 1.0, 352.0);
        if filter_rows > 0 {
            let label = self.label(
                footer,
                "",
                rect(20.0, height - 32.0, 352.0, 20.0),
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
                    &window.label(),
                    rect(20.0, y, 178.0, 20.0),
                    12.0,
                    0.0,
                    true,
                );
                let remaining = window
                    .remaining(now)
                    .map(|value| format!("{value}%"))
                    .unwrap_or_else(|| "—".into());
                let percent = self.label(
                    footer,
                    &remaining,
                    rect(202.0, y, 66.0, 20.0),
                    12.0,
                    0.0,
                    true,
                );
                percent.setAlignment(NSTextAlignment::Right);
                let reset = window
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
                    .unwrap_or_else(|| "—".into());
                let date = self.label(footer, &reset, rect(280.0, y, 92.0, 20.0), 12.0, 0.0, true);
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
                rect(20.0, 6.0, 220.0, 20.0),
                12.0,
                0.0,
                true,
            );
            label.setToolTip(Some(ns_string!(
                "Usage will appear after Codex reports its account limits."
            )));
            let value = self.label(footer, "—", rect(280.0, 6.0, 92.0, 20.0), 12.0, 0.0, true);
            value.setAlignment(NSTextAlignment::Right);
        }
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
            let usage = usage::latest(&core::codex_home());
            let _ = sender.send(Discovery {
                sessions: sessions::open(),
                usage,
            });
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
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Discovery {
                        sessions: Err(anyhow::anyhow!("session worker disconnected")),
                        usage: None,
                    }),
                });
        let Some(result) = result else {
            return;
        };
        state.incoming.borrow_mut().take();
        state.refreshing.set(false);
        *state.usage.borrow_mut() = result.usage;
        self.render_footer();
        match result.sessions {
            Ok(sessions) => {
                let requests: HashSet<_> = sessions
                    .iter()
                    .filter(|session| {
                        session.approval.as_ref().is_some_and(|request| {
                            !request.can_allow || request.id.starts_with("ipc:")
                        })
                    })
                    .filter_map(Session::approval_key)
                    .collect();
                let mut previous = state.session_notifications.borrow_mut();
                for old in previous.difference(&requests) {
                    native::remove_session_notification(old);
                }
                for session in &sessions {
                    if let Some(key) = session.approval_key()
                        && requests.contains(&key)
                        && !previous.contains(&key)
                    {
                        native::notify_session(session);
                    }
                }
                *previous = requests;
                drop(previous);
                *state.all.borrow_mut() = sessions;
                self.filter();
            }
            Err(error) => {
                eprintln!("ai-agent-monitor: cannot read sessions: {error:#}");
                state.all.borrow_mut().clear();
                self.filter();
                if let Some(caption) = state.caption.get() {
                    caption.setStringValue(ns_string!("Could not refresh sessions"));
                    caption.setHidden(false);
                }
            }
        }
        self.update_badge(sessions::pending_count());
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

pub fn start(mtm: MainThreadMarker, show: bool) -> Result<bool> {
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
            let pending_count = sessions::pending_count();
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
            approval: approval.map(|path| sessions::Approval {
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
