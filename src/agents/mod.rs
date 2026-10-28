//! Register additional agent implementations here; the UI uses only SessionProvider.
pub(crate) mod codex;
pub(crate) mod sessions;
use crate::agents::sessions::SessionProvider;

pub fn providers() -> &'static [&'static dyn SessionProvider] {
    &[&codex::Codex]
}
