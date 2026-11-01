use super::*;
pub enum Command {
    FinishedSeen {
        completions: HashMap<String, u64>,
    },
    Observe {
        source: &'static str,
        sessions: Result<Vec<Session>, String>,
        usage: Option<crate::agents::codex::usage::Usage>,
    },
    AgentStopped {
        source: &'static str,
        error: String,
    },
    SetApplicationEnabled {
        id: String,
        enabled: bool,
    },
    Submit {
        session: Box<Session>,
        response: Response,
    },
    SendMessage {
        id: String,
        session: String,
        text: String,
    },
    MessageSent {
        id: String,
        session: String,
        result: Result<(), String>,
    },
    Delivery {
        key: String,
        approval: Approval,
        delivery: crate::agents::interaction::Delivery,
    },
    Hook {
        key: String,
        session: Box<Session>,
        target: String,
    },
    HookGone(String),
    CancelSession {
        agent: String,
        id: String,
    },
    Passive(core::Notification),
    Refresh,
    Open(String),
    History {
        session: String,
        cursor: Option<String>,
        generation: u64,
    },
    HistoryObserved {
        session: String,
        generation: u64,
        result: Result<Option<crate::agents::history::Message>, String>,
    },
    #[cfg(target_os = "macos")]
    Telegram(crate::actors::human::telegram::Event),
    #[cfg(target_os = "macos")]
    TelegramObserved(crate::actors::human::telegram::Status),
    #[cfg(target_os = "macos")]
    Platform(crate::macos::bridge::PlatformCommand),
    #[cfg(target_os = "macos")]
    IntegrationsObserved {
        rows: Vec<crate::macos::onboarding::Row>,
        busy: bool,
        note: String,
    },
    #[cfg(target_os = "macos")]
    PreferencesObserved(crate::macos::settings::Preferences),
    Shutdown,
}
pub(crate) enum AgentCommand {
    Refresh,
    SendMessage {
        id: String,
        session: Box<Session>,
        text: String,
    },
    Respond {
        session: Box<Session>,
        response: Response,
    },
    History {
        session: Box<Session>,
        cursor: Option<String>,
        generation: u64,
    },
}
