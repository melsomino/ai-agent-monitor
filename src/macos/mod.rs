//! Native macOS interface, notifications and user preferences.
pub(crate) mod answers;
pub(crate) mod bridge;
pub(crate) mod hotkey;
pub(crate) mod inline_questions;
mod inline_settings;
pub(crate) mod keychain;
pub(crate) mod markdown;
pub(crate) mod menu;
pub(crate) mod motion;
pub(crate) mod native;
pub(crate) mod notification_actions;
pub(crate) mod notification_categories;
pub(crate) mod onboarding;
pub(crate) mod pointer;
pub(crate) mod search;
mod session_composer;
pub(crate) mod session_layout;
mod session_robot;
mod session_thread;
pub(crate) mod settings;
mod settings_pane;
mod telegram_setup;
pub(crate) mod terminal;
mod work_caption;

mod visual;

mod web_settings;

mod remote_tunnel_settings;

mod release_link;
