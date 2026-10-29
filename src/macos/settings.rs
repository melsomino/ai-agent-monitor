//! Persistent preferences and macOS login-item registration.
use crate::agents::sessions::Client;
use anyhow::{Context, Result, bail};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool},
};
use objc2_foundation::{NSError, NSUserDefaults, ns_string};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnownApplication {
    pub id: String,
    pub name: String,
    pub icon_path: Option<String>,
    pub enabled: bool,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Applications {
    apps: BTreeMap<String, KnownApplication>,
}
impl Applications {
    /// Keep previously seen applications, including disabled and currently closed ones.
    pub fn observe(&mut self, clients: impl IntoIterator<Item = Client>) -> bool {
        let mut changed = false;
        for client in clients {
            let id = client.application_id();
            let name = if client == Client::Unknown {
                "Other apps"
            } else {
                client.label()
            }
            .to_owned();
            let icon_path = match client {
                Client::VSCode => Some("/Applications/Visual Studio Code.app"),
                Client::Terminal => Some("/System/Applications/Utilities/Terminal.app"),
                Client::Desktop { app_path, .. } => Some(app_path),
                Client::Unknown => None,
            }
            .map(str::to_owned);
            if let Some(app) = self.apps.get_mut(&id) {
                if app.name != name || app.icon_path != icon_path {
                    app.name = name;
                    app.icon_path = icon_path;
                    changed = true;
                }
            } else {
                self.apps.insert(
                    id.clone(),
                    KnownApplication {
                        id,
                        name,
                        icon_path,
                        enabled: true,
                    },
                );
                changed = true;
            }
        }
        changed
    }
    pub fn enabled(&self, client: Client) -> bool {
        self.apps
            .get(&client.application_id())
            .is_none_or(|app| app.enabled)
    }
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        let Some(app) = self.apps.get_mut(id) else {
            return false;
        };
        if app.enabled == enabled {
            return false;
        }
        app.enabled = enabled;
        true
    }
    pub fn list(&self) -> Vec<&KnownApplication> {
        let mut apps: Vec<_> = self.apps.values().collect();
        apps.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        apps
    }
    fn decode(value: &str) -> Self {
        serde_json::from_str(value).unwrap_or_default()
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginStatus {
    Disabled,
    Enabled,
    RequiresApproval,
    Unavailable,
}

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
    fn system_login_service_is_linked_and_readable_without_registering() {
        let service = login_service().expect("ServiceManagement must be linked into the app");
        // SAFETY: This only reads SMAppService.status; no login item is changed.
        let status: isize = unsafe { msg_send![&*service, status] };
        assert!((0..=3).contains(&status));
    }
}
