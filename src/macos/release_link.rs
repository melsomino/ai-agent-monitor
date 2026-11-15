//! Read-only rendering of release metadata owned by the monitor actor.
use crate::domain::versions::Versions;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained, runtime::AnyObject, sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::cell::{OnceCell, RefCell};
#[derive(Default, Debug)]
pub(super) struct LinkState {
    url: RefCell<Option<String>>,
    flame: OnceCell<Retained<NSImage>>,
}
define_class!(
    // SAFETY: AppKit controls and their URL state are main-thread confined.
    #[unsafe(super(NSButton))]
    #[thread_kind=MainThreadOnly]
    #[ivars=LinkState]
    #[derive(Debug)]
    pub(super) struct ReleaseLink;
    unsafe impl NSObjectProtocol for ReleaseLink {}
    impl ReleaseLink {
        #[unsafe(method(resetCursorRects))]
        fn reset_cursors(&self){let cursor=if self.ivars().url.borrow().is_some(){NSCursor::pointingHandCursor()}else{NSCursor::arrowCursor()};self.addCursorRect_cursor(self.bounds(),&cursor);}
        #[unsafe(method(openRelease:))]
        fn open_release(&self,_sender:Option<&AnyObject>){
            if let Some(address)=self.ivars().url.borrow().as_ref() && let Some(url)=NSURL::URLWithString(&NSString::from_str(address)){NSWorkspace::sharedWorkspace().openURL(&url);}
        }
    }
);
impl ReleaseLink {
    pub(super) fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this: Retained<Self> = unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(LinkState::default())),initWithFrame:frame]
        };
        this.setTitle(ns_string!(""));
        this.setBordered(false);
        this.setAlignment(NSTextAlignment::Left);
        this.setFont(Some(&NSFont::systemFontOfSize(9.0)));
        this.setHidden(true);
        if let Some(cell) = this.cell() {
            cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        }
        unsafe {
            this.setTarget(Some(&*this));
            this.setAction(Some(sel!(openRelease:)));
        }
        this
    }
    pub(super) fn sync(&self, versions: &Versions) {
        self.setHidden(false);
        self.setEnabled(versions.available.is_some());
        self.setRefusesFirstResponder(versions.available.is_none());
        *self.ivars().url.borrow_mut() = versions
            .available
            .as_ref()
            .map(|release| release.url.clone());
        let text = NSString::from_str(&match versions.available.as_ref() {
            Some(release) => format!("{} available", release.version),
            None if versions.checked_at.is_some() => {
                format!("v{} ✓", versions.current.trim_start_matches('v'))
            }
            None => format!("v{}", versions.current.trim_start_matches('v')),
        });
        let color = if versions.available.is_some() {
            super::visual::appearance_color([105.0, 64.0, 189.0], [212.0, 172.0, 255.0], 1.0)
        } else {
            super::visual::tertiary_text_color()
        };
        let attributed =
            NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &text);
        unsafe {
            attributed.addAttribute_value_range(
                NSForegroundColorAttributeName,
                &color,
                NSRange::new(0, text.length()),
            );
            attributed.addAttribute_value_range(
                NSUnderlineStyleAttributeName,
                &NSNumber::new_i32(i32::from(versions.available.is_some())),
                NSRange::new(0, text.length()),
            );
            attributed.addAttribute_value_range(
                NSFontAttributeName,
                &NSFont::systemFontOfSize_weight(
                    9.0,
                    if versions.available.is_some() {
                        0.35
                    } else {
                        0.0
                    },
                ),
                NSRange::new(0, text.length()),
            );
        }
        self.setContentTintColor(Some(&color));
        if versions.available.is_some() {
            let image = self.ivars().flame.get_or_init(|| {
                let image = NSImage::initWithData(
                    NSImage::alloc(),
                    &NSData::with_bytes(include_bytes!("../../assets/update-flame.svg")),
                )
                .expect("valid update icon");
                image.setTemplate(true);
                image.setSize(NSSize::new(12.0, 12.0));
                image
            });
            self.setImage(Some(image));
            self.setImagePosition(NSCellImagePosition::ImageLeft);
        } else {
            self.setImage(None);
            self.setImagePosition(NSCellImagePosition::NoImage);
        }
        self.setAttributedTitle(&attributed);
        let tooltip = versions
            .available
            .as_ref()
            .map(|release| release.url.clone())
            .unwrap_or_else(|| {
                format!(
                    "AI Agent Monitor {} — {}",
                    versions.current,
                    if versions.checked_at.is_some() {
                        "Up to date"
                    } else {
                        "Not checked yet"
                    }
                )
            });
        self.setToolTip(Some(&NSString::from_str(&tooltip)));
    }
}
