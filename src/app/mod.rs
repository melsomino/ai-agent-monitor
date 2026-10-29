//! Application commands, shared logic and process utilities.
pub(crate) mod cli;
pub(crate) mod core;
pub(crate) mod git;
pub(crate) mod ipc;
pub(crate) mod monitor;
pub(crate) mod process;
pub(crate) mod setup;
pub(crate) mod tools;

#[cfg(test)]
mod tests;
