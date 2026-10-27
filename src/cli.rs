use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Native macOS notifications for Codex approval requests.
#[derive(Debug, Parser)]
#[command(name = "CodexNotify", version)]
pub struct Cli {
    // No command is intentional: macOS relaunches the app this way on a click.
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Process a Codex PermissionRequest event from JSON on stdin.
    Hook,
    /// Show a test notification without granting any Codex permissions.
    #[command(long_flag = "test")]
    Test {
        /// Include an Allow button and wait up to 60 seconds for a test response.
        #[arg(long)]
        approval: bool,
        /// Use the title of this Codex chat.
        #[arg(long, value_name = "ID")]
        session_id: Option<String>,
    },
    /// Package this binary and the icon into a signed macOS app.
    Bundle,
    /// Install or update the Codex approval hook.
    Install {
        /// Codex configuration directory (defaults to CODEX_HOME or ~/.codex).
        #[arg(long, value_name = "PATH")]
        codex_home: Option<PathBuf>,
    },
    /// Inspect hook discovery and trust without starting a model.
    CheckHooks {
        /// Codex engine to query.
        #[arg(long, default_value = "codex", value_name = "PATH")]
        binary: String,
    },
    /// Ask macOS for notification permission.
    #[command(long_flag = "setup")]
    Setup,
    /// Open a project using its VS Code URL.
    #[command(long_flag = "open-target")]
    OpenTarget {
        #[arg(value_name = "URL")]
        target: String,
    },
    /// Internal native notification sender.
    #[command(hide = true, long_flag = "send", disable_help_flag = true)]
    Send(Message),
    /// Internal native notification sender with an approval response path.
    #[command(hide = true, long_flag = "send-approval", disable_help_flag = true)]
    SendApproval {
        #[command(flatten)]
        message: Message,
        #[arg(allow_hyphen_values = true)]
        response: String,
    },
}

#[derive(Clone, Debug, Args)]
pub struct Message {
    #[arg(allow_hyphen_values = true)]
    pub title: String,
    #[arg(allow_hyphen_values = true)]
    pub subtitle: String,
    #[arg(allow_hyphen_values = true)]
    pub body: String,
    #[arg(allow_hyphen_values = true)]
    pub target: String,
}

pub enum NativeCommand {
    Listen,
    Setup,
    OpenTarget(String),
    Send {
        message: Message,
        response: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_definitions_are_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_arguments_preserves_notification_click_launch() {
        assert!(
            Cli::try_parse_from(["CodexNotify"])
                .unwrap()
                .command
                .is_none()
        );
    }

    #[test]
    fn native_sender_preserves_empty_fields_and_flag_like_text() {
        let cli = Cli::try_parse_from([
            "CodexNotify",
            "--send-approval",
            "--help",
            "",
            "--example\nТекст",
            "",
            "",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Command::SendApproval { message, response } => {
                assert_eq!(message.title, "--help");
                assert!(message.subtitle.is_empty());
                assert_eq!(message.body, "--example\nТекст");
                assert!(message.target.is_empty());
                assert!(response.is_empty());
            }
            _ => panic!("expected the native approval sender"),
        }
    }

    #[test]
    fn legacy_flags_and_subcommands_select_the_same_operations() {
        for command in ["test", "--test"] {
            let cli =
                Cli::try_parse_from(["CodexNotify", command, "--approval", "--session-id", "one"])
                    .unwrap();
            assert!(
                matches!(cli.command, Some(Command::Test { approval: true, session_id: Some(id) }) if id == "one")
            );
        }
        assert!(matches!(
            Cli::try_parse_from(["CodexNotify", "--setup"])
                .unwrap()
                .command,
            Some(Command::Setup)
        ));
        assert!(
            matches!(Cli::try_parse_from(["CodexNotify", "--open-target", "vscode://file/work"]).unwrap().command,
                         Some(Command::OpenTarget { target }) if target == "vscode://file/work")
        );
    }

    #[test]
    fn missing_values_and_unknown_arguments_are_rejected() {
        for args in [
            vec!["CodexNotify", "install", "--codex-home"],
            vec!["CodexNotify", "test", "--session-id"],
            vec!["CodexNotify", "check-hooks", "--binary"],
            vec!["CodexNotify", "hook", "--approval"],
            vec!["CodexNotify", "test", "--unknown"],
            vec!["CodexNotify", "--send", "title"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }
}
