//! Test-only access to the production menu's real view tree.
use super::*;
use crate::domain::interaction::{Choice, Interaction, Question};

fn button_with_identifier(view: &NSView, id: &str) -> Option<Retained<NSButton>> {
    if let Some(button) = view.downcast_ref::<NSButton>()
        && button
            .identifier()
            .is_some_and(|value| value.to_string() == id)
    {
        return Some(Retained::from(button));
    }
    let children = view.subviews();
    for index in 0..children.count() {
        let child = children.objectAtIndex(index);
        if let Some(button) = button_with_identifier(&child, id) {
            return Some(button);
        }
    }
    None
}

fn fixture(id: &str, activity: Activity) -> Session {
    {
        let mut __session = Session {
            target: None,
            agent: "codex",
            id: id.into(),
            title: id.into(),
            cwd: "/demo/task-board".into(),
            workspace: None,
            git_branch: Some("feature/task-lists".into()),
            model: Some("gpt-6.1-sol".into()),
            model_display_name: Some("GPT-6.1-Sol".into()),
            reasoning_effort: Some("medium".into()),
            updated_at: 0,
            client: Client::VSCode,
            activity,
            connection: if true {
                crate::domain::session::ConnectionState::live(true)
            } else {
                Default::default()
            },
            activities: Default::default(),
        };
        __session.seed_snapshot(
            "Local checks passed. The changes are ready for verification.".into(),
            vec![],
            None,
            None,
            Interaction::default(),
        );
        __session
    }
}

fn bitmap(view: &NSView) -> Retained<NSBitmapImageRep> {
    view.layoutSubtreeIfNeeded();
    let bitmap = view
        .bitmapImageRepForCachingDisplayInRect(view.bounds())
        .expect("bitmap context");
    view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), &bitmap);
    assert!(bitmap.pixelsWide() >= 460 && bitmap.pixelsHigh() > 100);
    bitmap
}
fn rgb(bitmap: &NSBitmapImageRep, x: isize, y: isize) -> [f64; 3] {
    let color = bitmap
        .colorAtX_y(x, y)
        .unwrap()
        .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
        .unwrap();
    assert!(
        color.alphaComponent() > 0.98,
        "monitor background must cover the viewport"
    );
    [
        color.redComponent(),
        color.greenComponent(),
        color.blueComponent(),
    ]
}
fn export(bitmap: &NSBitmapImageRep, name: &str) {
    if let Some(directory) = std::env::var_os("AAM_VISUAL_PREVIEW_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        // SAFETY: Empty PNG properties dictionary has no typed values to validate.
        let data = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }
        .unwrap();
        std::fs::write(directory.join(format!("{name}.png")), data.to_vec()).unwrap();
    }
}

fn table_view(view: &NSView) -> Option<Retained<NSTextView>> {
    if let Some(text) = view.downcast_ref::<NSTextView>()
        && !text.isEditable()
    {
        return Some(Retained::from(text));
    }
    let children = view.subviews();
    (0..children.count()).find_map(|i| table_view(&children.objectAtIndex(i)))
}

fn editable(view: &NSView) -> Option<Retained<NSTextField>> {
    if let Some(field) = view.downcast_ref::<NSTextField>()
        && field.isEditable()
    {
        // SAFETY: The attached fixture retains this native field for the whole test.
        return unsafe { Retained::retain(field as *const NSTextField as *mut NSTextField) };
    }
    let children = view.subviews();
    for index in 0..children.count() {
        let child = children.objectAtIndex(index);
        if let Some(field) = editable(&child) {
            return Some(field);
        }
    }
    None
}

fn text_label(view: &NSView, text: &str) -> Option<Retained<NSTextField>> {
    if let Some(label) = view.downcast_ref::<NSTextField>()
        && label.stringValue().to_string() == text
    {
        // SAFETY: The fixture view tree retains this native label.
        return unsafe { Retained::retain(label as *const NSTextField as *mut NSTextField) };
    }
    let children = view.subviews();
    (0..children.count()).find_map(|index| text_label(&children.objectAtIndex(index), text))
}

pub(crate) fn verify(mtm: MainThreadMarker) {
    let mut request = fixture("Task Board", Activity::Confirmation);
    request.set_approval(Some(crate::domain::session::Approval {
        id: "visual-approval".into(),
        body: "Allow running the full test suite before publishing the changes?".into(),
        expires: u64::MAX,
        can_allow: true,
    }));
    request.set_interaction(Interaction {
        command:Some("cargo test --locked --workspace --all-targets --all-features --no-fail-fast -- --test-threads=1 --nocapture tests::timer_countdown tests::timer_persistence tests::background_resume tests::request_delivery tests::session_reconnect tests::message_retry".into()),
        actions:vec![Action{id:"accept".into(),label:"Allow once".into(),detail:String::new()},Action{id:"similar".into(),label:"Allow similar".into(),detail:"cargo test".into()},Action{id:"decline".into(),label:"Deny".into(),detail:String::new()}],
        questions:vec![],
    });
    let mut questions = fixture("Verification", Activity::Confirmation);
    questions.set_approval(Some(crate::domain::session::Approval {
        id: "visual-questions".into(),
        body: "Choose the verification settings".into(),
        expires: u64::MAX,
        can_allow: true,
    }));
    questions.interaction_mut().questions = vec![
        Question {
            id: "scope".into(),
            title: "Which checks should run?".into(),
            choices: vec![
                Choice {
                    label: "All checks".into(),
                    description: String::new(),
                },
                Choice {
                    label: "Only unit tests".into(),
                    description: String::new(),
                },
            ],
            freeform: true,
            secret: false,
        },
        Question {
            id: "output".into(),
            title: "Where should the results be saved?".into(),
            choices: vec![
                Choice {
                    label: "Project folder".into(),
                    description: String::new(),
                },
                Choice {
                    label: "Temporary folder".into(),
                    description: String::new(),
                },
            ],
            freeform: true,
            secret: false,
        },
    ];
    let mut working = fixture("Return to metrics", Activity::Working);
    working.set_progress(Some("Running cargo test --offline --all-targets".into()));
    let mut sessions = vec![
        fixture("AI Agent Monitor", Activity::Idle),
        request.clone(),
        questions.clone(),
        working.clone(),
    ];
    for index in 0..10 {
        sessions.push(fixture(&format!("Workspace {index}"), Activity::Idle));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let usage = usage::Usage {
        observed_at: "visual fixture".into(),
        windows: vec![usage::Window {
            minutes: 10080,
            used_percent: 15.0,
            resets_at: Some(now + 4 * 86400),
        }],
    };
    let snapshot = crate::actors::monitor::State {
        sessions: sessions.clone(),
        usage: Some(usage.clone()),
        ..Default::default()
    };
    let (tx, mut commands) = tokio::sync::mpsc::channel(128);
    let (_published, state) = tokio::sync::watch::channel(std::sync::Arc::new(snapshot));
    let monitor = crate::actors::monitor::Handle { tx, state };
    // The resident must not retain popup views or their controller between
    // openings. Exercise real AppKit ownership, including the settings tree.
    let allocated = MenuController::alloc(mtm).set_ivars(State::default());
    let resident: Retained<MenuController> = unsafe { msg_send![super(allocated), init] };
    resident.ivars().monitor.set(monitor.clone()).ok();
    let resident_status_host =
        NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 80.0, 24.0));
    resident.build_status_artwork(&resident_status_host);
    let (ui, _ui_events) = tokio::sync::mpsc::channel(128);
    resident.ivars().ui_sender.set(ui).ok();
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Lifecycle test"));
    resident.ivars().menu.set(menu.clone()).ok();
    objc2::rc::autoreleasepool(|_| {
        let initial = resident.create_popup(&menu);
        assert_eq!(initial.ivars().revision.get(), u64::MAX);
        initial.receive_sessions_with_mouse(true);
        assert_eq!(
            initial.ivars().revision.get(),
            monitor.snapshot().revision,
            "holding the status icon must not delay the initial popup population"
        );
        assert_eq!(initial.ivars().all.borrow().len(), sessions.len());
        initial.ivars().menu_item.get().unwrap().setView(None);
        initial.shutdown();
        menu.removeAllItems();
    });
    for cycle in 0..3 {
        let (controller, root, settings) = objc2::rc::autoreleasepool(|_| {
            assert!(resident.ivars().root.get().is_none());
            resident
                .ivars()
                .expanded
                .borrow_mut()
                .insert(sessions[0].key());
            resident.will_open(sel!(menuWillOpen:), &menu);
            let popup = resident.ivars().popup.borrow().as_ref().unwrap().clone();
            // Drive only the popup's update path, as during native tracking.
            // All resident counters must advance without closing the menu or
            // needing a separate resident poll timer callback.
            let baseline = (*monitor.snapshot()).clone();
            let mut changed = baseline.clone();
            changed.revision += 1;
            changed.sessions[0].activity = Activity::Working;
            changed.sessions[1].set_approval(None);
            changed.sessions[1].activity = Activity::Idle;
            changed
                .unseen_finished
                .insert(changed.sessions[4].key(), changed.revision);
            let revision = changed.revision;
            _published.send_replace(std::sync::Arc::new(changed));
            popup.receive_sessions();
            assert_eq!(resident.ivars().revision.get(), revision);
            assert_eq!(resident.ivars().working_count(), 2);
            assert_eq!(resident.ivars().finished_count(), 1);
            assert_eq!(
                resident
                    .ivars()
                    .all
                    .borrow()
                    .iter()
                    .filter(|s| s.approval().is_some())
                    .count(),
                1
            );
            assert_eq!(
                resident
                    .ivars()
                    .status_working_count
                    .get()
                    .unwrap()
                    .stringValue()
                    .to_string(),
                "2"
            );
            assert_eq!(
                resident
                    .ivars()
                    .status_finished_count
                    .get()
                    .unwrap()
                    .stringValue()
                    .to_string(),
                "1"
            );
            assert!(resident.ivars().menu_open.get());
            assert!(resident.ivars().root.get().is_none());
            let mut restored = baseline;
            restored.revision = revision + 1;
            _published.send_replace(std::sync::Arc::new(restored));
            popup.receive_sessions();
            assert_eq!(resident.ivars().working_count(), 1);
            assert_eq!(resident.ivars().finished_count(), 0);
            assert!(
                resident
                    .ivars()
                    .status_finished_count
                    .get()
                    .unwrap()
                    .isHidden()
            );
            let held_row = popup.ivars().layout.borrow()[0].view.clone();
            let held_revision = popup.ivars().revision.get();
            let held_snapshot = (*monitor.snapshot()).clone();
            for next in 1..=3 {
                let mut update = held_snapshot.clone();
                update.revision = held_revision + next;
                update.sessions[0].model_display_name = Some("Updated model".into());
                _published.send_replace(std::sync::Arc::new(update));
                popup.receive_sessions_with_mouse(true);
                assert_eq!(
                    popup.ivars().revision.get(),
                    held_revision,
                    "a held click must not consume the pending UI revision"
                );
                assert!(
                    std::ptr::eq(&*popup.ivars().layout.borrow()[0].view, &*held_row),
                    "native mouse tracking must retain the exact pressed row and its controls"
                );
                assert_eq!(
                    resident.ivars().revision.get(),
                    held_revision + next,
                    "status counters keep receiving snapshots during a held click"
                );
            }
            popup.receive_sessions_with_mouse(false);
            assert_eq!(
                popup.ivars().revision.get(),
                held_revision + 3,
                "release applies the newest snapshot without replaying intermediate states"
            );
            assert!(!std::ptr::eq(
                &*popup.ivars().layout.borrow()[0].view,
                &*held_row
            ));
            let mut restored = held_snapshot.clone();
            restored.revision = held_revision + 4;
            _published.send_replace(std::sync::Arc::new(restored));
            popup.receive_sessions_with_mouse(false);
            let stable_row = popup.ivars().layout.borrow()[0].view.clone();
            let stable_quota = popup
                .ivars()
                .footer
                .get()
                .unwrap()
                .subviews()
                .objectAtIndex(0);
            let mut heartbeat = (*monitor.snapshot()).clone();
            heartbeat.revision += 1;
            for session in &mut heartbeat.sessions {
                session.updated_at += 1;
            }
            if let Some(usage) = &mut heartbeat.usage {
                usage.observed_at = "New observation, same limits".into();
            }
            _published.send_replace(std::sync::Arc::new(heartbeat));
            popup.receive_sessions_with_mouse(false);
            assert!(
                std::ptr::eq(&*popup.ivars().layout.borrow()[0].view, &*stable_row),
                "revision and observation timestamps must not recreate unchanged rows"
            );
            assert!(
                std::ptr::eq(
                    &*popup
                        .ivars()
                        .footer
                        .get()
                        .unwrap()
                        .subviews()
                        .objectAtIndex(0),
                    &*stable_quota
                ),
                "unchanged quota must retain its native view and cursor rectangles"
            );
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    rect(0.0, 0.0, 460.0, 500.0),
                    NSWindowStyleMask::Borderless,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            unsafe {
                window.setReleasedWhenClosed(false);
            }
            window.setContentView(Some(&**popup.ivars().root.get().unwrap()));
            assert_eq!(popup.ivars().all.borrow().len(), sessions.len());
            // Exercise the NSApplication entry point, which retains the
            // resident, rather than calling the popup's handlers directly.
            let key = |code| {
                NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                    NSEventType::KeyDown, NSPoint::new(0.0,0.0), NSEventModifierFlags::empty(), 0.0,
                    window.windowNumber(), None, ns_string!(""), ns_string!(""), false, code,
                ).unwrap()
            };
            popup.ivars().keyboard_focus.borrow_mut().take();
            assert!(resident.handle_session_key(&key(125)));
            let first = popup
                .ivars()
                .keyboard_focus
                .borrow()
                .clone()
                .expect("Down must focus a real popup target");
            assert!(resident.ivars().keyboard_focus.borrow().is_none());
            assert!(resident.handle_session_key(&key(125)));
            assert_ne!(popup.ivars().keyboard_focus.borrow().as_ref(), Some(&first));
            assert!(resident.handle_session_key(&key(126)));
            assert_eq!(popup.ivars().keyboard_focus.borrow().as_ref(), Some(&first));
            *popup.ivars().keyboard_focus.borrow_mut() = Some(sessions[0].key());
            assert!(resident.handle_session_key(&key(123)));
            assert!(!popup.ivars().expanded.borrow().contains(&sessions[0].key()));
            assert!(resident.handle_session_key(&key(124)));
            assert!(popup.ivars().expanded.borrow().contains(&sessions[0].key()));
            let composer = popup
                .ivars()
                .composers
                .borrow()
                .get(&sessions[0].key())
                .unwrap()
                .clone();
            if cycle > 0 {
                assert_eq!(
                    composer.field().stringValue().to_string(),
                    "Unsent popup draft"
                );
            }
            composer
                .field()
                .setStringValue(ns_string!("Unsent popup draft"));
            let pin = popup.ivars().pin_button.get().unwrap();
            let cursor_clip = popup.ivars().scroll.get().unwrap().contentView();
            let before_cursor_test = cursor_clip.bounds().origin;
            cursor_clip.scrollToPoint(NSPoint::new(0.0, 0.0));
            popup.sync_pin_position();
            let pin_frame = popup.ivars().pin_background.get().unwrap().frame();
            let brand_bounds = popup.ivars().root.get().unwrap().bounds();
            assert_eq!(pin_frame.size, NSSize::new(28.0, 28.0));
            assert_eq!(pin_frame.origin.x, 1.0);
            assert!(pin_frame.origin.x + pin_frame.size.width <= brand_bounds.size.width - 1.0);
            assert!(
                (brand_bounds.size.height - pin_frame.origin.y - pin_frame.size.height - 1.0).abs()
                    < 0.01,
                "pin hover is inset from the window content, independently of the list gutter"
            );
            let pin_point = pin.convertPoint_toView(NSPoint::new(14.0, 14.0), None);
            assert!(
                popup.action_cursor_at(pin_point),
                "enabled actions use a hand cursor"
            );
            pin.setEnabled(false);
            assert!(
                !popup.action_cursor_at(pin_point),
                "disabled actions do not use a hand cursor"
            );
            pin.setEnabled(true);
            cursor_clip.scrollToPoint(NSPoint::new(0.0, 10.0));
            popup.sync_pin_position();
            assert_eq!(
                pin.frame().origin.y,
                pin_frame.origin.y + 10.0,
                "pin scrolls with the header"
            );
            cursor_clip.scrollToPoint(NSPoint::new(0.0, 40.0));
            popup.sync_pin_position();
            assert!(
                pin.isHidden(),
                "scrolled-away pin has no visible or clickable overlay"
            );
            cursor_clip.scrollToPoint(before_cursor_test);
            popup.sync_pin_position();
            let field = composer.field();
            let field_point = field
                .convertPoint_toView(NSPoint::new(10.0, field.bounds().size.height / 2.0), None);
            assert!(
                !popup.action_cursor_at(field_point),
                "text input retains its native text cursor"
            );
            // Detaching reparents this exact UI instead of constructing another
            // session tree. Native menu dismissal must not dispose it.
            let same_root = popup.ivars().root.get().unwrap().clone();
            let pinned_width = popup.menu_width();
            let saved_detached = resident.ivars().detached_frame.get();
            let position = popup
                .ivars()
                .scroll
                .get()
                .unwrap()
                .contentView()
                .bounds()
                .origin;
            popup.ivars().detach_pending.set(true);
            resident.did_close(sel!(menuDidClose:), &menu);
            let detached = popup.ivars().detached_window.borrow().clone().unwrap();
            if let Some(frame) = saved_detached {
                assert_eq!(
                    detached.frame(),
                    detached.constrainFrameRect_toScreen(frame, None),
                    "detached geometry survives disposal of the previous popup"
                );
            }
            assert!(
                !detached.isVisible(),
                "lifecycle test must not open a visible window"
            );
            assert!(detached.styleMask().contains(NSWindowStyleMask::Titled));
            assert!(detached.styleMask().contains(NSWindowStyleMask::Resizable));
            assert_eq!(detached.title().to_string(), "AI Agent Monitor");
            assert_eq!(same_root.window().as_ref(), Some(&detached));
            assert!(popup.ivars().menu_item.get().unwrap().view().is_none());
            assert!(popup.ivars().pin_button.get().unwrap().isHidden());
            assert!(popup.ivars().menu_open.get());
            assert!(resident.ivars().popup.borrow().is_some());
            popup.dismiss_menu();
            assert!(
                popup.is_detached(),
                "opening an agent app must not close a detached monitor"
            );
            resident.did_close(sel!(menuDidClose:), &menu);
            assert!(
                resident.ivars().popup.borrow().is_some(),
                "status menu dismissal preserves standalone window"
            );
            assert_eq!(
                composer.field().stringValue().to_string(),
                "Unsent popup draft"
            );
            let restored_position = popup
                .ivars()
                .scroll
                .get()
                .unwrap()
                .contentView()
                .bounds()
                .origin;
            assert!(
                cycle > 0 || (restored_position.y - position.y).abs() <= 0.5,
                "host change preserves scroll within device-pixel rounding"
            );
            detached.setContentSize(NSSize::new(560.0, 520.0));
            popup.layout_detached();
            assert_eq!(popup.menu_width(), 560.0);
            assert_eq!(same_root.frame().size, NSSize::new(560.0, 520.0));
            detached.setFrameOrigin(NSPoint::new(80.0, 120.0));
            popup.remember_detached_frame();
            let detached_frame = detached.frame();
            popup.reattach_window();
            assert!(!popup.is_detached());
            assert_eq!(
                popup.menu_width(),
                pinned_width,
                "standalone resizing does not change pinned width"
            );
            assert_eq!(resident.ivars().detached_frame.get(), Some(detached_frame));
            assert_eq!(
                popup.ivars().pin_button.get().unwrap().isHidden(),
                popup
                    .ivars()
                    .scroll
                    .get()
                    .unwrap()
                    .contentView()
                    .bounds()
                    .origin
                    .y
                    >= 29.0
            );
            assert_eq!(
                popup.ivars().menu_item.get().unwrap().view().as_deref(),
                Some(&**same_root)
            );
            resident.will_open(sel!(menuWillOpen:), &menu);
            assert!(std::ptr::eq(
                &**resident.ivars().popup.borrow().as_ref().unwrap(),
                &*popup
            ));
            popup.change_width(500.0);
            popup.ivars().detach_pending.set(true);
            resident.did_close(sel!(menuDidClose:), &menu);
            let second = popup.ivars().detached_window.borrow().clone().unwrap();
            assert_eq!(
                second.frame(),
                second.constrainFrameRect_toScreen(detached_frame, None),
                "pinned resizing does not change standalone position or size"
            );
            assert_eq!(popup.menu_width(), 560.0);
            popup.reattach_window();
            resident.will_open(sel!(menuWillOpen:), &menu);
            assert_eq!(popup.menu_width(), 500.0);
            // Restore the hidden test host for native keyboard and editor checks.
            window.setContentView(Some(&same_root));
            popup.show_settings();
            window.makeFirstResponder(Some(&popup.ivars().settings_shell.get().unwrap().tabs[0]));
            assert!(resident.handle_session_key(&key(124)));
            assert_eq!(popup.ivars().settings_selected.get(), 1);
            assert!(resident.handle_session_key(&key(123)));
            assert_eq!(popup.ivars().settings_selected.get(), 0);
            let weak = (
                objc2::rc::Weak::new(&*popup),
                objc2::rc::Weak::new(&**popup.ivars().root.get().unwrap()),
                objc2::rc::Weak::new(&*popup.ivars().settings_shell.get().unwrap().scroll),
            );
            resident.did_close(sel!(menuDidClose:), &menu);
            assert!(
                popup
                    .ivars()
                    .scroll
                    .get()
                    .unwrap()
                    .ivars()
                    .controller
                    .borrow()
                    .is_none()
            );
            window.setContentView(None);
            assert!(popup.ivars().pointer_timer.borrow().is_none());
            assert!(popup.ivars().keyboard_handler.borrow().is_none());
            assert!(resident.ivars().popup.borrow().is_none());
            assert_eq!(menu.numberOfItems(), 1);
            assert!(menu.itemAtIndex(0).unwrap().view().is_none());
            weak
        });
        assert!(
            controller.load().is_none(),
            "closed popup controller leaked"
        );
        assert!(root.load().is_none(), "closed popup view tree leaked");
        assert!(settings.load().is_none(), "closed settings tree leaked");
        let mut update = (*monitor.snapshot()).clone();
        update.revision += 1;
        _published.send_replace(std::sync::Arc::new(update));
        resident.receive_sessions();
        assert_eq!(resident.ivars().all.borrow().len(), sessions.len());
        assert!(resident.ivars().root.get().is_none());
        assert!(resident.ivars().popup.borrow().is_none());
    }
    objc2::rc::autoreleasepool(|_| {
        resident.will_open(sel!(menuWillOpen:), &menu);
        let popup = resident.ivars().popup.borrow().as_ref().unwrap().clone();
        let root = objc2::rc::Weak::new(&**popup.ivars().root.get().unwrap());
        popup.ivars().detach_pending.set(true);
        resident.did_close(sel!(menuDidClose:), &menu);
        let window = popup.ivars().detached_window.borrow().clone().unwrap();
        // Native close reaches the window delegate, saves drafts and tears down
        // both tracking timers. A later opening constructs a fresh transient UI.
        window.close();
        assert!(!popup.is_detached());
        assert!(resident.ivars().popup.borrow().is_none());
        assert!(popup.ivars().pointer_timer.borrow().is_none());
        assert!(popup.ivars().keyboard_handler.borrow().is_none());
        assert!(
            root.load().is_some(),
            "only this local controller retains the old root"
        );
    });
    while commands.try_recv().is_ok() {}
    for (name, appearance_name) in [
        ("dark", unsafe { NSAppearanceNameDarkAqua }),
        ("light", unsafe { NSAppearanceNameAqua }),
    ] {
        // NSMenu adds padding outside its custom item. Those background slices
        // must match one uninterrupted mesh, including beyond the document ends.
        let appearance = NSAppearance::appearanceNamed(appearance_name).unwrap();
        let continuous = visual::MeshView::new(mtm, rect(0.0, 0.0, 460.0, 420.0));
        continuous.setAppearance(Some(&appearance));
        continuous.sync(0.0, -10.0, 400.0, 460.0);
        let split = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 460.0, 420.0));
        split.setAppearance(Some(&appearance));
        for (y, height, offset) in [(0.0, 10.0, 400.0), (10.0, 400.0, 0.0), (410.0, 10.0, -10.0)] {
            let slice = continuous.sibling(mtm, rect(0.0, y, 460.0, height));
            slice.sync(0.0, offset, 400.0, 460.0);
            split.addSubview(&slice);
        }
        for wave in [true, false] {
            continuous.set_brand_wave(wave);
            let children = split.subviews();
            for index in 0..children.count() {
                children
                    .objectAtIndex(index)
                    .downcast_ref::<visual::MeshView>()
                    .unwrap()
                    .set_brand_wave(wave);
            }
            let whole = bitmap(&continuous);
            let pieces = bitmap(&split);
            let scale = whole.pixelsHigh() as f64 / 420.0;
            for y in [1.0, 9.0, 10.0, 11.0, 409.0, 410.0, 411.0, 418.0] {
                for x in [40.0, 230.0, 420.0] {
                    let at = ((x * scale) as isize, (y * scale) as isize);
                    let expected = rgb(&whole, at.0, at.1);
                    let actual = rgb(&pieces, at.0, at.1);
                    assert!(
                        expected
                            .iter()
                            .zip(actual)
                            .all(|(a, b)| (a - b).abs() < 0.015),
                        "{name}: menu padding must continue the mesh at {x},{y}: {expected:?} vs {actual:?}"
                    );
                }
            }
            export(&pieces, &format!("{name}-menu-padding"));
        }
        let allocated = MenuController::alloc(mtm).set_ivars(State::default());
        // SAFETY: Same initializer as the production controller, with test-owned channels.
        let controller: Retained<MenuController> = unsafe { msg_send![super(allocated), init] };
        controller.ivars().monitor.set(monitor.clone()).ok();
        let status_host = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 80.0, 24.0));
        status_host.setAppearance(Some(&appearance));
        controller.build_status_artwork(&status_host);
        controller
            .ivars()
            .status_eye_animation
            .borrow_mut()
            .update(true, true);
        controller.paint_status_eyes();
        let first = controller
            .ivars()
            .status_content
            .get()
            .unwrap()
            .ivars()
            .images
            .borrow()
            .clone()
            .unwrap();
        for _ in 0..16 {
            controller.ivars().status_eye_animation.borrow_mut().tick();
            controller.paint_status_eyes();
        }
        assert_eq!(controller.ivars().status_frames.borrow().len(), 8);
        let status_content = controller.ivars().status_content.get().unwrap();
        assert_eq!(
            status_content.subviews().count(),
            0,
            "system status snapshots must contain only the cached artwork"
        );
        assert!(
            std::ptr::eq(
                &*first[0],
                &*status_content.ivars().images.borrow().as_ref().unwrap()[0]
            ),
            "the animation must reuse its prepared frames"
        );
        // Reproduce AppKit's temporary source-appearance switches while it
        // prepares status-item replicants. Neither switch may rebuild frames
        // or create a new source invalidation/redraw feedback loop.
        for _ in 0..6 {
            for name in unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] } {
                status_content.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
                let capture = status_content
                    .bitmapImageRepForCachingDisplayInRect(status_content.bounds())
                    .unwrap();
                status_content
                    .cacheDisplayInRect_toBitmapImageRep(status_content.bounds(), &capture);
                assert_eq!(
                    controller.ivars().status_frames.borrow().len(),
                    8,
                    "replicant snapshots must not invalidate the animation cache"
                );
                let current = status_content.ivars().images.borrow();
                for (original, selected) in first.iter().zip(current.as_ref().unwrap()) {
                    assert!(
                        std::ptr::eq(&**original, &**selected),
                        "appearance switches must only select prepared images"
                    );
                }
            }
        }
        status_content.setAppearance(Some(&appearance));
        let status_bitmap = status_content
            .bitmapImageRepForCachingDisplayInRect(status_content.bounds())
            .unwrap();
        status_content.cacheDisplayInRect_toBitmapImageRep(status_content.bounds(), &status_bitmap);
        assert!(
            (0..status_bitmap.pixelsWide()).any(|x| (0..status_bitmap.pixelsHigh()).any(|y| {
                status_bitmap
                    .colorAtX_y(x, y)
                    .is_some_and(|color| color.alphaComponent() > 0.5)
            })),
            "cached status artwork must not be empty"
        );
        export(&status_bitmap, &format!("{name}-status-cached"));
        *controller.ivars().all.borrow_mut() = sessions.clone();
        *controller.ivars().usage.borrow_mut() = Some(usage.clone());
        controller
            .ivars()
            .expanded
            .borrow_mut()
            .insert(request.key());
        controller
            .ivars()
            .reveal
            .borrow_mut()
            .insert(request.key(), 1.0);
        controller.build_menu();
        // Background discovery must retain fresh data without constructing
        // session controls while the native popup is closed. The first visible
        // render still consumes the pending update below.
        let mut refreshed = (*monitor.snapshot()).clone();
        refreshed.revision += 1;
        _published.send_replace(std::sync::Arc::new(refreshed));
        controller.receive_sessions();
        assert!(controller.ivars().layout.borrow().is_empty());
        assert!(controller.ivars().filter_dirty.get());
        assert_eq!(controller.ivars().all.borrow().len(), sessions.len());
        controller.filter();
        assert!(!controller.ivars().layout.borrow().is_empty());
        assert_eq!(
            overview_counts(&sessions, |session| session.id == sessions[0].id),
            [2, 1, 1, 10, 0]
        );
        assert_eq!(
            controller
                .ivars()
                .overview
                .get()
                .unwrap()
                .stringValue()
                .to_string(),
            "2 waiting for you · 1 working · 11 idle"
        );
        let mut unknown = sessions[0].clone();
        unknown.activity = Activity::Unknown;
        assert_eq!(
            overview_counts(&[unknown], |_| false),
            [0, 0, 0, 0, 1],
            "unknown must not be reported as idle"
        );
        let root = controller.ivars().root.get().unwrap();
        let appearance = NSAppearance::appearanceNamed(appearance_name).unwrap();
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                root.bounds(),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: The test owns the window throughout the view-cache captures.
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        window.setAppearance(Some(&appearance));
        root.setAppearance(Some(&appearance));
        window.setContentView(Some(root));

        let hover_row = controller
            .ivars()
            .layout
            .borrow()
            .iter()
            .find(|item| item.id.as_ref().is_some_and(|id| id == &request.key()))
            .and_then(|item| item.row.clone())
            .expect("request row");
        let destination = hover_row.ivars().destination.get().unwrap();
        let destination_icon = hover_row.ivars().destination_icon.get().unwrap();
        let destination_arrow = hover_row.ivars().destination_arrow.get().unwrap();
        assert!(!destination.isHidden());
        assert!(destination_arrow.isHidden());
        assert_eq!(destination_icon.frame().origin.x, 0.0);
        let highlight_height = hover_row
            .ivars()
            .background
            .get()
            .unwrap()
            .frame()
            .size
            .height;
        hover_row.set_highlight(true, false);
        assert!(!destination_arrow.isHidden());
        assert_eq!(destination_icon.frame().origin.x, 18.0);
        assert!(
            destination.frame().origin.x + destination_icon.frame().origin.x + 16.0
                < hover_row.ivars().background.get().unwrap().frame().max().x
        );
        assert_eq!(
            hover_row
                .ivars()
                .background
                .get()
                .unwrap()
                .frame()
                .size
                .height,
            highlight_height
        );
        export(&bitmap(root), &format!("{name}-title-hover"));
        hover_row.set_highlight(false, true);
        export(&bitmap(root), &format!("{name}-title-focus"));
        hover_row.set_highlight(false, false);
        assert!(!destination.isHidden());
        assert!(destination_arrow.isHidden());
        assert_eq!(destination_icon.frame().origin.x, 0.0);
        let initial = bitmap(root);
        let before = rgb(&initial, initial.pixelsWide() - 20, 200);
        export(&initial, name);
        let list = controller.ivars().list.get().unwrap();
        let scroll = controller.ivars().scroll.get().unwrap();
        let footer = controller.ivars().footer.get().unwrap();
        assert_eq!(
            scroll.frame().origin.y,
            0.0,
            "list must extend behind the quota overlay"
        );
        assert!(
            (scroll.frame().size.height - root.bounds().size.height).abs() <= 0.5,
            "scroll viewport must fill the pixel-rounded popup"
        );
        assert!(
            list.frame().size.height
                >= controller.ivars().content_height.get() + footer.frame().size.height,
            "document must reserve trailing clearance for the floating footer"
        );
        list.scrollPoint(NSPoint::new(0.0, 200.0));
        controller.sync_backdrop();
        let scrolled = bitmap(root);
        let after = rgb(&scrolled, scrolled.pixelsWide() - 20, 200);
        assert!(
            before.iter().zip(after).any(|(a, b)| (a - b).abs() > 0.005),
            "mesh must move with the scroll document"
        );
        export(&scrolled, &format!("{name}-scrolled"));
        list.scrollPoint(NSPoint::new(0.0, 0.0));
        let key = format!("context:{}:0", request.approval_key().unwrap());
        let disclosure =
            button_with_identifier(root, &key).expect("command disclosure inside Allow once");
        let command = request.interaction().command.unwrap();
        assert!(
            text_label(root, &command).is_none(),
            "main command starts fully folded"
        );
        assert!(
            text_label(root, "Command").is_none(),
            "no separate Command heading above approval actions"
        );
        let allow = controller
            .ivars()
            .buttons
            .borrow()
            .iter()
            .find(|item| {
                item.button.identifier().is_some_and(|id| {
                    controller
                        .ivars()
                        .request_controls
                        .borrow()
                        .get(&id.to_string())
                        .is_some_and(|control| {
                            control.label == "Allow once" && control.session.key() == request.key()
                        })
                })
            })
            .unwrap()
            .button
            .clone();
        assert_eq!(
            disclosure.frame().origin.x + disclosure.frame().size.width,
            allow.frame().origin.x + allow.frame().size.width
        );
        assert_eq!(disclosure.frame().origin.y, allow.frame().origin.y);
        assert_eq!(disclosure.frame().size.height, allow.frame().size.height);
        assert!(
            disclosure.frame().origin.x > allow.frame().origin.x,
            "arrow is a distinct trailing segment"
        );
        let disclosure_id = disclosure.identifier().unwrap().to_string();
        assert!(
            !controller
                .keyboard_targets(&controller.ivars().layout.borrow())
                .contains(&disclosure_id)
        );
        let height = list.frame().size.height;
        assert!(disclosure.refusesFirstResponder());
        while commands.try_recv().is_ok() {}
        // SAFETY: Exercise the real trailing button's native target/action.
        unsafe {
            disclosure.performClick(None);
        }
        assert!(
            commands.try_recv().is_err(),
            "command disclosure must never submit an approval"
        );
        assert!(
            controller.ivars().list.get().unwrap().frame().size.height > height,
            "long command must expand its actual native row"
        );
        let command_text = text_label(root, &command).expect("disclosure reveals the full command");
        assert_eq!(command_text.maximumNumberOfLines(), 0);
        export(&bitmap(root), &format!("{name}-command-expanded"));
        // SAFETY: The real controller implements this selector; the button is retained.
        unsafe {
            let _: () = msg_send![&*controller,respondToRequest:&*disclosure];
        }
        assert_eq!(
            controller.ivars().list.get().unwrap().frame().size.height,
            height
        );
        assert!(
            text_label(root, "cargo test").is_none(),
            "scope starts hidden"
        );
        let scope_id = controller
            .ivars()
            .request_controls
            .borrow()
            .iter()
            .find_map(|(id, control)| {
                (id.starts_with("context:")
                    && !id.ends_with(":0")
                    && control.session.key() == request.key())
                .then(|| id.clone())
            })
            .unwrap();
        let scope =
            button_with_identifier(root, &scope_id).expect("scope disclosure inside Allow similar");
        let similar = controller
            .ivars()
            .buttons
            .borrow()
            .iter()
            .find(|item| {
                item.button.identifier().is_some_and(|id| {
                    controller
                        .ivars()
                        .request_controls
                        .borrow()
                        .get(&id.to_string())
                        .is_some_and(|control| {
                            control.label == "Allow similar"
                                && control.session.key() == request.key()
                        })
                })
            })
            .unwrap()
            .button
            .clone();
        assert_eq!(
            scope.frame().origin.x + scope.frame().size.width,
            similar.frame().origin.x + similar.frame().size.width
        );
        assert_eq!(scope.frame().origin.y, similar.frame().origin.y);
        assert_eq!(scope.frame().size.height, similar.frame().size.height);
        assert!(scope.frame().origin.x > similar.frame().origin.x);
        assert!(scope.refusesFirstResponder());
        let scope_id = scope.identifier().unwrap().to_string();
        assert!(
            !controller
                .keyboard_targets(&controller.ivars().layout.borrow())
                .contains(&scope_id)
        );
        while commands.try_recv().is_ok() {}
        // SAFETY: Exercise the actual scope segment without submitting an approval.
        unsafe {
            scope.performClick(None);
        }
        assert!(
            commands.try_recv().is_err(),
            "scope disclosure must not submit an approval"
        );
        assert!(
            text_label(root, "cargo test").is_some(),
            "disclosure reveals the literal scope"
        );
        assert!(controller.ivars().list.get().unwrap().frame().size.height > height);
        export(&bitmap(root), &format!("{name}-scope-expanded"));
        // SAFETY: The retained button still identifies the same request scope.
        unsafe {
            let _: () = msg_send![&*controller,respondToRequest:&*scope];
        }
        assert!(text_label(root, "cargo test").is_none());
        assert_eq!(
            controller.ivars().list.get().unwrap().frame().size.height,
            height
        );
        controller.ivars().expanded.borrow_mut().clear();
        controller.ivars().reveal.borrow_mut().clear();
        controller
            .ivars()
            .expanded
            .borrow_mut()
            .insert(questions.key());
        controller
            .ivars()
            .reveal
            .borrow_mut()
            .insert(questions.key(), 1.0);
        controller.render_rows(&sessions);
        export(&bitmap(root), &format!("{name}-questions"));
        let answer = controller
            .ivars()
            .buttons
            .borrow()
            .iter()
            .find(|item| item.radio.is_some() && item.button.title().to_string() == "All checks")
            .expect("preset answer target")
            .button
            .identifier()
            .unwrap()
            .to_string();
        *controller.ivars().keyboard_focus.borrow_mut() = Some(answer);
        controller.apply_hover(None);
        export(&bitmap(root), &format!("{name}-answer-focused"));
        let field = editable(root).expect("native custom-answer field");
        // SAFETY: The test owns the attached field and hidden window.
        unsafe {
            field.selectText(None);
        }
        assert!(
            field.currentEditor().is_some(),
            "rounded input must still use the native editor"
        );
        controller.apply_hover(None);
        assert!(
            controller
                .ivars()
                .keyboard_focus
                .borrow()
                .as_ref()
                .is_some_and(|id| id.starts_with("response:")),
            "editing must own the single keyboard focus"
        );
        export(&bitmap(root), &format!("{name}-input-focused"));
        let open_before_resize = controller.ivars().menu_open.replace(true);
        field
            .currentEditor()
            .unwrap()
            .setString(ns_string!("Keep this custom answer while resizing."));
        field
            .currentEditor()
            .unwrap()
            .setSelectedRange(NSRange::new(5, 4));
        controller.change_width(680.0);
        assert_eq!(
            root.window().unwrap().frame().size.width,
            root.frame().size.width,
            "tracking host and custom content must agree on width"
        );
        for control in controller.ivars().toolbar_buttons.borrow().iter() {
            assert_eq!(
                control.button.frame(),
                control.background.frame(),
                "header hover must follow its icon after resizing"
            );
        }
        let resized_field = editable(root).expect("resized custom answer");
        assert_eq!(
            resized_field.currentEditor().unwrap().string().to_string(),
            "Keep this custom answer while resizing."
        );
        assert_eq!(
            resized_field.currentEditor().unwrap().selectedRange(),
            NSRange::new(5, 4)
        );
        assert!(resized_field.frame().size.width > field.frame().size.width + 200.0);
        export(&bitmap(root), &format!("{name}-wide-questions"));
        for width in [520.0, 740.0, 580.0, 680.0, 460.0] {
            controller.change_width(width);
            assert_eq!(
                root.window().unwrap().frame().size.width,
                root.frame().size.width
            );
            assert_eq!(
                controller.ivars().backdrop.get().unwrap().frame(),
                root.bounds(),
                "mesh must cover the complete item after every resize"
            );
            for control in controller.ivars().toolbar_buttons.borrow().iter() {
                assert_eq!(
                    control.button.frame(),
                    control.background.frame(),
                    "hover must remain aligned through grow/shrink reversals"
                );
            }
        }
        // Reproduce a later native-menu layout restoring its previous wider size.
        let mut stale = window.frame();
        stale.size.width += 110.0;
        window.setFrame_display(stale, false);
        controller.sync_window_width();
        controller.sync_backdrop();
        assert_eq!(window.frame().size.width, root.frame().size.width);
        assert_eq!(window.frame().size.width, 460.0);
        assert_eq!(
            controller.ivars().backdrop.get().unwrap().frame(),
            root.bounds()
        );
        controller.ivars().menu_open.set(open_before_resize);

        for panel in controller.ivars().question_panels.borrow().values() {
            panel.stop_editing();
        }

        controller.ivars().expanded.borrow_mut().clear();
        controller.ivars().reveal.borrow_mut().clear();
        controller
            .ivars()
            .expanded
            .borrow_mut()
            .insert(working.key());
        controller
            .ivars()
            .reveal
            .borrow_mut()
            .insert(working.key(), 1.0);
        controller.render_rows(&sessions);
        assert!(text_label(root, "Running cargo test --offline --all-targets").is_some());
        export(&bitmap(root), &format!("{name}-working"));
        let static_caption = bitmap(root);
        let glows = controller.ivars().work_glows.borrow().clone();
        assert_eq!(glows.len(), 1);
        glows[0].advance(1.9, true);
        let swept = bitmap(root);
        export(&swept, &format!("{name}-work-sweep-left"));
        let region = root.convertRect_fromView(glows[0].bounds(), Some(&glows[0]));
        let scale = swept.pixelsWide() as f64 / root.bounds().size.width;
        let x0 = (region.origin.x * scale).max(0.0) as isize;
        let y0 = (swept.pixelsHigh() as f64 - (region.origin.y + region.size.height) * scale)
            .max(0.0) as isize;
        let x1 =
            ((region.origin.x + region.size.width) * scale).min(swept.pixelsWide() as f64) as isize;
        let y1 = (y0 as f64 + region.size.height * scale).min(swept.pixelsHigh() as f64) as isize;
        let mut changed = 0;
        for y in (y0..y1).step_by(2) {
            for x in (x0..x1).step_by(2) {
                let before = rgb(&static_caption, x, y);
                let after = rgb(&swept, x, y);
                if before.iter().zip(after).any(|(a, b)| (a - b).abs() > 0.03) {
                    changed += 1;
                }
            }
        }
        assert!(
            changed > 10,
            "{name}: the green caption sweep must be visible on the native raster"
        );
        glows[0].advance(2.5, true);
        export(&bitmap(root), &format!("{name}-work-sweep-right"));
        glows[0].advance(2.5, false);
        let disabled = bitmap(root);
        for y in (y0..y1).step_by(4) {
            for x in (x0..x1).step_by(4) {
                assert!(
                    rgb(&static_caption, x, y)
                        .iter()
                        .zip(rgb(&disabled, x, y))
                        .all(|(a, b)| (a - b).abs() < 0.015),
                    "reduced motion restores the resting caption"
                );
            }
        }
        let mut generic_work = working.clone();
        generic_work.set_progress(None);
        assert_eq!(generic_work.work_caption(), Some("Working…"));
        controller.render_rows(std::slice::from_ref(&generic_work));
        assert!(
            text_label(root, "Working…").is_some(),
            "working sessions must retain the animated status line without a detailed caption"
        );
        assert_eq!(controller.ivars().work_glows.borrow().len(), 1);
        export(&bitmap(root), &format!("{name}-work-without-description"));
        generic_work.set_progress(Some("   ".into()));
        assert_eq!(generic_work.work_caption(), Some("Working…"));
        generic_work.activity = Activity::Idle;
        assert_eq!(generic_work.work_caption(), None);
        generic_work.activity = Activity::Confirmation;
        assert_eq!(generic_work.work_caption(), None);
        generic_work.activity = Activity::Working;
        generic_work.set_approval(request.approval().cloned());
        assert_eq!(generic_work.work_caption(), None);
        let mut table_session = fixture("Process memory", Activity::Idle);
        table_session.set_last_message("Memory usage:\n\n| Process | Memory |\n| :--- | ---: |\n| Dart 8651 | **1.46 GB** |\n| Dart VM 9735 | **1.40 GB** |\n| rust-analyzer 8285 | **1.01 GB** |\n| WindowServer | **830 MB** |\n| Code Helper with a long process title that wraps | **664 MB** |\n\nAll values are synthetic.".into());
        controller
            .ivars()
            .expanded
            .borrow_mut()
            .insert(table_session.key());
        controller
            .ivars()
            .reveal
            .borrow_mut()
            .insert(table_session.key(), 1.0);
        controller.render_rows(std::slice::from_ref(&table_session));
        export(&bitmap(root), &format!("{name}-markdown-table"));
        let saved_sessions = controller.ivars().all.borrow().clone();
        *controller.ivars().all.borrow_mut() = vec![table_session];
        let narrow_table_width = table_view(root)
            .unwrap()
            .enclosingScrollView()
            .unwrap()
            .frame()
            .size
            .width;
        controller.change_width(680.0);
        assert_eq!(
            table_view(root)
                .unwrap()
                .enclosingScrollView()
                .unwrap()
                .frame()
                .size
                .width,
            narrow_table_width + 220.0,
            "the native table must use the expanded message width"
        );
        let quota_name = text_label(root, "Codex Weekly").expect("quota label");
        // SAFETY: The fixture owns the complete attached view tree.
        let quota_group = unsafe { quota_name.superview() }.unwrap();
        assert_eq!(quota_group.frame().size.width, MENU_WIDTH);
        assert_eq!(
            quota_group.frame().origin.x,
            110.0,
            "default-width quota group must be centered in the wider host"
        );
        // Reset inventory is driven only by monitor state, with a retained
        // native spinner and one capsule that grows above its quota row.
        let baseline = (*monitor.snapshot()).clone();
        let closed_height = controller.ivars().footer_height.get();
        controller.ivars().usage_resets_expanded.set(true);
        let mut loading = baseline.clone();
        loading.usage_resets.phase = crate::domain::usage::ResetPhase::Loading;
        _published.send_replace(std::sync::Arc::new(loading));
        controller.render_footer();
        assert!(controller.ivars().usage_resets_spinning.get());
        assert_eq!(controller.ivars().footer_height.get(), closed_height + 78.0);
        let mut ready = baseline.clone();
        ready.usage_resets = crate::domain::usage::Resets {
            generation: 1,
            phase: crate::domain::usage::ResetPhase::Ready,
            available_count: Some(3),
            credits: Some(vec![
                crate::domain::usage::ResetCredit {
                    title: "Full reset".into(),
                    expires_at: Some(2_000_000_000)
                };
                3
            ]),
            error: None,
        };
        _published.send_replace(std::sync::Arc::new(ready));
        controller.render_footer();
        assert!(!controller.ivars().usage_resets_spinning.get());
        assert!(text_label(root, "Usage limit resets").is_some());
        assert!(text_label(root, "3 available").is_some());
        assert!(text_label(root, "Expires").is_some());
        export(&bitmap(root), &format!("{name}-usage-resets"));
        controller.ivars().usage_resets_expanded.set(false);
        controller.toggle_usage_resets(sel!(toggleUsageResets:), None);
        assert!(
            controller.ivars().usage_resets_spinning.get(),
            "reopening cached results must show loading synchronously"
        );
        assert!(
            text_label(root, "Full reset").is_none(),
            "cached rows must not flash before loading arrives"
        );
        let mut stale = (*monitor.snapshot()).clone();
        stale.revision += 1;
        _published.send_replace(std::sync::Arc::new(stale));
        controller.render_footer();
        assert!(
            controller.ivars().usage_resets_spinning.get(),
            "unrelated revisions must not reveal old results"
        );
        let mut fresh = (*monitor.snapshot()).clone();
        fresh.usage_resets.generation += 1;
        _published.send_replace(std::sync::Arc::new(fresh));
        controller.render_footer();
        assert!(
            !controller.ivars().usage_resets_spinning.get(),
            "a new result must resolve loading even if its intermediate loading snapshot was coalesced"
        );
        assert!(text_label(root, "Full reset").is_some());
        controller.ivars().usage_resets_expanded.set(false);
        _published.send_replace(std::sync::Arc::new(baseline));
        controller.render_footer();
        assert_eq!(controller.ivars().footer_height.get(), closed_height);
        export(&bitmap(root), &format!("{name}-wide-table"));
        controller.change_width(460.0);
        *controller.ivars().all.borrow_mut() = saved_sessions;
        let image = crate::domain::media::Image {
            id: "synthetic-mesh".into(),
            mime: "image/png",
            data: std::sync::Arc::from(include_bytes!("../../assets/monitor-mesh.png").as_slice()),
        };
        let preview =
            controller.message_with_images("[Image]", std::slice::from_ref(&image), 240.0, false);
        assert!(preview.frame().size.height <= 240.001);
        assert_eq!(
            preview.subviews().count(),
            1,
            "successful image replaces the placeholder"
        );
        let child = preview.subviews().objectAtIndex(0);
        let image_view = child
            .downcast_ref::<NSImageView>()
            .expect("native image preview");
        let (w, h) = image.dimensions().unwrap();
        assert!(
            (image_view.frame().size.width / image_view.frame().size.height
                - f64::from(w) / f64::from(h))
            .abs()
                < 0.001
        );
        let mut invalid = image.clone();
        invalid.data = std::sync::Arc::from(&b"invalid"[..]);
        assert!(
            controller
                .message_with_images("[Image]", &[invalid], 240.0, false)
                .downcast_ref::<NSTextField>()
                .is_some(),
            "invalid images retain their text fallback"
        );
        let mut image_session = fixture("Image preview", Activity::Idle);
        image_session.set_last_message("Agent output image".into());
        image_session.set_images(vec![image.clone()]);
        controller
            .ivars()
            .expanded
            .borrow_mut()
            .insert(image_session.key());
        controller
            .ivars()
            .reveal
            .borrow_mut()
            .insert(image_session.key(), 1.0);
        controller.ivars().message_history.borrow_mut().insert(
            image_session.key(),
            MessageHistory {
                messages: vec![crate::agents::history::Message {
                    cursor: "image-history".into(),
                    text: "[Image]".into(),
                    images: vec![image],
                }],
                ..Default::default()
            },
        );
        controller.render_rows(&[image_session]);
        export(&bitmap(root), &format!("{name}-image-preview"));
        let normal = crate::macos::markdown::message(
            "Plain **message**",
            240.0,
            &NSColor::labelColor(),
            false,
            mtm,
        );
        assert!(normal.downcast_ref::<NSTextField>().is_some());
        let short = crate::macos::markdown::message(
            "| A | B |\n| --- | --- |\n| x | y |",
            240.0,
            &NSColor::labelColor(),
            false,
            mtm,
        );
        let long = crate::macos::markdown::message(
            "| Process | Memory |\n| --- | ---: |\n| A very long process name that must wrap inside its column without losing the second column | **1.46 GB** |",
            240.0,
            &NSColor::labelColor(),
            false,
            mtm,
        );
        let short_text = table_view(&short).unwrap();
        assert!(
            !short_text
                .enclosingScrollView()
                .unwrap()
                .hasHorizontalScroller()
        );
        let long_text = table_view(&long).unwrap();
        let scroller = long_text.enclosingScrollView().unwrap();
        assert!(scroller.hasHorizontalScroller());
        assert_eq!(scroller.frame().size.width, 240.0);
        assert_eq!(
            long_text.frame().size.width,
            480.0,
            "wide table must be capped at twice the message width"
        );
        scroller
            .contentView()
            .scrollToPoint(NSPoint::new(120.0, 0.0));
        scroller.reflectScrolledClipView(&scroller.contentView());
        assert!(
            scroller.contentView().bounds().origin.x > 0.0,
            "the table must actually scroll horizontally"
        );
        export(&bitmap(root), &format!("{name}-scrollable-tables"));
        assert!(
            long.frame().size.height > short.frame().size.height,
            "table height must include wrapped cells"
        );
        // SAFETY: These test-owned TextKit views remain on the main thread.
        unsafe {
            let text = table_view(&long).unwrap();
            let manager = text.layoutManager().unwrap();
            let container = text.textContainer().unwrap();
            assert!(
                manager.usedRectForTextContainer(&container).size.width <= 481.0,
                "cell wrapping must stay inside the capped table width"
            );
        }
        let mixed = crate::macos::markdown::message(
            "Text before the table wraps at the normal message width.\n\n| Name | Count |\n| --- | ---: |\n| abcdefghijklmnopqrstuvwxy abcdefghijk | 10 |\n\nText after the table also wraps at the normal message width.",
            240.0,
            &NSColor::labelColor(),
            false,
            mtm,
        );
        assert_eq!(mixed.subviews().count(), 3, "only the table should scroll");
        assert!(
            mixed
                .subviews()
                .objectAtIndex(0)
                .downcast_ref::<NSTextField>()
                .is_some()
        );
        assert!(
            mixed
                .subviews()
                .objectAtIndex(2)
                .downcast_ref::<NSTextField>()
                .is_some()
        );
        for index in 0..3 {
            assert_eq!(
                mixed.subviews().objectAtIndex(index).frame().size.width,
                240.0
            );
        }
        let natural_width = table_view(&mixed).unwrap().frame().size.width;
        assert!(
            natural_width > 240.0 && natural_width < 480.0,
            "moderately wide tables must fit their measured content rather than always using the cap"
        );
        let idle = sessions[0].clone();
        controller.ivars().expanded.borrow_mut().insert(idle.key());
        controller
            .ivars()
            .reveal
            .borrow_mut()
            .insert(idle.key(), 1.0);
        controller.render_rows(std::slice::from_ref(&idle));
        let composer = controller
            .ivars()
            .composers
            .borrow()
            .get(&idle.key())
            .unwrap()
            .clone();
        assert!(composer.attached() && composer.field().isEnabled());
        assert!(
            composer.button().isHidden(),
            "empty composer must have no Send button"
        );
        assert_eq!(
            composer.view().frame().size.height,
            40.0,
            "32 pt empty fill plus drawing clearance"
        );
        let path = text_label(root, "/demo/task-board").unwrap();
        let input_frame =
            root.convertRect_fromView(composer.view().bounds(), Some(composer.view()));
        let path_frame = root.convertRect_fromView(path.bounds(), Some(&path));
        assert!(
            input_frame.origin.y > path_frame.origin.y + path_frame.size.height,
            "composer must precede session metadata in the unflipped root"
        );
        assert!(
            controller
                .keyboard_targets(&controller.ivars().layout.borrow())
                .contains(&composer.id())
        );
        while commands.try_recv().is_ok() {}
        composer.field().setStringValue(ns_string!("   "));
        composer.submit();
        assert!(commands.try_recv().is_err(), "blank drafts must not send");
        composer
            .field()
            .setStringValue(ns_string!("Please continue with the next phase."));
        composer.sync(&monitor.snapshot(), &idle);
        assert!(composer.button().isEnabled() && !composer.button().isHidden());
        assert_eq!(composer.button().frame().size, NSSize::new(28.0, 28.0));
        let field_frame = composer
            .view()
            .convertRect_fromView(composer.field().bounds(), Some(composer.field()));
        assert!(
            (field_frame.mid().y - composer.button().frame().mid().y).abs() < 0.5,
            "short input and Send must share a vertical center"
        );
        export(&bitmap(root), &format!("{name}-composer"));
        *controller.ivars().keyboard_focus.borrow_mut() = Some(composer.id());
        controller.reveal_keyboard_focus();
        controller.apply_hover(None);
        assert!(
            composer.editing(),
            "navigating to the input must start native editing directly"
        );
        export(&bitmap(root), &format!("{name}-composer-focus"));
        let compact_height = composer.view().frame().size.height;
        let long_draft =
            "Please continue with the next phase, including all checks and a detailed summary. "
                .repeat(12);
        let selection = NSRange::new(20, 3);
        composer
            .field()
            .currentEditor()
            .unwrap()
            .setString(&NSString::from_str(&long_draft));
        composer
            .field()
            .currentEditor()
            .unwrap()
            .setSelectedRange(selection);
        assert!(
            composer.resize_to_fit(),
            "long draft must grow while editing"
        );
        let was_open = controller.ivars().menu_open.replace(true);
        controller.render_rows(std::slice::from_ref(&idle));
        assert!(composer.view().frame().size.height > compact_height + 36.0);
        let text_frame = composer
            .view()
            .convertRect_fromView(composer.field().bounds(), Some(composer.field()));
        assert!(
            text_frame.origin.y >= composer.button().frame().max().y + 8.0,
            "multiline text must stay above the Send footer"
        );
        assert!(composer.editing(), "resizing must retain editing");
        assert_eq!(
            composer
                .field()
                .currentEditor()
                .unwrap()
                .string()
                .to_string(),
            long_draft
        );
        assert_eq!(
            composer.field().currentEditor().unwrap().selectedRange(),
            selection
        );
        // Horizontal resize must reflow native text without discarding a live draft.
        let original_sessions = controller.ivars().all.borrow().clone();
        *controller.ivars().all.borrow_mut() = vec![idle.clone()];
        let narrow_height = composer.view().frame().size.height;
        controller.change_width(680.0);
        assert_eq!(root.frame().size.width, 680.0);
        assert_eq!(
            controller.ivars().list.get().unwrap().frame().size.width,
            658.0
        );
        assert_eq!(composer.view().frame().size.width, DETAIL_WIDTH + 220.0);
        assert!(composer.view().frame().size.height < narrow_height);
        assert!(composer.editing());
        assert_eq!(
            composer
                .field()
                .currentEditor()
                .unwrap()
                .string()
                .to_string(),
            long_draft
        );
        assert_eq!(
            composer.field().currentEditor().unwrap().selectedRange(),
            selection
        );
        export(&bitmap(root), &format!("{name}-wide-composer"));
        controller.change_width(460.0);
        assert_eq!(composer.view().frame().size.height, narrow_height);
        assert_eq!(
            composer.field().currentEditor().unwrap().selectedRange(),
            selection
        );
        *controller.ivars().all.borrow_mut() = original_sessions;
        let path = text_label(root, "/demo/task-board").unwrap();
        let input_frame =
            root.convertRect_fromView(composer.view().bounds(), Some(composer.view()));
        let path_frame = root.convertRect_fromView(path.bounds(), Some(&path));
        assert!(
            input_frame.origin.y > path_frame.origin.y + path_frame.size.height,
            "metadata must move below the growing input"
        );
        export(&bitmap(root), &format!("{name}-composer-multiline"));
        composer
            .field()
            .currentEditor()
            .unwrap()
            .setString(ns_string!("Please continue with the next phase."));
        assert!(composer.resize_to_fit(), "short draft must shrink again");
        controller.render_rows(std::slice::from_ref(&idle));
        assert_eq!(composer.view().frame().size.height, compact_height);
        controller.ivars().menu_open.set(was_open);
        composer.stop_editing();
        composer.submit();
        let crate::actors::monitor::Command::SendMessage { id, session, text } =
            commands.try_recv().unwrap()
        else {
            panic!("expected message command")
        };
        assert_eq!(session, idle.key());
        assert_eq!(text, "Please continue with the next phase.");
        composer.submit();
        assert!(
            commands.try_recv().is_err(),
            "pending sends must not duplicate"
        );
        let mut response = (*monitor.snapshot()).clone();
        response.message_sends.insert(
            id,
            crate::actors::monitor::MessageSend {
                session: idle.key(),
                status: crate::actors::monitor::MessageStatus::Failed(
                    "Test delivery failure".into(),
                ),
            },
        );
        composer.sync(&response, &idle);
        assert_eq!(
            composer.field().stringValue().to_string(),
            text,
            "failure retains draft"
        );
        assert!(composer.button().isEnabled());
        composer.submit();
        let crate::actors::monitor::Command::SendMessage { id, .. } = commands.try_recv().unwrap()
        else {
            panic!("expected explicit retry")
        };
        response.message_sends.insert(
            id,
            crate::actors::monitor::MessageSend {
                session: idle.key(),
                status: crate::actors::monitor::MessageStatus::Sent,
            },
        );
        composer.sync(&response, &idle);
        assert!(
            composer.field().stringValue().is_empty(),
            "confirmed send clears draft"
        );
        composer
            .field()
            .setStringValue(ns_string!("Keep this draft"));
        controller.render_rows(std::slice::from_ref(&working));
        controller.render_rows(std::slice::from_ref(&idle));
        assert_eq!(
            composer.field().stringValue().to_string(),
            "Keep this draft"
        );
        assert!(!super::super::session_composer::available(&working));
        assert!(!super::super::session_composer::available(&request));
        let mut unknown = idle.clone();
        unknown.activity = Activity::Unknown;
        assert!(!super::super::session_composer::available(&unknown));
        let mut offline = idle.clone();
        offline.set_authoritative(false);
        composer.sync(&monitor.snapshot(), &offline);
        assert!(!composer.field().isEnabled() && !composer.button().isEnabled());
        // Settings must reuse this exact menu host, retaining native controls
        // and the existing session draft/scroll state across every pane switch.
        let windows_before = NSApplication::sharedApplication(mtm).windows().count();
        controller.ivars().menu_open.set(true);
        controller.ivars().filter_dirty.set(true);
        controller.filter();
        list.scrollPoint(NSPoint::new(0.0, 80.0));
        let old_scroll = controller
            .ivars()
            .scroll
            .get()
            .unwrap()
            .contentView()
            .bounds()
            .origin;
        controller.show_settings();
        assert_eq!(
            NSApplication::sharedApplication(mtm).windows().count(),
            windows_before,
            "settings must not create a second window"
        );
        let shell = controller.ivars().settings_shell.get().unwrap();
        assert!(controller.ivars().scroll.get().unwrap().isHidden());
        assert!(controller.ivars().footer.get().unwrap().isHidden());
        assert!(std::ptr::eq(&*shell.scroll.window().unwrap(), &*window));
        assert!(
            !window
                .firstResponder()
                .is_some_and(|responder| std::ptr::eq::<AnyObject>(
                    (&*responder as *const NSResponder).cast(),
                    (&*shell.back as *const NSButton).cast()
                )),
            "opening settings must not focus Sessions"
        );
        window.makeFirstResponder(None);
        shell.paint_at(None);
        let resting = bitmap(root);
        let point = shell
            .back
            .convertPoint_toView(NSPoint::new(42.0, 16.0), None);
        shell.paint_at(Some(point));
        let hovered = bitmap(root);
        let scale = hovered.pixelsWide() as f64 / 460.0;
        let before = rgb(&resting, (24.0 * scale) as isize, (25.0 * scale) as isize);
        let after = rgb(&hovered, (24.0 * scale) as isize, (25.0 * scale) as isize);
        assert!(
            before.iter().zip(after).any(|(a, b)| (a - b).abs() > 0.02),
            "Sessions must show a glass hover"
        );
        export(&hovered, &format!("{name}-settings-back-hover"));
        shell.paint_at(None);
        for index in 0..5 {
            controller.ivars().settings_selected.set(index);
            controller.layout_settings();
            let tab = controller
                .ivars()
                .settings_tabs
                .get()
                .unwrap()
                .tabViewItems()
                .objectAtIndex(index);
            let pane = tab.viewController(mtm).unwrap().view();
            assert!(!pane.isHiddenOrHasHiddenAncestor());
            assert!(
                pane.visibleRect().size.height > 0.0,
                "settings pane must remain visible"
            );
            assert!(shell.controls().len() >= 5);
            export(&bitmap(root), &format!("{name}-settings-{index}"));
            controller.change_width(680.0);
            let settings_parent = unsafe { pane.superview() }.unwrap();
            assert_eq!(settings_parent.frame().origin.x, 110.0);
            assert_eq!(settings_parent.frame().size.width, 460.0);
            assert_eq!(
                shell
                    .back
                    .convertPoint_toView(NSPoint::new(0.0, 0.0), Some(root))
                    .x,
                128.0
            );
            assert_eq!(
                shell.scroll.documentView().unwrap().frame().size.width,
                680.0
            );
            export(&bitmap(root), &format!("{name}-settings-{index}-wide"));
            controller.change_width(460.0);
        }
        let web = controller.ivars().web_settings.get().unwrap();
        web.set_report(serde_json::json!({"running":true,"urls":["http://192.0.2.1:8765/"],"pairingLinks":["http://192.0.2.1:8765/#pair=demo-preview-token"],"pairingExpiresIn":120,"devices":[]}));
        controller.layout_settings();
        let web_bitmap = bitmap(root);
        let qr = (0..web.subviews().count())
            .map(|index| web.subviews().objectAtIndex(index))
            .find(|view| {
                view.identifier()
                    .is_some_and(|id| id.to_string() == "web-pairing-qr")
            })
            .unwrap();
        let qr_bitmap = qr
            .bitmapImageRepForCachingDisplayInRect(qr.bounds())
            .unwrap();
        qr.cacheDisplayInRect_toBitmapImageRep(qr.bounds(), &qr_bitmap);
        let scale = qr_bitmap.pixelsWide() as f64 / 200.0;
        let code = qrcode::QrCode::new("http://192.0.2.1:8765/#pair=demo-preview-token").unwrap();
        let unit = (200.0 / (code.width() + 8) as f64).floor();
        let finder = (200.0 - unit * (code.width() + 8) as f64) / 2.0 + 4.5 * unit;
        assert!(
            rgb(
                &qr_bitmap,
                (finder * scale) as isize,
                (finder * scale) as isize
            )
            .iter()
            .all(|value| *value < 0.05),
            "QR finder must be black in both themes"
        );
        let web_action = |action| {
            (0..web.subviews().count())
                .find_map(|index| {
                    web.subviews()
                        .objectAtIndex(index)
                        .downcast::<NSButton>()
                        .ok()
                        .filter(|button| button.action() == Some(action))
                })
                .unwrap()
        };
        let copy = web_action(sel!(copyWebLink:));
        let renew = web_action(sel!(renewWebPairing:));
        let link = web_action(sel!(openWebLink:));
        assert_eq!(copy.frame().size, NSSize::new(28.0, 28.0));
        assert_eq!(copy.frame().origin.x, renew.frame().origin.x);
        assert_eq!(copy.frame().size, renew.frame().size);
        assert!(copy.title().is_empty() && renew.title().is_empty());
        assert!(copy.image().is_some() && renew.image().is_some());
        assert_eq!(link.title().to_string(), "http://192.0.2.1:8765/");
        assert!(!link.toolTip().unwrap().to_string().contains("pair="));
        export(&web_bitmap, &format!("{name}-settings-web-qr"));
        let connected_report = serde_json::json!({"running":true,"ngrok":{"tokenSaved":true,"status":"connected","url":"https://fixture.ngrok-free.app"},"tunnel":{"provider":"cloudflare","status":"connected","url":"https://fixture.trycloudflare.com"},"urls":["https://fixture.trycloudflare.com/","https://fixture.ngrok-free.app/","http://192.0.2.1:8765/"],"pairingLinks":["https://fixture.trycloudflare.com/#pair=demo-preview-token","https://fixture.ngrok-free.app/#pair=demo-preview-token","http://192.0.2.1:8765/#pair=demo-preview-token"],"pairingExpiresIn":120,"devices":[]});
        web.set_report(connected_report.clone());
        let disclosure = shell
            .controls()
            .into_iter()
            .find_map(|control| {
                control
                    .downcast::<NSButton>()
                    .ok()
                    .filter(|button| button.action() == Some(sel!(toggleRemoteSection:)))
            })
            .unwrap();
        let collapsed_height = web.frame().size.height;
        // SAFETY: The retained settings pane owns this action target.
        unsafe {
            disclosure.performClick(None);
        }
        web.poll(false);
        assert!(web.frame().size.height > collapsed_height);
        assert!(web.take_layout_changed());
        assert!(text_label(web, "ready").is_some());
        controller.layout_settings();
        export(&bitmap(root), &format!("{name}-settings-web-ngrok"));
        let sections = web.subviews();
        let ngrok_section = (0..sections.count())
            .find_map(|index| {
                let view = sections.objectAtIndex(index);
                view.identifier()
                    .is_some_and(|id| id.to_string() == "ngrok-section")
                    .then_some(view)
            })
            .unwrap();
        let action_button = |action| {
            let children = ngrok_section.subviews();
            (0..children.count())
                .find_map(|index| {
                    children
                        .objectAtIndex(index)
                        .downcast::<NSButton>()
                        .ok()
                        .filter(|button| button.action() == Some(action))
                })
                .expect("fixture settings action")
        };
        assert_eq!(disclosure.frame().size, NSSize::new(28.0, 28.0));
        assert!(disclosure.title().to_string().is_empty());
        assert!(
            !text_label(web, "stored securely in macOS Keychain")
                .unwrap()
                .isHidden()
        );
        // SAFETY: These controls target the fixture-owned pane, not live IPC.
        unsafe {
            action_button(sel!(changeRemoteKey:)).performClick(None);
        }
        web.poll(false);
        let children = ngrok_section.subviews();
        let token = (0..children.count())
            .find_map(|index| {
                children
                    .objectAtIndex(index)
                    .downcast::<NSSecureTextField>()
                    .ok()
            })
            .unwrap();
        assert!(!token.isHidden());
        assert!(
            text_label(web, "Token:").unwrap().frame().origin.y
                >= token.frame().origin.y + token.frame().size.height
        );
        assert_eq!(
            token.frame().origin.x,
            text_label(web, "Will be stored securely in macOS Keychain")
                .unwrap()
                .frame()
                .origin
                .x
        );
        let connect = action_button(sel!(connectRemote:));
        assert!(!connect.isEnabled());
        if let Some(editor) = token.currentEditor() {
            editor.setString(ns_string!("synthetic-replacement-token"));
        } else {
            token.setStringValue(ns_string!("synthetic-replacement-token"));
        }
        web.set_report(connected_report.clone());
        assert!(connect.isEnabled());
        assert!(
            !token.isHidden(),
            "status polling must preserve replacement editing"
        );
        controller.layout_settings();
        export(&bitmap(root), &format!("{name}-settings-web-ngrok-change"));
        // SAFETY: Cancel only updates this fixture's local draft state.
        unsafe {
            action_button(sel!(cancelRemoteKey:)).performClick(None);
        }
        web.poll(false);
        assert!(token.isHidden());
        assert!(
            !text_label(web, "stored securely in macOS Keychain")
                .unwrap()
                .isHidden()
        );
        let mut failed = connected_report;
        failed["ngrok"]["status"] = serde_json::json!("failed");
        failed["ngrok"]["error"] =
            serde_json::json!("Connection interrupted. Check your account and network.");
        failed["ngrok"]["url"] = serde_json::Value::Null;
        failed["urls"].as_array_mut().unwrap().remove(1);
        failed["pairingLinks"].as_array_mut().unwrap().remove(1);
        web.set_report(failed);
        assert!(text_label(web, "connection failed").is_some());
        assert!(
            shell
                .controls()
                .iter()
                .filter_map(|control| control.downcast_ref::<NSButton>())
                .any(|button| button.title().to_string() == "Reconnect")
        );
        controller.layout_settings();
        export(&bitmap(root), &format!("{name}-settings-web-ngrok-failed"));
        let tuna_section = (0..sections.count())
            .find_map(|index| {
                let view = sections.objectAtIndex(index);
                view.identifier()
                    .is_some_and(|id| id.to_string() == "tuna-section")
                    .then_some(view)
            })
            .unwrap();
        let tuna_buttons = tuna_section.subviews();
        let tuna_disclosure = (0..tuna_buttons.count())
            .find_map(|index| {
                tuna_buttons
                    .objectAtIndex(index)
                    .downcast::<NSButton>()
                    .ok()
                    .filter(|button| button.action() == Some(sel!(toggleRemoteSection:)))
            })
            .unwrap();
        // SAFETY: The Tuna section belongs to this fixture; opening it performs no IPC.
        unsafe {
            tuna_disclosure.performClick(None);
        }
        web.poll(false);
        controller.layout_settings();
        assert!(text_label(&tuna_section, "API key:").is_some());
        export(
            &bitmap(root),
            &format!("{name}-settings-web-tuna-unconfigured"),
        );
        web.set_report(serde_json::json!({"running":true,"tuna":{"status":"connected","tokenSaved":true,"clientInstalled":true,"url":"https://fixture.ru.tuna.am"},"ngrok":{"status":"connected","tokenSaved":true,"url":"https://fixture.ngrok-free.app"},"tunnel":{"status":"connected","url":"https://fixture.trycloudflare.com"},"urls":["https://fixture.trycloudflare.com/","https://fixture.ngrok-free.app/","https://fixture.ru.tuna.am/","http://192.0.2.1:8765/"],"pairingLinks":["https://fixture.trycloudflare.com/#pair=synthetic-test-link","https://fixture.ngrok-free.app/#pair=synthetic-test-link","https://fixture.ru.tuna.am/#pair=synthetic-test-link","http://192.0.2.1:8765/#pair=synthetic-test-link"],"pairingExpiresIn":120,"devices":[]}));
        controller.layout_settings();
        assert!(
            !text_label(&tuna_section, "stored securely in macOS Keychain")
                .unwrap()
                .isHidden()
        );
        export(
            &bitmap(root),
            &format!("{name}-settings-web-tuna-connected"),
        );
        unsafe {
            tuna_disclosure.performClick(None);
        }
        web.poll(false);
        // SAFETY: The retained settings pane owns this action target.
        unsafe {
            disclosure.performClick(None);
        }
        web.poll(false);
        let children = web.subviews();
        let progress = (0..children.count())
            .find_map(|i| {
                children
                    .objectAtIndex(i)
                    .downcast::<NSProgressIndicator>()
                    .ok()
            })
            .expect("pairing countdown progress");
        assert!(!progress.isHidden());
        assert!(
            !web.refresh_countdown_at(
                std::time::Instant::now() + std::time::Duration::from_secs(110)
            )
        );
        assert!(progress.doubleValue() > 8.0 && progress.doubleValue() <= 10.0);
        assert!(
            web.refresh_countdown_at(
                std::time::Instant::now() + std::time::Duration::from_secs(121)
            ),
            "expired code must trigger automatic renewal"
        );
        assert_eq!(progress.doubleValue(), 0.0);
        web.set_report(serde_json::json!({"running":true,"urls":["http://192.0.2.1:8765/"],"pairingLinks":["http://192.0.2.1:8765/#pair=renewed-preview-token"],"pairingExpiresIn":120,"devices":[]}));
        assert!(
            !web.refresh_countdown_at(std::time::Instant::now()),
            "renewed code must reset countdown"
        );
        assert!(progress.doubleValue() > 119.0);
        let report = serde_json::json!({"running":true,"urls":["http://198.51.100.10:8765/","http://192.0.2.10:8765/","http://127.0.0.1:8765/"],"pairingLinks":[],"devices":[]});
        web.set_report(report.clone());
        assert!(
            web.refresh_countdown_at(std::time::Instant::now()),
            "used token must trigger renewal before its old deadline"
        );
        fn address_rows(view: &NSView) -> Vec<Retained<NSButton>> {
            let mut rows = Vec::new();
            if let Some(button) = view.downcast_ref::<NSButton>()
                && button.action() == Some(sel!(selectWebAddress:))
                && button.tag() >= 0
            {
                // SAFETY: The pane retains this row throughout the test.
                rows.push(
                    unsafe { Retained::retain(button as *const NSButton as *mut NSButton) }
                        .unwrap(),
                );
            }
            let children = view.subviews();
            for index in 0..children.count() {
                rows.extend(address_rows(&children.objectAtIndex(index)));
            }
            rows
        }
        let compact = web.frame().size.height;
        let mut many = report.clone();
        many["urls"] = serde_json::json!(
            (1..=8)
                .map(|n| format!("http://192.0.2.{n}:8765/"))
                .collect::<Vec<_>>()
        );
        web.set_report(many);
        controller.layout_settings();
        assert!(
            web.frame().size.height > compact,
            "pane must grow for extra addresses"
        );
        assert!(
            (0..web.subviews().count()).all(|i| web
                .subviews()
                .objectAtIndex(i)
                .downcast_ref::<NSScrollView>()
                .is_none()),
            "addresses must not have an inner scroller"
        );
        for row in address_rows(web) {
            assert!(
                row.visibleRect().size.height >= 28.0,
                "all rows fit without an inner clip"
            );
            // SAFETY: The fixture pane retains every address and its document.
            let parent = unsafe { row.superview() }.unwrap();
            assert!(
                row.frame().origin.y >= 0.0
                    && row.frame().origin.y + row.frame().size.height
                        <= parent.bounds().size.height,
                "every row must fit in the address document"
            );
        }
        web.set_report(report.clone());
        controller.layout_settings();
        assert_eq!(
            web.frame().size.height,
            compact,
            "pane shrinks when addresses disappear"
        );
        let rows = address_rows(web);
        assert_eq!(
            rows.len(),
            3,
            "every server address must be exposed as an inline choice"
        );
        for row in &rows {
            assert!(row.isEnabled());
            assert!(!row.isHiddenOrHasHiddenAncestor());
            assert!(row.visibleRect().size.height > 0.0);
        }
        // SAFETY: Production selector is implemented by the retained WebSettings pane.
        unsafe {
            let _: () = msg_send![web,selectWebAddress: &*rows[1]];
        }
        assert_eq!(rows[1].state(), NSControlStateValueOn);
        web.set_report(report.clone());
        let updated = address_rows(web);
        assert!(
            std::ptr::eq(&*rows[1], &*updated[1]),
            "routine polling must not rebuild address controls"
        );
        assert_eq!(updated[1].state(), NSControlStateValueOn);
        let mut reordered = report;
        reordered["urls"] =
            serde_json::json!(["http://192.0.2.10:8765/", "http://198.51.100.10:8765/"]);
        web.set_report(reordered);
        assert_eq!(
            address_rows(web)[0].state(),
            NSControlStateValueOn,
            "keep selected address when interface order changes"
        );
        web.set_report(serde_json::json!({"running":false}));
        controller.layout_settings();
        assert_eq!(
            web.frame().size.height,
            92.0,
            "disabled web pane must shrink"
        );
        let children = web.subviews();
        for index in 0..children.count() {
            let child = children.objectAtIndex(index);
            if child.downcast_ref::<NSButton>().is_some()
                || child.downcast_ref::<NSScrollView>().is_some()
            {
                assert!(
                    child.isHidden(),
                    "connection actions and addresses must disappear when off"
                );
            }
        }

        assert!(!web.refresh_countdown_at(std::time::Instant::now()));
        assert!(progress.isHidden());
        controller.ivars().settings_selected.set(3);
        controller.layout_settings();
        let token = controller.ivars().telegram_token.get().unwrap();
        token.setStringValue(ns_string!("Synthetic draft token"));
        let mut configured = (*monitor.snapshot()).clone();
        configured.telegram.connected = true;
        configured.telegram.paired = true;
        configured.telegram.bot_username = "aiagmon_demo_bot".into();
        _published.send_replace(std::sync::Arc::new(configured.clone()));
        controller.refresh_telegram_settings();
        assert!(
            !controller
                .ivars()
                .telegram_connected_view
                .get()
                .unwrap()
                .isHiddenOrHasHiddenAncestor()
        );
        export(
            &bitmap(root),
            &format!("{name}-settings-telegram-connected"),
        );
        configured.telegram.setting_up = true;
        _published.send_replace(std::sync::Arc::new(configured.clone()));
        controller.refresh_telegram_settings();
        assert_eq!(token.stringValue().to_string(), "Synthetic draft token");
        assert!(!token.isHiddenOrHasHiddenAncestor());
        configured.telegram.setting_up = false;
        _published.send_replace(std::sync::Arc::new(configured));
        controller.refresh_telegram_settings();
        assert!(
            !controller
                .ivars()
                .telegram_connected_view
                .get()
                .unwrap()
                .isHiddenOrHasHiddenAncestor(),
            "Cancel must restore connected bot in the same popup"
        );
        let key = |code, text: &str| {
            NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown, NSPoint::new(0.0,0.0), NSEventModifierFlags::empty(), 0.0,
            window.windowNumber(), None, &NSString::from_str(text), &NSString::from_str(text), false, code,
        ).unwrap()
        };
        // Arrow navigation belongs to settings, never the session filter.
        window.makeFirstResponder(Some(&shell.tabs[3]));
        assert!(controller.handle_session_key(&key(123, "\u{f702}")));
        assert_eq!(controller.ivars().settings_selected.get(), 2);
        assert!(controller.handle_session_key(&key(124, "\u{f703}")));
        assert_eq!(controller.ivars().settings_selected.get(), 3);
        configured = (*monitor.snapshot()).clone();
        configured.telegram.setting_up = true;
        _published.send_replace(std::sync::Arc::new(configured));
        controller.refresh_telegram_settings();
        inline_questions::begin_editing(token, None);
        let editor = token.currentEditor().expect("token field editor");
        unsafe {
            editor.selectAll(None);
        }
        assert!(controller.handle_session_key(&key(6, "z")));
        assert_eq!(token.stringValue().to_string(), "z");
        assert!(controller.handle_session_key(&key(48, "\t")));
        assert!(
            token.currentEditor().is_none(),
            "Tab must leave the token editor"
        );
        assert!(controller.handle_session_key(&key(53, "\u{1b}")));
        assert!(
            !controller.ivars().settings_active.get(),
            "Escape returns to sessions"
        );
        controller.ivars().menu_open.set(false);
        assert!(!controller.ivars().scroll.get().unwrap().isHidden());
        assert!(!controller.ivars().footer.get().unwrap().isHidden());
        assert!(shell.scroll.isHidden());
        assert_eq!(
            controller
                .ivars()
                .scroll
                .get()
                .unwrap()
                .contentView()
                .bounds()
                .origin,
            old_scroll
        );
        let original = monitor.snapshot();
        let mut available = (*original).clone();
        available.revision += 1;
        available.versions.available = crate::domain::versions::Release::stable("v99.0.0");
        _published.send_replace(std::sync::Arc::new(available));
        controller.receive_sessions();
        let release = controller.ivars().release.get().unwrap();
        assert!(!release.isHidden());
        assert_eq!(release.title().to_string(), "v99.0.0 available");
        assert!(
            release.image().is_some(),
            "Available update has the filled flame"
        );
        assert!(
            (release.frame().origin.y + release.frame().size.height / 2.0 - 60.5).abs() < 0.01,
            "Single version line centered on title"
        );
        assert_eq!(release.frame().size.height, 14.0);
        let toolbar = controller.ivars().toolbar.get().unwrap();
        let heading = (0..toolbar.subviews().count())
            .map(|index| toolbar.subviews().objectAtIndex(index))
            .find(|view| {
                view.identifier()
                    .is_some_and(|id| id.to_string() == "brand-title")
            })
            .unwrap();
        for width in [680.0, 460.0] {
            controller.change_width(width);
            assert!(
                (release.frame().origin.x
                    - heading.frame().origin.x
                    - heading.frame().size.width
                    - 8.0)
                    .abs()
                    < 0.01,
                "Version stays 8pt after the title at every popup width"
            );
        }

        let overview = controller.ivars().overview.get().unwrap();
        assert!(
            overview.frame().origin.y + overview.frame().size.height <= release.frame().origin.y
        );
        controller
            .ivars()
            .scroll
            .get()
            .unwrap()
            .contentView()
            .scrollToPoint(NSPoint::new(0.0, 0.0));
        export(&bitmap(root), &format!("{name}-release-available"));
        let mut restored = (*original).clone();
        restored.versions.checked_at = None;
        restored.revision += 2;
        _published.send_replace(std::sync::Arc::new(restored));
        controller.receive_sessions();
        assert!(!release.isHidden());
        assert!(!release.isEnabled());
        assert_eq!(
            release.title().to_string(),
            format!("v{}", monitor.snapshot().versions.current)
        );
        assert!(release.image().is_none());
        assert!(
            release
                .toolTip()
                .unwrap()
                .to_string()
                .contains("Not checked yet")
        );
        let mut checked = (*monitor.snapshot()).clone();
        checked.revision += 1;
        checked.versions.checked_at = Some(100);
        _published.send_replace(std::sync::Arc::new(checked));
        controller.receive_sessions();
        assert_eq!(
            release.title().to_string(),
            format!("v{} ✓", monitor.snapshot().versions.current)
        );
        assert!(!release.isEnabled());
        export(&bitmap(root), &format!("{name}-release-current"));
        window.setContentView(None);
    }
    println!(
        "AppKit visual renderer passed in both appearances: mesh scrolling, command disclosure, question forms and quota surfaces"
    );
}
