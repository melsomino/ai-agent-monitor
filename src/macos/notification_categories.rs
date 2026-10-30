//! Serialized read/modify/write registration shared by resident and hook senders.
use super::notification_actions::{Kind, Plan};
use block2::RcBlock;
use objc2::{msg_send, rc::Retained};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSSet, NSString, ns_string};
use objc2_user_notifications::*;
use std::ptr::NonNull;

#[derive(Clone)]
pub struct Notice {
    pub key: String,
    pub title: String,
    pub body: String,
    pub target: String,
    pub token: String,
}
fn category(plan: Option<&Plan>) -> Retained<UNNotificationCategory> {
    let mut actions: Vec<Retained<UNNotificationAction>> = Vec::new();
    if let Some(plan) = plan {
        for (index, button) in plan.buttons.iter().enumerate() {
            let id = NSString::from_str(&format!("AGENT_REPLY_{index}"));
            let title = NSString::from_str(&button.title);
            let options = UNNotificationActionOptions::AuthenticationRequired
                | if matches!(button.kind, Kind::Form | Kind::More | Kind::Editor) {
                    UNNotificationActionOptions::Foreground
                } else {
                    UNNotificationActionOptions::empty()
                };
            if button.kind == Kind::Text {
                actions.push(UNTextInputNotificationAction::actionWithIdentifier_title_options_textInputButtonTitle_textInputPlaceholder(
                    &id, &title, options, ns_string!("Send"), ns_string!("Your answer")
                ).into_super());
            } else {
                actions.push(UNNotificationAction::actionWithIdentifier_title_options(
                    &id, &title, options,
                ));
            }
        }
    } else {
        actions.push(UNNotificationAction::actionWithIdentifier_title_options(
            ns_string!("ALLOW_ONCE"),
            ns_string!("Allow"),
            UNNotificationActionOptions::AuthenticationRequired,
        ));
    }
    UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &NSString::from_str(
            &plan
                .map(Plan::category)
                .unwrap_or_else(|| "CODEX_APPROVAL".into()),
        ),
        &NSArray::from_slice(&actions.iter().map(|action| &**action).collect::<Vec<_>>()),
        &NSArray::new(),
        UNNotificationCategoryOptions::CustomDismissAction,
    )
}
pub fn deliver(
    plan: Plan,
    notice: Notice,
    plans: Vec<Plan>,
    receipts: tokio::sync::mpsc::Sender<String>,
) {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let categories: Vec<_> = plans.iter().map(|p| category(Some(p))).collect();
    center.setNotificationCategories(&NSSet::from_slice(
        &categories.iter().map(|p| &**p).collect::<Vec<_>>(),
    ));
    let callback = RcBlock::new(move |registered: NonNull<NSSet<UNNotificationCategory>>| {
        if receipts.is_closed() {
            return;
        }
        let registered = unsafe { registered.as_ref() }.allObjects();
        let name = plan.category();
        let found = (0..registered.count())
            .any(|i| registered.objectAtIndex(i).identifier().to_string() == name);
        post(&notice, found.then_some(name.as_str()), receipts.clone());
    });
    center.getNotificationCategoriesWithCompletionHandler(&callback);
}
fn post(notice: &Notice, category: Option<&str>, receipts: tokio::sync::mpsc::Sender<String>) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&notice.title));
    content.setBody(&NSString::from_str(&notice.body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    if let Some(category) = category {
        content.setCategoryIdentifier(&NSString::from_str(category));
    }
    let target = NSString::from_str(&notice.target);
    let token = NSString::from_str(&notice.token);
    let key = NSString::from_str(&notice.key);
    let info = NSDictionary::<NSString, NSString>::from_slices(
        &[
            ns_string!("target"),
            ns_string!("response"),
            ns_string!("requestKey"),
        ],
        &[&target, &token, &key],
    );
    // SAFETY: Property-list dictionary containing only retained strings.
    unsafe {
        let _: () = msg_send![&*content, setUserInfo: &*info];
    }
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&format!("session-{}", notice.key)),
        &content,
        None,
    );
    let key = notice.key.clone();
    let callback = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: Valid NSError or null supplied by UserNotifications.
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!("ai-agent-monitor: {}", error.localizedDescription());
        }
        if receipts.try_send(key.clone()).is_err() {
            super::native::remove_session_notification(&key);
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&callback));
}
