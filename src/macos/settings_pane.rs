//! Replace the actual NSTabViewItem content, keeping its controller in sync.
use objc2::MainThreadMarker;
use objc2_app_kit::{NSTabViewItem, NSView, NSViewController, NSWindow};
use objc2_foundation::{NSPoint, NSSize};

pub(crate) fn replace(
    controller: &NSViewController,
    item: &NSTabViewItem,
    pane: &NSView,
    mtm: MainThreadMarker,
) {
    let controller_matches = std::ptr::eq(&*controller.view(), pane);
    let item_matches = item
        .view(mtm)
        .is_some_and(|view| std::ptr::eq(&*view, pane));
    if !controller_matches {
        controller.setView(pane);
    }
    if !item_matches {
        item.setView(Some(pane));
    }
    pane.setHidden(false);
    pane.setNeedsDisplay(true);
}

pub(crate) fn resize(window: &NSWindow, size: NSSize) {
    let old = window.frame();
    let mut frame =
        window.frameRectForContentRect(objc2_foundation::NSRect::new(NSPoint::new(0.0, 0.0), size));
    frame.origin = NSPoint::new(
        old.origin.x,
        old.origin.y + old.size.height - frame.size.height,
    );
    if let Some(screen) = window.screen() {
        let visible = screen.visibleFrame();
        frame.origin.y = frame.origin.y.clamp(
            visible.origin.y,
            (visible.origin.y + visible.size.height - frame.size.height).max(visible.origin.y),
        );
    }
    // Resize the actual window frame, after the tab controller's preferred size
    // has changed. This cannot rely on a child view being attached during layout.
    if window.frame() != frame {
        window.setFrame_display(frame, true);
    }
}
