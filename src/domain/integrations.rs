use crate::domain::agent::Agent;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Detected,
    Installing,
    Installed,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub agent: Agent,
    pub phase: Phase,
    pub detail: String,
    pub directory: String,
}
impl Row {
    pub fn status(&self) -> &'static str {
        match self.phase {
            Phase::Detected => "Needs setup",
            Phase::Installing => "Setting up…",
            Phase::Installed if self.agent == Agent::Codex => "No setup required",
            Phase::Installed => "Hooks installed",
            Phase::Failed => "Needs attention",
        }
    }
}
