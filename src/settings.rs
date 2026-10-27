//! Persistent preferences and macOS login-item registration.
use anyhow::{Context, Result, bail};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool},
};
use objc2_foundation::{NSError, NSUserDefaults, ns_string};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_login_service_is_linked_and_readable_without_registering() {
        let service = login_service().expect("ServiceManagement must be linked into the app");
        // SAFETY: This only reads SMAppService.status; no login item is changed.
        let status: isize = unsafe { msg_send![&*service, status] };
        assert!((0..=3).contains(&status));
    }
}
