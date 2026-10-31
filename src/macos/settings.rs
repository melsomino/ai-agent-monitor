//! Persistent preferences and macOS login-item registration.
#[cfg(test)]
use crate::domain::session::Client;
use anyhow::{Context, Result, bail};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool},
};
use objc2_foundation::{NSBundle, NSError, NSString, NSUserDefaults, ns_string};

pub use crate::domain::preferences::{Applications, LoginStatus, Preferences};

/// Keep user preferences when moving away from the original notification identity.
/// OS notification authorization is intentionally requested again by onboarding.
pub fn migrate_legacy_preferences() {
    if NSBundle::mainBundle().bundleIdentifier().as_deref()
        != Some(ns_string!("com.melsomino.ai-agent-monitor"))
    {
        return;
    }
    let first_identity_launch =
        !NSUserDefaults::standardUserDefaults().boolForKey(ns_string!("LegacyPreferencesMigrated"));
    migrate_preferences(
        &NSUserDefaults::standardUserDefaults(),
        ns_string!("local.codex.agent-monitor.notify"),
        ns_string!("com.melsomino.ai-agent-monitor"),
    );
    let defaults = NSUserDefaults::standardUserDefaults();
    if first_identity_launch
        && defaults.boolForKey(ns_string!("LaunchAtLoginEnabled"))
        && !matches!(
            login_status(),
            LoginStatus::Enabled | LoginStatus::RequiresApproval
        )
        && let Err(error) = set_launch_at_login(true)
    {
        eprintln!("ai-agent-monitor: restore launch at login: {error:#}");
    }
}

fn migrate_preferences(defaults: &NSUserDefaults, legacy_name: &NSString, current_name: &NSString) {
    let marker = ns_string!("LegacyPreferencesMigrated");
    if defaults.boolForKey(marker) {
        return;
    }
    let current = defaults.persistentDomainForName(current_name);
    if let Some(legacy) = defaults.persistentDomainForName(legacy_name) {
        for key in [
            ns_string!("KnownApplications"),
            ns_string!("AnimateMenuBar"),
            ns_string!("MenuShortcut"),
            ns_string!("LaunchAtLoginEnabled"),
        ] {
            if current
                .as_ref()
                .is_some_and(|domain| domain.objectForKey(key).is_some())
            {
                continue;
            }
            if let Some(value) = legacy.objectForKey(key) {
                // SAFETY: The original persistent domain supplies valid property-list values.
                unsafe {
                    defaults.setObject_forKey(Some(&value), key);
                }
            }
        }
    }
    // First-launch and notification results are intentionally not inherited:
    // onboarding reconnects existing hooks idempotently and requests permission.
    defaults.setBool_forKey(true, marker);
}

pub fn applications() -> Applications {
    NSUserDefaults::standardUserDefaults()
        .stringForKey(ns_string!("KnownApplications"))
        .map(|value| Applications::decode(&value.to_string()))
        .unwrap_or_default()
}
pub fn save_applications(applications: &Applications) {
    let value = objc2_foundation::NSString::from_str(
        &serde_json::to_string(applications).expect("serializable application preferences"),
    );
    // SAFETY: A copied NSString is a valid NSUserDefaults property-list value.
    unsafe {
        NSUserDefaults::standardUserDefaults()
            .setObject_forKey(Some(&value), ns_string!("KnownApplications"));
    }
}

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

fn login_service() -> Result<Retained<AnyObject>> {
    let class = AnyClass::get(c"SMAppService").context("macOS login items are unavailable")?;
    // SAFETY: SMAppService.mainAppService returns an SMAppService on macOS 13+.
    Ok(unsafe { msg_send![class, mainAppService] })
}

pub fn login_status() -> LoginStatus {
    let Ok(service) = login_service() else {
        return LoginStatus::Unavailable;
    };
    // SAFETY: SMAppService.status is an NSInteger, with values from the system header.
    match unsafe { msg_send![&*service, status] } {
        0_isize => LoginStatus::Disabled,
        1 => LoginStatus::Enabled,
        2 => LoginStatus::RequiresApproval,
        _ => LoginStatus::Unavailable,
    }
}

pub fn set_launch_at_login(enabled: bool) -> Result<LoginStatus> {
    let status = login_status();
    if (enabled && matches!(status, LoginStatus::Enabled | LoginStatus::RequiresApproval))
        || (!enabled && status == LoginStatus::Disabled)
    {
        NSUserDefaults::standardUserDefaults()
            .setBool_forKey(enabled, ns_string!("LaunchAtLoginEnabled"));
        return Ok(status);
    }
    let service = login_service()?;
    let mut error: *mut NSError = std::ptr::null_mut();
    // SAFETY: The synchronous SMAppService methods return BOOL and optionally
    // an autoreleased NSError valid for this call's autorelease pool.
    let success: Bool = unsafe {
        if enabled {
            msg_send![&*service, registerAndReturnError: &mut error]
        } else {
            msg_send![&*service, unregisterAndReturnError: &mut error]
        }
    };
    let status = login_status();
    if !success.as_bool() && !(enabled && status == LoginStatus::RequiresApproval) {
        // SAFETY: A failed SMAppService operation supplies a live NSError or null.
        if let Some(error) = unsafe { error.as_ref() } {
            bail!("{}", error.localizedDescription());
        }
        bail!("Could not change launch at login");
    }
    NSUserDefaults::standardUserDefaults()
        .setBool_forKey(enabled, ns_string!("LaunchAtLoginEnabled"));
    Ok(status)
}

pub fn open_login_settings() {
    if let Some(class) = AnyClass::get(c"SMAppService") {
        // SAFETY: This class method opens the system Login Items settings pane.
        unsafe {
            let _: () = msg_send![class, openSystemSettingsLoginItems];
        }
    }
}

pub fn menu_animation() -> bool {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = ns_string!("AnimateMenuBar");
    // Preserve the existing animation until the user explicitly changes it.
    defaults.objectForKey(key).is_none() || defaults.boolForKey(key)
}

pub fn set_menu_animation(enabled: bool) {
    NSUserDefaults::standardUserDefaults().setBool_forKey(enabled, ns_string!("AnimateMenuBar"));
}

pub fn menu_shortcut() -> Option<crate::macos::hotkey::Shortcut> {
    let value = NSUserDefaults::standardUserDefaults().stringForKey(ns_string!("MenuShortcut"))?;
    crate::macos::hotkey::Shortcut::decode(&value.to_string())
}

pub fn set_menu_shortcut(shortcut: Option<&crate::macos::hotkey::Shortcut>) {
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = ns_string!("MenuShortcut");
    if let Some(shortcut) = shortcut {
        let value = objc2_foundation::NSString::from_str(&shortcut.encode());
        // SAFETY: NSString is a valid property-list value copied by NSUserDefaults.
        unsafe {
            defaults.setObject_forKey(Some(&value), key);
        }
    } else {
        defaults.removeObjectForKey(key);
    }
}

pub fn snapshot() -> Preferences {
    Preferences {
        applications: applications(),
        login: login_status(),
        animation: menu_animation(),
        shortcut: menu_shortcut(),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_only_observed_apps_and_keeps_hidden_apps_after_their_sessions_close() {
        let mut apps = Applications::default();
        assert!(apps.list().is_empty());
        assert!(apps.enabled(Client::VSCode));
        assert!(apps.observe([Client::VSCode, Client::Terminal, Client::VSCode]));
        assert_eq!(apps.list().len(), 2);
        assert!(apps.set_enabled(&Client::VSCode.application_id(), false));
        assert!(!apps.observe([Client::Terminal]));
        assert!(!apps.observe([Client::VSCode]));
        assert!(!apps.enabled(Client::VSCode));
        assert!(apps.enabled(Client::Terminal));
        assert_eq!(apps.list().len(), 2);
        let restored = Applications::decode(&serde_json::to_string(&apps).unwrap());
        assert!(!restored.enabled(Client::VSCode));
        assert!(restored.enabled(Client::Terminal));
        assert!(Applications::decode("invalid").list().is_empty());
    }

    #[test]
    fn applications_with_equal_names_do_not_share_checkbox_preferences() {
        let first = Client::Desktop {
            name: "Editor",
            app_path: "/Applications/First.app",
        };
        let second = Client::Desktop {
            name: "Editor",
            app_path: "/Applications/Second.app",
        };
        let mut apps = Applications::default();
        apps.observe([first, second]);
        apps.set_enabled(&first.application_id(), false);
        assert!(!apps.enabled(first));
        assert!(apps.enabled(second));
    }

    #[test]
    fn migration_preserves_choices_and_new_values_without_reimporting_deleted_preferences() {
        use objc2::AllocAnyThread;
        let unique = format!(
            "com.melsomino.ai-agent-monitor.test.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let legacy_name = NSString::from_str(&format!("{unique}.old"));
        let current_name = NSString::from_str(&format!("{unique}.new"));
        let legacy =
            NSUserDefaults::initWithSuiteName(NSUserDefaults::alloc(), Some(&legacy_name)).unwrap();
        let current =
            NSUserDefaults::initWithSuiteName(NSUserDefaults::alloc(), Some(&current_name))
                .unwrap();
        legacy.setBool_forKey(false, ns_string!("AnimateMenuBar"));
        legacy.setBool_forKey(true, ns_string!("FirstLaunchSetupCompleted"));
        // SAFETY: NSStrings are valid preference values.
        unsafe {
            legacy.setObject_forKey(
                Some(ns_string!("saved application filters")),
                ns_string!("KnownApplications"),
            );
            legacy.setObject_forKey(
                Some(ns_string!("saved shortcut")),
                ns_string!("MenuShortcut"),
            );
        }
        legacy.synchronize();
        current.setBool_forKey(true, ns_string!("AnimateMenuBar"));
        current.synchronize();
        migrate_preferences(&current, &legacy_name, &current_name);
        assert!(current.boolForKey(ns_string!("AnimateMenuBar")));
        assert_eq!(
            current
                .stringForKey(ns_string!("KnownApplications"))
                .unwrap()
                .to_string(),
            "saved application filters"
        );
        assert_eq!(
            current
                .stringForKey(ns_string!("MenuShortcut"))
                .unwrap()
                .to_string(),
            "saved shortcut"
        );
        assert!(!current.boolForKey(ns_string!("FirstLaunchSetupCompleted")));
        current.removeObjectForKey(ns_string!("MenuShortcut"));
        migrate_preferences(&current, &legacy_name, &current_name);
        assert!(current.objectForKey(ns_string!("MenuShortcut")).is_none());
        assert!(!legacy.boolForKey(ns_string!("AnimateMenuBar")));
        legacy.removePersistentDomainForName(&legacy_name);
        current.removePersistentDomainForName(&current_name);
    }

    #[test]
    fn system_login_service_is_linked_and_readable_without_registering() {
        let service = login_service().expect("ServiceManagement must be linked into the app");
        // SAFETY: This only reads SMAppService.status; no login item is changed.
        let status: isize = unsafe { msg_send![&*service, status] };
        assert!((0..=3).contains(&status));
    }
}
