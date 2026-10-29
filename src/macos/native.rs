//! macOS notification APIs. All Objective-C glue stays in this module.
use crate::app::cli::{Message, NativeCommand};
use crate::app::core::{self, Action};
use anyhow::{Result, bail};
use block2::{DynBlock, RcBlock};
use dispatch2::DispatchQueue;
use objc2::{
    ClassType, MainThreadMarker, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy,
    NSRunningApplication, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSObject, NSObjectProtocol, NSSet, NSString, NSURL, ns_string,
};
use objc2_user_notifications::*;
use std::{path::Path, ptr::NonNull};

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
            let response_path = info_string(&info, ns_string!("response"));
            let path = Path::new(&response_path);
            // SAFETY: These immutable framework constants are valid NSStrings.
            let (is_default, is_dismiss) = unsafe {
                (
                    action == UNNotificationDefaultActionIdentifier.to_string(),
                    action == UNNotificationDismissActionIdentifier.to_string(),
                )
            };
            if action == "ALLOW_ONCE" && response_path.starts_with("ipc:") {
                let identifier = response.notification().request().identifier().to_string();
                completion.call(());
                std::thread::spawn(move || {
                    let allowed = crate::agents::codex::runtime::allow(&response_path);
                    DispatchQueue::main().exec_async(move || {
                        if allowed {
                            remove_notification(&identifier);
                        } else {
                            open_target(&target);
                        }
                        if !crate::macos::menu::is_resident() {
                            std::process::exit(0);
                        }
                    });
                });
                return;
            }
            let should_open = if action == "ALLOW_ONCE" {
                !core::respond(path, Action::Allow)
            } else if is_default {
                core::respond(path, Action::Open);
                true
            } else {
                if is_dismiss {
                    core::respond(path, Action::Open);
                }
                false
            };
            completion.call(());
            DispatchQueue::main().exec_async(move || {
                if should_open {
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

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("ai-agent-monitor: {message}");
    std::process::exit(2)
}

fn remove_notification(identifier: &str) {
    let identifiers = NSArray::from_slice(&[&*NSString::from_str(identifier)]);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.removePendingNotificationRequestsWithIdentifiers(&identifiers);
    center.removeDeliveredNotificationsWithIdentifiers(&identifiers);
}

pub fn dismiss_approval(response: &Path) {
    let Some(identifier) = crate::agents::pending::notification_id(response) else {
        return;
    };
    remove_notification(&identifier);
    // Match pre-update notifications too: those used random identifiers but
    // already included the exact response path in their property-list userInfo.
    let callback = RcBlock::new(move |notifications: NonNull<NSArray<UNNotification>>| {
        // SAFETY: The notification center supplies a live array for this callback.
        let notifications = unsafe { notifications.as_ref() };
        for index in 0..notifications.count() {
            let request = notifications.objectAtIndex(index).request();
            let stored = info_string(&request.content().userInfo(), ns_string!("response"));
            if crate::agents::pending::notification_id(Path::new(&stored)).as_deref()
                == Some(identifier.as_str())
            {
                remove_notification(&request.identifier().to_string());
            }
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getDeliveredNotificationsWithCompletionHandler(&callback);
}

/// Live requests without a file-backed hook: only addressed Codex requests get Allow.
pub fn notify_session(session: &crate::agents::sessions::Session) {
    let Some(approval) = session
        .approval
        .as_ref()
        .filter(|request| !request.can_allow || request.id.starts_with("ipc:"))
    else {
        return;
    };
    let Some(key) = session.approval_key() else {
        return;
    };
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&session.title));
    content.setSubtitle(&NSString::from_str(session.agent_label()));
    content.setBody(&NSString::from_str(&approval.body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let target = NSString::from_str(&session.open_target().unwrap_or_default());
    let response_token =
        NSString::from_str(if approval.id.starts_with("ipc:") && approval.can_allow {
            &approval.id
        } else {
            ""
        });
    let info = NSDictionary::<NSString, NSString>::from_slices(
        &[ns_string!("target"), ns_string!("response")],
        &[&target, &response_token],
    );
    if approval.id.starts_with("ipc:") && approval.can_allow {
        content.setCategoryIdentifier(ns_string!("CODEX_APPROVAL"));
    }
    // SAFETY: The property-list dictionary contains only retained NSString values.
    unsafe {
        let _: () = msg_send![&*content, setUserInfo: &*info];
    }
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&format!("session-{key}")),
        &content,
        None,
    );
    let callback = RcBlock::new(|error: *mut NSError| {
        // SAFETY: The framework supplies a live NSError or null.
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!("ai-agent-monitor: {}", error.localizedDescription());
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&callback));
}

pub fn remove_session_notification(key: &str) {
    remove_notification(&format!("session-{key}"));
}

fn send(message: &Message, response: Option<&str>) {
    // The panel may answer while notification authorization/delivery is in flight.
    if response.is_some_and(|path| crate::agents::pending::is_resolved(Path::new(path))) {
        std::process::exit(0);
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&message.title));
    content.setSubtitle(&NSString::from_str(&message.subtitle));
    content.setBody(&NSString::from_str(&message.body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let target = NSString::from_str(&message.target);
    let response_value = NSString::from_str(response.unwrap_or(""));
    let info = NSDictionary::<NSString, NSString>::from_slices(
        &[ns_string!("target"), ns_string!("response")],
        &[&target, &response_value],
    );
    // SAFETY: setUserInfo accepts an NSDictionary of property-list values. This
    // dictionary contains only NSString keys and NSString values, retained/copied
    // by UNMutableNotificationContent.
    unsafe {
        let _: () = msg_send![&*content, setUserInfo: &*info];
    }
    if response.is_some() {
        content.setCategoryIdentifier(ns_string!("CODEX_APPROVAL"));
    }
    let identifier = response
        .and_then(|path| crate::agents::pending::notification_id(Path::new(path)))
        .unwrap_or_else(|| {
            format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            )
        });
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&identifier),
        &content,
        None,
    );
    let response_path = response.map(std::path::PathBuf::from);
    let callback = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: Apple guarantees a valid NSError pointer (or null) for this callback.
        if let Some(error) = unsafe { error.as_ref() } {
            fail(error.localizedDescription());
        }
        if response_path
            .as_deref()
            .is_some_and(crate::agents::pending::is_resolved)
        {
            remove_notification(&identifier);
        }
        std::process::exit(0);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&callback));
}

pub fn run(command: NativeCommand) -> Result<()> {
    let mtm = MainThreadMarker::new().expect("native UI must start on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
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
    let action = UNNotificationAction::actionWithIdentifier_title_options(
        ns_string!("ALLOW_ONCE"),
        ns_string!("Allow"),
        UNNotificationActionOptions::AuthenticationRequired,
    );
    let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        ns_string!("CODEX_APPROVAL"),
        &NSArray::from_slice(&[&*action]),
        &NSArray::new(),
        UNNotificationCategoryOptions::CustomDismissAction,
    );
    center.setNotificationCategories(&NSSet::from_slice(&[&*category]));
    match command {
        NativeCommand::NotificationSettings => {
            let callback = RcBlock::new(|settings: NonNull<UNNotificationSettings>| {
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
                println!(
                    "{}",
                    serde_json::json!({
                        "authorization": authorization,
                        "alertStyle": style,
                        "alerts": enabled(settings.alertSetting()),
                        "notificationCenter": enabled(settings.notificationCenterSetting()),
                        "sound": enabled(settings.soundSetting()),
                        "timeSensitive": enabled(settings.timeSensitiveSetting()),
                        "scheduledDelivery": enabled(settings.scheduledDeliverySetting()),
                    })
                );
                std::process::exit(0);
            });
            center.getNotificationSettingsWithCompletionHandler(&callback);
        }
        NativeCommand::Setup => {
            let callback = RcBlock::new(|allowed: Bool, error: *mut NSError| {
                // SAFETY: The pointer is valid (or null) for the lifetime of the callback.
                if let Some(error) = unsafe { error.as_ref() } {
                    fail(error.localizedDescription());
                }
                if !allowed.as_bool() {
                    fail("Notifications are disabled for AI Agent Monitor");
                }
                send(
                    &Message {
                        title: "Notifications are ready".into(),
                        subtitle: String::new(),
                        body: "You will be notified when an agent needs approval.".into(),
                        target: String::new(),
                    },
                    None,
                );
            });
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &callback,
            );
        }
        NativeCommand::Send { message, response } => authorize_and_send(&center, message, response),
        NativeCommand::OpenTarget(target) => {
            if !open_target(&target) {
                bail!("VS Code could not open this target");
            }
            return Ok(());
        }
        NativeCommand::Listen | NativeCommand::Sessions => {}
    }
    app.run();
    // Keep the weak delegate alive for the entire run loop.
    drop(delegate);
    Ok(())
}

fn authorize_and_send(
    center: &UNUserNotificationCenter,
    message: Message,
    response: Option<String>,
) {
    let callback = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: The framework passes a valid non-null settings object.
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        if status != UNAuthorizationStatus::Authorized
            && status != UNAuthorizationStatus::Provisional
        {
            fail("Run --setup to allow notifications");
        }
        send(&message, response.as_deref());
    });
    center.getNotificationSettingsWithCompletionHandler(&callback);
}
