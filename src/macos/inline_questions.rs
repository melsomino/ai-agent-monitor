//! Stateful question replies embedded in the sessions menu.
use super::session_layout::{BUTTON_HORIZONTAL_INSET, BUTTON_MIN_HEIGHT, BUTTON_VERTICAL_INSET};
use crate::agents::{
    interaction::{Question, Response},
    sessions::Session,
};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::BTreeMap,
    time::Instant,
};

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn begin_editing(field: &NSTextField, selection: Option<NSRange>) {
    let Some(window) = field.window() else {
        return;
    };
    if !field.isEnabled() || field.isHiddenOrHasHiddenAncestor() {
        return;
    }
    let already_editing = field.currentEditor().is_some();
    if !already_editing {
        // SAFETY: This enabled field is retained in the attached menu.
        unsafe {
            field.selectText(None);
        }
    }
    if let Some(editor) = field.currentEditor() {
        if let Some(selection) = selection {
            editor.setSelectedRange(selection);
        } else if !already_editing {
            editor.setSelectedRange(NSRange::new(editor.string().length(), 0));
        }
        window.makeFirstResponder(Some(&editor));
        // NSTextField's field editor is normally NSTextView. Use its public
        // insertion-point API without assuming a particular editor subclass.
        if editor.respondsToSelector(sel!(updateInsertionPointStateAndRestartTimer:)) {
            // SAFETY: Selector support was checked and its public argument is BOOL.
            unsafe {
                let _: () = msg_send![&*editor, updateInsertionPointStateAndRestartTimer: true];
            }
        }
    }
}

fn answer(question: &Question, selected: usize, custom: &str) -> Option<String> {
    if let Some(choice) = question.choices.get(selected) {
        Some(choice.label.clone())
    } else if selected == question.choices.len() && question.freeform && !custom.trim().is_empty() {
        Some(custom.to_owned())
    } else {
        None
    }
}

struct Input {
    question: Question,
    selected: Cell<usize>,
    radios: Vec<Retained<NSButton>>,
    field: Option<Retained<NSTextField>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonKind {
    Radio,
    Field,
    Choice,
    Submit,
}

#[derive(Clone)]
pub struct Button {
    pub button: Retained<NSButton>,
    pub background: Retained<NSBox>,
    pub base_alpha: f64,
    pub radio: Option<Radio>,
    pub caption: Option<Retained<NSTextField>>,
    pub submit: bool,
}

#[derive(Clone)]
pub struct Radio {
    mark: Retained<RadioMark>,
    label: Retained<NSTextField>,
    focus: Retained<NSBox>,
    field_ring: OnceCell<Retained<FieldFocusRing>>,
}

#[derive(Default)]
struct RadioState {
    selected: Cell<bool>,
    focused: Cell<bool>,
    hovered: Cell<bool>,
}
define_class!(
    // SAFETY: Decorative NSView, accessed only on the UI thread.
    #[unsafe(super(NSView))]
    #[name = "AAMQuestionRadioMark"]
    #[thread_kind = MainThreadOnly]
    #[ivars = RadioState]
    struct RadioMark;
    unsafe impl NSObjectProtocol for RadioMark {}
    impl RadioMark {
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            let selected = self.ivars().selected.get();
            let color = if selected { NSColor::controlAccentColor() } else { NSColor::secondaryLabelColor() };
            color.setFill();
            NSBezierPath::bezierPathWithOvalInRect(self.bounds()).fill();
            if selected {
                NSColor::whiteColor().setFill();
                NSBezierPath::bezierPathWithOvalInRect(rect(5.0, 5.0, 6.0, 6.0)).fill();
            }
        }
    }
);

impl Radio {
    pub fn paint(&self, focused: bool, hovered: bool) {
        self.mark.ivars().focused.set(focused);
        self.mark.ivars().hovered.set(hovered);
        self.focus
            .setHidden(self.field_ring.get().is_some() || (!focused && !hovered));
        if let Some(ring) = self.field_ring.get() {
            ring.ivars().focused.set(focused);
            ring.advance(Instant::now());
        }
        let background = if focused {
            NSColor::controlAccentColor()
        } else {
            super::menu::hover_fill_color()
        };
        self.focus.setFillColor(&background);
        let color = if focused {
            NSColor::selectedMenuItemTextColor()
        } else {
            NSColor::labelColor()
        };
        self.label.setTextColor(Some(&color));
        self.mark.setNeedsDisplay(true);
    }
    fn select(&self, selected: bool) {
        self.mark.ivars().selected.set(selected);
        self.paint(
            self.mark.ivars().focused.get(),
            self.mark.ivars().hovered.get(),
        );
    }
}

struct FieldFocusState {
    field: Retained<NSTextField>,
    focused: Cell<bool>,
    caret: Cell<Option<NSRect>>,
    blink: RefCell<CaretBlink>,
}

struct CaretBlink {
    position: Option<NSRange>,
    epoch: Instant,
}
impl CaretBlink {
    fn visible(&mut self, position: Option<NSRange>, now: Instant) -> bool {
        if self.position != position {
            self.position = position;
            self.epoch = now;
        }
        position.is_some_and(|range| range.length == 0)
            && now.saturating_duration_since(self.epoch).as_millis() % 1000 < 500
    }
}

define_class!(
    // SAFETY: Decorative view using AppKit's public focus-ring renderer.
    #[unsafe(super(NSView))]
    #[name = "AAMQuestionFieldFocusRing"]
    #[thread_kind = MainThreadOnly]
    #[ivars = FieldFocusState]
    struct FieldFocusRing;
    unsafe impl NSObjectProtocol for FieldFocusRing {}
    impl FieldFocusRing {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> { None }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            NSGraphicsContext::saveGraphicsState_class();
            NSSetFocusRingStyle(NSFocusRingPlacement::Only);
            let bounds = self.bounds();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(6.0, 6.0, bounds.size.width - 12.0, bounds.size.height - 12.0), 5.0, 5.0).fill();
            NSGraphicsContext::restoreGraphicsState_class();
            if let Some(caret) = self.ivars().caret.get() {
                NSColor::labelColor().setFill();
                NSBezierPath::bezierPathWithRect(caret).fill();
            }
        }
    }
);

impl FieldFocusRing {
    fn advance(&self, now: Instant) {
        let state = self.ivars();
        let field = &state.field;
        let editor = field
            .currentEditor()
            .filter(|_| field.window().is_some() && !field.isHiddenOrHasHiddenAncestor());
        if editor.is_some() {
            // AppKit may attach its field editor above existing decorations.
            // Keep this click-through overlay above the editor's background.
            // SAFETY: This view is retained and confined to the UI thread.
            if let Some(parent) = unsafe { self.superview() } {
                let children = parent.subviews();
                if children.count() > 0
                    && !std::ptr::eq(
                        &*children.objectAtIndex(children.count() - 1),
                        self.as_super(),
                    )
                {
                    parent.addSubview_positioned_relativeTo(
                        self,
                        NSWindowOrderingMode::Above,
                        None,
                    );
                }
            }
        }
        self.setHidden(!state.focused.get());
        let position = editor.as_ref().map(|editor| editor.selectedRange());
        let visible = state.blink.borrow_mut().visible(position, now);
        let caret = editor.and_then(|editor| {
            let window = field.window()?;
            // The native caret remains responsible in a key window. NSMenu's
            // non-key tracking window needs a cursor driven in common modes.
            if window.isKeyWindow() || !visible || !editor.respondsToSelector(sel!(firstRectForCharacterRange:actualRange:)) { return None; }
            let range = position?;
            // SAFETY: Public NSTextInputClient API, checked on the live field
            // editor; actualRange is documented as optional.
            let screen: NSRect = unsafe { msg_send![&*editor, firstRectForCharacterRange: range, actualRange: std::ptr::null_mut::<NSRange>()] };
            let local = self.convertRect_fromView(window.convertRectFromScreen(screen), None);
            let bounds = self.bounds();
            let y = local.origin.y.max(6.0);
            let height = (local.origin.y + local.size.height).min(bounds.size.height - 6.0) - y;
            if !local.origin.x.is_finite() || !y.is_finite() || height <= 0.0 || local.origin.x < 6.0 || local.origin.x > bounds.size.width - 6.0 { return None; }
            Some(rect(local.origin.x, y, 1.0, height))
        });
        if state.caret.replace(caret) != caret {
            self.setNeedsDisplay(true);
            // Repaint behind an extinguished caret as well as the cursor itself.
            // SAFETY: The retained decoration and its parent are UI-thread only.
            if let Some(parent) = unsafe { self.superview() } {
                parent.setNeedsDisplay(true);
            }
        }
    }
}

pub struct State {
    session: Session,
    monitor: crate::actors::monitor::Handle,
    view: OnceCell<Retained<Document>>,
    content_height: Cell<f64>,
    inputs: RefCell<Vec<Input>>,
    buttons: RefCell<Vec<Button>>,
    submit: OnceCell<Retained<NSButton>>,
    status: OnceCell<Retained<NSTextField>>,
    enabled: Cell<bool>,
    single: bool,
    editing: RefCell<Option<(Retained<NSTextField>, NSRange)>>,
    focus_change: RefCell<Option<String>>,
}

define_class!(
    // SAFETY: UI-only NSObject; the menu owns this controller for the lifetime
    // of its controls' weak target and delegate references.
    #[unsafe(super(NSObject))]
    #[name = "AAMInlineQuestionPanel"]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    pub struct Panel;
    unsafe impl NSObjectProtocol for Panel {}
    unsafe impl NSControlTextEditingDelegate for Panel {
        #[unsafe(method(controlTextDidBeginEditing:))]
        fn began_editing(&self, _notification: &NSNotification) {
            *self.ivars().focus_change.borrow_mut() = self.editing_target();
            self.sync();
        }
        #[unsafe(method(controlTextDidChange:))]
        fn changed_text(&self, notification: &NSNotification) { self.select_custom(notification); }
    }
    unsafe impl NSTextFieldDelegate for Panel {}
    impl Panel {
        #[unsafe(method(selectAnswer:))]
        fn select_answer(&self, sender: &NSButton) {
            *self.ivars().focus_change.borrow_mut() = sender.identifier().map(|id| id.to_string());
            let mut preset = false;
            for input in self.ivars().inputs.borrow().iter() {
                if let Some(index) = input.radios.iter().position(|radio| std::ptr::eq(&**radio, sender)) {
                    preset = index < input.question.choices.len();
                    input.selected.set(index);
                    if index == input.question.choices.len() {
                        if let Some(field) = &input.field {
                            begin_editing(field, None);
                        }
                    } else if let Some(field) = &input.field && let Some(window) = field.window() {
                        // SAFETY: Commit the existing field editor before choosing a preset.
                        unsafe { window.endEditingFor(None); }
                    }
                    input.selected.set(index);
                    break;
                }
            }
            if self.ivars().single && preset { self.submit(false); } else { self.sync(); }
        }
        #[unsafe(method(sendAnswers:))]
        fn send_answers(&self, _sender: Option<&AnyObject>) {
            self.submit(self.ivars().single);
        }
    }
);

define_class!(
    // SAFETY: Ivar-free NSView with flipped layout coordinates.
    #[unsafe(super(NSView))]
    #[name = "AAMInlineQuestionDocument"]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    struct Document;
    unsafe impl NSObjectProtocol for Document {}
    impl Document {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
    }
);

impl Panel {
    pub fn create(
        session: Session,
        monitor: crate::actors::monitor::Handle,
        width: f64,
        color: &NSColor,
    ) -> Retained<Self> {
        let mtm = MainThreadMarker::new().expect("inline questions are main-thread only");
        let single = session.interaction().questions.len() == 1;
        let allocated = Self::alloc(mtm).set_ivars(State {
            session,
            monitor,
            view: OnceCell::new(),
            content_height: Cell::new(0.0),
            inputs: RefCell::new(vec![]),
            buttons: RefCell::new(vec![]),
            submit: OnceCell::new(),
            status: OnceCell::new(),
            enabled: Cell::new(true),
            single,
            editing: RefCell::new(None),
            focus_change: RefCell::new(None),
        });
        // SAFETY: NSObject initializer after initializing all instance state.
        let panel: Retained<Self> = unsafe { msg_send![super(allocated), init] };
        // SAFETY: Standard NSView initializer for this ivar-free subclass.
        let view: Retained<Document> =
            unsafe { msg_send![Document::alloc(mtm), initWithFrame: rect(0.0, 0.0, width, 1.0)] };
        let key = panel.ivars().session.approval_key().unwrap();
        let submit_label = wrapping_label(ns_string!("Send answer"), mtm);
        submit_label.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, 0.23)));
        submit_label.sizeToFit();
        let submit_width =
            (submit_label.frame().size.width.ceil() + 4.0 + 2.0 * BUTTON_HORIZONTAL_INSET)
                .min(width);
        let mut y = 0.0;
        for (q, question) in panel
            .ivars()
            .session
            .interaction()
            .questions
            .into_iter()
            .enumerate()
        {
            if q > 0 {
                y += 14.0;
            }
            y += label(&view, &question.title, y, width, 13.0, color, mtm) + 10.0;
            // Separate superviews keep AppKit's radio grouping local to a question.
            // SAFETY: Use our flipped subclass so rows share the card's coordinates.
            let group: Retained<Document> =
                unsafe { msg_send![Document::alloc(mtm), initWithFrame: rect(0.0, y, width, 1.0)] };
            let mut row_y = 0.0;
            let mut radios = Vec::new();
            for (index, choice) in question.choices.iter().enumerate() {
                let measure = wrapping_label(&NSString::from_str(&choice.label), mtm);
                measure.setFont(Some(&NSFont::systemFontOfSize_weight(
                    12.0,
                    if single { 0.23 } else { 0.0 },
                )));
                measure.sizeToFit();
                let button_width = if single {
                    (measure.frame().size.width.ceil() + 4.0 + 2.0 * BUTTON_HORIZONTAL_INSET)
                        .clamp(52.0, width)
                } else {
                    width
                };
                let height = measure
                    .cell()
                    .map(|cell| {
                        cell.cellSizeForBounds(rect(
                            0.0,
                            0.0,
                            button_width
                                - if single {
                                    2.0 * BUTTON_HORIZONTAL_INSET
                                } else {
                                    24.0
                                },
                            100000.0,
                        ))
                        .height
                        .ceil()
                            + if single {
                                2.0 * BUTTON_VERTICAL_INSET
                            } else {
                                4.0
                            }
                    })
                    .unwrap_or(BUTTON_MIN_HEIGHT)
                    .max(BUTTON_MIN_HEIGHT);
                let radio = panel.button(
                    &group,
                    &choice.label,
                    &format!("response:{key}:q{q}:{index}"),
                    rect(0.0, row_y, button_width, height),
                    if single {
                        ButtonKind::Choice
                    } else {
                        ButtonKind::Radio
                    },
                );
                if let Some(cell) = radio.cell() {
                    cell.setWraps(true);
                    cell.setUsesSingleLineMode(false);
                }
                radios.push(radio);
                row_y += height;
                if !choice.description.is_empty() {
                    let description = NSView::initWithFrame(
                        NSView::alloc(mtm),
                        rect(24.0, row_y, width - 24.0, 1.0),
                    );
                    let height = label(
                        &description,
                        &choice.description,
                        0.0,
                        width - 24.0,
                        11.0,
                        &NSColor::secondaryLabelColor(),
                        mtm,
                    );
                    description.setFrameSize(NSSize::new(width - 24.0, height));
                    group.addSubview(&description);
                    row_y += height + 3.0;
                }
                row_y += if single { 8.0 } else { 3.0 };
            }
            let field = if question.freeform {
                let frame = if single {
                    rect(4.0, row_y + 3.0, width - submit_width - 16.0, 27.0)
                } else {
                    rect(26.0, row_y + 2.0, width - 30.0, 27.0)
                };
                let radio = panel.button(
                    &group,
                    "",
                    &format!("response:{key}:q{q}:custom"),
                    if single {
                        frame
                    } else {
                        rect(0.0, row_y, 22.0, 28.0)
                    },
                    if single {
                        ButtonKind::Field
                    } else {
                        ButtonKind::Radio
                    },
                );
                radio.setToolTip(Some(ns_string!("Your own answer")));
                radios.push(radio);
                let field: Retained<NSTextField> = if question.secret {
                    NSSecureTextField::initWithFrame(NSSecureTextField::alloc(mtm), frame)
                        .into_super()
                } else {
                    NSTextField::initWithFrame(NSTextField::alloc(mtm), frame)
                };
                field.setPlaceholderString(Some(ns_string!("Your own answer")));
                field.setFont(Some(&NSFont::systemFontOfSize(12.0)));
                field.setEditable(true);
                field.setSelectable(true);
                // SAFETY: The owning menu retains the delegate until the field is removed.
                unsafe {
                    field.setDelegate(Some(ProtocolObject::from_ref(&*panel)));
                }
                group.addSubview(&field);
                {
                    let allocated = FieldFocusRing::alloc(mtm).set_ivars(FieldFocusState {
                        field: field.clone(),
                        focused: Cell::new(false),
                        caret: Cell::new(None),
                        blink: RefCell::new(CaretBlink {
                            position: None,
                            epoch: Instant::now(),
                        }),
                    });
                    // SAFETY: Initialize the decorative NSView after its state.
                    let ring: Retained<FieldFocusRing> = unsafe {
                        msg_send![super(allocated), initWithFrame: rect(frame.origin.x - 6.0, frame.origin.y - 6.0, frame.size.width + 12.0, frame.size.height + 12.0)]
                    };
                    ring.setHidden(true);
                    group.addSubview(&ring);
                    if let Some(custom) = radios.last()
                        && let Some(item) = panel
                            .ivars()
                            .buttons
                            .borrow()
                            .iter()
                            .find(|item| std::ptr::eq(&*item.button, &**custom))
                        && let Some(radio) = &item.radio
                    {
                        radio.field_ring.set(ring).ok();
                    }
                }
                if single {
                    let submit = panel.button(
                        &group,
                        "Send answer",
                        &format!("response:{key}:send"),
                        rect(
                            width - submit_width,
                            row_y + 3.0,
                            submit_width,
                            BUTTON_MIN_HEIGHT,
                        ),
                        ButtonKind::Submit,
                    );
                    panel.ivars().submit.set(submit).unwrap();
                }
                row_y += 35.0;
                Some(field)
            } else {
                None
            };
            group.setFrameSize(NSSize::new(width, row_y));
            view.addSubview(&group);
            panel.ivars().inputs.borrow_mut().push(Input {
                question,
                selected: Cell::new(0),
                radios,
                field,
            });
            y += row_y;
        }
        if !single {
            y += 8.0;
            let submit = panel.button(
                &view,
                "Send answer",
                &format!("response:{key}:send"),
                rect(0.0, y, submit_width, BUTTON_MIN_HEIGHT),
                ButtonKind::Submit,
            );
            panel.ivars().submit.set(submit).unwrap();
            y += BUTTON_MIN_HEIGHT;
        }
        let status = NSTextField::labelWithString(ns_string!(""), mtm);
        status.setFrame(rect(0.0, y + 4.0, width, 16.0));
        status.setFont(Some(&NSFont::systemFontOfSize(10.0)));
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        view.addSubview(&status);
        panel.ivars().status.set(status).unwrap();
        panel.ivars().content_height.set(y);
        view.setFrameSize(NSSize::new(width, y));
        panel.ivars().view.set(view).unwrap();
        panel.sync();
        panel
    }

    fn button(
        &self,
        view: &NSView,
        title: &str,
        id: &str,
        frame: NSRect,
        kind: ButtonKind,
    ) -> Retained<NSButton> {
        let mtm = self.mtm();
        let background = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
        background.setBoxType(NSBoxType::Custom);
        background.setTitlePosition(NSTitlePosition::NoTitle);
        background.setBorderWidth(0.0);
        background.setCornerRadius(frame.size.width.min(frame.size.height) / 2.0);
        background.setFillColor(&super::menu::hover_fill_color());
        background.setHidden(matches!(kind, ButtonKind::Radio | ButtonKind::Field));
        view.addSubview(&background);
        // SAFETY: Both selectors are implemented by this retained controller.
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(self),
                Some(if kind != ButtonKind::Submit {
                    sel!(selectAnswer:)
                } else {
                    sel!(sendAnswers:)
                }),
                mtm,
            )
        };
        button.setFrame(frame);
        button.setIdentifier(Some(&NSString::from_str(id)));
        button.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        let radio = if matches!(kind, ButtonKind::Radio | ButtonKind::Field) {
            if kind == ButtonKind::Radio {
                button.setButtonType(NSButtonType::Radio);
            }
            button.setTransparent(true);
            let center = frame.origin.y + frame.size.height / 2.0;
            let natural = NSTextField::labelWithString(&NSString::from_str(title), mtm);
            natural.setFont(Some(&NSFont::systemFontOfSize(12.0)));
            natural.sizeToFit();
            let focus_width = if title.is_empty() {
                view.frame().size.width - 22.0
            } else {
                (natural.frame().size.width + 12.0).min(frame.size.width - 20.0)
            };
            let focus = NSBox::initWithFrame(
                NSBox::alloc(mtm),
                rect(
                    frame.origin.x + 20.0,
                    frame.origin.y - 1.0,
                    focus_width,
                    frame.size.height + 2.0,
                ),
            );
            focus.setBoxType(NSBoxType::Custom);
            focus.setTitlePosition(NSTitlePosition::NoTitle);
            focus.setBorderWidth(0.0);
            focus.setCornerRadius(8.0);
            focus.setFillColor(&NSColor::controlAccentColor());
            focus.setHidden(true);
            view.addSubview(&focus);
            let allocated = RadioMark::alloc(mtm).set_ivars(RadioState::default());
            // SAFETY: NSView initializer after initializing the decoration's state.
            let mark: Retained<RadioMark> = unsafe {
                msg_send![super(allocated), initWithFrame: rect(frame.origin.x, center - 8.0, 16.0, 16.0)]
            };
            view.addSubview(&mark);
            let label = wrapping_label(&NSString::from_str(title), mtm);
            label.setFont(Some(&NSFont::systemFontOfSize(12.0)));
            let label_width = (frame.size.width - 24.0).max(0.0);
            let label_height = label
                .cell()
                .map(|cell| {
                    cell.cellSizeForBounds(rect(0.0, 0.0, label_width.max(1.0), 100000.0))
                        .height
                        .ceil()
                })
                .unwrap_or(16.0)
                .min(frame.size.height);
            label.setFrame(rect(
                frame.origin.x + 24.0,
                center - label_height / 2.0,
                label_width,
                label_height,
            ));
            view.addSubview(&label);
            if kind == ButtonKind::Field {
                mark.setHidden(true);
                label.setHidden(true);
            }
            Some(Radio {
                mark,
                label,
                focus,
                field_ring: OnceCell::new(),
            })
        } else {
            button.setBordered(false);
            button.setBezelStyle(NSBezelStyle::Push);
            None
        };
        let caption = if matches!(kind, ButtonKind::Choice | ButtonKind::Submit) {
            // Keep the visible caption independent of NSButton's transient
            // attributed title/highlight state while the menu tracks controls.
            button.setTransparent(true);
            let label = wrapping_label(&NSString::from_str(title), mtm);
            label.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, 0.23)));
            label.setAlignment(NSTextAlignment::Center);
            let height = label
                .cell()
                .map(|cell| {
                    cell.cellSizeForBounds(rect(
                        0.0,
                        0.0,
                        frame.size.width - 2.0 * BUTTON_HORIZONTAL_INSET,
                        100000.0,
                    ))
                    .height
                    .ceil()
                })
                .unwrap_or(16.0);
            label.setFrame(rect(
                frame.origin.x + BUTTON_HORIZONTAL_INSET,
                frame.origin.y + (frame.size.height - height) / 2.0,
                frame.size.width - 2.0 * BUTTON_HORIZONTAL_INSET,
                height,
            ));
            view.addSubview(&label);
            Some(label)
        } else {
            None
        };
        view.addSubview(&button);
        self.ivars().buttons.borrow_mut().push(Button {
            button: button.clone(),
            background,
            base_alpha: if matches!(kind, ButtonKind::Radio | ButtonKind::Field) {
                0.0
            } else {
                0.5
            },
            radio,
            caption,
            submit: kind == ButtonKind::Submit,
        });
        button
    }

    fn select_custom(&self, notification: &NSNotification) {
        if let Some(object) = notification.object() {
            for input in self.ivars().inputs.borrow().iter() {
                if input.field.as_ref().is_some_and(|field| {
                    std::ptr::eq(std::ptr::from_ref(&**field).cast::<AnyObject>(), &*object)
                }) {
                    input.selected.set(input.question.choices.len());
                }
            }
        }
        self.sync();
    }

    fn answers(&self, custom_only: bool) -> BTreeMap<String, Vec<String>> {
        self.ivars()
            .inputs
            .borrow()
            .iter()
            .filter_map(|input| {
                let custom = input
                    .field
                    .as_ref()
                    .map(|field| {
                        field
                            .currentEditor()
                            .map(|editor| editor.string().to_string())
                            .unwrap_or_else(|| field.stringValue().to_string())
                    })
                    .unwrap_or_default();
                let selected = if custom_only {
                    input.question.choices.len()
                } else {
                    input.selected.get()
                };
                answer(&input.question, selected, &custom)
                    .map(|value| (input.question.id.clone(), vec![value]))
            })
            .collect()
    }

    fn sync(&self) {
        let enabled = self.ivars().enabled.get();
        for input in self.ivars().inputs.borrow().iter() {
            for (index, radio) in input.radios.iter().enumerate() {
                if !self.ivars().single {
                    if let Some(item) = self
                        .ivars()
                        .buttons
                        .borrow()
                        .iter()
                        .find(|item| std::ptr::eq(&*item.button, &**radio))
                        && let Some(decoration) = &item.radio
                    {
                        decoration.select(index == input.selected.get());
                    }
                    radio.setState(if index == input.selected.get() {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                }
                radio.setEnabled(enabled);
            }
            if let Some(field) = &input.field {
                field.setEnabled(enabled);
                field.setTextColor(Some(&NSColor::labelColor()));
            }
        }
        if let Some(submit) = self.ivars().submit.get() {
            submit.setEnabled(
                enabled
                    && self
                        .ivars()
                        .session
                        .interaction()
                        .valid_answers(&self.answers(self.ivars().single)),
            );
        }
    }

    fn submit(&self, custom_only: bool) {
        if !self.ivars().enabled.get() {
            return;
        }
        let answers = self.answers(custom_only);
        if !self.ivars().session.interaction().valid_answers(&answers) {
            self.ivars()
                .status
                .get()
                .unwrap()
                .setStringValue(ns_string!("Please answer every question."));
            return;
        }
        match self
            .ivars()
            .monitor
            .submit(&self.ivars().session, Response::Answers(answers))
        {
            Ok(()) => {
                self.ivars().enabled.set(false);
                self.ivars()
                    .status
                    .get()
                    .unwrap()
                    .setStringValue(ns_string!("Sending…"));
                self.sync();
            }
            Err(error) => self
                .ivars()
                .status
                .get()
                .unwrap()
                .setStringValue(&NSString::from_str(&error)),
        }
    }

    pub fn reconcile(&self, enabled: bool, status: &str) {
        self.ivars().enabled.set(enabled);
        self.ivars()
            .status
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(status));
        let view = self.view();
        view.setFrameSize(NSSize::new(
            view.frame().size.width,
            self.ivars().content_height.get() + if status.is_empty() { 0.0 } else { 20.0 },
        ));
        self.sync();
    }
    pub fn view(&self) -> &NSView {
        self.ivars().view.get().unwrap()
    }
    pub fn buttons(&self) -> Vec<Button> {
        self.ivars().buttons.borrow().clone()
    }

    pub fn take_focus_change(&self) -> Option<String> {
        self.ivars().focus_change.borrow_mut().take()
    }

    pub fn advance_carets(&self, now: Instant) {
        for item in self.ivars().buttons.borrow().iter() {
            if let Some(radio) = &item.radio
                && let Some(ring) = radio.field_ring.get()
            {
                ring.advance(now);
            }
        }
    }

    pub fn focus_field(&self, id: &str) -> bool {
        let field = self.ivars().inputs.borrow().iter().find_map(|input| {
            let field = input.field.as_ref()?;
            input
                .radios
                .last()?
                .identifier()
                .filter(|key| key.to_string() == id)?;
            Some(field.clone())
        });
        if let Some(field) = field {
            begin_editing(&field, None);
            true
        } else {
            false
        }
    }

    pub fn preset_button(&self, id: &str) -> Option<Retained<NSButton>> {
        self.ivars().inputs.borrow().iter().find_map(|input| {
            input
                .radios
                .iter()
                .take(input.question.choices.len())
                .find(|button| {
                    button.isEnabled()
                        && button
                            .identifier()
                            .is_some_and(|value| value.to_string() == id)
                })
                .cloned()
        })
    }

    pub fn suspend_editing(&self) {
        *self.ivars().editing.borrow_mut() =
            self.ivars().inputs.borrow().iter().find_map(|input| {
                let field = input.field.as_ref()?;
                let editor = field.currentEditor()?;
                let selection = editor.selectedRange();
                field.setStringValue(&editor.string());
                Some((field.clone(), selection))
            });
    }
    pub fn resume_editing(&self) {
        let editing = self.ivars().editing.borrow_mut().take();
        if let Some((field, selection)) = editing
            && field.window().is_some()
            && !field.isHiddenOrHasHiddenAncestor()
        {
            begin_editing(&field, Some(selection));
        }
    }
    pub fn handle_key(&self, event: &NSEvent, focus: Option<&str>) -> bool {
        // Typing while the custom-answer radio has keyboard focus starts editing
        // that answer instead of adding characters to the session filter.
        let typing = !event
            .modifierFlags()
            .intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control)
            && (event.keyCode() == 51
                || event
                    .characters()
                    .is_some_and(|text| crate::macos::search::printable(&text.to_string())));
        let editing = self.ivars().inputs.borrow().iter().any(|input| {
            input
                .field
                .as_ref()
                .is_some_and(|field| field.currentEditor().is_some())
        });
        if typing && !editing {
            let field = self.ivars().inputs.borrow().iter().find_map(|input| {
                if !input.question.freeform {
                    return None;
                }
                let radio = input.radios.last()?;
                if !radio
                    .identifier()
                    .is_some_and(|id| Some(id.to_string()).as_deref() == focus)
                {
                    return None;
                }
                let field = input.field.as_ref()?;
                if !field.isEnabled()
                    || field.isHiddenOrHasHiddenAncestor()
                    || field.window().is_none()
                {
                    return None;
                }
                input.selected.set(input.question.choices.len());
                Some(field.clone())
            });
            if let Some(field) = field
                && field.currentEditor().is_none()
            {
                begin_editing(&field, None);
            }
        }
        let editor = self.ivars().inputs.borrow().iter().find_map(|input| {
            let field = input.field.as_ref()?;
            if field.window().is_none() || field.isHiddenOrHasHiddenAncestor() {
                return None;
            }
            let editor = field.currentEditor()?;
            Some(editor)
        });
        if let Some(editor) = editor
            && event.keyCode() != 53
        {
            if matches!(event.keyCode(), 36 | 76) {
                // Return chooses this custom answer. Do not forward insertNewline:
                // to a single-line field: AppKit can activate/close the menu.
                for input in self.ivars().inputs.borrow().iter() {
                    if input
                        .field
                        .as_ref()
                        .is_some_and(|field| field.currentEditor().is_some())
                    {
                        input.selected.set(input.question.choices.len());
                        *self.ivars().focus_change.borrow_mut() = input
                            .radios
                            .last()
                            .and_then(|button| button.identifier())
                            .map(|id| id.to_string());
                    }
                }
                self.sync();
                return true;
            }
            self.sync();
            if event
                .modifierFlags()
                .contains(NSEventModifierFlags::Command)
            {
                // SAFETY: Standard editing actions on the active, owned field editor.
                unsafe {
                    match event.keyCode() {
                        0 => editor.selectAll(None),
                        7 => editor.cut(None),
                        8 => editor.copy(None),
                        9 => editor.paste(None),
                        _ => return editor.performKeyEquivalent(event),
                    }
                }
                return true;
            }
            editor.keyDown(event);
            return true;
        }
        false
    }

    pub fn editing_target(&self) -> Option<String> {
        self.ivars().inputs.borrow().iter().find_map(|input| {
            let field = input.field.as_ref()?;
            if field.window().is_none() || field.isHiddenOrHasHiddenAncestor() {
                return None;
            }
            field.currentEditor()?;
            input.radios.last()?.identifier().map(|id| id.to_string())
        })
    }

    pub fn stop_editing(&self) {
        self.ivars().focus_change.borrow_mut().take();
        self.ivars().editing.borrow_mut().take();
        let window = self.ivars().inputs.borrow().iter().find_map(|input| {
            let field = input.field.as_ref()?;
            field.currentEditor()?;
            field.window()
        });
        if let Some(window) = window {
            // SAFETY: Commit the field editor while its controls are still retained.
            unsafe {
                window.endEditingFor(None);
            }
            window.makeFirstResponder(None);
        }
    }
}

fn wrapping_label(text: &NSString, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(text, mtm);
    // Fill the available line width; do not push words to the next line
    // just to balance the last two lines of a question or answer.
    label.setLineBreakStrategy(NSLineBreakStrategy::None);
    label
}

fn label(
    view: &NSView,
    value: &str,
    y: f64,
    width: f64,
    size: f64,
    color: &NSColor,
    mtm: MainThreadMarker,
) -> f64 {
    let label = wrapping_label(&NSString::from_str(value), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize_weight(
        size,
        if size >= 13.0 { 0.23 } else { 0.0 },
    )));
    label.setTextColor(Some(color));
    let height = label
        .cell()
        .map(|cell| {
            cell.cellSizeForBounds(rect(0.0, 0.0, width, 100000.0))
                .height
                .ceil()
        })
        .unwrap_or(20.0)
        .max(16.0);
    label.setFrame(rect(0.0, y, width, height));
    view.addSubview(&label);
    height
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::interaction::{Choice, Interaction};
    use std::time::Duration;
    #[test]
    fn caret_blinks_in_tracking_time_and_restarts_after_cursor_movement() {
        let start = Instant::now();
        let mut blink = CaretBlink {
            position: None,
            epoch: start,
        };
        let cursor = Some(NSRange::new(2, 0));
        assert!(blink.visible(cursor, start));
        assert!(!blink.visible(cursor, start + Duration::from_millis(500)));
        assert!(blink.visible(cursor, start + Duration::from_millis(1000)));
        assert!(blink.visible(
            Some(NSRange::new(3, 0)),
            start + Duration::from_millis(1700)
        ));
        assert!(!blink.visible(
            Some(NSRange::new(1, 2)),
            start + Duration::from_millis(1800)
        ));
        assert!(!blink.visible(None, start + Duration::from_millis(1900)));
        assert!(blink.visible(cursor, start + Duration::from_millis(2000)));
    }
    #[test]
    fn explicit_selection_controls_answers_and_each_question_is_required() {
        let first = Question {
            id: "one".into(),
            title: "First?".into(),
            choices: vec![
                Choice {
                    label: "A".into(),
                    description: String::new(),
                },
                Choice {
                    label: "B".into(),
                    description: String::new(),
                },
            ],
            freeform: true,
            secret: false,
        };
        let mut second = first.clone();
        second.id = "two".into();
        let interaction = Interaction {
            command: None,
            questions: vec![first.clone(), second],
            actions: vec![],
        };
        assert_eq!(answer(&first, 0, "saved custom text").as_deref(), Some("A"));
        assert_eq!(answer(&first, 2, "custom").as_deref(), Some("custom"));
        assert!(answer(&first, 2, " \n").is_none());
        let answers = BTreeMap::from([
            ("one".into(), vec![answer(&first, 1, "ignored").unwrap()]),
            ("two".into(), vec![answer(&first, 2, "Custom 3").unwrap()]),
        ]);
        assert!(interaction.valid_answers(&answers));
        let mut incomplete = answers;
        incomplete.remove("two");
        assert!(!interaction.valid_answers(&incomplete));
        let mut restricted = first;
        restricted.freeform = false;
        assert!(answer(&restricted, 2, "unexpected").is_none());
    }
}
