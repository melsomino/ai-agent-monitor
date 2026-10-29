mod agents;
mod app;
#[cfg(target_os = "macos")]
mod macos;

use app::{cli, core, tools};
#[cfg(target_os = "macos")]
use macos::native;

use anyhow::{Context, Result, bail};
use clap::Parser;
use cli::{AgentKind, Cli, Command, NativeCommand};
use serde_json::Value;
use std::{
    io::{self, Read},
    path::Path,
};

fn run(command: Option<Command>) -> Result<()> {
    match command {
        Some(Command::Hook) => {
            let event: Value = serde_json::from_reader(io::stdin().take(2 * 1024 * 1024))
                .context("invalid hook event")?;
            if event["hook_event_name"] != "PermissionRequest" {
                return Ok(());
            }
            let result = app::ipc::request(&app::ipc::Request::Hook {
                event,
                owner: agents::tracking::owner("codex").ok(),
            })?;
            if !result.is_null() {
                println!("{result}");
            }
        }
        Some(Command::AgentHook { agent, event }) => {
            let input: Value = serde_json::from_reader(io::stdin().take(2 * 1024 * 1024))
                .context("invalid agent hook event")?;
            let owner = agents::tracking::owner(agent.id())?;
            let result = app::ipc::request(&app::ipc::Request::AgentHook {
                agent: agent.id().into(),
                event,
                input,
                owner,
            })?;
            if !result.is_null() {
                println!("{result}");
            }
        }
        Some(Command::InstallAgent { agent, home }) => {
            let executable = core::bundle_executable();
            anyhow::ensure!(executable.is_file(), "Build the app bundle first");
            let home = home.unwrap_or_else(|| match agent {
                AgentKind::Claude => agents::claude::home(),
                AgentKind::Cline => agents::cline::hooks_home(),
            });
            let changed = match agent {
                AgentKind::Claude => agents::claude::install(&home, &executable)?,
                AgentKind::Cline => agents::cline::install(&home, &executable)?,
            };
            println!(
                "{} {} hooks: {}",
                if changed {
                    "Installed"
                } else {
                    "Already installed"
                },
                agent.id(),
                home.display()
            );
            if matches!(agent, AgentKind::Cline) {
                println!(
                    "Enable Hooks in Cline settings, then reopen/resume the task. Approvals remain in Cline."
                );
            } else {
                println!("Restart Claude Code to load lifecycle and permission hooks.");
            }
        }
        Some(Command::Test {
            approval,
            session_id,
        }) => {
            let result = app::ipc::request(&app::ipc::Request::Test {
                approval,
                session_id,
            })?;
            eprintln!(
                "Test completed: {}; no agent permissions granted.",
                if result["allowed"] == true {
                    "Allow received"
                } else {
                    "no approval"
                }
            );
        }
        Some(Command::Bundle) => tools::bundle()?,
        Some(Command::Dmg) => tools::dmg()?,
        Some(Command::Sessions { json: true }) => {
            let result = app::ipc::request(&app::ipc::Request::Sessions)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Some(Command::Sessions { json: false }) => run_native(NativeCommand::Sessions)?,
        Some(Command::Install { codex_home }) => {
            let home = codex_home.unwrap_or_else(core::codex_home);
            let executable = core::bundle_executable();
            if !executable.is_file() {
                bail!("Build the app first: cargo run --release -- bundle");
            }
            let legacy = Path::new(env!("CARGO_MANIFEST_DIR")).join("notify.py");
            let changed = core::install(&home, &executable, &legacy)?;
            println!(
                "{}: {}",
                if changed {
                    "Installed"
                } else {
                    "Already installed"
                },
                home.join("hooks.json").display()
            );
            if changed {
                println!("Restart Codex / VS Code and review the updated hook in /hooks.");
            }
        }
        Some(Command::CheckHooks { binary }) => tools::check_hooks(&binary)?,
        Some(Command::Setup) => {
            let result = app::ipc::request(&app::ipc::Request::Setup)?;
            anyhow::ensure!(
                result["allowed"] == true,
                "Notifications are disabled for AI Agent Monitor"
            );
            println!("Notifications are ready");
        }
        Some(Command::NotificationSettings) => {
            println!(
                "{}",
                app::ipc::request(&app::ipc::Request::NotificationSettings)?
            );
        }
        Some(Command::OpenTarget { target }) => run_native(NativeCommand::OpenTarget(target))?,
        Some(Command::Send(message)) => {
            app::ipc::request(&app::ipc::Request::Notice {
                title: message.title,
                body: message.body,
                target: message.target,
            })?;
        }
        Some(Command::SendApproval { .. }) => {
            bail!("Legacy approval senders are no longer supported; use the hook client")
        }
        None => run_native(NativeCommand::Listen)?,
    }
    Ok(())
}

fn run_native(command: NativeCommand) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        native::run(command)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = command;
        bail!("native notifications require macOS")
    }
}

fn main() {
    let cli = Cli::parse();
    let is_cline_hook = matches!(
        cli.command,
        Some(Command::AgentHook {
            agent: AgentKind::Cline,
            ..
        })
    );
    let is_hook = matches!(cli.command, Some(Command::Hook | Command::AgentHook { .. }));
    if let Err(error) = run(cli.command) {
        eprintln!("ai-agent-monitor: {error:#}");
        if is_cline_hook {
            println!("{{\"cancel\":false}}");
        }
        // Failed hooks return control to the agent without granting permission.
        if !is_hook {
            std::process::exit(1);
        }
    }
}
