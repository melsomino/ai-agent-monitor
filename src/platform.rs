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

/// Desktop presentation order, also used by the web projection on this host.
/// Keep locale-sensitive title collation on the host, not the phone's locale.
pub fn session_order(
    a: &crate::domain::session::Session,
    b: &crate::domain::session::Session,
) -> std::cmp::Ordering {
    use crate::domain::session::Client;
    let rank = |client| match client {
        Client::VSCode => 0,
        Client::Terminal => 1,
        Client::Desktop { .. } => 2,
        Client::Unknown => 3,
    };
    rank(a.client)
        .cmp(&rank(b.client))
        .then_with(|| a.client.application_id().cmp(&b.client.application_id()))
        .then_with(|| {
            #[cfg(target_os = "macos")]
            {
                use objc2_foundation::{NSComparisonResult, NSString};
                match NSString::from_str(&a.title)
                    .localizedStandardCompare(&NSString::from_str(&b.title))
                {
                    NSComparisonResult::Ascending => std::cmp::Ordering::Less,
                    NSComparisonResult::Descending => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                }
            }
            #[cfg(target_os = "linux")]
            {
                gtk::glib::FilenameCollationKey::from(a.title.to_lowercase()).cmp(
                    &gtk::glib::FilenameCollationKey::from(b.title.to_lowercase()),
                )
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                a.title.cmp(&b.title)
            }
        })
        .then_with(|| a.key().cmp(&b.key()))
}
