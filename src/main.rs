mod agents;
mod app;
#[cfg(target_os = "macos")]
mod macos;

use agents::sessions;
use app::{cli, core, tools};
#[cfg(target_os = "macos")]
use macos::native;

use anyhow::{Context, Result, bail};
use clap::Parser;
use cli::{Cli, Command, NativeCommand};
use serde_json::{Value, json};
use std::{env, io, path::Path};

fn run(command: Option<Command>) -> Result<()> {
    match command {
        Some(Command::Hook) => {
            let event: Value =
                serde_json::from_reader(io::stdin()).context("invalid hook event")?;
            let action = core::deliver(&event, true)?;
            if let Some(result) = core::decision(action, false) {
                println!("{result}");
            }
        }
        Some(Command::Test {
            approval,
            session_id,
        }) => {
            let mut event = json!({"hook_event_name": "PermissionRequest", "cwd": env::current_dir()?,
                "tool_name": "Bash", "tool_input": {"description": "Test notification sent from Rust. Allow does not run any commands."}});
            if let Some(id) = session_id {
                event["session_id"] = id.into();
            }
            let action = core::deliver(&event, approval)?;
            if let Some(core::Action::Allow) = action {
                eprintln!("Test: Allow clicked; no Codex permission granted.");
            } else {
                eprintln!("Test completed: {action:?}");
            }
        }
        Some(Command::Bundle) => tools::bundle()?,
        Some(Command::Sessions { json: true }) => {
            let sessions = sessions::open()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "sessions": sessions.iter().map(|session| serde_json::json!({
                        "agent":session.agent,"id":session.id,"title":session.title,"cwd":session.cwd,"workspace":session.workspace,"gitBranch":session.git_branch,"updatedAt":session.updated_at,
                        "client":session.client.label(),"status":session.activity.label(),"lastMessage":session.last_message,
                        "approval":session.approval.as_ref().map(|request| serde_json::json!({"body":request.body,"expiresAt":request.expires}))
                    })).collect::<Vec<_>>()
                }))?
            );
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
        Some(Command::Setup) => run_native(NativeCommand::Setup)?,
        Some(Command::NotificationSettings) => run_native(NativeCommand::NotificationSettings)?,
        Some(Command::OpenTarget { target }) => run_native(NativeCommand::OpenTarget(target))?,
        Some(Command::Send(message)) => run_native(NativeCommand::Send {
            message,
            response: None,
        })?,
        Some(Command::SendApproval { message, response }) => run_native(NativeCommand::Send {
            message,
            response: Some(response),
        })?,
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
    let is_hook = matches!(cli.command, Some(Command::Hook));
    if let Err(error) = run(cli.command) {
        eprintln!("ai-agent-monitor: {error:#}");
        // A failed hook must return control to Codex without granting permission.
        if !is_hook {
            std::process::exit(1);
        }
    }
}
