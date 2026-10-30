//! Explicit main-thread mailbox, owned by the application's root controller.
pub use crate::actors::human::window::{Event, Sender};
use crate::actors::monitor;
pub enum PlatformCommand {
    Open(String),
    Setup(bool),
    Login(bool),
    Animation(bool),
    Applications(super::settings::Applications),
    Shortcut(Option<super::hotkey::Shortcut>),
    LoginSettings,
    SetupFinished(Vec<super::onboarding::InstallResult>),
    NotificationPermission(bool, Option<String>),
}
pub struct Platform {
    monitor: monitor::Handle,
    setup: super::onboarding::Setup,
}
impl Platform {
    pub fn new(monitor: monitor::Handle, events: Sender) -> Self {
        let this = Self {
            setup: super::onboarding::Setup::new(monitor.clone(), events),
            monitor,
        };
        this.publish(None);
        this.setup.publish();
        this
    }
    pub fn apply(&mut self, command: PlatformCommand) {
        match command {
            PlatformCommand::Open(target) => {
                super::native::open_target(&target);
            }
            PlatformCommand::Setup(force) => {
                self.setup.begin(force);
            }
            PlatformCommand::Login(enabled) => {
                let error = super::settings::set_launch_at_login(enabled)
                    .err()
                    .map(|e| e.to_string());
                self.publish(error);
                return;
            }
            PlatformCommand::Animation(enabled) => super::settings::set_menu_animation(enabled),
            PlatformCommand::Applications(apps) => super::settings::save_applications(&apps),
            PlatformCommand::Shortcut(shortcut) => {
                super::settings::set_menu_shortcut(shortcut.as_ref())
            }
            PlatformCommand::LoginSettings => super::settings::open_login_settings(),
            PlatformCommand::SetupFinished(results) => self.setup.installed(results),
            PlatformCommand::NotificationPermission(allowed, error) => {
                self.setup.permission(allowed, error)
            }
        }
        self.publish(None);
    }
    fn publish(&self, error: Option<String>) {
        let mut preferences = super::settings::snapshot();
        preferences.error = error;
        let _ = self
            .monitor
            .send(monitor::Command::PreferencesObserved(preferences));
    }
}
