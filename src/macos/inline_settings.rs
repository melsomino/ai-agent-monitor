//! Native settings hosted by the existing status menu, with retained pane views.
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained, runtime::AnyObject, sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::cell::{Cell, RefCell};

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
define_class!(
    // SAFETY: Ivar-free document view, used only on the AppKit thread.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    struct SettingsDocument;
    unsafe impl NSObjectProtocol for SettingsDocument {}
    impl SettingsDocument {
        #[unsafe(method(isFlipped))]
        fn flipped(&self)->bool {true}
    }
);

struct ActionSurface {
    button: Retained<NSButton>,
    surface: Retained<NSBox>,
}

pub(super) struct Shell {
    pub scroll: Retained<NSScrollView>,
    document: Retained<SettingsDocument>,
    content: Retained<SettingsDocument>,
    pub back: Retained<NSButton>,
    back_surface: Retained<NSBox>,
    pub tabs: Vec<Retained<NSButton>>,
    surfaces: Vec<Retained<NSBox>>,
    labels: Vec<Retained<NSTextField>>,
    actions: RefCell<Vec<ActionSurface>>,
    pane: RefCell<Option<Retained<NSView>>>,
    selected: Cell<usize>,
    natural_height: Cell<f64>,
}
impl Shell {
    pub fn new(mtm: MainThreadMarker, target: &AnyObject) -> Self {
        let scroll =
            NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(0.0, 0.0, 460.0, 420.0));
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setHasHorizontalScroller(false);
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        let document: Retained<SettingsDocument> = unsafe {
            msg_send![SettingsDocument::alloc(mtm),initWithFrame:rect(0.0,0.0,460.0,420.0)]
        };
        scroll.setDocumentView(Some(&document));
        let content: Retained<SettingsDocument> = unsafe {
            msg_send![SettingsDocument::alloc(mtm),initWithFrame:rect(0.0,0.0,460.0,420.0)]
        };
        document.addSubview(&content);
        scroll.setHidden(true);
        let back = unsafe {
            NSButton::buttonWithTitle_target_action(
                ns_string!("‹ Sessions"),
                Some(target),
                Some(sel!(backToSessions:)),
                mtm,
            )
        };
        back.setFrame(rect(18.0, 17.0, 84.0, 32.0));
        back.setBordered(false);
        back.setRefusesFirstResponder(false);
        back.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        back.setContentTintColor(Some(&super::visual::secondary_text_color()));
        back.setToolTip(Some(ns_string!("Back to sessions (Escape)")));
        let back_surface = super::visual::glass_box(mtm, back.frame(), 0);
        back_surface.setCornerRadius(12.0);
        back_surface.setFillColor(&NSColor::clearColor());
        back_surface.setHidden(true);
        content.addSubview(&back_surface);
        content.addSubview(&back);
        let title = NSTextField::labelWithString(ns_string!("Settings"), mtm);
        title.setFrame(rect(112.0, 21.0, 190.0, 26.0));
        title.setFont(Some(&NSFont::boldSystemFontOfSize(18.0)));
        title.setTextColor(Some(&super::visual::primary_text_color()));
        content.addSubview(&title);
        let version =
            NSTextField::labelWithString(&NSString::from_str(crate::app::core::VERSION), mtm);
        version.setFrame(rect(322.0, 26.0, 114.0, 18.0));
        version.setAlignment(NSTextAlignment::Right);
        version.setFont(Some(&NSFont::systemFontOfSize(10.0)));
        version.setTextColor(Some(&super::visual::tertiary_text_color()));
        content.addSubview(&version);
        let mut tabs = Vec::new();
        let mut surfaces = Vec::new();
        let mut labels = Vec::new();
        for (index, (name, symbol)) in [
            ("General", "gearshape"),
            ("Applications", "square.grid.2x2"),
            ("Agents", "bot"),
            ("Telegram", "paperplane"),
            ("Web", "network"),
        ]
        .iter()
        .enumerate()
        {
            let width = (424.0 - 16.0) / 5.0;
            let frame = rect(18.0 + index as f64 * (width + 4.0), 68.0, width, 66.0);
            let surface = super::visual::glass_box(mtm, frame, 0);
            surface.setCornerRadius(12.0);
            surface.setFillColor(&NSColor::clearColor());
            content.addSubview(&surface);
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(name),
                    Some(target),
                    Some(sel!(selectSettingsTab:)),
                    mtm,
                )
            };
            button.setFrame(frame);
            button.setTag(index as isize);
            button.setBordered(false);
            button.setRefusesFirstResponder(false);
            button.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            button.setTitle(ns_string!(""));
            // SAFETY: NSButton implements the standard accessibility label setter.
            unsafe {
                let _: () = msg_send![&*button,setAccessibilityLabel:&*NSString::from_str(name)];
            }
            let image = if *symbol == "bot" {
                Some(super::session_robot::settings_icon())
            } else {
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(symbol),
                    Some(&NSString::from_str(name)),
                )
            };
            if let Some(image) = image {
                image.setSize(NSSize::new(23.0, 23.0));
                image.setTemplate(true);
                let icon = NSImageView::imageViewWithImage(&image, mtm);
                icon.setFrame(rect(
                    frame.origin.x + (frame.size.width - 23.0) / 2.0,
                    76.0,
                    23.0,
                    23.0,
                ));
                icon.setContentTintColor(Some(&super::visual::secondary_text_color()));
                content.addSubview(&icon);
            }
            let label = NSTextField::labelWithString(&NSString::from_str(name), mtm);
            label.setFrame(rect(frame.origin.x, 108.0, frame.size.width, 18.0));
            label.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            label.setAlignment(NSTextAlignment::Center);
            label.setTextColor(Some(&super::visual::secondary_text_color()));
            content.addSubview(&label);
            labels.push(label);
            content.addSubview(&button);
            tabs.push(button);
            surfaces.push(surface);
        }
        Self {
            scroll,
            document,
            content,
            back,
            back_surface,
            tabs,
            surfaces,
            labels,
            actions: RefCell::new(Vec::new()),
            pane: RefCell::new(None),
            selected: Cell::new(0),
            natural_height: Cell::new(420.0),
        }
    }
    pub fn select(&self, index: usize, pane: &NSView, height: f64) {
        let changed = self.selected.replace(index) != index;
        let replaced = self
            .pane
            .borrow()
            .as_ref()
            .is_none_or(|old| !std::ptr::eq(&**old, pane));
        if replaced {
            if let Some(old) = self.pane.borrow_mut().take() {
                old.removeFromSuperview();
            }
            self.content.addSubview(pane);
            *self.pane.borrow_mut() =
                unsafe { Retained::retain(pane as *const NSView as *mut NSView) };
        }
        self.style_actions(pane);
        pane.setAutoresizesSubviews(false);
        pane.setFrame(rect(0.0, 150.0, 460.0, height));
        pane.setHidden(false);
        self.natural_height.set(height + 166.0);
        self.resize(self.scroll.frame().size);
        if changed || replaced {
            self.scroll
                .contentView()
                .scrollToPoint(NSPoint::new(0.0, 0.0));
        }
        self.paint();
    }
    fn style_actions(&self, pane: &NSView) {
        fn collect(view: &NSView, buttons: &mut Vec<Retained<NSButton>>) {
            if let Some(button) = view.downcast_ref::<NSButton>() {
                // Keep checkbox and address radio controls native.
                if !matches!(button.action(), Some(action) if action == sel!(toggleLogin:) || action == sel!(toggleMenuAnimation:) || action == sel!(selectWebAddress:) || action == sel!(openWebLink:))
                {
                    // SAFETY: The pane retains this control throughout main-thread decoration.
                    if let Some(button) =
                        unsafe { Retained::retain(button as *const NSButton as *mut NSButton) }
                    {
                        buttons.push(button);
                    }
                }
            }
            let children = view.subviews();
            for index in 0..children.count() {
                collect(&children.objectAtIndex(index), buttons);
            }
        }
        let mut buttons = Vec::new();
        collect(pane, &mut buttons);
        let mut actions = self.actions.borrow_mut();
        for button in buttons {
            if actions
                .iter()
                .any(|action| std::ptr::eq(&*action.button, &*button))
            {
                continue;
            }
            // SAFETY: The button is retained in this pane's main-thread view tree.
            let Some(parent) = (unsafe { button.superview() }) else {
                continue;
            };
            let surface = super::visual::glass_box(self.document.mtm(), button.frame(), 5);
            surface.setCornerRadius(if matches!(button.action(), Some(action) if action==sel!(toggleRemoteSection:) || action==sel!(copyWebLink:) || action==sel!(renewWebPairing:)) {
                14.0
            } else {
                12.0
            });
            surface.setFillColor(&super::visual::appearance_color(
                [226.0, 215.0, 238.0],
                [68.0, 55.0, 85.0],
                0.72,
            ));
            parent.addSubview_positioned_relativeTo(
                &surface,
                NSWindowOrderingMode::Below,
                Some(&button),
            );
            button.setBordered(false);
            button.setRefusesFirstResponder(false);
            button.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, 0.23)));
            actions.push(ActionSurface { button, surface });
        }
    }
    pub fn natural_height(&self) -> f64 {
        self.natural_height.get()
    }
    pub fn resize(&self, size: NSSize) {
        self.scroll
            .setFrame(rect(0.0, 0.0, size.width, size.height));
        let height = self.natural_height.get().max(size.height);
        self.document.setFrameSize(NSSize::new(size.width, height));
        // Settings retain their compact native form geometry at wider popup
        // widths. Center the complete surface, including its header and tabs.
        self.content.setFrame(rect(
            ((size.width - 460.0) / 2.0).max(0.0),
            0.0,
            460.0,
            height,
        ));
    }
    pub fn paint(&self) {
        let point = self
            .scroll
            .window()
            .map(|w| w.convertPointFromScreen(NSEvent::mouseLocation()));
        self.paint_at(point);
    }
    pub(super) fn paint_at(&self, point: Option<NSPoint>) {
        fn paint(
            button: &NSButton,
            surface: &NSBox,
            point: Option<NSPoint>,
            selected: bool,
            resting: bool,
        ) {
            let enabled = button.isEnabled();
            let hovered = enabled
                && point.is_some_and(|p| {
                    let local = button.convertPoint_fromView(p, None);
                    button.mouse_inRect(local, button.bounds())
                        && button.mouse_inRect(local, button.visibleRect())
                });
            let focused = button
                .window()
                .and_then(|w| w.firstResponder())
                .is_some_and(|r| {
                    std::ptr::eq::<AnyObject>(
                        (&*r as *const NSResponder).cast(),
                        (button as *const NSButton).cast(),
                    )
                });
            surface.setHidden(
                button.isHiddenOrHasHiddenAncestor()
                    || !(resting || selected || hovered || focused),
            );
            surface.setFrame(button.frame());
            surface.setAlphaValue(if enabled { 1.0 } else { 0.45 });
            if let Some(glass) = surface.downcast_ref::<super::visual::GlassBox>() {
                glass.ivars().hovered.set(hovered || selected);
                glass.ivars().focused.set(focused && enabled);
                glass.ivars().enabled.set(enabled);
                surface.setNeedsDisplay(true);
            }
        }
        paint(&self.back, &self.back_surface, point, false, false);
        for action in self.actions.borrow().iter() {
            paint(
                &action.button,
                &action.surface,
                point,
                false,
                !matches!(action.button.action(), Some(action) if action==sel!(toggleRemoteSection:) || action==sel!(copyWebLink:) || action==sel!(renewWebPairing:)),
            );
            let icon = matches!(action.button.action(),Some(action) if action==sel!(toggleRemoteSection:) || action==sel!(copyWebLink:) || action==sel!(renewWebPairing:));
            let color = if icon {
                super::visual::secondary_text_color()
            } else {
                super::visual::primary_text_color()
            };
            action.button.setContentTintColor(Some(&color));
        }
        for (index, (button, surface)) in self.tabs.iter().zip(&self.surfaces).enumerate() {
            paint(button, surface, point, index == self.selected.get(), false);
            let color = if index == self.selected.get() {
                super::visual::primary_text_color()
            } else {
                super::visual::secondary_text_color()
            };
            button.setContentTintColor(Some(&color));
            self.labels[index].setTextColor(Some(&color));
        }
    }
    pub fn controls(&self) -> Vec<Retained<NSControl>> {
        fn collect(view: &NSView, out: &mut Vec<Retained<NSControl>>) {
            if view.isHiddenOrHasHiddenAncestor() {
                return;
            }
            if let Some(control) = view.downcast_ref::<NSControl>() {
                let editable = control
                    .downcast_ref::<NSTextField>()
                    .is_none_or(|f| f.isEditable());
                let input = control.isKindOfClass(NSButton::class())
                    || control.isKindOfClass(NSSwitch::class())
                    || control.isKindOfClass(NSTextField::class());
                // SAFETY: The view tree retains this main-thread control; retain it for traversal.
                if input
                    && control.isEnabled()
                    && editable
                    && let Some(value) =
                        unsafe { Retained::retain(control as *const NSControl as *mut NSControl) }
                {
                    out.push(value);
                }
            }
            let children = view.subviews();
            for index in 0..children.count() {
                collect(&children.objectAtIndex(index), out);
            }
        }
        let mut controls = Vec::new();
        collect(&self.document, &mut controls);
        controls
    }
}
