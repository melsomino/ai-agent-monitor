//! Register additional agent implementations here; the UI uses only SessionProvider.
pub(crate) mod claude;
pub(crate) mod cline;
pub(crate) mod codex;
pub(crate) mod history;
pub(crate) use crate::domain::interaction;
pub(crate) mod pending;
pub(crate) mod sessions;
pub(crate) mod tracking;
