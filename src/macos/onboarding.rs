//! Automatic, retryable setup on resident app launch (never in hook senders).
use crate::{
    agents,
    app::{core, setup},
};
use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::{MainThreadMarker, runtime::Bool};
use objc2_app_kit::{NSAlert, NSApplication};
use objc2_foundation::{NSError, NSString, NSUserDefaults, ns_string};
use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

static BUSY: AtomicBool = AtomicBool::new(false);

fn signature(integration: &setup::Integration, executable: &Path) -> String {
    format!(
        "1\n{}\n{}",
        executable.display(),
        integration.directory.display()
    )
}
fn key(agent: setup::Agent) -> objc2::rc::Retained<NSString> {
    NSString::from_str(&format!("IntegrationSetup.{}", agent.id()))
}

/// Reject transient disk-image paths before acquiring the resident-process lock.
pub fn installed_location(mtm: MainThreadMarker) -> bool {
    let executable = core::bundle_executable();
    if executable.starts_with("/Volumes")
        || executable
            .components()
            .any(|v| v.as_os_str() == "AppTranslocation")
    {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(ns_string!("Move AI Agent Monitor to Applications"));
        alert.setInformativeText(ns_string!("Drag the app to Applications in the disk image, then open the installed copy. Agent integrations will be set up automatically there."));
        alert.addButtonWithTitle(ns_string!("OK"));
        activate(mtm);
        alert.runModal();
        return false;
    }
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Detected,
    Installing,
    Installed,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub agent: setup::Agent,
    pub phase: Phase,
    pub detail: String,
    pub directory: String,
}
impl Row {
    pub fn status(&self) -> &'static str {
        match self.phase {
            Phase::Detected => "Needs setup",
            Phase::Installing => "Setting up…",
            Phase::Installed => "Hooks installed",
            Phase::Failed => "Needs attention",
        }
    }
}
static INSTALLING: AtomicU8 = AtomicU8::new(0);
fn bit(agent: setup::Agent) -> u8 {
    match agent {
        setup::Agent::Codex => 1,
        setup::Agent::Claude => 2,
        setup::Agent::Cline => 4,
    }
}
fn error_key(agent: setup::Agent) -> objc2::rc::Retained<NSString> {
    NSString::from_str(&format!("IntegrationSetup.{}.Error", agent.id()))
}
fn save_string(key: &NSString, value: &str) {
    // SAFETY: NSString is a valid copied property-list value.
    unsafe {
        NSUserDefaults::standardUserDefaults()
            .setObject_forKey(Some(&NSString::from_str(value)), key);
    }
}
pub fn busy() -> bool {
    BUSY.load(Ordering::Relaxed)
}
pub fn notification_note() -> String {
    NSUserDefaults::standardUserDefaults()
        .stringForKey(ns_string!("IntegrationSetup.NotificationError"))
        .map(|value| value.to_string())
        .unwrap_or_default()
}
pub fn rows() -> Vec<Row> {
    let executable = core::bundle_executable();
    let defaults = NSUserDefaults::standardUserDefaults();
    setup::discover(&core::home(), core::codex_home(), agents::claude::home())
        .into_iter()
        .map(|integration| {
            let (phase, detail) = if INSTALLING.load(Ordering::Relaxed) & bit(integration.agent)
                != 0
            {
                (
                    Phase::Installing,
                    "Connecting hooks to AI Agent Monitor.".into(),
                )
            } else {
                match integration.installed(&executable) {
                    Ok(true) => (Phase::Installed, integration.agent.next_step().into()),
                    Err(error) => (Phase::Failed, format!("{error:#}")),
                    Ok(false) => {
                        if let Some(error) = defaults.stringForKey(&error_key(integration.agent)) {
                            (Phase::Failed, error.to_string())
                        } else {
                            (
                                Phase::Detected,
                                "Detected on this Mac. Click Check & set up to connect.".into(),
                            )
                        }
                    }
                }
            };
            Row {
                agent: integration.agent,
                phase,
                detail,
                directory: integration.directory.display().to_string(),
            }
        })
        .collect()
}

pub fn begin(_mtm: MainThreadMarker, force: bool) {
    if BUSY.swap(true, Ordering::Relaxed) {
        return;
    }
    let executable = core::bundle_executable();
    let defaults = NSUserDefaults::standardUserDefaults();
    let first_launch = !defaults.boolForKey(ns_string!("FirstLaunchSetupCompleted"));
    let pending: Vec<_> =
        setup::discover(&core::home(), core::codex_home(), agents::claude::home())
            .into_iter()
            .filter(|integration| {
                force
                    || defaults
                        .stringForKey(&key(integration.agent))
                        .is_none_or(|saved| {
                            saved.to_string() != signature(integration, &executable)
                        })
                    || !integration.installed(&executable).unwrap_or(false)
            })
            .collect();
    if !first_launch && pending.is_empty() && !force {
        BUSY.store(false, Ordering::Relaxed);
        return;
    }
    INSTALLING.store(
        pending
            .iter()
            .fold(0, |mask, integration| mask | bit(integration.agent)),
        Ordering::Relaxed,
    );
    std::thread::spawn(move || {
        let results: Vec<_> = pending
            .into_iter()
            .map(|integration| {
                let result = integration
                    .install(&executable)
                    .map_err(|error| format!("{error:#}"));
                (
                    integration.agent,
                    signature(&integration, &executable),
                    result,
                )
            })
            .collect();
        DispatchQueue::main().exec_async(move || {
            let defaults = NSUserDefaults::standardUserDefaults();
            for (agent, signature, result) in results {
                match result {
                    Ok(_) => { save_string(&key(agent), &signature); defaults.removeObjectForKey(&error_key(agent)); }
                    Err(error) => save_string(&error_key(agent), &error),
                }
            }
            INSTALLING.store(0, Ordering::Relaxed);
            defaults.setBool_forKey(true, ns_string!("FirstLaunchSetupCompleted"));
            crate::macos::menu::refresh_integrations();
            let callback = RcBlock::new(move |allowed: Bool, error: *mut NSError| {
                // SAFETY: UserNotifications keeps NSError alive for this callback.
                let error = unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
                DispatchQueue::main().exec_async(move || {
                    let key = ns_string!("IntegrationSetup.NotificationError");
                    if let Some(error) = error {
                        save_string(key, &format!("Notifications: {error}"));
                    } else if !allowed.as_bool() {
                        save_string(key, "Enable notifications in System Settings → Notifications → AI Agent Monitor.");
                    } else {
                        NSUserDefaults::standardUserDefaults().removeObjectForKey(key);
                    }
                    BUSY.store(false, Ordering::Relaxed);
                    crate::macos::menu::refresh_integrations();
                });
            });
            UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &callback,
            );
        });
    });
}

fn activate(mtm: MainThreadMarker) {
    // Available on the minimum supported macOS version.
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}
