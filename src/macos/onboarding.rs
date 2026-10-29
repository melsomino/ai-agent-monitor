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
    sync::atomic::{AtomicBool, Ordering},
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
            })
            .collect();
    if !first_launch && pending.is_empty() && !force {
        BUSY.store(false, Ordering::Relaxed);
        return;
    }
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
            let mut sections = Vec::new();
            let mut errors = false;
            for (agent, signature, result) in results {
                match result {
                    Ok(_) => {
                        // SAFETY: NSString is a valid copied property-list value.
                        unsafe { defaults.setObject_forKey(Some(&NSString::from_str(&signature)), &key(agent)); }
                        sections.push(format!("{} is connected. {}", agent.name(), agent.next_step()));
                    }
                    Err(error) => {
                        errors = true;
                        sections.push(format!("{} could not be connected: {error}", agent.name()));
                    }
                }
            }
            if sections.is_empty() {
                sections.push("No supported agents were found. Open Codex, Claude Code, or Cline, then choose Set up integrations in General settings. Agents are installed separately.".into());
            }
            if errors {
                sections.push("Existing configuration was preserved where setup could not proceed. Fix the issue and use Set up integrations in General settings to retry.".into());
            }
            defaults.setBool_forKey(true, ns_string!("FirstLaunchSetupCompleted"));
            let title = if errors { "Setup needs attention" } else { "AI Agent Monitor is ready" };
            let body = sections.join("\n\n");
            let show = move |allowed: bool, error: Option<String>| {
                let mut body = body.clone();
                if let Some(error) = error {
                    body.push_str(&format!("\n\nNotification permission: {error}"));
                } else if !allowed {
                    body.push_str("\n\nNotifications are disabled. You can enable them in System Settings → Notifications → AI Agent Monitor.");
                }
                DispatchQueue::main().exec_async(move || {
                    let mtm = MainThreadMarker::new().expect("setup UI on main thread");
                    let alert = NSAlert::new(mtm);
                    alert.setMessageText(&NSString::from_str(title));
                    alert.setInformativeText(&NSString::from_str(&body));
                    alert.addButtonWithTitle(ns_string!("Done"));
                    activate(mtm);
                    alert.runModal();
                    BUSY.store(false, Ordering::Relaxed);
                });
            };
            let callback = RcBlock::new(move |allowed: Bool, error: *mut NSError| {
                // SAFETY: UserNotifications keeps NSError alive for this callback.
                let error = unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
                show(allowed.as_bool(), error);
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .requestAuthorizationWithOptions_completionHandler(
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
