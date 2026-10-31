//! Real AppKit lifecycle regression; runs on the process main thread, never displays a window.
#[cfg(target_os = "macos")]
#[path = "../src/macos/settings_pane.rs"]
mod settings_pane;

#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::*;
    use objc2_foundation::{NSPoint, NSRect, NSSize, ns_string};
    let mtm = MainThreadMarker::new().expect("test must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let rect = |h| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(460.0, h));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(200.0),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: The test retains the window and never transfers ownership to AppKit.
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    let tabs = NSTabViewController::new(mtm);
    tabs.setTabStyle(NSTabViewControllerTabStyle::Toolbar);
    let controller = NSViewController::new(mtm);
    let setup = NSView::initWithFrame(NSView::alloc(mtm), rect(494.0));
    let connected = NSView::initWithFrame(NSView::alloc(mtm), rect(116.0));
    for pane in [&setup, &connected] {
        pane.setAutoresizesSubviews(false);
    }
    let title = NSTextField::labelWithString(ns_string!("Connected bot"), mtm);
    title.setFrame(NSRect::new(
        NSPoint::new(24.0, 76.0),
        NSSize::new(412.0, 24.0),
    ));
    connected.addSubview(&title);
    controller.setView(&setup);
    let item = NSTabViewItem::tabViewItemWithViewController(&controller);
    item.setLabel(ns_string!("Telegram"));
    tabs.addTabViewItem(&item);
    window.setContentViewController(Some(&tabs));
    window.setToolbarStyle(NSWindowToolbarStyle::Preference);
    for _ in 0..10 {
        for (pane, height) in [(&setup, 494.0), (&connected, 116.0)] {
            let size = NSSize::new(460.0, height);
            settings_pane::replace(&controller, &item, pane, mtm);
            controller.setPreferredContentSize(size);
            tabs.setPreferredContentSize(size);
            settings_pane::resize(&window, size);
            tabs.view().layoutSubtreeIfNeeded();
            assert!(std::ptr::eq(&*controller.view(), &**pane));
            assert!(std::ptr::eq(&*item.view(mtm).unwrap(), &**pane));
            assert!(!pane.isHiddenOrHasHiddenAncestor());
            assert!(
                pane.visibleRect().size.height > 0.0,
                "selected pane must not be clipped away after Cancel"
            );
            assert!(
                unsafe { pane.superview() }.is_some(),
                "selected pane must stay attached after Cancel"
            );
        }
        assert!(
            title.frame().origin.y + title.frame().size.height <= connected.bounds().size.height,
            "connected bot title must remain inside the pane"
        );
    }
    assert!(
        !window.isVisible(),
        "regression test must never open a window"
    );
    println!("AppKit settings pane switch regression passed");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
