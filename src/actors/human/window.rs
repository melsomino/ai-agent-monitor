//! Typed data crossing the AppKit main-thread boundary.
use crate::{domain::session::Session, macos::bridge::PlatformCommand};
use tokio::sync::mpsc;
pub type Sender = mpsc::Sender<Event>;
pub enum Event {
    Toggle,
    Stop,
    ShowRequest {
        session: Box<Session>,
        form: bool,
        draft: Option<std::collections::BTreeMap<String, Vec<String>>>,
    },
    FormClosed(String),
    Platform(PlatformCommand),
}
