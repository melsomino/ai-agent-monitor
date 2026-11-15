//! In-popup management of the resident web listener. IPC work never blocks AppKit.
use crate::web::Control;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained, runtime::AnyObject, sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use serde_json::Value;
use std::{
    cell::{Cell, OnceCell, RefCell},
    sync::mpsc,
    time::{Duration, Instant},
};
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
#[derive(Default)]
struct QrState {
    code: RefCell<Option<qrcode::QrCode>>,
}
impl std::fmt::Debug for QrState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QrState")
    }
}
define_class!(
    // SAFETY: Main-thread, decoration-only QR view with initialized Rust state.
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[ivars=QrState]
    #[derive(Debug)]
    struct QrView;
    unsafe impl NSObjectProtocol for QrView {}
    impl QrView {
        #[unsafe(method(drawRect:))]
        fn draw(&self,_dirty:NSRect){
            let code=self.ivars().code.borrow();let Some(code)=code.as_ref() else{return;};
            NSColor::whiteColor().setFill();NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(self.bounds(),10.0,10.0).fill();
            let size=code.width()+8;let unit=(self.bounds().size.width/size as f64).floor();let offset=(self.bounds().size.width-unit*size as f64)/2.0;
            NSColor::colorWithSRGBRed_green_blue_alpha(0.0,0.0,0.0,1.0).setFill();
            let path=NSBezierPath::bezierPath();
            for y in 0..code.width(){for x in 0..code.width(){if code[(x,y)]==qrcode::Color::Dark {path.appendBezierPathWithRect(rect(offset+(x+4) as f64*unit,offset+(code.width()-1-y+4) as f64*unit,unit,unit));}}}
            path.fill();
        }
    }
);
define_class!(
    // SAFETY: Link-only button; AppKit calls cursor setup on the main thread.
    #[unsafe(super(NSButton))]
    #[thread_kind=MainThreadOnly]
    #[derive(Debug)]
    struct WebLink;
    unsafe impl NSObjectProtocol for WebLink {}
    impl WebLink {
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) { self.addCursorRect_cursor(self.bounds(), &NSCursor::pointingHandCursor()); }
    }
);
#[derive(Debug)]
pub(super) struct State {
    toggle: OnceCell<Retained<NSSwitch>>,
    ngrok: OnceCell<Retained<super::remote_tunnel_settings::RemoteTunnelSettings>>,
    tuna: OnceCell<Retained<super::remote_tunnel_settings::RemoteTunnelSettings>>,
    layout_dirty: Cell<bool>,
    title: OnceCell<Retained<NSTextField>>,
    subtitle: OnceCell<Retained<NSTextField>>,
    remote_note: OnceCell<Retained<NSTextField>>,
    address_document: OnceCell<Retained<NSView>>,
    addresses: RefCell<Vec<Retained<NSButton>>>,
    urls: RefCell<Vec<String>>,
    qr: OnceCell<Retained<QrView>>,
    note: OnceCell<Retained<NSTextField>>,
    progress: OnceCell<Retained<NSProgressIndicator>>,
    remaining: OnceCell<Retained<NSTextField>>,
    deadline: Cell<Option<Instant>>,
    devices: OnceCell<Retained<NSTextField>>,
    renew: OnceCell<Retained<NSButton>>,
    copy: OnceCell<Retained<NSButton>>,
    link: OnceCell<Retained<WebLink>>,
    separator: OnceCell<Retained<NSBox>>,
    footer: OnceCell<Retained<NSTextField>>,
    report: RefCell<Value>,
    links: RefCell<Vec<String>>,
    selected: Cell<usize>,
    busy: Cell<bool>,
    pending: RefCell<Option<Control>>,
    error: RefCell<Option<String>>,
    last_poll: Cell<Option<Instant>>,
    tx: mpsc::Sender<anyhow::Result<Value>>,
    rx: RefCell<mpsc::Receiver<anyhow::Result<Value>>>,
}
impl Default for State {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            toggle: OnceCell::new(),
            ngrok: OnceCell::new(),
            tuna: OnceCell::new(),
            layout_dirty: Cell::new(false),
            title: OnceCell::new(),
            subtitle: OnceCell::new(),
            remote_note: OnceCell::new(),
            address_document: OnceCell::new(),
            addresses: RefCell::new(vec![]),
            urls: RefCell::new(vec![]),
            qr: OnceCell::new(),
            note: OnceCell::new(),
            progress: OnceCell::new(),
            remaining: OnceCell::new(),
            deadline: Cell::new(None),
            devices: OnceCell::new(),
            renew: OnceCell::new(),
            copy: OnceCell::new(),
            link: OnceCell::new(),
            separator: OnceCell::new(),
            footer: OnceCell::new(),
            report: RefCell::new(Value::Null),
            links: RefCell::new(vec![]),
            selected: Cell::new(0),
            busy: Cell::new(false),
            pending: RefCell::new(None),
            error: RefCell::new(None),
            last_poll: Cell::new(None),
            tx,
            rx: RefCell::new(rx),
        }
    }
}
define_class!(
    // SAFETY: Main-thread settings pane; asynchronous workers capture only a Rust sender.
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[ivars=State]
    #[derive(Debug)]
    pub(super) struct WebSettings;
    unsafe impl NSObjectProtocol for WebSettings {}
    impl WebSettings {
        #[unsafe(method(toggleWeb:))]
        fn toggle(&self,_sender:Option<&AnyObject>){
            let enabled=self.ivars().toggle.get().unwrap().state()==NSControlStateValueOn;
            self.request(if enabled{Control::Start{listen:"0.0.0.0:8765".parse().unwrap()}}else{Control::Stop});
        }
        #[unsafe(method(renewWebPairing:))]
        fn renew(&self,_sender:Option<&AnyObject>){self.request(Control::Pair);}
        #[unsafe(method(selectWebAddress:))]
        fn select_address(&self,sender:Option<&AnyObject>){
            if let Some(button)=sender.and_then(|sender|sender.downcast_ref::<NSButton>()) {
                let index=button.tag();
                if index>=0 && (index as usize)<self.ivars().urls.borrow().len(){
                    self.ivars().selected.set(index as usize);self.render();
                }
            }
        }
        #[unsafe(method(openWebLink:))]
        fn open_link(&self,_sender:Option<&AnyObject>){
            if let Some(link)=self.ivars().links.borrow().get(self.ivars().selected.get())
                && let Some(url)=NSURL::URLWithString(&NSString::from_str(link)){NSWorkspace::sharedWorkspace().openURL(&url);}
        }
        #[unsafe(method(copyWebLink:))]
        fn copy_link(&self,_sender:Option<&AnyObject>){
            if let Some(link)=self.ivars().links.borrow().get(self.ivars().selected.get()) {
                let board=NSPasteboard::generalPasteboard();board.clearContents();board.setString_forType(&NSString::from_str(link),unsafe{NSPasteboardTypeString});
            }
        }
    }
);
impl WebSettings {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: State is initialized before NSView's designated initializer.
        let this: Retained<Self> = unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(State::default())),initWithFrame:rect(0.0,0.0,460.0,492.0)]
        };
        this.setAutoresizesSubviews(false);
        let label = |text: &str, frame: NSRect, size: f64, bold: bool| {
            let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
            field.setFrame(frame);
            let font = if bold {
                NSFont::boldSystemFontOfSize(size)
            } else {
                NSFont::systemFontOfSize(size)
            };
            field.setFont(Some(&font));
            field.setTextColor(Some(&super::visual::secondary_text_color()));
            this.addSubview(&field);
            field
        };
        let title = label("Web access", rect(24.0, 454.0, 330.0, 24.0), 15.0, true);
        title.setTextColor(Some(&super::visual::primary_text_color()));
        this.ivars().title.set(title).unwrap();
        let subtitle = label(
            "Open your monitor in a browser on your phone.",
            rect(24.0, 424.0, 412.0, 22.0),
            11.0,
            false,
        );
        this.ivars().subtitle.set(subtitle).unwrap();
        let toggle = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), rect(394.0, 452.0, 42.0, 28.0));
        // SAFETY: Implemented selectors target this retained pane.
        unsafe {
            toggle.setTarget(Some(&*this));
            toggle.setAction(Some(sel!(toggleWeb:)));
            let _: () = msg_send![&*toggle,setAccessibilityLabel:ns_string!("Enable web access")];
        }
        this.addSubview(&toggle);
        this.ivars().toggle.set(toggle).unwrap();
        for (provider, storage) in [
            (
                super::remote_tunnel_settings::Provider::Ngrok,
                &this.ivars().ngrok,
            ),
            (
                super::remote_tunnel_settings::Provider::Tuna,
                &this.ivars().tuna,
            ),
        ] {
            let section = super::remote_tunnel_settings::RemoteTunnelSettings::new(mtm, provider);
            this.addSubview(&section);
            storage.set(section).unwrap();
        }
        let remote_note = label("", rect(24.0, 388.0, 412.0, 28.0), 11.0, false);
        this.ivars().remote_note.set(remote_note).unwrap();
        // Inline choices avoid nesting another tracking menu inside the status menu.
        let document = NSView::initWithFrame(NSView::alloc(mtm), rect(24.0, 300.0, 412.0, 0.0));
        document.setAutoresizesSubviews(false);
        this.addSubview(&document);
        this.ivars().address_document.set(document).unwrap();
        let qr: Retained<QrView> = unsafe {
            msg_send![super(QrView::alloc(mtm).set_ivars(QrState::default())),initWithFrame:rect(24.0,88.0,200.0,200.0)]
        };
        qr.setIdentifier(Some(ns_string!("web-pairing-qr")));
        this.addSubview(&qr);
        this.ivars().qr.set(qr).unwrap();
        let progress = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(mtm),
            rect(24.0, 66.0, 148.0, 12.0),
        );
        progress.setIndeterminate(false);
        progress.setMinValue(0.0);
        progress.setMaxValue(120.0);
        this.addSubview(&progress);
        this.ivars().progress.set(progress).unwrap();
        let remaining = label("", rect(178.0, 62.0, 48.0, 20.0), 11.0, false);
        this.ivars().remaining.set(remaining).unwrap();
        let note = label(
            "Web access is off.",
            rect(240.0, 156.0, 196.0, 128.0),
            12.0,
            false,
        );
        this.ivars().note.set(note).unwrap();
        for (text, action, y, storage) in [
            (
                "arrow.clockwise",
                sel!(renewWebPairing:),
                124.0,
                &this.ivars().renew,
            ),
            ("doc.on.doc", sel!(copyWebLink:), 88.0, &this.ivars().copy),
        ] {
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(text),
                    Some(&*this),
                    Some(action),
                    mtm,
                )
            };
            button.setTitle(ns_string!(""));
            button.setImage(
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(text),
                    None,
                )
                .as_deref(),
            );
            button.setImagePosition(NSCellImagePosition::ImageOnly);
            let help = if text == "doc.on.doc" {
                "Copy link"
            } else {
                "Generate new QR code"
            };
            button.setToolTip(Some(&NSString::from_str(help)));
            unsafe {
                let _: () = msg_send![&*button,setAccessibilityLabel:&*NSString::from_str(help)];
            }
            button.setFrame(rect(408.0, y, 28.0, 28.0));
            this.addSubview(&button);
            storage.set(button).unwrap();
        }
        let devices = label("", rect(24.0, 32.0, 412.0, 22.0), 11.0, false);
        this.ivars().devices.set(devices).unwrap();
        let footer = label(
            "Turning Web access off disconnects all browsers and stops the tunnels.",
            rect(24.0, 4.0, 412.0, 26.0),
            10.0,
            false,
        );
        this.ivars().footer.set(footer).unwrap();
        let link: Retained<WebLink> =
            unsafe { msg_send![WebLink::alloc(mtm),initWithFrame:rect(24.0,300.0,376.0,28.0)] };
        link.setBordered(false);
        link.setAlignment(NSTextAlignment::Left);
        link.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        if let Some(cell) = link.cell() {
            cell.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        }
        unsafe {
            link.setTarget(Some(&*this));
            link.setAction(Some(sel!(openWebLink:)));
        }
        this.addSubview(&link);
        this.ivars().link.set(link).unwrap();
        let separator = NSBox::initWithFrame(NSBox::alloc(mtm), rect(24.0, 330.0, 412.0, 1.0));
        separator.setBoxType(NSBoxType::Separator);
        this.addSubview(&separator);
        this.ivars().separator.set(separator).unwrap();
        this.render();
        this
    }
    pub fn take_layout_changed(&self) -> bool {
        self.ivars().layout_dirty.replace(false)
    }
    pub fn poll(&self, visible: bool) {
        for section in [
            self.ivars().ngrok.get().unwrap(),
            self.ivars().tuna.get().unwrap(),
        ] {
            if let Some(action) = section.take_action() {
                self.request(action);
            }
        }
        let section_changed = self.ivars().ngrok.get().unwrap().take_layout_changed()
            | self.ivars().tuna.get().unwrap().take_layout_changed();
        if section_changed {
            self.render();
        }
        while let Ok(result) = self.ivars().rx.borrow_mut().try_recv() {
            self.ivars().busy.set(false);
            match result {
                Ok(report) => {
                    self.set_report(report);
                }
                Err(error) => {
                    *self.ivars().error.borrow_mut() = Some(format!("{error:#}"));
                    self.render();
                }
            }
        }
        if visible {
            self.refresh_countdown_at(Instant::now());
        }
        let pending = self.ivars().pending.borrow_mut().take();
        if let Some(action) = pending {
            self.request(action);
        }
        if visible
            && !self.ivars().busy.get()
            && self
                .ivars()
                .last_poll
                .get()
                .is_none_or(|at| at.elapsed() > Duration::from_secs(2))
        {
            let expired = self.refresh_countdown_at(Instant::now());
            self.request(if expired {
                Control::Pair
            } else {
                Control::Status
            });
        }
    }
    fn request(&self, action: Control) {
        if self.ivars().busy.replace(true) {
            if !matches!(action, Control::Status) {
                *self.ivars().pending.borrow_mut() = Some(action);
            }
            return;
        }
        self.ivars().last_poll.set(Some(Instant::now()));
        if !matches!(action, Control::Status) {
            self.ivars().error.borrow_mut().take();
            self.ivars().toggle.get().unwrap().setEnabled(false);
            self.ivars().renew.get().unwrap().setEnabled(false);
        }
        let tx = self.ivars().tx.clone();
        std::thread::spawn(move || {
            let result = enable_access(action, |action| {
                crate::app::ipc::request(&crate::app::ipc::Request::Web { action })
            });
            let _ = tx.send(result);
        });
    }
    pub(super) fn set_report(&self, report: Value) {
        self.ivars().deadline.set(
            report["pairingExpiresIn"]
                .as_u64()
                .map(|seconds| Instant::now() + Duration::from_secs(seconds)),
        );
        *self.ivars().report.borrow_mut() = report;
        self.render();
    }
    fn render(&self) {
        let report = self.ivars().report.borrow();
        let running = report["running"] == true;
        let toggle = self.ivars().toggle.get().unwrap();
        let error = self.ivars().error.borrow();
        let ngrok = &report["ngrok"];
        let tuna = &report["tuna"];
        for (section, status) in [
            (self.ivars().ngrok.get().unwrap(), ngrok),
            (self.ivars().tuna.get().unwrap(), tuna),
        ] {
            section.set_report(status.clone(), self.ivars().busy.get());
        }
        let section_height = self.ivars().ngrok.get().unwrap().frame().size.height
            + self.ivars().tuna.get().unwrap().frame().size.height;
        let remote_text = if let Some(error) = error.as_deref() {
            error
        } else if !running {
            error.as_deref().unwrap_or("")
        } else if let Some(error) = report["tunnel"]["error"].as_str() {
            error
        } else {
            match report["tunnel"]["status"].as_str().unwrap_or("off") {
                "starting" => "Connecting to Cloudflare…",
                "reconnecting" => "Cloudflare connection interrupted. Reconnecting…",
                _ => "",
            }
        };
        self.ivars()
            .remote_note
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(remote_text));
        let urls: Vec<String> = report["urls"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let provider_index = |provider: &Value| {
            urls.iter().position(|url| {
                provider["url"]
                    .as_str()
                    .is_some_and(|origin| url.trim_end_matches('/') == origin.trim_end_matches('/'))
            })
        };
        let ngrok_index = provider_index(ngrok);
        let tuna_index = provider_index(tuna);
        let cloud_index = provider_index(&report["tunnel"]).or_else(|| {
            urls.iter()
                .enumerate()
                .find(|(index, url)| {
                    Some(*index) != ngrok_index
                        && Some(*index) != tuna_index
                        && url.starts_with("https://")
                })
                .map(|(index, _)| index)
        });
        let mut route_indices = vec![cloud_index];
        route_indices.extend(
            (0..urls.len())
                .filter(|index| {
                    Some(*index) != ngrok_index
                        && Some(*index) != tuna_index
                        && Some(*index) != cloud_index
                })
                .map(Some),
        );
        let address_height = route_indices.len() as f64 * 34.0;
        let pane_height = if running {
            426.0
                + address_height
                + section_height
                + if remote_text.is_empty() { 0.0 } else { 40.0 }
        } else if error.is_some() {
            132.0
        } else {
            92.0
        };
        if (self.frame().size.height - pane_height).abs() > 0.5 {
            self.ivars().layout_dirty.set(true);
        }
        self.setFrameSize(NSSize::new(460.0, pane_height));
        let title = self.ivars().title.get().unwrap();
        let subtitle = self.ivars().subtitle.get().unwrap();
        title.setFrameOrigin(NSPoint::new(24.0, pane_height - 38.0));
        subtitle.setFrameOrigin(NSPoint::new(24.0, pane_height - 68.0));
        toggle.setFrameOrigin(NSPoint::new(394.0, pane_height - 40.0));
        let status = self.ivars().remote_note.get().unwrap();
        status.setFrameOrigin(NSPoint::new(24.0, if running { 346.0 } else { 4.0 }));
        let children = self.subviews();
        for index in 0..children.count() {
            let child = children.objectAtIndex(index);
            let header = std::ptr::eq(&*child, &**title as &NSView)
                || std::ptr::eq(&*child, &**subtitle as &NSView)
                || std::ptr::eq(&*child, &**toggle as &NSView);
            child.setHidden(!running && !header);
        }
        status.setHidden(remote_text.is_empty());
        let mut section_top = pane_height - 80.0;
        for section in [
            self.ivars().ngrok.get().unwrap(),
            self.ivars().tuna.get().unwrap(),
        ] {
            section_top -= section.frame().size.height;
            section.setFrameOrigin(NSPoint::new(0.0, section_top));
            section.setHidden(!running);
        }
        toggle.setState(if running {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        toggle.setEnabled(!self.ivars().busy.get());
        self.ivars().address_document.get().unwrap().setFrame(rect(
            24.0,
            section_top - address_height,
            412.0,
            address_height,
        ));
        let mut old_urls = self.ivars().urls.borrow_mut();
        if *old_urls != urls {
            let selected = old_urls.get(self.ivars().selected.get()).cloned();
            let new_remote = urls
                .iter()
                .position(|url| url.starts_with("https://") && !old_urls.contains(url));
            self.ivars().selected.set(
                new_remote
                    .or_else(|| {
                        selected
                            .as_ref()
                            .and_then(|value| urls.iter().position(|url| url == value))
                    })
                    .unwrap_or(0),
            );
            let document = self.ivars().address_document.get().unwrap();
            let mut buttons = self.ivars().addresses.borrow_mut();
            buttons.clear();
            let children = document.subviews();
            for index in 0..children.count() {
                children.objectAtIndex(index).removeFromSuperview();
            }
            let height = address_height;
            document.setFrameSize(NSSize::new(412.0, height));
            for (row, index) in route_indices.iter().enumerate() {
                let url = index.and_then(|index| urls.get(index));
                let cloud = row == 0;
                let name = if cloud {
                    "Cloudflare"
                } else {
                    address_kind(url.unwrap())
                };
                let button = unsafe {
                    NSButton::radioButtonWithTitle_target_action(
                        &NSString::from_str(name),
                        Some(self),
                        Some(sel!(selectWebAddress:)),
                        self.mtm(),
                    )
                };
                let y = height - (row + 1) as f64 * 34.0;
                let note = NSTextField::labelWithString(ns_string!("ready"), self.mtm());
                note.setIdentifier(Some(ns_string!("route-status")));
                note.setFont(Some(&NSFont::systemFontOfSize(12.0)));
                note.setTextColor(Some(&super::visual::secondary_text_color()));
                note.setAlignment(NSTextAlignment::Right);
                note.setFrame(rect(260.0, y + 6.0, 152.0, 18.0));
                document.addSubview(&note);
                button.setFrame(rect(0.0, y, 252.0, 28.0));
                button.setTag(index.map_or(-1, |index| index as isize));
                button.setToolTip(url.map(|url| NSString::from_str(url)).as_deref());
                button.setFont(Some(&NSFont::systemFontOfSize(12.0)));
                document.addSubview(&button);
                buttons.push(button);
            }
            *old_urls = urls;
        }
        for button in self.ivars().addresses.borrow().iter() {
            button.setEnabled(running && button.tag() >= 0);
            button.setState(if button.tag() == self.ivars().selected.get() as isize {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        for (section, index) in [
            (self.ivars().ngrok.get().unwrap(), ngrok_index),
            (self.ivars().tuna.get().unwrap(), tuna_index),
        ] {
            section.set_route(index, self.ivars().selected.get(), self);
        }
        let document = self.ivars().address_document.get().unwrap();
        let children = document.subviews();
        for index in 0..children.count() {
            if let Some(label) = children.objectAtIndex(index).downcast_ref::<NSTextField>()
                && label.frame().origin.y > address_height - 34.0
            {
                let caption = match report["tunnel"]["status"].as_str() {
                    Some("connected") => "ready",
                    Some("starting" | "reconnecting") => "connecting…",
                    Some("failed") => "connection failed",
                    _ => "not configured",
                };
                label.setStringValue(&NSString::from_str(caption));
            }
        }
        *self.ivars().links.borrow_mut() = report["pairingLinks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let valid = running && !self.ivars().links.borrow().is_empty();
        let link = self.ivars().link.get().unwrap();
        let address = old_urls
            .get(self.ivars().selected.get())
            .map(String::as_str)
            .unwrap_or("");
        let title = NSString::from_str(address);
        let attributed =
            NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &title);
        unsafe {
            attributed.addAttribute_value_range(
                NSForegroundColorAttributeName,
                &NSColor::linkColor(),
                NSRange::new(0, title.length()),
            );
            attributed.addAttribute_value_range(
                NSUnderlineStyleAttributeName,
                &NSNumber::new_i32(1),
                NSRange::new(0, title.length()),
            );
            attributed.addAttribute_value_range(
                NSFontAttributeName,
                &NSFont::systemFontOfSize(12.0),
                NSRange::new(0, title.length()),
            );
        }
        link.setAttributedTitle(&attributed);
        link.setToolTip(Some(&title));
        link.setEnabled(valid);
        link.setFrame(rect(24.0, 298.0, 376.0, 28.0));
        self.ivars()
            .copy
            .get()
            .unwrap()
            .setFrame(rect(408.0, 298.0, 28.0, 28.0));
        self.ivars()
            .renew
            .get()
            .unwrap()
            .setFrame(rect(408.0, 268.0, 28.0, 28.0));
        self.ivars()
            .progress
            .get()
            .unwrap()
            .setFrame(rect(24.0, 276.0, 316.0, 12.0));
        self.ivars()
            .remaining
            .get()
            .unwrap()
            .setFrame(rect(348.0, 272.0, 52.0, 20.0));
        self.ivars()
            .separator
            .get()
            .unwrap()
            .setFrame(rect(24.0, 334.0, 412.0, 1.0));
        self.ivars()
            .qr
            .get()
            .unwrap()
            .setFrame(rect(24.0, 56.0, 200.0, 200.0));
        self.ivars()
            .note
            .get()
            .unwrap()
            .setFrame(rect(240.0, 80.0, 196.0, 176.0));
        self.ivars()
            .devices
            .get()
            .unwrap()
            .setFrame(rect(24.0, 30.0, 412.0, 18.0));
        self.ivars()
            .footer
            .get()
            .unwrap()
            .setFrame(rect(24.0, 4.0, 412.0, 22.0));
        let is_remote = old_urls
            .get(self.ivars().selected.get())
            .is_some_and(|url| url.starts_with("https://"));
        self.ivars()
            .renew
            .get()
            .unwrap()
            .setEnabled(running && !self.ivars().busy.get());

        self.ivars().note.get().unwrap().setStringValue(&NSString::from_str(if !running {
            "Web access is off. Turn it on to connect your phone."
        } else if valid && is_remote {
            if old_urls.get(self.ivars().selected.get()).is_some_and(|url|ngrok["url"].as_str().is_some_and(|origin|url.trim_end_matches('/')==origin.trim_end_matches('/'))) {
                "Scan with your phone’s camera.\n\nWorks outside your Wi-Fi. Traffic passes through ngrok.\n\nThe QR code renews automatically."
            } else if old_urls.get(self.ivars().selected.get()).is_some_and(|url|tuna["url"].as_str().is_some_and(|origin|url.trim_end_matches('/')==origin.trim_end_matches('/'))) {
                "Scan with your phone’s camera.\n\nWorks outside your Wi-Fi. Traffic passes through Tuna.\n\nThe QR code renews automatically."
            } else {
                "Scan with your phone’s camera.\n\nWorks outside your Wi-Fi. Traffic passes through Cloudflare.\n\nThe QR code renews automatically."
            }
        } else if valid {
            "Scan with your phone’s camera.\n\nSame Wi-Fi network required.\n\nSingle-use link. The QR code renews automatically."
        } else {
            "Refreshing QR code…"
        }));
        let count = report["devices"].as_array().map_or(0, Vec::len);
        self.ivars()
            .devices
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&format!(
                "{count} paired {}",
                if count == 1 { "browser" } else { "browsers" }
            )));
        if let Some(error) = self.ivars().error.borrow().as_ref() {
            self.ivars()
                .note
                .get()
                .unwrap()
                .setStringValue(&NSString::from_str(error));
        }
        self.draw_qr();
        self.refresh_countdown_at(Instant::now());
    }
    pub(super) fn refresh_countdown_at(&self, now: Instant) -> bool {
        let running = self.ivars().report.borrow()["running"] == true;
        let remaining = self
            .ivars()
            .deadline
            .get()
            .map(|until| until.saturating_duration_since(now))
            .unwrap_or_default();
        let seconds = remaining.as_secs_f64().ceil() as u64;
        self.ivars().progress.get().unwrap().setHidden(!running);
        self.ivars().remaining.get().unwrap().setHidden(!running);
        self.ivars()
            .progress
            .get()
            .unwrap()
            .setDoubleValue(remaining.as_secs_f64().min(120.0));
        self.ivars()
            .remaining
            .get()
            .unwrap()
            .setStringValue(&NSString::from_str(&format!("{seconds} sec")));
        let valid = running
            && seconds > 0
            && self
                .ivars()
                .links
                .borrow()
                .get(self.ivars().selected.get())
                .is_some();
        self.ivars().copy.get().unwrap().setEnabled(valid);
        self.ivars().link.get().unwrap().setEnabled(valid);
        running && (seconds == 0 || self.ivars().links.borrow().is_empty())
    }
    fn draw_qr(&self) {
        let qr = self.ivars().qr.get().unwrap();
        let code = if self.ivars().report.borrow()["running"] == true {
            self.ivars()
                .links
                .borrow()
                .get(self.ivars().selected.get())
                .and_then(|link| qrcode::QrCode::new(link).ok())
        } else {
            None
        };
        // Keep the last image visible during renewal; its expired link cannot be copied.
        // Stop still removes it immediately. The replacement is drawn on arrival.
        if code.is_some() || self.ivars().report.borrow()["running"] != true {
            qr.setHidden(code.is_none());
            *qr.ivars().code.borrow_mut() = code;
        }
        qr.setNeedsDisplay(true);
    }
}

fn address_kind(address: &str) -> &'static str {
    let Ok(url) = url::Url::parse(address) else {
        return "Local network";
    };
    if url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    }) {
        "This computer"
    } else if url.scheme() == "https" {
        "Cloudflare tunnel"
    } else {
        "Local network"
    }
}

/// Enabling the settings switch attempts every available access path. A tunnel
/// failure must not roll back the already running LAN listener.
fn enable_access(
    action: Control,
    mut request: impl FnMut(Control) -> anyhow::Result<Value>,
) -> anyhow::Result<Value> {
    let start = matches!(action, Control::Start { .. });
    let report = request(action)?;
    if start && report["running"] == true {
        match request(Control::TunnelStart) {
            Ok(report) => Ok(report),
            Err(_) => request(Control::Status),
        }
    } else {
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enabling_web_attempts_tunnel_and_retains_local_access_on_failure() {
        let mut calls = 0;
        let report = enable_access(Control::Start {listen:"0.0.0.0:8765".parse().unwrap()}, |action| {
            calls += 1;
            match calls {
                1 => { assert!(matches!(action, Control::Start { .. })); Ok(serde_json::json!({"running":true})) }
                2 => { assert!(matches!(action, Control::TunnelStart)); anyhow::bail!("connector unavailable") }
                3 => { assert!(matches!(action, Control::Status)); Ok(serde_json::json!({"running":true,"tunnel":{"status":"failed","error":"connector unavailable"}})) }
                _ => unreachable!(),
            }
        }).unwrap();
        assert_eq!(calls, 3);
        assert_eq!(report["running"], true);
        assert_eq!(report["tunnel"]["status"], "failed");
    }
    #[test]
    fn enabling_web_returns_the_automatic_tunnel_status() {
        let mut calls = 0;
        let report = enable_access(
            Control::Start {
                listen: "0.0.0.0:8765".parse().unwrap(),
            },
            |action| {
                calls += 1;
                if calls == 1 {
                    assert!(matches!(action, Control::Start { .. }));
                    Ok(serde_json::json!({"running":true}))
                } else {
                    assert!(matches!(action, Control::TunnelStart));
                    Ok(serde_json::json!({"running":true,"tunnel":{"status":"starting"}}))
                }
            },
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(report["tunnel"]["status"], "starting");
    }
    #[test]
    fn failed_listener_does_not_start_tunnel_and_stop_remains_a_single_request() {
        let mut calls = 0;
        assert!(
            enable_access(
                Control::Start {
                    listen: "0.0.0.0:8765".parse().unwrap()
                },
                |_| {
                    calls += 1;
                    anyhow::bail!("bind failed")
                }
            )
            .is_err()
        );
        assert_eq!(calls, 1);
        calls = 0;
        enable_access(Control::Stop, |action| {
            calls += 1;
            assert!(matches!(action, Control::Stop));
            Ok(serde_json::json!({"running":false}))
        })
        .unwrap();
        assert_eq!(calls, 1);
    }
}
