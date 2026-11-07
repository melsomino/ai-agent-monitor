#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub id: String,
    pub body: String,
    pub expires: u64,
    pub can_allow: bool,
}

use crate::domain::interaction::Interaction;
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
    pub last_message: String,
    pub images: Vec<crate::domain::media::Image>,
    pub interaction_data: Interaction,
    pub target: Option<String>,
    pub authoritative: bool,
    pub approval: Option<Approval>,
}

impl Session {
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
        self.approval
            .as_ref()
            .map(|approval| format!("{}:{}", self.agent, approval.id))
    }

    pub fn open_target(&self) -> Option<String> {
        self.target.clone()
    }
    pub fn interaction(&self) -> Interaction {
        self.interaction_data.clone()
    }
}
