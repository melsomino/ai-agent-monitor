//! Serialized read/modify/write registration shared by resident and hook senders.
use super::notification_actions::{self, Kind, Plan};
use block2::RcBlock;
use objc2::{msg_send, rc::Retained};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSSet, NSString, ns_string};
use objc2_user_notifications::*;
use std::{
    fs::{self, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    ptr::NonNull,
    sync::Arc,
};

#[derive(Clone)]
pub struct Notice {
    pub key: String,
    pub title: String,
    pub body: String,
    pub target: String,
    pub token: String,
}
pub fn legacy() {
    update(None, None, None);
}
pub fn deliver(plan: Plan, notice: Notice) {
    update(Some(plan), Some(notice), None);
}
pub fn prune() {
    update(None, None, None);
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
fn update(
    plan: Option<Plan>,
    notice: Option<Notice>,
    completion: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    let prune = super::menu::is_resident();
    std::thread::spawn(move || {
        let fallback = notice.clone();
        let failed_completion = completion.clone();
        let register = || -> anyhow::Result<()> {
            let directory =
                crate::app::core::home().join("Library/Application Support/AI Agent Monitor");
            fs::create_dir_all(&directory)?;
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(directory.join("notification-categories.lock"))?;
            let metadata = lock.metadata()?;
            // SAFETY: geteuid has no preconditions; lock belongs to this user.
            anyhow::ensure!(
                metadata.is_file()
                    && metadata.uid() == unsafe { libc::geteuid() }
                    && metadata.mode() & 0o077 == 0,
                "Untrusted notification registration lock"
            );
            // SAFETY: Hold this process-shared lock until the async registration
            // acknowledgement below; never block the UI waiting for another sender.
            anyhow::ensure!(
                unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == 0,
                "Could not lock notification registration"
            );
            let lock = Arc::new(lock);
            let callback = RcBlock::new(move |existing: NonNull<NSSet<UNNotificationCategory>>| {
                let _lock = &lock;
                // SAFETY: The system supplies a valid category set for this callback.
                let existing = unsafe { existing.as_ref() }.allObjects();
                let current = category(plan.as_ref());
                let current_id = current.identifier().to_string();
                let active = notification_actions::categories();
                let mut categories = vec![current];
                for index in 0..existing.count() {
                    let old = existing.objectAtIndex(index);
                    let id = old.identifier().to_string();
                    if id != current_id
                        && (!prune || !id.starts_with("AGENT_DYNAMIC_") || active.contains(&id))
                    {
                        categories.push(old);
                    }
                }
                // Preserve the standard hook action alongside every dynamic set.
                if !categories
                    .iter()
                    .any(|category| category.identifier().to_string() == "CODEX_APPROVAL")
                {
                    categories.push(category(None));
                }
                let center = UNUserNotificationCenter::currentNotificationCenter();
                center.setNotificationCategories(&NSSet::from_slice(
                    &categories
                        .iter()
                        .map(|category| &**category)
                        .collect::<Vec<_>>(),
                ));
                let lock = lock.clone();
                let notice = notice.clone();
                let completion = completion.clone();
                let confirmed =
                    RcBlock::new(move |registered: NonNull<NSSet<UNNotificationCategory>>| {
                        let _lock = &lock;
                        if let Some(notice) = &notice
                            && notification_actions::active(&notice.key)
                        {
                            // SAFETY: The system supplies a valid registration snapshot.
                            let registered = unsafe { registered.as_ref() }.allObjects();
                            let category = (0..registered.count()).any(|index| {
                                registered.objectAtIndex(index).identifier().to_string()
                                    == current_id
                            });
                            post(notice, category.then_some(current_id.as_str()));
                        }
                        if let Some(completion) = &completion {
                            completion();
                        }
                    });
                center.getNotificationCategoriesWithCompletionHandler(&confirmed);
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .getNotificationCategoriesWithCompletionHandler(&callback);
            Ok(())
        };
        if let Err(error) = register() {
            eprintln!("ai-agent-monitor: notification categories: {error:#}");
            if let Some(notice) = fallback
                && notification_actions::active(&notice.key)
            {
                post(&notice, None);
            }
            if let Some(completion) = failed_completion {
                completion();
            }
        }
    });
}
fn post(notice: &Notice, category: Option<&str>) {
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
        if !notification_actions::active(&key) {
            let ids = NSArray::from_slice(&[&*NSString::from_str(&format!("session-{key}"))]);
            let center = UNUserNotificationCenter::currentNotificationCenter();
            center.removePendingNotificationRequestsWithIdentifiers(&ids);
            center.removeDeliveredNotificationsWithIdentifiers(&ids);
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&callback));
}
