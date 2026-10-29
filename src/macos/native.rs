//! macOS notification APIs. All Objective-C glue stays in this module.
use crate::app::cli::NativeCommand;
use crate::app::core;
use anyhow::{Context, Result, bail};
use block2::{DynBlock, RcBlock};
use dispatch2::DispatchQueue;
use objc2::{
    ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSEvent,
    NSEventMask, NSEventType, NSRunningApplication, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSDate, NSDictionary, NSError, NSObject, NSObjectProtocol, NSRunLoopMode, NSString,
    NSURL, ns_string,
};
use objc2_user_notifications::*;
use std::ptr::NonNull;

define_class!(
    // SAFETY: NSApplication permits event-loop subclassing. All AppKit events
    // and the sessions controller are confined to the main thread.
    #[unsafe(super(NSApplication))]
    #[thread_kind = MainThreadOnly]
    struct MonitorApplication;
    unsafe impl NSObjectProtocol for MonitorApplication {}
    impl MonitorApplication {
        #[unsafe(method_id(nextEventMatchingMask:untilDate:inMode:dequeue:))]
        fn next_event(&self, mask: NSEventMask, expiration: Option<&NSDate>, mode: &NSRunLoopMode, dequeue: bool) -> Option<Retained<NSEvent>> {
            loop {
                // SAFETY: Preserve AppKit's requested mask, deadline, mode and queue semantics.
                let event: Option<Retained<NSEvent>> = unsafe {
                    msg_send![super(self), nextEventMatchingMask: mask, untilDate: expiration, inMode: mode, dequeue: dequeue]
                };
                let Some(event) = event else { break None; };
                if dequeue && event.r#type() == NSEventType::KeyDown
                    && crate::macos::menu::handle_menu_key(&event)
                {
                    continue;
                }
                break Some(event);
            }
        }
    }
);

pub fn open_target(value: &str) -> bool {
    let Some(path) = core::vscode_path(value) else {
        return false;
    };
    if !path.exists() {
        // A loaded chat can outlive its deleted worktree. Focus VS Code without
        // asking it to open a path that would produce a "Path does not exist" dialog.
        let applications = NSRunningApplication::runningApplicationsWithBundleIdentifier(
            ns_string!("com.microsoft.VSCode"),
        );
        for index in 0..applications.count() {
            if applications
                .objectAtIndex(index)
                .activateWithOptions(NSApplicationActivationOptions::empty())
            {
                return true;
            }
        }
        return NSWorkspace::sharedWorkspace()
            .URLForApplicationWithBundleIdentifier(ns_string!("com.microsoft.VSCode"))
            .is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url));
    }
    let Some(url) = NSURL::URLWithString(&NSString::from_str(value)) else {
        return false;
    };
    NSWorkspace::sharedWorkspace().openURL(&url)
}

fn info_string(info: &NSDictionary, key: &NSString) -> String {
    info.objectForKey(key)
        .and_then(|value| value.downcast::<NSString>().ok())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

define_class!(
    // SAFETY: NSObject has no additional subclassing requirements. This class
    // has no ivars and no main-thread-bound state; callbacks dispatch UI work.
    #[unsafe(super(NSObject))]
    #[name = "AIAgentMonitorRustDelegate"]
    struct NotificationDelegate;

    // SAFETY: NSObjectProtocol has no extra requirements.
    unsafe impl NSObjectProtocol for NotificationDelegate {}

    // SAFETY: Signatures match UNUserNotificationCenterDelegate. The completion
    // handlers are called exactly once; any AppKit UI work runs on the main queue.
    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn received(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            let action = response.actionIdentifier().to_string();
            let info = response.notification().request().content().userInfo();
            let target = info_string(&info, ns_string!("target"));
            // SAFETY: These immutable framework constants are valid NSStrings.
            let is_default = unsafe { action == UNNotificationDefaultActionIdentifier.to_string() };
            if let Some(index) = action
                .strip_prefix("AGENT_REPLY_")
                .and_then(|index| index.parse::<usize>().ok())
            {
                let key = info_string(&info, ns_string!("requestKey"));
                let category = response
                    .notification()
                    .request()
                    .content()
                    .categoryIdentifier()
                    .to_string();
                let binding = crate::macos::notification_actions::lookup(&key, &category);
                let text = response
                    .downcast_ref::<UNTextInputNotificationResponse>()
                    .map(|response| response.userText().to_string());
                let identifier = response.notification().request().identifier().to_string();
                completion.call(());
                if let Some(binding) = binding {
                    use crate::macos::notification_actions::Kind;
                    let button = binding.plan.buttons.get(index).cloned();
                    let reply = binding.plan.response(index, text.as_deref());
                    if let Some(reply) = reply {
                        std::thread::spawn(move || {
                            let sent = binding.session.respond(&reply);
                            DispatchQueue::main().exec_async(move || {
                                if sent {
                                    remove_session_notification(&key);
                                } else {
                                    let draft = match reply {
                                        crate::agents::interaction::Response::Answers(answers) => {
                                            Some(answers)
                                        }
                                        _ => None,
                                    };
                                    let form = !binding.session.interaction().questions.is_empty();
                                    crate::macos::menu::show_notification_request(
                                        binding.session,
                                        form,
                                        draft,
                                    );
                                }
                                crate::macos::menu::refresh_after_response();
                                if !crate::macos::menu::is_resident() {
                                    std::process::exit(0);
                                }
                            });
                        });
                    } else {
                        DispatchQueue::main().exec_async(move || {
                            match button.map(|button| button.kind) {
                                Some(Kind::Form) => crate::macos::menu::show_notification_request(
                                    binding.session,
                                    true,
                                    None,
                                ),
                                Some(Kind::More) => {
                                    let form = !binding.session.interaction().questions.is_empty();
                                    crate::macos::menu::show_notification_request(
                                        binding.session,
                                        form,
                                        None,
                                    );
                                }
                                Some(Kind::Text) => crate::macos::menu::show_notification_request(
                                    binding.session,
                                    true,
                                    None,
                                ),
                                _ => {
                                    open_target(&target);
                                }
                            }
                        });
                    }
                } else {
                    DispatchQueue::main().exec_async(move || {
                        remove_notification(&identifier);
                        open_target(&target);
                        if !crate::macos::menu::is_resident() {
                            std::process::exit(0);
                        }
                    });
                }
                return;
            }
            // Old notifications may outlive an upgrade. They cannot bypass the
            // monitor's current request store; take the user to the agent instead.
            let hook_key = info_string(&info, ns_string!("requestKey"));
            completion.call(());
            DispatchQueue::main().exec_async(move || {
                if is_default {
                    crate::app::monitor::defer_hook(&hook_key);
                }
                if is_default || action == "ALLOW_ONCE" {
                    open_target(&target);
                }
                if !crate::macos::menu::is_resident() {
                    std::process::exit(0);
                }
            });
        }

        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::Sound,));
        }
    }
);

fn remove_notification(identifier: &str) {
    let identifiers = NSArray::from_slice(&[&*NSString::from_str(identifier)]);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.removePendingNotificationRequestsWithIdentifiers(&identifiers);
    center.removeDeliveredNotificationsWithIdentifiers(&identifiers);
}

/// A restarted monitor cannot reuse the old process's opaque request bindings.
fn remove_stale_notifications() {
    let callback = RcBlock::new(|notifications: NonNull<NSArray<UNNotification>>| {
        // SAFETY: The framework supplies a live array for this callback.
        let notifications = unsafe { notifications.as_ref() };
        for index in 0..notifications.count() {
            let request = notifications.objectAtIndex(index).request();
            let content = request.content();
            let category = content.categoryIdentifier().to_string();
            let key = info_string(&content.userInfo(), ns_string!("requestKey"));
            if category == "CODEX_APPROVAL"
                || (category.starts_with("AGENT_DYNAMIC_")
                    && crate::macos::notification_actions::lookup(&key, &category).is_none())
            {
                remove_notification(&request.identifier().to_string());
            }
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getDeliveredNotificationsWithCompletionHandler(&callback);
}

/// Live requests get a request-specific action set, registered before delivery.
pub fn notify_session(session: &crate::agents::sessions::Session) {
    if !crate::macos::menu::is_resident() {
        return;
    }
    if !crate::app::monitor::hook_pending(session) {
        return;
    }
    let Some(approval) = session.approval.as_ref() else {
        return;
    };
    let Some(key) = session.approval_key() else {
        return;
    };
    if crate::macos::notification_actions::active(&key) {
        return;
    }
    let Some(plan) = crate::macos::notification_actions::bind(session) else {
        return;
    };
    let mut body = approval.body.clone();
    let mut shown = std::collections::HashSet::new();
    for action in session.interaction().actions {
        if !action.detail.is_empty()
            && !body.contains(&action.detail)
            && shown.insert(action.detail.clone())
        {
            body.push_str(&format!("\n\n{}\n{}", action.label, action.detail));
        }
    }
    crate::macos::notification_categories::deliver(
        plan,
        crate::macos::notification_categories::Notice {
            key,
            title: session.title.clone(),
            subtitle: session.agent_label().into(),
            body,
            target: session.open_target().unwrap_or_default(),
            token: approval.id.clone(),
        },
    );
}

pub fn remove_session_notification(key: &str) {
    let bound = crate::macos::notification_actions::remove(key);
    remove_notification(&format!("session-{key}"));
    if bound && crate::macos::menu::is_resident() {
        crate::macos::notification_categories::prune();
    }
}

/// Only the resident calls this; passive messages have no approval side channel.
pub fn notify_passive(message: &core::Notification) {
    if !crate::macos::menu::is_resident() {
        return;
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&message.title));
    content.setBody(&NSString::from_str(&message.body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let info = NSDictionary::<NSString, NSString>::from_slices(
        &[ns_string!("target")],
        &[&NSString::from_str(&message.target)],
    );
    // SAFETY: Strings are copied into the property-list notification userInfo.
    unsafe {
        let _: () = msg_send![&*content, setUserInfo: &*info];
    }
    let identifier = format!(
        "notice-{}-{}",
        std::process::id(),
        crate::agents::tracking::now()
    );
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&identifier),
        &content,
        None,
    );
    let callback = RcBlock::new(|error: *mut NSError| {
        // SAFETY: Valid NSError or null for the lifetime of this callback.
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!("ai-agent-monitor: {}", error.localizedDescription());
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&callback));
}

pub fn run(command: NativeCommand) -> Result<()> {
    let mtm = MainThreadMarker::new().expect("native UI must start on main thread");
    // SAFETY: Create the application singleton as our public NSApplication subclass
    // before any other code requests it. No change to key-window or activation behavior.
    let app: Retained<NSApplication> =
        unsafe { msg_send![MonitorApplication::class(), sharedApplication] };
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    if let NativeCommand::OpenTarget(target) = &command {
        if !open_target(target) {
            bail!("VS Code could not open this target");
        }
        return Ok(());
    }
    if matches!(command, NativeCommand::Listen | NativeCommand::Sessions)
        && !crate::macos::onboarding::installed_location(mtm)
    {
        return Ok(());
    }
    if matches!(command, NativeCommand::Listen | NativeCommand::Sessions)
        && !crate::macos::menu::start(mtm, matches!(command, NativeCommand::Sessions))?
    {
        return Ok(());
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    // SAFETY: This NSObject subclass has no custom initialization or ivars.
    let delegate: Retained<NotificationDelegate> =
        unsafe { msg_send![NotificationDelegate::class(), new] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    if crate::macos::menu::is_resident() {
        crate::macos::notification_categories::legacy();
        remove_stale_notifications();
    }
    crate::macos::onboarding::begin(mtm, false);
    app.run();
    // Keep the weak delegate alive for the entire run loop.
    drop(delegate);
    Ok(())
}
/// Queried inside the resident so diagnostics cannot register a competing notification client.
pub fn notification_settings() -> Result<serde_json::Value> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let callback = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: The framework supplies a valid settings object for this callback.
        let settings = unsafe { settings.as_ref() };
        let enabled = |value: UNNotificationSetting| match value {
            UNNotificationSetting::Enabled => "enabled",
            UNNotificationSetting::Disabled => "disabled",
            _ => "unsupported",
        };
        let authorization = match settings.authorizationStatus() {
            UNAuthorizationStatus::Authorized => "authorized",
            UNAuthorizationStatus::Provisional => "provisional",
            UNAuthorizationStatus::Denied => "denied",
            UNAuthorizationStatus::NotDetermined => "notDetermined",
            _ => "unknown",
        };
        let style = match settings.alertStyle() {
            UNAlertStyle::Banner => "banner",
            UNAlertStyle::Alert => "alert",
            _ => "none",
        };
        let summary = serde_json::json!({
            "authorization": authorization,
            "alertStyle": style,
            "alerts": enabled(settings.alertSetting()),
            "notificationCenter": enabled(settings.notificationCenterSetting()),
            "sound": enabled(settings.soundSetting()),
            "timeSensitive": enabled(settings.timeSensitiveSetting()),
            "scheduledDelivery": enabled(settings.scheduledDeliverySetting()),
        });
        let sender = sender.clone();
        let categories = RcBlock::new(
            move |categories: NonNull<objc2_foundation::NSSet<UNNotificationCategory>>| {
                // SAFETY: UserNotifications supplies a live set for this callback.
                let categories = unsafe { categories.as_ref() }.allObjects();
                let mut summary = summary.clone();
                summary["categories"] = serde_json::json!((0..categories.count()).map(|index| {
                        let category = categories.objectAtIndex(index);
                        let actions = category.actions();
                        serde_json::json!({"id":category.identifier().to_string(),"actions":(0..actions.count()).map(|index| {
                            let action = actions.objectAtIndex(index);
                            serde_json::json!({"id":action.identifier().to_string(),"title":action.title().to_string(),"textInput":action.downcast_ref::<UNTextInputNotificationAction>().is_some()})
                        }).collect::<Vec<_>>()})
                    }).collect::<Vec<_>>());
                let sender = sender.clone();
                let delivered = RcBlock::new(
                    move |notifications: NonNull<NSArray<UNNotification>>| {
                        // SAFETY: The center supplies a live array during this callback.
                        let notifications = unsafe { notifications.as_ref() };
                        let mut summary = summary.clone();
                        summary["delivered"] = serde_json::json!((0..notifications.count()).map(|index| {
                                let request = notifications.objectAtIndex(index).request();
                                let content = request.content();
                                let info = content.userInfo();
                                serde_json::json!({"id": request.identifier().to_string(), "category": content.categoryIdentifier().to_string(), "requestKey": info_string(&info, ns_string!("requestKey")), "response": info_string(&info, ns_string!("response"))})
                            }).collect::<Vec<_>>());
                        let _ = sender.send(summary);
                    },
                );
                UNUserNotificationCenter::currentNotificationCenter()
                    .getDeliveredNotificationsWithCompletionHandler(&delivered);
            },
        );
        UNUserNotificationCenter::currentNotificationCenter()
            .getNotificationCategoriesWithCompletionHandler(&categories);
    });
    center.getNotificationSettingsWithCompletionHandler(&callback);

    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .context("Notification settings timed out")
}

pub fn setup_notifications() -> Result<bool> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let callback = RcBlock::new(move |allowed: Bool, error: *mut NSError| {
        // SAFETY: NSError is valid or null for the duration of this callback.
        let result = match unsafe { error.as_ref() } {
            Some(error) => Err(error.localizedDescription().to_string()),
            None => Ok(allowed.as_bool()),
        };
        let _ = sender.send(result);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &callback,
        );
    receiver
        .recv_timeout(std::time::Duration::from_secs(60))
        .context("Notification permission timed out")?
        .map_err(anyhow::Error::msg)
}
