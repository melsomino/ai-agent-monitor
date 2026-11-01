# AI Agent Monitor

<div align="center">
  <img src="assets/AppIcon-robots.png" width="128" alt="AI Agent Monitor">

  **A macOS menu bar app for monitoring coding agents.**

  [Download for Apple Silicon](https://github.com/melsomino/ai-agent-monitor/releases/latest/download/AI-Agent-Monitor-apple-silicon.dmg) · [Releases](https://github.com/melsomino/ai-agent-monitor/releases)

  macOS 13+ · Rust · Native AppKit · Apache-2.0
</div>

AI Agent Monitor keeps track of your coding agent sessions across terminals, VS Code, and desktop apps.

It shows which agents are working, which are waiting for input, and what they're asking you to approve. You can respond from the menu bar, macOS notifications, or Telegram without switching back to the agent's window.

The monitor connects to agents you already have running. It doesn't launch or manage them.

Codex is fully supported. Claude Code and Cline support is experimental.

<!-- TODO: Add a short demo showing sessions, an approval request, and a response. -->

## Features

- **Session monitoring.** See active, waiting, and idle sessions in one menu. Sessions are grouped by their host application.
- **Approvals and questions.** Review commands, permission scopes, and questions. Respond using the actions provided by the agent.
- **macOS notifications.** Receive requests and respond without opening the agent's window.
- **Telegram.** Browse sessions, read messages, answer requests, and send messages to supported live sessions from your phone.
- **Project navigation.** Return to the relevant VS Code workspace or terminal window. Terminal and iTerm support matching-tab selection.
- **Local operation.** Session discovery and monitoring run on your Mac. No account or hosted monitor service is required.

The app uses a native AppKit interface and runs as a single Rust executable.

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

1. Download the [latest DMG](https://github.com/melsomino/ai-agent-monitor/releases/latest).
2. Open it and drag AI Agent Monitor into Applications.
3. Launch the app.

The monitor appears as a robot icon in the menu bar.

On first launch, it detects supported agents and sets up the required integrations. Codex needs no additional configuration. Claude Code may need to be restarted, and Cline requires hooks to be enabled.

You can check or repeat setup under **Settings → Integrations**.

The release DMG is signed with Developer ID and notarized by Apple.

Prebuilt releases require macOS 13 or later and Apple Silicon. Intel builds are possible from source.

## Using the monitor

Click the menu bar icon to see your sessions.

The icon shows a green count when agents are working and an orange count when requests need your attention.

A blue count shows sessions that have just finished working and have not been seen yet. Those sessions have a blue checkmark in the menu. Their indicators clear when you expand or collapse that session, open it in its application, or load its previous message. Merely viewing or closing the menu does not clear them. A session that starts working again returns to its working state.

Select a session to see its latest messages, working directory, Git branch, and any pending requests. Earlier messages can be loaded from the session history.

When an agent requests permission or asks a question, the available responses appear in the session panel. Supported requests can also be answered from macOS notifications.

Responses are checked against the current request before being sent. Requests already resolved in the agent's own interface are removed from the monitor.

Click a session title to return to its application. Terminal and iTerm can select the corresponding tab when it can be identified.

You can configure a global keyboard shortcut under **Settings → General**.

## Telegram

Telegram integration is optional. It lets you check your agents and respond to requests when you're away from your Mac.

You can:

- Browse sessions and read message history.
- See pending approvals and questions.
- Respond using the agent's available choices.
- Send messages to supported live Codex sessions.
- View the current working directory and Git branch.

The selected session stays available between messages. Session views update automatically as the agent's state changes.

### Why a personal bot?

We chose personal Telegram bots over shared ones to protect your privacy. Your bot connects directly to the app running on your Mac, so your conversations never pass through our servers — only Telegram's.

The bot token is stored in macOS Keychain.

### Setup

Open **Settings → Telegram**.

1. Click **Open** to start BotFather and send `/newbot`.
2. Copy the suggested bot name and username from the app into BotFather.
3. Copy the token provided by BotFather, paste it into the app, and click **Connect**.

The monitor opens your bot in Telegram. Press **Start** to complete the connection. If the chat was already authorized, connection completes automatically.

The settings screen then shows the connected bot, with options to test it, open it, replace it, or unlink it.

You can change bots without interrupting the current connection. The old bot remains active until the replacement is successfully connected.

**Your Mac must be awake and AI Agent Monitor must be running** for Telegram integration to work.

## Privacy and permissions

AI Agent Monitor runs locally. It does not require an account or a hosted service.

- **Agent sessions:** Session information, message history, and Git metadata are read locally.
- **Agent responses:** Approvals and messages are sent to existing agent sessions only after you submit them.
- **Telegram:** When enabled, relevant session information and messages are sent through Telegram's servers. The monitor does not use a separate relay service.
- **Credentials:** Your Telegram bot token is stored in macOS Keychain.
- **macOS permissions:** Notifications require notification permission. Switching to specific terminal tabs may require Automation permission.

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

- Monitoring is local to the Mac running the agents.
- Remote agents running over SSH or inside containers require a separate integration mechanism.
- Claude Code and Cline integrations are experimental.
- Cline approvals must currently be handled in Cline.
- Sending new messages through Telegram requires a supported live Codex connection.
- The Telegram integration requires the Mac and monitor to remain running.
- Linux is not currently supported.

## Build from source

Requires Rust with support for the 2024 edition and Xcode Command Line Tools.

```sh
git clone https://github.com/melsomino/ai-agent-monitor.git
cd ai-agent-monitor

AI_AGENT_MONITOR_SIGN_IDENTITY=- cargo run --release --locked -- bundle
```

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

The menu bar interface, macOS notifications, and Telegram integration share the same monitor state.

Agent adapters are separate from the interfaces that display requests and submit responses.

See the technical documentation:

- [Actor architecture](docs/actor-architecture.md)
- [Agent integrations](docs/integrations.md)

## License

AI Agent Monitor is free to use and distributed under the [Apache License 2.0](LICENSE).

Agent services and Telegram are separate from the monitor.
