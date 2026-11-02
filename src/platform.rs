//! Messages from the monitor to the desktop owner.
#[cfg(target_os = "macos")]
pub use crate::macos::bridge::{Event, PlatformCommand, Sender};
#[cfg(not(target_os = "macos"))]
pub type Sender = tokio::sync::mpsc::Sender<Event>;
#[cfg(not(target_os = "macos"))]
pub enum Event {
    Toggle,
    Platform(PlatformCommand),
}
#[cfg(not(target_os = "macos"))]
pub enum PlatformCommand {
    Open(String),
    Animation(bool),
    Applications(crate::domain::preferences::Applications),
}

#[cfg(target_os = "linux")]
pub(crate) use crate::linux::credentials;
#[cfg(target_os = "macos")]
pub(crate) use crate::macos::keychain as credentials;
