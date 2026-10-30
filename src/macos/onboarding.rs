//! Automatic, retryable setup on resident app launch (never in hook senders).
use crate::{
    agents,
    app::{core, setup},
};
use block2::RcBlock;
use objc2::{MainThreadMarker, runtime::Bool};
use objc2_app_kit::{NSAlert, NSApplication};
use objc2_foundation::{NSError, NSString, NSUserDefaults, ns_string};
use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};
use std::path::Path;

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

pub use crate::domain::integrations::{Phase, Row};

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
pub fn notification_note() -> String {
    NSUserDefaults::standardUserDefaults()
        .stringForKey(ns_string!("IntegrationSetup.NotificationError"))
        .map(|value| value.to_string())
        .unwrap_or_default()
}
pub fn rows(installing: u8) -> Vec<Row> {
    let executable = core::bundle_executable();
    let defaults = NSUserDefaults::standardUserDefaults();
    setup::discover(&core::home(), core::codex_home(), agents::claude::home())
        .into_iter()
        .map(|integration| {
            let (phase, detail) = if installing & bit(integration.agent) != 0 {
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

pub type InstallResult = (setup::Agent, String, Result<bool, String>);
pub struct Setup {
    busy: bool,
    installing: u8,
    monitor: crate::actors::monitor::Handle,
    events: crate::macos::bridge::Sender,
}
impl Setup {
    pub fn new(
        monitor: crate::actors::monitor::Handle,
        events: crate::macos::bridge::Sender,
    ) -> Self {
        Self {
            busy: false,
            installing: 0,
            monitor,
            events,
        }
    }
    pub fn publish(&self) {
        let _ = self
            .monitor
            .send(crate::actors::monitor::Command::IntegrationsObserved {
                rows: rows(self.installing),
                busy: self.busy,
                note: notification_note(),
            });
    }
    pub fn begin(&mut self, force: bool) {
        if self.busy {
            return;
        }
        let executable = core::bundle_executable();
        let defaults = NSUserDefaults::standardUserDefaults();
        let first_launch = !defaults.boolForKey(ns_string!("FirstLaunchSetupCompleted"));
        let pending: Vec<_> =
            setup::discover(&core::home(), core::codex_home(), agents::claude::home())
                .into_iter()
                .filter(|i| {
                    force
                        || defaults
                            .stringForKey(&key(i.agent))
                            .is_none_or(|v| v.to_string() != signature(i, &executable))
                        || !i.installed(&executable).unwrap_or(false)
                })
                .collect();
        if !first_launch && pending.is_empty() && !force {
            self.publish();
            return;
        }
        self.busy = true;
        self.installing = pending.iter().fold(0, |mask, i| mask | bit(i.agent));
        self.publish();
        let events = self.events.clone();
        std::thread::spawn(move || {
            let results = pending
                .into_iter()
                .map(|i| {
                    let result = i.install(&executable).map_err(|e| e.to_string());
                    (i.agent, signature(&i, &executable), result)
                })
                .collect();
            let _ = events.blocking_send(crate::macos::bridge::Event::Platform(
                crate::macos::bridge::PlatformCommand::SetupFinished(results),
            ));
        });
    }
    pub fn installed(&mut self, results: Vec<InstallResult>) {
        let defaults = NSUserDefaults::standardUserDefaults();
        for (agent, signature, result) in results {
            match result {
                Ok(_) => {
                    save_string(&key(agent), &signature);
                    defaults.removeObjectForKey(&error_key(agent));
                }
                Err(e) => save_string(&error_key(agent), &e),
            }
        }
        self.installing = 0;
        defaults.setBool_forKey(true, ns_string!("FirstLaunchSetupCompleted"));
        self.publish();
        let events = self.events.clone();
        let callback = RcBlock::new(move |allowed: Bool, error: *mut NSError| {
            let error = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
            let _ = events.try_send(crate::macos::bridge::Event::Platform(
                crate::macos::bridge::PlatformCommand::NotificationPermission(
                    allowed.as_bool(),
                    error,
                ),
            ));
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &callback,
            );
    }
    pub fn permission(&mut self, allowed: bool, error: Option<String>) {
        let key = ns_string!("IntegrationSetup.NotificationError");
        if let Some(error) = error {
            save_string(key, &format!("Notifications: {error}"));
        } else if !allowed {
            save_string(
                key,
                "Enable notifications in System Settings → Notifications → AI Agent Monitor.",
            );
        } else {
            NSUserDefaults::standardUserDefaults().removeObjectForKey(key);
        }
        self.busy = false;
        self.publish();
    }
}

fn activate(mtm: MainThreadMarker) {
    // Available on the minimum supported macOS version.
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}
