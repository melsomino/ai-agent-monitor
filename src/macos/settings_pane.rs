//! Replace the actual NSTabViewItem content, keeping its controller in sync.
use objc2::MainThreadMarker;
use objc2_app_kit::{NSTabViewItem, NSView, NSViewController};

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
