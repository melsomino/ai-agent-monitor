//! macOS notification APIs. All Objective-C glue stays in this module.
use crate::app::cli::NativeCommand;
use crate::app::core;
use anyhow::{Context, Result, bail};
use block2::{DynBlock, RcBlock};
use objc2::{
    AllocAnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class,
    msg_send,
    rc::{Allocated, Retained},
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

#[derive(Default)]
struct ApplicationState {
    controller: std::cell::OnceCell<Retained<crate::macos::menu::MenuController>>,
}
define_class!(
    // SAFETY: NSApplication permits event-loop subclassing. All AppKit events
    // and the sessions controller are confined to the main thread.
    #[unsafe(super(NSApplication))]
    #[thread_kind = MainThreadOnly]
    #[ivars=ApplicationState]
    struct MonitorApplication;
    unsafe impl NSObjectProtocol for MonitorApplication {}
    impl MonitorApplication {
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(ApplicationState::default());
            // SAFETY: Initialize our state before AppKit creates the shared application.
            unsafe { msg_send![super(this), init] }
        }
        #[unsafe(method_id(nextEventMatchingMask:untilDate:inMode:dequeue:))]
        fn next_event(&self, mask: NSEventMask, expiration: Option<&NSDate>, mode: &NSRunLoopMode, dequeue: bool) -> Option<Retained<NSEvent>> {
            loop {
                // SAFETY: Preserve AppKit's requested mask, deadline, mode and queue semantics.
                let event: Option<Retained<NSEvent>> = unsafe {
                    msg_send![super(self), nextEventMatchingMask: mask, untilDate: expiration, inMode: mode, dequeue: dequeue]
                };
                let Some(event) = event else { break None; };
                if dequeue && event.r#type() == NSEventType::KeyDown
                    && self.ivars().controller.get().is_some_and(|c|c.handle_session_key(&event))
                {
                    continue;
                }
                break Some(event);
            }
        }
    }
);

pub fn open_target(value: &str) -> bool {
    if let Some(result) = super::terminal::open(value) {
        return result;
    }
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

#[derive(Clone)]
pub struct NotificationInput {
    pub action: String,
    pub key: String,
    pub category: String,
    pub target: String,
    pub text: Option<String>,
}
struct NotificationDelegateState {
    input: tokio::sync::mpsc::Sender<NotificationInput>,
    monitor: crate::actors::monitor::Handle,
    ui: crate::macos::bridge::Sender,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[ivars=NotificationDelegateState]
    struct NotificationDelegate;
    unsafe impl NSObjectProtocol for NotificationDelegate {}
    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn received(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            let content = response.notification().request().content();
            let info = content.userInfo();
            let input = NotificationInput {
                action: response.actionIdentifier().to_string(),
                key: info_string(&info, ns_string!("requestKey")),
                category: content.categoryIdentifier().to_string(),
                target: info_string(&info, ns_string!("target")),
                text: response
                    .downcast_ref::<UNTextInputNotificationResponse>()
                    .map(|r| r.userText().to_string()),
            };
            if let Err(error) = self.ivars().input.try_send(input) {
                let input = error.into_inner();
                if let Some(session) = self
                    .ivars()
                    .monitor
                    .snapshot()
                    .sessions
                    .iter()
                    .find(|s| s.approval_key().as_deref() == Some(input.key.as_str()))
                    .cloned()
                {
                    let form = !session.interaction().questions.is_empty();
                    let _ = self
                        .ivars()
                        .ui
                        .try_send(crate::macos::bridge::Event::ShowRequest {
                            session: Box::new(session),
                            form,
                            draft: None,
                        });
                }
            }
            completion.call(());
        }
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
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
            if category == "CODEX_APPROVAL"
                || (category.starts_with("AGENT_DYNAMIC_")
                    && !category.starts_with(&format!("AGENT_DYNAMIC_{}_", std::process::id())))
            {
                remove_notification(&request.identifier().to_string());
            }
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getDeliveredNotificationsWithCompletionHandler(&callback);
}

pub fn remove_session_notification(key: &str) {
    remove_notification(&format!("session-{key}"));
}

/// Only the resident calls this; passive messages have no approval side channel.
pub fn notify_passive(message: &core::Notification) {
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
    let app: Retained<MonitorApplication> =
        unsafe { msg_send![MonitorApplication::class(), sharedApplication] };
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    crate::macos::settings::migrate_legacy_preferences();
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
    let Some(resident) =
        crate::macos::menu::start(mtm, matches!(command, NativeCommand::Sessions))?
    else {
        return Ok(());
    };
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let allocated = NotificationDelegate::alloc().set_ivars(NotificationDelegateState {
        input: resident.notifications.clone(),
        monitor: resident.monitor.clone(),
        ui: resident.ui.clone(),
    });
    let delegate: Retained<NotificationDelegate> = unsafe { msg_send![super(allocated), init] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    remove_stale_notifications();
    app.ivars().controller.set(resident.controller.clone()).ok();
    app.run();
    // Keep the weak delegate alive for the entire run loop.
    drop(delegate);
    drop(resident);
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

/// Stop the native loop even when shutdown arrived from a timer rather than an input event.
pub fn stop_application(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    app.stop(None);
    if let Some(event)=NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::ApplicationDefined,objc2_foundation::NSPoint::new(0.0,0.0),objc2_app_kit::NSEventModifierFlags::empty(),0.0,0,None,0,0,0){app.postEvent_atStart(&event,false);}
}
