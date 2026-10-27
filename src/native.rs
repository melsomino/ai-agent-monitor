//! macOS notification APIs. All Objective-C glue stays in this module.
use crate::cli::{Message, NativeCommand};
use crate::core::{self, Action};
use anyhow::{Result, bail};
use block2::{DynBlock, RcBlock};
use dispatch2::DispatchQueue;
use objc2::{
    ClassType, MainThreadMarker, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSWorkspace};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSObject, NSObjectProtocol, NSSet, NSString, NSURL, ns_string,
};
use objc2_user_notifications::*;
use std::{path::Path, ptr::NonNull, time::Duration};

pub fn open_target(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    if url.scheme() != "vscode"
        || url.host_str() != Some("file")
        || !url.path().starts_with('/')
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
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
    #[name = "CodexNotifyRustDelegate"]
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
                std::process::exit(0);
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
    eprintln!("codex-notify: {message}");
    std::process::exit(2)
}

fn send(message: &Message, response: Option<&str>) {
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
    let identifier = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&identifier),
        &content,
        None,
    );
    let callback = RcBlock::new(|error: *mut NSError| {
        // SAFETY: Apple guarantees a valid NSError pointer (or null) for this callback.
        if let Some(error) = unsafe { error.as_ref() } {
            fail(error.localizedDescription());
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
        NativeCommand::Setup => {
            let callback = RcBlock::new(|allowed: Bool, error: *mut NSError| {
                // SAFETY: The pointer is valid (or null) for the lifetime of the callback.
                if let Some(error) = unsafe { error.as_ref() } {
                    fail(error.localizedDescription());
                }
                if !allowed.as_bool() {
                    fail("Notifications are disabled for Codex Notify");
                }
                send(
                    &Message {
                        title: "Уведомления готовы".into(),
                        subtitle: String::new(),
                        body: "Сообщу, когда агенту понадобится разрешение.".into(),
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
        NativeCommand::Listen => {
            // macOS relaunches the app without arguments when a notification is
            // clicked, even though the original notification sender has exited.
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(15));
                std::process::exit(0);
            });
        }
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
