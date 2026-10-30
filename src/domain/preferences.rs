use crate::domain::session::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownApplication {
    pub id: String,
    pub name: String,
    pub icon_path: Option<String>,
    pub enabled: bool,
}
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applications {
    apps: BTreeMap<String, KnownApplication>,
}
impl Applications {
    /// Keep previously seen applications, including disabled and currently closed ones.
    pub fn observe(&mut self, clients: impl IntoIterator<Item = Client>) -> bool {
        let mut changed = false;
        for client in clients {
            let id = client.application_id();
            let name = if client == Client::Unknown {
                "Other apps"
            } else {
                client.label()
            }
            .to_owned();
            let icon_path = match client {
                Client::VSCode => Some("/Applications/Visual Studio Code.app"),
                Client::Terminal => Some("/System/Applications/Utilities/Terminal.app"),
                Client::Desktop { app_path, .. } => Some(app_path),
                Client::Unknown => None,
            }
            .map(str::to_owned);
            if let Some(app) = self.apps.get_mut(&id) {
                if app.name != name || app.icon_path != icon_path {
                    app.name = name;
                    app.icon_path = icon_path;
                    changed = true;
                }
            } else {
                self.apps.insert(
                    id.clone(),
                    KnownApplication {
                        id,
                        name,
                        icon_path,
                        enabled: true,
                    },
                );
                changed = true;
            }
        }
        changed
    }
    pub fn enabled(&self, client: Client) -> bool {
        self.apps
            .get(&client.application_id())
            .is_none_or(|app| app.enabled)
    }
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        let Some(app) = self.apps.get_mut(id) else {
            return false;
        };
        if app.enabled == enabled {
            return false;
        }
        app.enabled = enabled;
        true
    }
    pub fn list(&self) -> Vec<&KnownApplication> {
        let mut apps: Vec<_> = self.apps.values().collect();
        apps.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        apps
    }
    pub(crate) fn decode(value: &str) -> Self {
        serde_json::from_str(value).unwrap_or_default()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginStatus {
    Disabled,
    Enabled,
    RequiresApproval,
    Unavailable,
}

#[derive(Clone)]
pub struct Preferences {
    pub applications: Applications,
    pub login: LoginStatus,
    pub animation: bool,
    pub shortcut: Option<crate::domain::shortcut::Shortcut>,
    pub error: Option<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            applications: Applications::default(),
            login: LoginStatus::Disabled,
            animation: true,
            shortcut: None,
            error: None,
        }
    }
}
