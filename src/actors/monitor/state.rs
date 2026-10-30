use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Submission {
    Sending,
    Reconciling,
    Failed(String),
}
#[derive(Clone, Default)]
pub struct History {
    pub messages: Vec<crate::domain::history::Message>,
    pub cursor: Option<String>,
    pub loading: bool,
    pub exhausted: bool,
    pub error: Option<String>,
    pub generation: u64,
}
#[derive(Clone)]
pub struct HookState {
    pub session: Session,
    pub decision: Option<Response>,
}
#[derive(Clone)]
pub struct Notice {
    pub id: u64,
    pub content: core::Notification,
}
#[derive(Clone, Default)]
pub struct State {
    pub revision: u64,
    pub sessions: Vec<Session>,
    pub submissions: HashMap<String, Submission>,
    pub history: HashMap<String, Arc<History>>,
    pub hooks: HashMap<String, HookState>,
    pub notices: Vec<Notice>,
    pub usage: Option<crate::domain::usage::Usage>,
    pub discovery_error: Option<String>,
    pub agent_errors: HashMap<String, String>,
    pub stopping: bool,
    #[cfg(target_os = "macos")]
    pub telegram: crate::domain::telegram::Status,
    #[cfg(target_os = "macos")]
    pub integrations: Vec<crate::domain::integrations::Row>,
    #[cfg(target_os = "macos")]
    pub integration_busy: bool,
    #[cfg(target_os = "macos")]
    pub integration_note: String,
    #[cfg(target_os = "macos")]
    pub preferences: crate::domain::preferences::Preferences,
}
impl State {
    pub fn pending_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|s| s.approval.is_some())
            .count()
    }
}
