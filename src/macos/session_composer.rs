//! Persistent, session-scoped drafts; delivery uses the shared monitor actor.
use crate::{
    actors::monitor,
    domain::session::{Activity, Session},
};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::{
    cell::{Cell, RefCell},
    sync::atomic::{AtomicU64, Ordering},
};

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
pub(super) fn available(session: &Session) -> bool {
    session.activity == Activity::Idle && session.approval().is_none()
}

#[derive(Debug)]
struct SurfaceState {
    field: Retained<NSTextField>,
}
define_class!(
    // SAFETY: Main-thread input surface, using a native field editor.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars=SurfaceState]
    struct Surface;
    unsafe impl NSObjectProtocol for Surface {}
    impl Surface {
        #[unsafe(method(drawRect:))]
        fn draw(&self,_dirty:NSRect) {
            let b=self.bounds();let r=rect(4.0,4.0,b.size.width-8.0,b.size.height-8.0);
            super::visual::appearance_color([232.0,222.0,241.0],[61.0,53.0,75.0],1.0).setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r,9.0,9.0).fill();
        }
        #[unsafe(method(mouseDown:))]
        fn clicked(&self,_event:&NSEvent) { super::inline_questions::begin_editing(&self.ivars().field,None); }
    }
);

/// Plain data retained across disposable popup lifetimes; contains no UI.
pub(super) struct Draft {
    text: String,
    pending: Option<String>,
}

pub(super) struct State {
    monitor: monitor::Handle,
    session: String,
    field: Retained<NSTextField>,
    surface: Retained<Surface>,
    focus_ring: Retained<super::inline_questions::FieldFocusRing>,
    view: Retained<NSView>,
    button: Retained<NSButton>,
    caption: Retained<NSTextField>,
    symbol: Retained<super::visual::DecisionIcon>,
    background: Retained<NSBox>,
    feedback: Retained<NSTextField>,
    pending: RefCell<Option<String>>,
    eligible: Cell<bool>,
    selection: Cell<Option<NSRange>>,
}
define_class!(
    // SAFETY: UI-thread controller retained by MenuController while its weak
    // AppKit target/delegate references are installed.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars=State]
    pub(super) struct Composer;
    unsafe impl NSObjectProtocol for Composer {}
    unsafe impl NSControlTextEditingDelegate for Composer {
        #[unsafe(method(controlTextDidBeginEditing:))]
        fn began_editing(&self,_notification:&NSNotification) {
            self.clear_editor_background();
        }
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self,_notification:&NSNotification) { self.sync_button(); }
    }
    unsafe impl NSTextFieldDelegate for Composer {}
    impl Composer {
        #[unsafe(method(sendMessage:))]
        fn send_action(&self,_sender:Option<&AnyObject>) { self.submit(); }
    }
);

impl Composer {
    pub(super) fn draft(&self) -> Draft {
        Draft {
            text: self.text(),
            pending: self.ivars().pending.borrow().clone(),
        }
    }
    pub(super) fn restore_draft(&self, draft: Draft) {
        self.ivars()
            .field
            .setStringValue(&NSString::from_str(&draft.text));
        *self.ivars().pending.borrow_mut() = draft.pending;
    }
    fn clear_editor_background(&self) {
        if let Some(editor) = self.field().currentEditor() {
            editor.setBackgroundColor(Some(&NSColor::clearColor()));
            editor.setDrawsBackground(false);
            // AppKit embeds the shared editor in a clip view for horizontal
            // scrolling. That view can paint independently of the text editor.
            if let Some(scroll) = editor.enclosingScrollView() {
                scroll.setDrawsBackground(false);
                let clip = scroll.contentView();
                clip.setBackgroundColor(&NSColor::clearColor());
                clip.setDrawsBackground(false);
            }
        }
    }
    pub(super) fn new(
        mtm: MainThreadMarker,
        monitor: monitor::Handle,
        session: &Session,
        width: f64,
    ) -> Retained<Self> {
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, width, 56.0));
        let field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            rect(14.0, 11.0, width - 28.0, 18.0),
        );
        field.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        field.setTextColor(Some(&super::visual::primary_text_color()));
        field.setPlaceholderString(Some(ns_string!("Message agent…")));
        // SAFETY: Public accessibility label on this native input control.
        unsafe {
            let _: () = msg_send![&*field, setAccessibilityLabel: ns_string!("Message agent")];
        }
        field.setBezeled(false);
        field.setBordered(false);
        field.setBackgroundColor(Some(&NSColor::clearColor()));
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        field.setEditable(true);
        field.setSelectable(true);
        if let Some(cell) = field.cell() {
            cell.setWraps(true);
            cell.setScrollable(false);
            cell.setUsesSingleLineMode(false);
            cell.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        }
        // SAFETY: Initialize a retained native input surface with its field.
        let surface: Retained<Surface> = unsafe {
            msg_send![super(Surface::alloc(mtm).set_ivars(SurfaceState {field:field.clone()})),initWithFrame:rect(0.0,0.0,width,40.0)]
        };
        surface.addSubview(&field);
        let focus_ring = super::inline_questions::FieldFocusRing::for_field(
            mtm,
            &field,
            rect(-2.0, -2.0, width + 4.0, 44.0),
        );
        focus_ring.setHidden(true);
        surface.addSubview(&focus_ring);
        view.addSubview(&surface);
        let frame = rect(width - 40.0, 12.0, 28.0, 28.0);
        let background = super::visual::glass_box(mtm, frame, 4);
        background.setBoxType(NSBoxType::Custom);
        background.setTitlePosition(NSTitlePosition::NoTitle);
        background.setBorderWidth(0.0);
        background.setCornerRadius(14.0);
        background.setHidden(true);
        view.addSubview(&background);
        // SAFETY: No target/action until the retained controller is initialized.
        let button =
            unsafe { NSButton::buttonWithTitle_target_action(ns_string!(""), None, None, mtm) };
        button.setFrame(frame);
        button.setBordered(false);
        button.setTransparent(true);
        button.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        button.setContentTintColor(Some(&super::visual::primary_text_color()));
        button.setIdentifier(Some(&NSString::from_str(&format!(
            "compose-send:{}",
            session.key()
        ))));
        button.setToolTip(Some(ns_string!("Send message (Return)")));
        let caption = NSTextField::labelWithString(ns_string!(""), mtm);
        caption.setFrame(rect(width - 60.0, 27.0, 60.0, 18.0));
        caption.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        caption.setAlignment(NSTextAlignment::Center);
        caption.setTextColor(Some(&NSColor::whiteColor()));
        let symbol = super::visual::DecisionIcon::new(
            mtm,
            rect(frame.origin.x + 6.0, frame.origin.y + 6.0, 16.0, 16.0),
            caption.clone(),
            super::visual::DecisionGlyph::Up,
        );
        symbol.setHidden(true);
        button.setHidden(true);
        view.addSubview(&symbol);
        view.addSubview(&button);
        // SAFETY: The icon-only submit control retains an explicit accessible name.
        unsafe {
            let _: () = msg_send![&*button,setAccessibilityLabel:ns_string!("Send message")];
        }
        let feedback = NSTextField::labelWithString(ns_string!(""), mtm);
        feedback.setFrame(rect(4.0, 0.0, width - 8.0, 16.0));
        feedback.setFont(Some(&NSFont::systemFontOfSize(10.0)));
        feedback.setTextColor(Some(&super::visual::secondary_text_color()));
        feedback.setMaximumNumberOfLines(1);
        view.addSubview(&feedback);
        let state = State {
            monitor,
            session: session.key(),
            field,
            surface,
            focus_ring,
            view,
            button,
            caption,
            symbol,
            background,
            feedback,
            pending: RefCell::new(None),
            eligible: Cell::new(false),
            selection: Cell::new(None),
        };
        // SAFETY: NSObject initialization after all state is installed.
        let composer: Retained<Self> =
            unsafe { msg_send![super(Self::alloc(mtm).set_ivars(state)), init] };
        // SAFETY: MenuController retains this controller as long as its controls.
        unsafe {
            composer
                .ivars()
                .field
                .setDelegate(Some(ProtocolObject::from_ref(&*composer)));
            composer.ivars().button.setTarget(Some(&composer));
            composer.ivars().button.setAction(Some(sel!(sendMessage:)));
        }
        composer
    }
    pub(super) fn id(&self) -> String {
        format!("compose:{}", self.ivars().session)
    }
    pub(super) fn session(&self) -> &str {
        &self.ivars().session
    }
    pub(super) fn view(&self) -> &NSView {
        &self.ivars().view
    }
    pub(super) fn button(&self) -> Retained<NSButton> {
        self.ivars().button.clone()
    }
    pub(super) fn background(&self) -> Retained<NSBox> {
        self.ivars().background.clone()
    }
    pub(super) fn caption(&self) -> Retained<NSTextField> {
        self.ivars().caption.clone()
    }
    pub(super) fn symbol(&self) -> Retained<super::visual::DecisionIcon> {
        self.ivars().symbol.clone()
    }
    pub(super) fn field(&self) -> &NSTextField {
        &self.ivars().field
    }
    pub(super) fn set_width(&self, width: f64) {
        let s = self.ivars();
        s.view
            .setFrameSize(NSSize::new(width, s.view.frame().size.height));
        s.feedback.setFrameSize(NSSize::new(width - 8.0, 16.0));
        self.resize_to_fit();
    }
    /// The empty surface is 32 pt; a fitting line shares 44 pt with Send.
    /// Wrapped/newline text reserves a separate 44 pt footer for Send.
    pub(super) fn resize_to_fit(&self) -> bool {
        let state = self.ivars();
        let width = state.view.frame().size.width;
        let text = self.text();
        let has_text = !text.trim().is_empty();
        let measure = NSTextField::wrappingLabelWithString(&NSString::from_str(&text), self.mtm());
        measure.setFont(state.field.font().as_deref());
        measure.setMaximumNumberOfLines(0);
        let natural = measure
            .cell()
            .map(|cell| cell.cellSize().width.ceil())
            .unwrap_or(width);
        let inline = has_text && !text.contains(['\r', '\n']) && natural <= width - 66.0;
        let text_width = width - if inline { 66.0 } else { 28.0 };
        let text_height = measure
            .cell()
            .map(|cell| {
                cell.cellSizeForBounds(rect(0.0, 0.0, text_width, 100000.0))
                    .height
                    .ceil()
            })
            .unwrap_or(18.0)
            .max(18.0);
        let visible_height = if !has_text {
            32.0_f64.max(text_height + 14.0)
        } else if inline {
            44.0
        } else {
            text_height + 54.0
        };
        let surface_height = visible_height + 8.0;
        let feedback_height = if state.feedback.stringValue().is_empty() {
            0.0
        } else {
            16.0
        };
        let field_y = if !has_text || inline {
            (surface_height - text_height) / 2.0
        } else {
            surface_height - 14.0 - text_height
        };
        let field_frame = rect(14.0, field_y, text_width, text_height);
        let surface_frame = rect(0.0, feedback_height, width, surface_height);
        let button_frame = rect(
            width - 40.0,
            feedback_height
                + if inline {
                    (surface_height - 28.0) / 2.0
                } else {
                    12.0
                },
            28.0,
            28.0,
        );
        let height = surface_height + feedback_height;
        let changed = state.field.frame() != field_frame
            || state.surface.frame() != surface_frame
            || state.button.frame() != button_frame
            || state.view.frame().size.height != height;
        if state.field.frame() != field_frame {
            state.field.setFrame(field_frame);
        }
        if state.surface.frame() != surface_frame {
            state.surface.setFrame(surface_frame);
            state.surface.setNeedsDisplay(true);
        }
        state
            .focus_ring
            .setFrameSize(NSSize::new(width + 4.0, surface_height + 4.0));
        state.view.setFrameSize(NSSize::new(width, height));
        if state.button.frame() != button_frame {
            state.button.setFrame(button_frame);
        }
        state.background.setFrame(rect(
            button_frame.origin.x - 4.0,
            button_frame.origin.y - 4.0,
            36.0,
            36.0,
        ));
        state.symbol.setFrame(rect(
            button_frame.origin.x + 6.0,
            button_frame.origin.y + 6.0,
            16.0,
            16.0,
        ));
        state.feedback.setHidden(feedback_height == 0.0);
        self.paint_button();
        changed
    }
    fn text(&self) -> String {
        self.ivars()
            .field
            .currentEditor()
            .map(|e| e.string().to_string())
            .unwrap_or_else(|| self.ivars().field.stringValue().to_string())
    }
    fn feedback(&self, text: &str) {
        self.ivars()
            .feedback
            .setStringValue(&NSString::from_str(text));
        self.ivars()
            .feedback
            .setToolTip(Some(&NSString::from_str(text)));
    }
    fn sync_button(&self) {
        let text = self.text();
        let count = text.chars().count();
        self.ivars().button.setEnabled(
            self.ivars().eligible.get()
                && self.ivars().pending.borrow().is_none()
                && !text.trim().is_empty()
                && count <= 16000,
        );
        self.paint_button();
        if count > 16000 {
            self.feedback("Message must be at most 16000 characters.");
        } else if self.ivars().pending.borrow().is_none() && self.ivars().eligible.get() {
            self.feedback("");
        }
    }
    fn paint_button(&self) {
        let state = self.ivars();
        let hidden = self.text().trim().is_empty();
        state.button.setHidden(hidden);
        state.symbol.setHidden(hidden);
        state.background.setHidden(hidden);
        state.caption.setTextColor(Some(
            &NSColor::whiteColor().colorWithAlphaComponent(if state.button.isEnabled() {
                1.0
            } else {
                0.45
            }),
        ));
        if let Some(surface) = state.background.downcast_ref::<super::visual::GlassBox>() {
            surface.ivars().enabled.set(state.button.isEnabled());
        }
        state.symbol.setNeedsDisplay(true);
        state.background.setNeedsDisplay(true);
    }
    pub(super) fn sync(&self, snapshot: &monitor::State, session: &Session) {
        let state = self.ivars();
        let pending = state.pending.borrow().clone();
        if let Some(id) = pending
            && let Some(send) = snapshot.message_sends.get(&id)
        {
            match &send.status {
                monitor::MessageStatus::Sending => self.feedback("Sending…"),
                monitor::MessageStatus::Sent => {
                    self.stop_editing();
                    state.field.setStringValue(ns_string!(""));
                    state.pending.borrow_mut().take();
                    self.feedback("");
                }
                monitor::MessageStatus::Failed(error) => {
                    state.pending.borrow_mut().take();
                    self.feedback(error);
                }
            }
        }
        state
            .eligible
            .set(available(session) && session.can_send_message());
        state
            .field
            .setEnabled(state.eligible.get() && state.pending.borrow().is_none());
        let text = self.text();
        state.button.setEnabled(
            state.field.isEnabled() && !text.trim().is_empty() && text.chars().count() <= 16000,
        );
        self.paint_button();
        self.resize_to_fit();
        if !session.can_send_message() && state.pending.borrow().is_none() {
            self.feedback("No live message connection. Open this session in the agent app.");
        }
    }
    pub(super) fn submit(&self) {
        let state = self.ivars();
        let snapshot = state.monitor.snapshot();
        if !state.eligible.get()
            || state.pending.borrow().is_some()
            || !snapshot
                .sessions
                .iter()
                .any(|s| s.key() == state.session && available(s) && s.can_send_message())
        {
            return;
        }
        let text = self.text();
        if text.trim().is_empty() || text.chars().count() > 16000 {
            self.sync_button();
            return;
        }
        self.stop_editing();
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = format!(
            "window:{}:{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        match state.monitor.send(monitor::Command::SendMessage {
            id: id.clone(),
            session: state.session.clone(),
            text,
        }) {
            Ok(()) => {
                *state.pending.borrow_mut() = Some(id);
                state.field.setEnabled(false);
                state.button.setEnabled(false);
                self.paint_button();
                self.feedback("Sending…");
            }
            Err(error) => self.feedback(&error),
        }
    }
    pub(super) fn attached(&self) -> bool {
        self.view().window().is_some() && !self.view().isHiddenOrHasHiddenAncestor()
    }
    pub(super) fn editing(&self) -> bool {
        self.attached() && self.field().currentEditor().is_some()
    }
    pub(super) fn stop_editing(&self) {
        if self.editing()
            && let Some(window) = self.field().window()
        {
            unsafe {
                window.endEditingFor(None);
            }
        }
    }
    pub(super) fn suspend(&self) {
        self.ivars()
            .selection
            .set(self.field().currentEditor().map(|e| e.selectedRange()));
        self.stop_editing();
    }
    pub(super) fn resume(&self) {
        if let Some(selection) = self.ivars().selection.take()
            && self.attached()
        {
            super::inline_questions::begin_editing(self.field(), Some(selection));
        }
    }
    pub(super) fn focus(&self, id: &str) -> bool {
        if id != self.id() || !self.attached() || !self.field().isEnabled() {
            return false;
        }
        super::inline_questions::begin_editing(self.field(), None);
        true
    }
    pub(super) fn paint(&self, focus: Option<&str>) {
        self.clear_editor_background();
        self.ivars().focus_ring.paint(
            self.field().isEnabled() && (self.editing() || focus == Some(self.id().as_str())),
            std::time::Instant::now(),
        );
    }
    pub(super) fn handle_key(&self, event: &NSEvent, focus: Option<&str>) -> bool {
        if !self.attached() {
            return false;
        }
        if !self.editing() && focus == Some(self.id().as_str()) {
            self.focus(&self.id());
        }
        if !self.editing() {
            return false;
        }
        if matches!(event.keyCode(), 36 | 76) {
            if event.modifierFlags().contains(NSEventModifierFlags::Shift)
                && let Some(editor) = self.field().currentEditor()
            {
                // SAFETY: Native field editor supports an explicit newline without ending editing.
                unsafe {
                    let _: () =
                        msg_send![&*editor, insertNewlineIgnoringFieldEditor: None::<&AnyObject>];
                }
                self.sync_button();
                return true;
            }
            self.submit();
            return true;
        }
        if event.keyCode() == 53 {
            self.stop_editing();
            return false;
        }
        let Some(editor) = self.field().currentEditor() else {
            return false;
        };
        if event
            .modifierFlags()
            .contains(NSEventModifierFlags::Command)
        {
            // SAFETY: Standard edit actions on this attached native field editor.
            unsafe {
                match event.keyCode() {
                    0 => editor.selectAll(None),
                    7 => editor.cut(None),
                    8 => editor.copy(None),
                    9 => editor.paste(None),
                    _ => return editor.performKeyEquivalent(event),
                }
            }
        } else {
            editor.keyDown(event);
        }
        self.sync_button();
        true
    }
}
