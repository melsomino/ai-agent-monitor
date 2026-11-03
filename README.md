# AI Agent Monitor

<div align="center">
  <img src="assets/AppIcon-robots.png" width="128" alt="AI Agent Monitor">

  **A desktop tray app for monitoring coding agents on macOS and Ubuntu.**

  [Download for macOS (Apple Silicon)](https://github.com/melsomino/ai-agent-monitor/releases/latest/download/AI-Agent-Monitor-apple-silicon.dmg) · [Install on Ubuntu](#build-from-source-ubuntu) · [Releases](https://github.com/melsomino/ai-agent-monitor/releases)

  macOS 13+ · Ubuntu GNOME (experimental) · Rust · Apache-2.0
</div>

AI Agent Monitor keeps track of your coding agent sessions across terminals, VS Code, and desktop apps.

It shows which agents are working, which are waiting for input, and what they're asking you to approve. You can respond from the monitor, desktop notifications, or Telegram without switching back to the agent's window.

The monitor connects to agents you already have running. It doesn't launch or manage them.

<!-- TODO: Add the demo video preview here, followed by macOS and Ubuntu screenshots when ready. -->

Codex is fully supported. Claude Code and Cline support is experimental.

| | macOS | Ubuntu GNOME |
|---|---|---|
| System integration | Menu bar | GNOME top panel extension |
| Monitor interface | Native AppKit | Native GTK 3 |
| Codex connection | Automatic, local | Automatic, local |
| Notifications | Action buttons and text replies for supported requests | Action buttons; open the monitor for custom or multi-question replies |
| Credential storage | macOS Keychain | Secret Service keyring |
| Availability | Signed DMG; source builds | Source builds |
| Platform status | Supported | Experimental |

The GNOME Shell extension provides the panel icon, global shortcut, and Wayland window positioning. Ubuntu capabilities and limitations are described in the [platform guide](docs/ubuntu.md).

## Features

- **Session monitoring.** See working, waiting, and idle sessions in one place. Sessions are grouped by their host application.
- **Approvals and questions.** Review commands, permission scopes, and questions. Respond using the actions provided by the agent.
- **Desktop notifications.** Receive requests and use supported response actions. Available controls depend on the platform and request type.
- **Telegram.** Browse sessions, read messages, answer requests, and send messages to supported live sessions from your phone.
- **Project navigation.** Return to the relevant VS Code workspace. On macOS, Terminal and iTerm also support matching-tab selection.
- **Local operation.** Session discovery and monitoring run locally on the machine hosting your agents. No monitor account or hosted monitor service is required.

## Supported agents

| Agent | Session monitoring | External responses | Setup |
|---|---|---|---|
| **Codex** | CLI, VS Code, desktop | Approvals, questions, custom answers | Automatic |
| **Claude Code** (experimental) | Terminal and VS Code | One-time permission approval | Hooks |
| **Cline** (experimental) | Task activity and messages | Open the request in Cline | Hooks |

### Codex

Codex sessions are discovered automatically. The monitor uses Codex's local client connection to receive requests and send responses.

No hooks or changes to your Codex configuration are needed.

### Claude Code

Claude Code uses lifecycle and permission hooks. The monitor can display session activity and forward one-time permission approvals.

Restart Claude Code after installing the hooks. Sessions already running may not appear until another hook event.

### Cline

Cline uses hooks for activity tracking and message updates.

Requests appear in the monitor, but must currently be approved in Cline itself.

**Note:** Agent integrations depend on the agent's version and available interfaces. Some request types may not be supported. You can always respond in the agent's own interface.

## Installation

### macOS

1. Download the [latest DMG](https://github.com/melsomino/ai-agent-monitor/releases/latest).
2. Open it and drag AI Agent Monitor into Applications.
3. Launch the app.

The monitor appears as a robot icon in the menu bar.

On first launch, it detects supported agents and sets up the required integrations. Codex needs no additional configuration. Claude Code may need to be restarted, and Cline requires hooks to be enabled.

You can check or repeat setup under **Settings → Integrations**.

The release DMG is signed with Developer ID and notarized by Apple.

Prebuilt releases require macOS 13 or later and Apple Silicon. Intel builds are possible from source.

### Ubuntu GNOME (experimental)

[Build and install from source](#build-from-source-ubuntu), including the companion GNOME Shell extension. There is no prebuilt Ubuntu package yet.

Codex connects automatically. For Claude Code or Cline, open **Settings → Integrations → Check & set up** to install the required hooks.

## Using the monitor

Click the robot icon in the macOS menu bar or GNOME top panel to see your sessions.

The counters use three colors: **green** for working sessions, **orange** for pending requests, and **blue** for unseen completions. Sessions with unseen completions have a blue checkmark.

Completion indicators clear when you interact with that session. See the [completion rules](docs/design/state-models.md#state-activity) for details.

Select a session to see its latest messages, working directory, Git branch, and any pending requests. Earlier messages can be loaded from the session history.

When an agent requests permission or asks a question, the available responses appear in the session panel. Supported actions are also available in desktop notifications; more complex requests can be answered in the monitor.

Responses are checked against the current request before being sent. Requests already resolved in the agent's own interface are removed from the monitor.

Click a session title to open its project or return to its application. On macOS, Terminal and iTerm can select the corresponding tab when it can be identified. Terminal-tab selection is not implemented on Ubuntu.

You can configure a global keyboard shortcut under **Settings → General**.

## Telegram

Telegram integration is optional. It lets you check your agents and respond to requests when you're away from the computer running them.

You can:

- Browse sessions and read message history.
- See pending approvals and questions.
- Respond using the agent's available choices.
- Send messages to supported live Codex sessions.
- View the current working directory and Git branch.

The selected session stays available between messages. Session views update automatically as the agent's state changes.

### Why a personal bot?

We chose personal Telegram bots over shared ones to protect your privacy. Your bot connects directly to the app running on your computer, so your conversations never pass through our servers — only Telegram's.

The bot token is stored in macOS Keychain or Ubuntu's Secret Service keyring.

### Setup

Open **Settings → Telegram**.

1. Click **Open** to open BotFather in Telegram, then send `/newbot`.
2. Copy the suggested bot name and username from the app into BotFather.
3. Copy the token provided by BotFather, paste it into the app, and click **Connect**.

The monitor opens your bot in Telegram. Press **Start** to complete the connection. If the chat was already authorized, connection completes automatically.

The settings screen then shows the connected bot, with options to test it, open it, replace it, or unlink it.

You can change bots without interrupting the current connection. The old bot remains active until the replacement is successfully connected.

**Your computer must be awake and AI Agent Monitor must be running** for Telegram integration to work.

## Privacy and permissions

AI Agent Monitor runs locally. It does not require an account or a hosted service.

- **Agent sessions:** Session information, message history, and Git metadata are read locally.
- **Agent responses:** Approvals and messages are sent to existing agent sessions only after you submit them.
- **Telegram:** When enabled, relevant session information and messages are sent through Telegram's servers. The monitor does not use a separate relay service.
- **Credentials:** Your Telegram bot token is stored in macOS Keychain or Ubuntu's Secret Service keyring.
- **macOS permissions:** Notifications require notification permission. Switching to specific terminal tabs may require Automation permission.
- **Ubuntu integration:** The GNOME Shell extension provides panel positioning and global shortcuts; the desktop notification service controls notification presentation.

Telegram messages are not end-to-end encrypted. Avoid enabling Telegram delivery for sessions containing information you don't want to send through Telegram.

The monitor does not transmit your project files to a separate monitoring service.

## Settings

The settings window has four sections:

| Section | Options |
|---|---|
| General | Launch at login, menu bar animation, keyboard shortcut |
| Applications | Show or hide sessions from individual applications |
| Integrations | Agent detection, hook setup, integration status |
| Telegram | Bot setup, connection, testing, replacement, unlinking |

Application visibility preferences are retained across restarts.

## Limitations

- Monitoring is local to the machine hosting your agents.
- Remote agents running over SSH or inside containers require a separate integration mechanism.
- Claude Code and Cline integrations are experimental.
- Cline approvals must currently be handled in Cline.
- Sending new messages through Telegram requires a supported live Codex connection.
- The Telegram integration requires the computer and monitor to remain running.
- Ubuntu support is experimental; see [Ubuntu setup and limitations](docs/ubuntu.md).

## Build from source (Ubuntu)

Requires Ubuntu with GNOME and Rust with support for the 2024 edition.

Install Rust using [rustup](https://rustup.rs/) if it is not already installed.

Install the system dependencies, then build:

```sh
sudo apt-get update
sudo apt-get install git build-essential pkg-config libgtk-3-dev libsqlite3-dev \
  libsecret-tools lsof curl xdg-utils zip

git clone https://github.com/melsomino/ai-agent-monitor.git
cd ai-agent-monitor
cargo build --release --locked
```

Install the executable, desktop launcher, and GNOME top-panel extension:

```sh
./scripts/install-ubuntu.sh
```

Log out and back in if GNOME has not loaded the extension, then launch **AI Agent Monitor** from the application menu or run:

```sh
"$HOME/.local/bin/AIAgentMonitor"
```

For development, run `cargo run --locked` from the project directory. See [Ubuntu setup and limitations](docs/ubuntu.md) for desktop integration details and verification commands.

## Build from source (macOS)

Requires Rust with support for the 2024 edition and Xcode Command Line Tools.

```sh
git clone https://github.com/melsomino/ai-agent-monitor.git
cd ai-agent-monitor

cargo run --release --locked -- bundle
```

Local builds use ad-hoc signing by default. To sign with a Developer ID certificate from your Keychain, set `AI_AGENT_MONITOR_SIGN_IDENTITY` to that identity.

The app bundle is created at:

```text
build/AI Agent Monitor.app
```

For local installation:

```sh
mkdir -p "$HOME/Applications"
ditto "build/AI Agent Monitor.app" \
  "$HOME/Applications/AI Agent Monitor.app"
```

This produces an ad-hoc signed build for local development. Public releases use Developer ID signing and Apple notarization.

## Development

The application is written in Rust, using Tokio actors for session state, agent adapters, and user interactions.

The native frontends, desktop notifications, and Telegram integration share the same monitor state.

Agent adapters are separate from the interfaces that display requests and submit responses.

The [UI/UX Specification](docs/design/README.md) is the authoritative cross-platform reference for behavior and presentation. It compares the interactive HTML prototype, macOS AppKit, and Ubuntu GNOME, with acceptance criteria and a [three-way parity matrix](docs/design/parity.md).

Additional technical documentation:

- [Actor architecture](docs/actor-architecture.md)
- [Agent integrations](docs/integrations.md)

## License

AI Agent Monitor is free to use and distributed under the [Apache License 2.0](LICENSE).

Agent services and Telegram are separate from the monitor.
