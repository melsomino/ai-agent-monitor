//! First-launch integration discovery and installation, independent of AppKit.
use crate::{agents, app::core};
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Codex,
    Claude,
    Cline,
}
impl Agent {
    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Cline => "cline",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::Cline => "Cline",
        }
    }
    pub fn next_step(self) -> &'static str {
        match self {
            Self::Codex => {
                "Restart Codex / VS Code, then review and trust “macOS notification” in /hooks."
            }
            Self::Claude => "Restart Claude Code to load the hooks.",
            Self::Cline => {
                "Turn on Enable Hooks in Cline, then reopen or resume a task. Approvals stay in Cline."
            }
        }
    }
}
#[derive(Debug)]
pub struct Integration {
    pub agent: Agent,
    pub directory: PathBuf,
}
impl Integration {
    pub fn install(&self, executable: &Path) -> Result<bool> {
        match self.agent {
            Agent::Codex => core::install(
                &self.directory,
                executable,
                &core::home().join(".ai-agent-monitor-legacy/notify.py"),
            ),
            Agent::Claude => agents::claude::install(&self.directory, executable),
            Agent::Cline => agents::cline::install(&self.directory, executable),
        }
    }
}

fn has_extension(home: &Path, prefix: &str) -> bool {
    [".vscode/extensions", ".vscode-insiders/extensions"]
        .iter()
        .any(|directory| {
            std::fs::read_dir(home.join(directory)).is_ok_and(|entries| {
                entries.filter_map(Result::ok).any(|entry| {
                    entry.file_type().is_ok_and(|kind| kind.is_dir())
                        && entry.file_name().to_string_lossy().starts_with(prefix)
                })
            })
        })
}

pub fn discover(home: &Path, codex: PathBuf, claude: PathBuf) -> Vec<Integration> {
    let mut found = Vec::new();
    if codex.is_dir()
        || home.join(".codex/bin/codex").is_file()
        || has_extension(home, "openai.chatgpt-")
    {
        found.push(Integration {
            agent: Agent::Codex,
            directory: codex,
        });
    }
    if claude.is_dir()
        || home.join(".local/bin/claude").is_file()
        || has_extension(home, "anthropic.claude-code-")
    {
        found.push(Integration {
            agent: Agent::Claude,
            directory: claude,
        });
    }
    if home.join(".cline").is_dir()
        || has_extension(home, "saoudrizwan.claude-dev-")
        || home
            .join("Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev")
            .is_dir()
    {
        found.push(Integration {
            agent: Agent::Cline,
            directory: home.join("Documents/Cline/Hooks"),
        });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn only_detected_agents_are_installed_and_reruns_are_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let codex = home.join(".codex");
        let claude = home.join(".claude");
        assert!(discover(home, codex.clone(), claude.clone()).is_empty());
        fs::create_dir_all(&codex).unwrap();
        fs::create_dir_all(home.join(".vscode/extensions/anthropic.claude-code-1.0")).unwrap();
        fs::create_dir_all(home.join(".vscode/extensions/saoudrizwan.claude-dev-1.0")).unwrap();
        let found = discover(home, codex, claude);
        assert_eq!(
            found.iter().map(|v| v.agent).collect::<Vec<_>>(),
            [Agent::Codex, Agent::Claude, Agent::Cline]
        );
        let exe = Path::new("/Applications/AI Agent Monitor.app/Contents/MacOS/AIAgentMonitor");
        for integration in found {
            assert!(integration.install(exe).unwrap());
            assert!(!integration.install(exe).unwrap());
        }
    }

    #[test]
    fn custom_configuration_directories_are_respected() {
        let root = tempfile::tempdir().unwrap();
        let codex = root.path().join("custom-codex");
        let claude = root.path().join("custom-claude");
        fs::create_dir_all(&claude).unwrap();
        let found = discover(root.path(), codex, claude.clone());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].directory, claude);
        assert_eq!(found[0].agent, Agent::Claude);
    }
}
