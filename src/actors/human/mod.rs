//! State-driven human interaction channels.
#[cfg(target_os = "macos")]
pub mod notifications;
mod retry;
pub mod telegram;
#[cfg(target_os = "macos")]
pub mod window;
