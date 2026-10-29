//! Register additional agent implementations here; the UI uses only SessionProvider.
pub(crate) mod claude;
pub(crate) mod cline;
pub(crate) mod codex;
pub(crate) mod interaction;
pub(crate) mod pending;
pub(crate) mod sessions;
pub(crate) mod tracking;
use crate::agents::sessions::SessionProvider;

pub fn providers() -> &'static [&'static dyn SessionProvider] {
    &[&codex::Codex, &claude::Claude, &cline::Cline]
}
