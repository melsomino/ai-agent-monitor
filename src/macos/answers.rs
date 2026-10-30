//! Explicit structured replies in a normal editable window, outside NSMenu tracking.
use crate::agents::{
    interaction::{Interaction, Question, Response},
    sessions::Session,
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
    cell::{OnceCell, RefCell},
    collections::BTreeMap,
};

struct Input {
    question: Question,
    choice: Option<Retained<NSPopUpButton>>,
    text: Option<Retained<NSTextField>>,
}
pub struct State {
    monitor: crate::actors::monitor::Handle,
    ui: crate::macos::bridge::Sender,
    session: Session,
    interaction: Interaction,
    window: OnceCell<Retained<NSWindow>>,
    inputs: RefCell<Vec<Input>>,
    submit: OnceCell<Retained<NSButton>>,
    status: OnceCell<Retained<NSTextField>>,
}
define_class!(
    // SAFETY: NSObject subclass confined to the UI thread; weak action/delegate
    // targets are retained in FORMS until after the window-close callback.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    pub struct AnswerForm;
    unsafe impl NSObjectProtocol for AnswerForm {}
    unsafe impl NSWindowDelegate for AnswerForm {
        #[unsafe(method(windowWillClose:))]
        fn closed(&self, _notification: &NSNotification) {
            if let Some(key)=self.key(){let _=self.ivars().ui.try_send(crate::macos::bridge::Event::FormClosed(key));}
        }
    }
    impl AnswerForm {
        #[unsafe(method(sendAnswers:))]
        fn send(&self, _sender: Option<&AnyObject>) {
            let mut answers = BTreeMap::new();
            for input in self.ivars().inputs.borrow().iter() {
                let custom = input.text.as_ref().map(|field| field.stringValue().to_string()).unwrap_or_default();
                let value = if !custom.trim().is_empty() { Some(custom) } else {
                    input.choice.as_ref().and_then(|choice| {
                        let index = choice.indexOfSelectedItem();
                        (index > 0).then(|| input.question.choices.get(index as usize - 1).map(|choice| choice.label.clone())).flatten()
                    })
                };
                if let Some(value) = value { answers.insert(input.question.id.clone(), vec![value]); }
            }
            if !self.ivars().interaction.valid_answers(&answers) {
                self.ivars().status.get().unwrap().setStringValue(ns_string!("Please answer every question."));
                return;
            }
            if let Err(error)=self.ivars().monitor.submit(&self.ivars().session,Response::Answers(answers)){
                self.ivars().submit.get().unwrap().setEnabled(true);
                self.ivars().status.get().unwrap().setStringValue(&NSString::from_str(&error));
            }
        }
        #[unsafe(method(openEditor:))]
        fn open_editor(&self, _sender: Option<&AnyObject>) {
            self.ivars().monitor.open(&self.ivars().session);
        }
    }
);
define_class!(
    // SAFETY: Standard NSView with flipped document coordinates, no ivars.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    struct FormDocument;
    unsafe impl NSObjectProtocol for FormDocument {}
    impl FormDocument {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
    }
);
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
fn text(
    view: &NSView,
    value: &str,
    y: f64,
    size: f64,
    secondary: bool,
    mtm: MainThreadMarker,
) -> f64 {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(value), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(size)));
    if secondary {
        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
    }
    let height = label
        .cell()
        .map(|cell| {
            cell.cellSizeForBounds(rect(0.0, 0.0, 472.0, 100000.0))
                .height
                .ceil()
        })
        .unwrap_or(24.0)
        .max(20.0);
    label.setFrame(rect(0.0, y, 472.0, height));
    label.setSelectable(true);
    view.addSubview(&label);
    height
}
impl AnswerForm {
    pub fn key(&self) -> Option<String> {
        self.ivars().session.approval_key()
    }
    pub fn reconcile(&self, state: &crate::actors::monitor::State) {
        let key = self.key();
        let active = state.sessions.iter().any(|s| {
            s.key() == self.ivars().session.key()
                && s.approval.is_some()
                && s.approval == self.ivars().session.approval
        });
        if !active {
            if let Some(window) = self.ivars().window.get() {
                window.close();
            }
            return;
        }
        if let Some(key) = key {
            let phase = state.submissions.get(&key);
            self.ivars().submit.get().unwrap().setEnabled(
                state
                    .sessions
                    .iter()
                    .find(|s| s.key() == self.ivars().session.key())
                    .is_some_and(|s| s.authoritative)
                    && !matches!(
                        phase,
                        Some(
                            crate::actors::monitor::Submission::Sending
                                | crate::actors::monitor::Submission::Reconciling
                        )
                    ),
            );
            self.ivars()
                .status
                .get()
                .unwrap()
                .setStringValue(&NSString::from_str(match phase {
                    Some(crate::actors::monitor::Submission::Sending) => "Sending…",
                    Some(crate::actors::monitor::Submission::Reconciling) => "Checking delivery…",
                    Some(crate::actors::monitor::Submission::Failed(e)) => e,
                    None => "",
                }));
        }
    }
}
pub fn show_with_draft(
    session: Session,
    draft: Option<BTreeMap<String, Vec<String>>>,
    monitor: crate::actors::monitor::Handle,
    ui: crate::macos::bridge::Sender,
    forms: &[Retained<AnswerForm>],
) -> Option<Retained<AnswerForm>> {
    let mtm = MainThreadMarker::new().expect("answer forms are main-thread only");
    if let Some(form) = forms
        .iter()
        .find(|form| form.key() == session.approval_key())
    {
        form.ivars()
            .window
            .get()
            .unwrap()
            .makeKeyAndOrderFront(None);
        return None;
    }
    let interaction = session.interaction();
    if interaction.questions.is_empty() {
        return None;
    }
    let allocated = AnswerForm::alloc(mtm).set_ivars(State {
        monitor,
        ui,
        session,
        interaction,
        window: OnceCell::new(),
        inputs: RefCell::new(vec![]),
        submit: OnceCell::new(),
        status: OnceCell::new(),
    });
    // SAFETY: NSObject initializer after fully initializing the form's ivars.
    let form: Retained<AnswerForm> = unsafe { msg_send![super(allocated), init] };
    // SAFETY: Ivar-free NSView subclass initialized with its designated initializer.
    let document: Retained<FormDocument> =
        unsafe { msg_send![FormDocument::alloc(mtm), initWithFrame: rect(0.0, 0.0, 488.0, 100.0)] };
    let mut y = 4.0;
    for question in &form.ivars().interaction.questions {
        y += text(&document, &question.title, y, 14.0, false, mtm) + 8.0;
        let descriptions = question
            .choices
            .iter()
            .filter(|choice| !choice.description.is_empty())
            .map(|choice| format!("{} — {}", choice.label, choice.description))
            .collect::<Vec<_>>()
            .join("\n");
        if !descriptions.is_empty() {
            y += text(&document, &descriptions, y, 12.0, true, mtm) + 8.0;
        }
        let choice = if !question.choices.is_empty() {
            let popup = NSPopUpButton::initWithFrame_pullsDown(
                NSPopUpButton::alloc(mtm),
                rect(0.0, y, 472.0, 28.0),
                false,
            );
            popup.addItemWithTitle(ns_string!("Choose an answer…"));
            for choice in &question.choices {
                popup.addItemWithTitle(&NSString::from_str(&choice.label));
            }
            popup.selectItemAtIndex(0);
            document.addSubview(&popup);
            y += 34.0;
            Some(popup)
        } else {
            None
        };
        let field = if question.freeform {
            let field: Retained<NSTextField> = if question.secret {
                NSSecureTextField::initWithFrame(
                    NSSecureTextField::alloc(mtm),
                    rect(0.0, y, 472.0, 28.0),
                )
                .into_super()
            } else {
                NSTextField::initWithFrame(NSTextField::alloc(mtm), rect(0.0, y, 472.0, 28.0))
            };
            field.setPlaceholderString(Some(if choice.is_some() {
                ns_string!("Or type your own answer")
            } else {
                ns_string!("Your answer")
            }));
            field.setEditable(true);
            field.setSelectable(true);
            field.setFont(Some(&NSFont::systemFontOfSize(13.0)));
            document.addSubview(&field);
            y += 34.0;
            Some(field)
        } else {
            None
        };
        if let Some(answer) = draft
            .as_ref()
            .and_then(|answers| answers.get(&question.id))
            .and_then(|values| values.first())
        {
            if let Some(index) = question
                .choices
                .iter()
                .position(|option| option.label == *answer)
            {
                if let Some(choice) = &choice {
                    choice.selectItemAtIndex(index as isize + 1);
                }
            } else if let Some(field) = &field {
                field.setStringValue(&NSString::from_str(answer));
            }
        }
        form.ivars().inputs.borrow_mut().push(Input {
            question: question.clone(),
            choice,
            text: field,
        });
        y += 16.0;
    }
    document.setFrameSize(NSSize::new(488.0, y));
    let maximum = NSScreen::mainScreen(mtm)
        .map(|screen| screen.visibleFrame().size.height * 0.8)
        .unwrap_or(640.0);
    let height = (y + 84.0).max(220.0).min(maximum);
    // SAFETY: Create an owned standard window on the main thread.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(0.0, 0.0, 520.0, height),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setTitle(&NSString::from_str(&form.ivars().session.title));
    // SAFETY: The form owns the window; NSWindow must not release itself on close.
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    window.setDelegate(Some(ProtocolObject::from_ref(&*form)));
    window.setLevel(NSFloatingWindowLevel);
    let content = window.contentView().unwrap();
    let scroll = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        rect(16.0, 68.0, 488.0, height - 84.0),
    );
    scroll.setDrawsBackground(false);
    scroll.setHasVerticalScroller(true);
    scroll.setAutohidesScrollers(true);
    scroll.setDocumentView(Some(&document));
    content.addSubview(&scroll);
    let status = NSTextField::labelWithString(ns_string!(""), mtm);
    status.setFrame(rect(16.0, 46.0, 488.0, 18.0));
    status.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    content.addSubview(&status);
    form.ivars().status.set(status).unwrap();
    // SAFETY: Implemented selectors; FORMS retains their weak target.
    let submit = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Send answer"),
            Some(&*form),
            Some(sel!(sendAnswers:)),
            mtm,
        )
    };
    submit.setFrame(rect(360.0, 12.0, 144.0, 30.0));
    submit.setBezelStyle(NSBezelStyle::Push);
    submit.setKeyEquivalent(ns_string!("\r"));
    content.addSubview(&submit);
    form.ivars().submit.set(submit).unwrap();
    // SAFETY: Implemented selector on the retained form.
    let editor = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Open in editor"),
            Some(&*form),
            Some(sel!(openEditor:)),
            mtm,
        )
    };
    editor.setFrame(rect(16.0, 12.0, 144.0, 30.0));
    editor.setBezelStyle(NSBezelStyle::Push);
    content.addSubview(&editor);
    window.center();
    form.ivars().window.set(window.clone()).unwrap();

    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    window.makeKeyAndOrderFront(None);
    Some(form)
}
