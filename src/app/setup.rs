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
    /// Inspect actual hook configuration; a remembered setup flag alone is insufficient.
    pub fn installed(&self, executable: &Path) -> Result<bool> {
        let config_path = self.directory.join(match self.agent {
            Agent::Codex => "hooks.json",
            Agent::Claude => "settings.json",
            Agent::Cline => "",
        });
        let read = |path: &Path| -> Result<Option<String>> {
            let metadata = match std::fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            anyhow::ensure!(
                metadata.is_file() && metadata.len() <= 2 * 1024 * 1024,
                "Hook configuration is not a regular file or is too large"
            );
            Ok(Some(std::fs::read_to_string(path)?))
        };
        let executable = executable
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Non-UTF8 executable path"))?;
        let matches = |command: &str, arguments: &[&str]| -> bool {
            shlex::split(command).is_some_and(|words| {
                words.first().is_some_and(|word| word == executable)
                    && words
                        .iter()
                        .skip(1)
                        .map(String::as_str)
                        .eq(arguments.iter().copied())
            })
        };
        if self.agent == Agent::Cline {
            for event in agents::cline::EVENTS {
                let Some(script) = read(&self.directory.join(event))? else {
                    return Ok(false);
                };
                if !script.starts_with("#!/bin/sh\n# AI Agent Monitor Cline integration\n")
                    || !script.lines().any(|line| {
                        line.strip_prefix("exec ").is_some_and(|command| {
                            matches(command, &["agent-hook", "cline", "--event", event])
                        })
                    })
                {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        let Some(contents) = read(&config_path)? else {
            return Ok(false);
        };
        let config: serde_json::Value = serde_json::from_str(&contents)?;
        if self.agent == Agent::Claude && config["disableAllHooks"] == true {
            return Ok(false);
        }
        let events: &[&str] = if self.agent == Agent::Codex {
            &["PermissionRequest"]
        } else {
            agents::claude::EVENTS
        };
        Ok(events.iter().all(|event| {
            config["hooks"][event].as_array().is_some_and(|groups| {
                groups.iter().any(|group| {
                    group["hooks"].as_array().is_some_and(|hooks| {
                        hooks.iter().any(|hook| {
                            hook["type"] == "command"
                                && hook["command"].as_str().is_some_and(|command| {
                                    matches(
                                        command,
                                        if self.agent == Agent::Codex {
                                            &["hook"]
                                        } else {
                                            &["agent-hook", "claude"]
                                        },
                                    )
                                })
                        })
                    })
                })
            })
        }))
    }

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
            assert!(!integration.installed(exe).unwrap());
            assert!(integration.install(exe).unwrap());
            assert!(integration.installed(exe).unwrap());
            assert!(
                !integration
                    .installed(Path::new(
                        "/moved/AI Agent Monitor.app/Contents/MacOS/AIAgentMonitor"
                    ))
                    .unwrap()
            );
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
    #[test]
    fn integration_status_detects_disabled_and_broken_hook_configuration() {
        let root = tempfile::tempdir().unwrap();
        let integration = Integration {
            agent: Agent::Claude,
            directory: root.path().join("claude"),
        };
        let executable =
            Path::new("/Applications/AI Agent Monitor.app/Contents/MacOS/AIAgentMonitor");
        assert!(!integration.installed(executable).unwrap());
        integration.install(executable).unwrap();
        assert!(integration.installed(executable).unwrap());
        let path = integration.directory.join("settings.json");
        let mut config: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        config["disableAllHooks"] = true.into();
        fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        assert!(!integration.installed(executable).unwrap());
        fs::write(&path, "invalid JSON").unwrap();
        assert!(integration.installed(executable).is_err());
    }
}
