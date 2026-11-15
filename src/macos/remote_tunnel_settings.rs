//! Shared native credential/disclosure UI for account-backed web tunnels.
use crate::web::Control;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use serde_json::Value;
use std::cell::{Cell, OnceCell, RefCell};
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
#[derive(Clone, Copy, Debug)]
pub(super) enum Provider {
    Ngrok,
    Tuna,
}
impl Provider {
    fn name(self) -> &'static str {
        match self {
            Self::Ngrok => "ngrok",
            Self::Tuna => "Tuna",
        }
    }
    fn key_label(self) -> &'static str {
        match self {
            Self::Ngrok => "Token:",
            Self::Tuna => "API key:",
        }
    }
    fn action(self, token: Option<String>) -> Control {
        match (self, token) {
            (Self::Ngrok, Some(token)) => Control::NgrokConfigure { token },
            (Self::Ngrok, None) => Control::NgrokStart,
            (Self::Tuna, Some(token)) => Control::TunaConfigure { token },
            (Self::Tuna, None) => Control::TunaStart,
        }
    }
}
define_class!(
    // SAFETY: Decoration-only view, confined to AppKit's main thread.
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[derive(Debug)]
    struct ThreadLine;
    unsafe impl NSObjectProtocol for ThreadLine{}
    impl ThreadLine{
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self,_point:NSPoint)->Option<Retained<NSView>>{None}
        #[unsafe(method(drawRect:))]
        fn draw(&self,_dirty:NSRect){super::visual::draw_thread_line(0.5,self.bounds().size.height,0.0,&[],false,false);}
    }
);
#[derive(Debug)]
struct Controls {
    name: Retained<NSButton>,
    status: Retained<NSTextField>,
    disclosure: Retained<NSButton>,
    line: Retained<ThreadLine>,
    label: Retained<NSTextField>,
    saved: Retained<NSTextField>,
    input: Retained<NSSecureTextField>,
    connect: Retained<NSButton>,
    change: Retained<NSButton>,
    cancel: Retained<NSButton>,
    dashboard: Retained<NSButton>,
    hint: Retained<NSTextField>,
    error: Retained<NSTextField>,
    images: [Retained<NSImage>; 2],
}
#[derive(Debug)]
pub(super) struct State {
    provider: Provider,
    controls: OnceCell<Controls>,
    report: RefCell<Value>,
    expanded: Cell<bool>,
    changing: Cell<bool>,
    busy: Cell<bool>,
    dirty: Cell<bool>,
    pending: RefCell<Option<Control>>,
}
define_class!(
    // SAFETY: Native controls, callbacks and state are main-thread confined.
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[ivars=State]
    #[derive(Debug)]
    pub(super) struct RemoteTunnelSettings;
    unsafe impl NSObjectProtocol for RemoteTunnelSettings{}
    unsafe impl NSControlTextEditingDelegate for RemoteTunnelSettings{
        #[unsafe(method(controlTextDidChange:))]
        fn text_changed(&self,_notification:&NSNotification){self.update_connect();}
    }
    unsafe impl NSTextFieldDelegate for RemoteTunnelSettings{}
    impl RemoteTunnelSettings{
        #[unsafe(method(toggleRemoteSection:))]
        fn toggle(&self,_sender:Option<&AnyObject>){self.ivars().expanded.set(!self.ivars().expanded.get());self.render();}
        #[unsafe(method(changeRemoteKey:))]
        fn change(&self,_sender:Option<&AnyObject>){
            self.ivars().changing.set(true);self.ivars().expanded.set(true);self.controls().input.setStringValue(ns_string!(""));self.render();
            if let Some(window)=self.window(){window.makeFirstResponder(Some(&*self.controls().input));}
        }
        #[unsafe(method(cancelRemoteKey:))]
        fn cancel(&self,_sender:Option<&AnyObject>){
            if let Some(window)=self.window(){unsafe{window.endEditingFor(None);}}
            self.ivars().changing.set(false);self.controls().input.setStringValue(ns_string!(""));self.render();
        }
        #[unsafe(method(connectRemote:))]
        fn connect(&self,_sender:Option<&AnyObject>){
            let editing=self.editing();let token=self.draft();if editing&&token.trim().is_empty(){return;}
            if let Some(window)=self.window(){unsafe{window.endEditingFor(None);}}
            self.ivars().changing.set(false);self.controls().input.setStringValue(ns_string!(""));
            *self.ivars().pending.borrow_mut()=Some(self.ivars().provider.action(editing.then_some(token)));
            self.ivars().busy.set(true);self.render();
        }
        #[unsafe(method(openRemoteDashboard:))]
        fn dashboard(&self,_sender:Option<&AnyObject>){
            let address=match self.ivars().provider{Provider::Ngrok=>"https://dashboard.ngrok.com/get-started/your-authtoken",Provider::Tuna=>"https://my.tuna.am/api_keys?lang=en"};
            if let Some(url)=NSURL::URLWithString(&NSString::from_str(address)){NSWorkspace::sharedWorkspace().openURL(&url);}
        }
    }
);
impl RemoteTunnelSettings {
    fn controls(&self) -> &Controls {
        self.ivars().controls.get().unwrap()
    }
    pub(super) fn new(mtm: MainThreadMarker, provider: Provider) -> Retained<Self> {
        let state = State {
            provider,
            controls: OnceCell::new(),
            report: RefCell::new(Value::Null),
            expanded: Cell::new(false),
            changing: Cell::new(false),
            busy: Cell::new(false),
            dirty: Cell::new(false),
            pending: RefCell::new(None),
        };
        let this: Retained<Self> = unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(state)),initWithFrame:rect(0.0,0.0,460.0,40.0)]
        };
        this.setAutoresizesSubviews(false);
        this.setIdentifier(Some(&NSString::from_str(&format!(
            "{}-section",
            provider.name().to_ascii_lowercase()
        ))));
        let label = |text: &str, size: f64| {
            let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
            field.setFont(Some(&NSFont::systemFontOfSize(size)));
            field.setTextColor(Some(&super::visual::secondary_text_color()));
            this.addSubview(&field);
            field
        };
        let button = |text: &str, action| {
            // SAFETY: Each selector is implemented by this retained view.
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(text),
                    Some(&*this),
                    Some(action),
                    mtm,
                )
            };
            this.addSubview(&button);
            button
        };
        let name = unsafe {
            NSButton::radioButtonWithTitle_target_action(
                &NSString::from_str(provider.name()),
                None,
                Some(sel!(selectWebAddress:)),
                mtm,
            )
        };
        name.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        this.addSubview(&name);
        let status = label("not configured", 12.0);
        let disclosure = button("", sel!(toggleRemoteSection:));
        disclosure.setImagePosition(NSCellImagePosition::ImageOnly);
        let line: Retained<ThreadLine> =
            unsafe { msg_send![ThreadLine::alloc(mtm),initWithFrame:rect(24.0,0.0,1.0,1.0)] };
        this.addSubview(&line);
        let key_label = label(provider.key_label(), 12.0);
        let saved = label("stored securely in macOS Keychain", 12.0);
        let input = NSSecureTextField::initWithFrame(
            NSSecureTextField::alloc(mtm),
            rect(38.0, 0.0, 234.0, 28.0),
        );
        input.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        input.setTextColor(Some(&super::visual::primary_text_color()));
        input.setBezeled(true);
        input.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
        input.setDrawsBackground(true);
        input.setBackgroundColor(Some(&super::visual::appearance_color(
            [237.0, 234.0, 244.0],
            [44.0, 37.0, 58.0],
            1.0,
        )));
        input.setPlaceholderString(Some(match provider {
            Provider::Ngrok => ns_string!("Paste token"),
            Provider::Tuna => ns_string!("Paste API key"),
        }));
        if let Some(cell) = input.cell() {
            cell.setUsesSingleLineMode(true);
            cell.setScrollable(true);
        }
        // SAFETY: This retained view implements the text delegate and action.
        unsafe {
            input.setDelegate(Some(ProtocolObject::from_ref(&*this)));
            input.setTarget(Some(&*this));
            input.setAction(Some(sel!(connectRemote:)));
            let _: () = msg_send![&*input,setAccessibilityLabel:&*NSString::from_str(&format!("{} {}",provider.name(),provider.key_label()))];
        }
        this.addSubview(&input);
        let connect = button("Connect", sel!(connectRemote:));
        let change = button("Change", sel!(changeRemoteKey:));
        let cancel = button("Cancel", sel!(cancelRemoteKey:));
        let dashboard = button("", sel!(openRemoteDashboard:));
        dashboard.setToolTip(Some(&NSString::from_str(&format!(
            "Get your {} credential in a browser",
            provider.name()
        ))));
        unsafe {
            let _: () = msg_send![&*dashboard,setAccessibilityLabel:&*NSString::from_str(&format!("Open {} dashboard in browser",provider.name()))];
        }
        let images = [
            include_bytes!("../../assets/token-dashboard-light.svg").as_slice(),
            include_bytes!("../../assets/token-dashboard-dark.svg").as_slice(),
        ]
        .map(|bytes| {
            let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))
                .expect("valid dashboard artwork");
            image.setSize(NSSize::new(36.0, 16.0));
            image
        });
        let hint = label("Will be stored securely in macOS Keychain", 10.0);
        let error = label("", 11.0);
        this.ivars()
            .controls
            .set(Controls {
                name,
                status,
                disclosure,
                line,
                label: key_label,
                saved,
                input,
                connect,
                change,
                cancel,
                dashboard,
                hint,
                error,
                images,
            })
            .unwrap();
        this.render();
        this
    }
    pub(super) fn set_report(&self, report: Value, busy: bool) {
        *self.ivars().report.borrow_mut() = report;
        self.ivars().busy.set(busy);
        self.render();
    }
    pub(super) fn set_route(&self, index: Option<usize>, selected: usize, target: &AnyObject) {
        let button = &self.controls().name;
        unsafe {
            button.setTarget(Some(target));
        }
        button.setTag(index.map_or(-1, |index| index as isize));
        button.setEnabled(index.is_some());
        button.setState(if index == Some(selected) {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
    }
    pub(super) fn take_action(&self) -> Option<Control> {
        self.ivars().pending.borrow_mut().take()
    }
    pub(super) fn take_layout_changed(&self) -> bool {
        self.ivars().dirty.replace(false)
    }
    fn editing(&self) -> bool {
        self.ivars().report.borrow()["tokenSaved"] != true || self.ivars().changing.get()
    }
    fn draft(&self) -> String {
        let field = &self.controls().input;
        field
            .currentEditor()
            .map(|editor| editor.string().to_string())
            .unwrap_or_else(|| field.stringValue().to_string())
    }
    fn update_connect(&self) {
        self.controls().connect.setEnabled(
            !self.ivars().busy.get()
                && self.ivars().report.borrow()["status"] != "starting"
                && (!self.editing() || !self.draft().trim().is_empty()),
        );
    }
    fn render(&self) {
        let state = self.ivars();
        let report = state.report.borrow();
        let c = self.controls();
        let saved = report["tokenSaved"] == true;
        let phase = report["status"].as_str().unwrap_or("not_configured");
        let failed = phase == "failed" || (phase == "off" && saved);
        let editing = !saved || state.changing.get();
        let expanded = state.expanded.get();
        let help = if let Some(error) = report["error"].as_str().filter(|_| failed) {
            Some(error)
        } else if matches!(state.provider, Provider::Tuna) && report["clientInstalled"] == false {
            Some("Install Tuna CLI: brew install yuccastream/tap/tuna")
        } else {
            None
        };
        let details = if !expanded {
            0.0
        } else {
            (if editing {
                104.0
            } else {
                48.0 + if failed { 36.0 } else { 0.0 }
            }) + if help.is_some() { 48.0 } else { 0.0 }
        };
        let height = 34.0 + details;
        if (self.frame().size.height - height).abs() > 0.5 {
            state.dirty.set(true);
        }
        self.setFrameSize(NSSize::new(460.0, height));
        let y = height - 28.0;
        let caption = match phase {
            "connected" => "ready",
            "starting" | "reconnecting" => "connecting…",
            "failed" => "connection failed",
            "off" if saved => "connection failed",
            _ => "not configured",
        };
        c.name.sizeToFit();
        c.name
            .setFrame(rect(24.0, y, c.name.frame().size.width + 4.0, 28.0));
        c.status.setStringValue(&NSString::from_str(caption));
        c.status.sizeToFit();
        c.status.setAlignment(NSTextAlignment::Right);
        c.status.setFrame(rect(280.0, y + 6.0, 156.0, 18.0));
        c.disclosure
            .setFrame(rect(24.0 + c.name.frame().size.width + 6.0, y, 28.0, 28.0));
        c.disclosure.setImage(
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                if expanded {
                    ns_string!("chevron.down")
                } else {
                    ns_string!("chevron.right")
                },
                None,
            )
            .as_deref(),
        );
        unsafe {
            let _: () = msg_send![&*c.disclosure,setAccessibilityExpanded:expanded];
            let _: () = msg_send![&*c.disclosure,setAccessibilityLabel:&*NSString::from_str(&format!("{} {} settings",if expanded{"Collapse"}else{"Expand"},state.provider.name()))];
        }
        c.line
            .setFrame(rect(24.0, y - details, 1.0, (details - 2.0).max(0.0)));
        c.line.setHidden(!expanded);
        c.line.setNeedsDisplay(true);
        c.label.sizeToFit();
        c.label.setFrameOrigin(NSPoint::new(
            38.0,
            if editing { y - 24.0 } else { y - 30.0 },
        ));
        c.label.setHidden(!expanded);
        let token_y = y - 60.0;
        let dashboard_width = 56.0;
        c.input
            .setFrame(rect(38.0, token_y, 290.0 - dashboard_width, 28.0));
        c.input.setHidden(!expanded || !editing);
        c.input.setEnabled(!state.busy.get());
        c.saved.sizeToFit();
        c.saved.setFrameOrigin(NSPoint::new(
            38.0 + c.label.frame().size.width + 2.0,
            y - 30.0,
        ));
        c.saved.setHidden(!expanded || editing);
        c.change.setFrame(rect(
            c.saved.frame().origin.x + c.saved.frame().size.width + 8.0,
            y - 36.0,
            72.0,
            28.0,
        ));
        c.change.setHidden(!expanded || editing);
        c.change.setEnabled(!state.busy.get());
        c.cancel.setFrame(rect(364.0, y - 28.0, 72.0, 24.0));
        c.cancel.setHidden(!expanded || !state.changing.get());
        c.cancel.setEnabled(!state.busy.get());
        c.connect.setTitle(if editing {
            ns_string!("Connect")
        } else {
            ns_string!("Reconnect")
        });
        c.connect.setFrame(if editing {
            rect(344.0, token_y, 92.0, 28.0)
        } else {
            rect(38.0, y - 72.0, 108.0, 28.0)
        });
        c.connect.setHidden(!expanded || (!editing && !failed));
        self.update_connect();
        c.hint.setFrame(rect(38.0, y - 86.0, 398.0, 18.0));
        c.hint.setHidden(!expanded || !editing);
        c.error
            .setStringValue(&NSString::from_str(help.unwrap_or("")));
        c.error.setFrame(rect(38.0, y - details + 8.0, 398.0, 40.0));
        c.error.setHidden(!expanded || help.is_none());
        c.dashboard.setFrame(rect(
            336.0 - dashboard_width,
            token_y,
            dashboard_width,
            28.0,
        ));
        c.dashboard.setHidden(!expanded || !editing);
        c.dashboard.setTitle(ns_string!(""));
        c.dashboard.setImagePosition(NSCellImagePosition::ImageOnly);
        c.dashboard
            .setImage(Some(&c.images[usize::from(super::visual::is_dark(self))]));
    }
}
