//! Application commands, shared logic and process utilities.
#[cfg(target_os = "macos")]
pub(crate) mod actor_system;
pub(crate) mod cli;
pub(crate) mod core;
pub(crate) mod git;
pub(crate) mod ipc;
pub(crate) mod process;
pub(crate) mod setup;
pub(crate) mod tools;

pub(crate) mod ids;
#[cfg(test)]
mod tests;
