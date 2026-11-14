#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub id: String,
    pub body: String,
    pub expires: u64,
    pub can_allow: bool,
}

use crate::domain::interaction::Interaction;
use crate::domain::session_activity::*;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Client {
    Terminal,
    VSCode,
    Desktop {
        name: &'static str,
        app_path: &'static str,
    },
    #[default]
    Unknown,
}
impl Client {
    /// Stable settings identity independent of the displayed group name.
    pub fn application_id(self) -> String {
        match self {
            Self::VSCode => "com.microsoft.VSCode".into(),
            Self::Terminal => "terminal".into(),
            Self::Desktop { app_path, .. } => format!("desktop:{app_path}"),
            Self::Unknown => "other".into(),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::VSCode => "VS Code",
            Self::Desktop { name, .. } => name,
            Self::Unknown => "Unknown",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Activity {
    Working,
    Idle,
    Confirmation,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connected {
        can_send_message: bool,
    },
}
impl ConnectionState {
    pub fn live(can_send_message: bool) -> Self {
        Self::Connected { can_send_message }
    }
    pub fn is_live(self) -> bool {
        matches!(self, Self::Connected { .. })
    }
    pub fn can_send_message(self) -> bool {
        matches!(
            self,
            Self::Connected {
                can_send_message: true
            }
        )
    }
}
impl Activity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Idle => "Idle",
            Self::Confirmation => "Needs response",
            Self::Unknown => "Unknown status",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub agent: &'static str,
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub workspace: Option<PathBuf>,
    pub git_branch: Option<String>,
    /// Last known model reported by this session; never inferred from global defaults.
    pub model: Option<String>,
    /// Provider catalog name, used verbatim; raw model is the fallback.
    pub model_display_name: Option<String>,
    pub reasoning_effort: Option<String>,
    pub updated_at: i64,
    pub client: Client,
    pub activity: Activity,
    pub target: Option<String>,
    pub connection: ConnectionState,
    pub activities: VecDeque<SessionActivity>,
}

impl Session {
    pub fn authoritative(&self) -> bool {
        self.connection.is_live()
    }
    pub fn can_send_message(&self) -> bool {
        self.connection.can_send_message()
    }
    pub fn set_authoritative(&mut self, live: bool) {
        self.connection = if live {
            ConnectionState::live(true)
        } else {
            ConnectionState::Disconnected
        };
    }
    pub fn latest_message(&self) -> Option<&SessionMessage> {
        self.activities.iter().rev().find_map(|a| match a {
            SessionActivity::Message(m) if m.role == MessageRole::Agent => Some(m),
            _ => None,
        })
    }
    pub fn last_message(&self) -> &str {
        self.latest_message().map_or("", |m| m.text.as_str())
    }
    pub fn images(&self) -> &[crate::domain::media::Image] {
        self.latest_message().map_or(&[], |m| m.images.as_slice())
    }
    pub fn pending_request(&self) -> Option<&SessionRequest> {
        self.activities.iter().find_map(|a| match a {
            SessionActivity::Request(r) if r.status == RequestStatus::Pending => Some(r),
            _ => None,
        })
    }
    pub fn pending_request_mut(&mut self) -> Option<&mut SessionRequest> {
        self.activities.iter_mut().find_map(|a| match a {
            SessionActivity::Request(r) if r.status == RequestStatus::Pending => Some(r),
            _ => None,
        })
    }
    pub fn approval(&self) -> Option<&Approval> {
        self.pending_request().map(|r| &r.approval)
    }
    #[cfg(test)]
    pub fn approval_mut(&mut self) -> Option<&mut Approval> {
        self.pending_request_mut().map(|r| &mut r.approval)
    }
    pub fn progress(&self) -> Option<&str> {
        for a in self.activities.iter().rev() {
            match a {
                SessionActivity::Message(m) if m.role == MessageRole::Agent => return None,
                SessionActivity::Action(a) if a.current && !a.text.trim().is_empty() => {
                    return Some(&a.text);
                }
                _ => {}
            }
        }
        None
    }
    pub fn merge_previous(&mut self, previous: &Session) {
        let pending: Vec<_> = self
            .activities
            .iter()
            .filter(|a| a.pending())
            .map(|a| a.id().to_owned())
            .collect();
        let incoming = std::mem::take(&mut self.activities);
        self.activities = previous.activities.clone();
        if self.authoritative() {
            for activity in &mut self.activities {
                if let SessionActivity::Request(r) = activity
                    && r.status == RequestStatus::Pending
                    && !pending.contains(&r.id)
                {
                    r.status = if r.approval.expires
                        <= std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    {
                        RequestStatus::Expired
                    } else {
                        RequestStatus::Resolved
                    };
                    r.updated_at = now();
                }
            }
        }
        for mut activity in incoming {
            if !self.authoritative()
                && matches!(&activity, SessionActivity::Request(r) if !r.approval.id.starts_with("transcript:"))
                && self.activities.iter().any(|a| a.id() == activity.id())
            {
                continue;
            }
            if let Some(old) = self.activities.iter().find(|a| a.id() == activity.id()) {
                let updated = activity.updated_at();
                activity.set_times(old.started_at(), old.updated_at());
                if activity == *old {
                    continue;
                }
                activity.set_times(old.started_at(), updated);
            }
            self.upsert_activity(activity);
        }
        trim(&mut self.activities);
    }
    pub fn upsert_activity(&mut self, activity: SessionActivity) {
        upsert(&mut self.activities, activity);
    }
    #[cfg(test)]
    pub fn set_last_message(&mut self, text: String) {
        let time = now();
        self.upsert_activity(SessionActivity::Message(SessionMessage {
            id: format!(
                "observed-message:{}:{}",
                time.timestamp_nanos_opt().unwrap_or(0),
                self.activities.len()
            ),
            started_at: time,
            updated_at: time,
            role: MessageRole::Agent,
            text,
            images: vec![],
        }));
    }
    pub fn set_images(&mut self, images: Vec<crate::domain::media::Image>) {
        if let Some(SessionActivity::Message(message)) = self
            .activities
            .iter_mut()
            .rev()
            .find(|a| matches!(a,SessionActivity::Message(m) if m.role==MessageRole::Agent))
        {
            message.images = images;
        } else if !images.is_empty() {
            let time = now();
            self.upsert_activity(SessionActivity::Message(SessionMessage {
                id: format!("image:{}", images[0].id),
                started_at: time,
                updated_at: time,
                role: MessageRole::Agent,
                text: "[Image]".into(),
                images,
            }));
        }
        trim(&mut self.activities);
    }
    pub fn clear_current_action(&mut self) {
        for activity in &mut self.activities {
            if let SessionActivity::Action(action) = activity {
                action.current = false;
            }
        }
    }
    /// Reconcile a complete live view without treating disconnected absence as resolution.
    pub fn observe_activities(&mut self, incoming: VecDeque<SessionActivity>) {
        for activity in &mut self.activities {
            match activity {
                SessionActivity::Request(request)
                    if request.status == RequestStatus::Pending
                        && !incoming.iter().any(|a| a.id() == request.id && a.pending()) =>
                {
                    request.status = RequestStatus::Resolved;
                    request.updated_at = now();
                }
                SessionActivity::Action(action) => {
                    action.current=incoming.iter().any(|a|matches!(a,SessionActivity::Action(next) if next.id==action.id && next.current));
                }
                _ => {}
            }
        }
        for activity in incoming {
            self.upsert_activity(activity);
        }
        trim(&mut self.activities);
    }
    #[cfg(test)]
    pub fn set_progress(&mut self, text: Option<String>) {
        self.clear_current_action();
        if let Some(text) = text {
            for a in &mut self.activities {
                if let SessionActivity::Action(a) = a {
                    a.current = false;
                }
            }
            let time = now();
            self.upsert_activity(SessionActivity::Action(SessionAction {
                id: format!(
                    "observed-action:{}:{}",
                    time.timestamp_nanos_opt().unwrap_or(0),
                    self.activities.len()
                ),
                started_at: time,
                updated_at: time,
                kind: ActionKind::Other,
                current: true,
                text,
                status: ActionStatus::Running,
            }));
        }
    }
    pub fn settle_request(&mut self, approval: &Approval, status: RequestStatus) {
        for activity in &mut self.activities {
            if let SessionActivity::Request(r) = activity
                && r.approval == *approval
                && r.status == RequestStatus::Pending
            {
                r.status = status;
                r.updated_at = now();
            }
        }
        trim(&mut self.activities);
    }
    pub fn set_approval(&mut self, approval: Option<Approval>) {
        if let Some(approval) = approval {
            if let Some(request) = self.pending_request_mut()
                && request.approval.id == approval.id
            {
                request.approval = approval;
                return;
            }
            let time = now();
            self.upsert_activity(SessionActivity::Request(SessionRequest {
                id: format!("request:{}", approval.id),
                started_at: time,
                updated_at: time,
                approval,
                interaction: Interaction::default(),
                status: RequestStatus::Pending,
            }));
        } else {
            for a in &mut self.activities {
                if let SessionActivity::Request(r) = a
                    && r.status == RequestStatus::Pending
                {
                    r.status = RequestStatus::Resolved;
                    r.updated_at = now();
                }
            }
            trim(&mut self.activities);
        }
    }
    pub fn set_interaction(&mut self, interaction: Interaction) {
        if let Some(request) = self.pending_request_mut() {
            request.interaction = interaction;
        }
    }
    #[cfg(test)]
    pub fn interaction_mut(&mut self) -> &mut Interaction {
        &mut self
            .pending_request_mut()
            .expect("interaction belongs to a pending request")
            .interaction
    }
    /// Snapshot-only ingress and fixtures; event-capable adapters replace these
    /// seed identities with their source message/item/request identities.
    pub fn seed_snapshot(
        &mut self,
        text: String,
        images: Vec<crate::domain::media::Image>,
        progress: Option<String>,
        approval: Option<Approval>,
        interaction: Interaction,
    ) {
        let time = seconds(self.updated_at);
        if !text.is_empty() || !images.is_empty() {
            self.upsert_activity(SessionActivity::Message(SessionMessage {
                id: "snapshot:message".into(),
                started_at: time,
                updated_at: time,
                role: MessageRole::Agent,
                text,
                images,
            }));
        }
        if let Some(text) = progress {
            self.upsert_activity(SessionActivity::Action(SessionAction {
                id: "snapshot:progress".into(),
                started_at: time,
                updated_at: time,
                kind: ActionKind::Other,
                current: true,
                text,
                status: ActionStatus::Running,
            }));
        }
        if let Some(approval) = approval {
            self.upsert_activity(SessionActivity::Request(SessionRequest {
                id: format!("request:{}", approval.id),
                started_at: time,
                updated_at: time,
                approval,
                interaction,
                status: RequestStatus::Pending,
            }));
        }
    }
    /// Keep the work-status surface present while active, including when the
    /// agent exposes no public description. Never infer a command or thoughts.
    pub fn work_caption(&self) -> Option<&str> {
        (self.activity == Activity::Working && self.approval().is_none()).then(|| {
            self.progress()
                .filter(|text| !text.trim().is_empty())
                .unwrap_or("Working…")
        })
    }

    /// Human-readable model name and session-reported reasoning level.
    pub fn model_label(&self) -> Option<String> {
        let raw = self.model.as_deref()?.trim();
        if raw.is_empty() {
            return None;
        }
        let mut label = self
            .model_display_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(raw)
            .to_owned();
        if let Some(effort) = self
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            label.push(' ');
            label.push_str(match effort {
                "none" => "None",
                "minimal" => "Minimal",
                "low" => "Low",
                "medium" => "Medium",
                "high" => "High",
                "xhigh" => "Extra High",
                "max" => "Max",
                "ultra" => "Ultra",
                other => other,
            });
        }
        Some(label)
    }

    pub fn project_path(&self) -> &Path {
        self.workspace.as_deref().unwrap_or(&self.cwd)
    }

    pub fn working_directory(&self) -> &Path {
        match self.workspace.as_deref() {
            Some(path)
                if path
                    .extension()
                    .is_some_and(|extension| extension == "code-workspace") =>
            {
                // A multi-root workspace file does not identify one project directory.
                &self.cwd
            }
            _ => self.project_path(),
        }
    }

    /// UI identity includes the backend; native session IDs may overlap.
    pub fn key(&self) -> String {
        format!("{}:{}", self.agent, self.id)
    }

    /// Request identity is independent of the session's next approval.
    pub fn approval_key(&self) -> Option<String> {
        self.approval()
            .map(|approval| format!("{}:{}", self.agent, approval.id))
    }

    pub fn open_target(&self) -> Option<String> {
        self.target.clone()
    }
    pub fn interaction(&self) -> Interaction {
        self.pending_request()
            .map(|r| r.interaction.clone())
            .unwrap_or_default()
    }
}
