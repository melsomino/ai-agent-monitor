<div align="center">
  <img src="assets/AppIcon-robots.png" width="128" alt="AI Agent Monitor — two robots">

  <h1>AI Agent Monitor</h1>

  <p><strong>Your coding agents, one menu away.</strong></p>
  <p>A native macOS menu bar app for Codex, Claude Code, and Cline.<br>See what is running, catch requests for input, and get back to the right project.</p>

  <p>
    <img src="https://img.shields.io/badge/macOS-13%2B-111827?style=flat-square&logo=apple&logoColor=white" alt="macOS 13 or later">
    <img src="https://img.shields.io/badge/built_with-Rust-dea584?style=flat-square&logo=rust&logoColor=white" alt="Built with Rust">
    <img src="https://img.shields.io/badge/UI-native_AppKit-147EFB?style=flat-square" alt="Native AppKit UI">
  </p>

  <p><a href="https://github.com/melsomino/ai-agent-monitor/releases/latest/download/AI-Agent-Monitor-apple-silicon.dmg"><strong>Download for Mac · Apple Silicon</strong></a></p>

  <p><a href="#features">Features</a> · <a href="#installation">Installation</a> · <a href="#connect-your-agents">Agent setup</a> · <a href="#using-the-monitor">Usage</a> · <a href="#troubleshooting">Troubleshooting</a></p>
</div>

## Features

- **All open sessions in one place.** Sessions are grouped by application and sorted by name. Working, waiting, and idle sessions stay visible; closed chat history stays out of the list.
- **Status at a glance.** Green robot eyes show activity in the menu bar. A count badge takes priority when requests need a response.
- **Useful details, inline.** Expand a session to see its latest message or pending question, working directory, and Git branch. Working sessions and pending requests expand automatically when you open the menu.
- **Answer without switching windows.** Approve supported requests, choose an offered answer, or open a form for custom text and multiple questions. Actions also appear in macOS notifications when supported.
- **Jump back to the project.** Click a session or notification to open its project in VS Code. Live VS Code workspace information takes priority over directories used by individual commands.
- **Keyboard navigation and instant filtering.** Move through sessions with the arrow keys, expand details, or start typing to filter titles. Matching words are highlighted.
- **A compact, live view.** The list refreshes in the background every two seconds. The menu follows its contents up to 80% of the screen height, then scrolls.
- **Your preferences, remembered.** Launch at login, choose a global shortcut, toggle menu bar animation, and hide sessions from selected applications.
- **Codex usage in the footer.** See the last reported remaining quota and reset date alongside the monitor's build version.

One Rust executable, native AppKit controls, and macOS notifications. No Python or Swift runtime is needed.

## Supported agents

| Agent | Session discovery | Respond from the monitor | Setup |
| --- | --- | --- | --- |
| **Codex** | Local CLI, VS Code, and desktop sessions | Approval actions, supported questions, custom answers | Automatic local discovery; optional approval hook for CLI requests |
| **Claude Code** | Lifecycle hooks in terminal and VS Code | One-time **Allow** for permission requests | Install hooks and restart Claude Code |
| **Cline** | Lifecycle hooks and the current task's messages | Open the project and respond in Cline | Install hooks and enable them in Cline |

The monitor runs on the Mac that hosts the agents. SSH, containers, and remote development environments need a separate delivery mechanism. Linux is not supported yet.

## Installation

### Download the app — no terminal required

1. Download the [latest DMG for Apple Silicon](https://github.com/melsomino/ai-agent-monitor/releases/latest/download/AI-Agent-Monitor-apple-silicon.dmg) from [GitHub Releases](https://github.com/melsomino/ai-agent-monitor/releases/latest).
2. Open the DMG and drag **AI Agent Monitor** to **Applications**.
3. Open the app from Applications. It connects detected agents automatically and requests notification permission.
4. Open **Settings → Integrations** to check status and next steps: restart the connected agents, review Codex's hook in `/hooks`, or enable Cline hooks if needed.

Requires **macOS 13+** and **Apple Silicon** for the prebuilt DMG. Agents are installed separately. Rust, Xcode, and terminal commands are not needed to run the downloaded app.

The current release is ad-hoc signed and **not notarized by Apple**. If macOS blocks the first launch, follow [Apple's instructions for opening an unnotarized app](https://support.apple.com/en-us/102445): after attempting to open it, use **System Settings → Privacy & Security → Open Anyway** if you trust this release. No system-wide security settings need to be disabled.

The robot icon appears in the menu bar. To start automatically next time, enable **Settings → General → Launch at login**. Setup can be rerun from **Settings → Integrations → Check & set up**, including after installing another agent. Existing unrelated hooks are preserved.

### Build from source

For development or a native Intel build, install Rust/Cargo with support for the **2024 edition** and Xcode Command Line Tools, then:

```sh
git clone https://github.com/melsomino/ai-agent-monitor.git
cd ai-agent-monitor

cargo run --release --locked -- bundle

mkdir -p "$HOME/Applications"
ditto "build/AI Agent Monitor.app" "$HOME/Applications/AI Agent Monitor.app"
```

The build creates a locally signed app bundle. Install it at its final location **before** configuring hooks: hooks store the absolute executable path.

Define these paths for the commands below:

```sh
monitor_app="$HOME/Applications/AI Agent Monitor.app"
monitor_bin="$monitor_app/Contents/MacOS/AIAgentMonitor"
```

Start the installed app; first-launch setup handles notification permission and detected integrations:

```sh
open -g "$monitor_app"
```

Normal launches keep only the robot icon in the menu bar. Click it to see your sessions.

## Connect your agents

**First-launch setup does this automatically for detected agents.** The commands below are optional: use them for custom configuration directories, manual setup, or diagnostics. For the DMG installation, the executable is `/Applications/AI Agent Monitor.app/Contents/MacOS/AIAgentMonitor`; set `monitor_bin` to that path before using these examples.

<details>
<summary><strong>Manual agent setup and custom paths</strong></summary>

### Codex

Open local Codex sessions are discovered automatically from local state and process activity. The VS Code and desktop integrations can expose live requests and questions through Codex's local client connection.

For approval notifications from Codex CLI, install the permission hook:

```sh
"$monitor_bin" install
```

Restart Codex CLI and VS Code. In Codex, open `/hooks`, review **macOS notification**, and trust the updated hook. Changing its executable path requires reviewing it again.

The installer preserves unrelated hooks, backs up the configuration, and avoids duplicate entries. It also replaces this project's older notification hooks.

By default, the monitor uses `$CODEX_HOME` or `~/.codex`. To install the hook in a different configuration directory:

```sh
"$monitor_bin" install --codex-home /path/to/codex-home
```

Run the monitor with the same `CODEX_HOME` when monitoring that configuration.

### Claude Code

```sh
"$monitor_bin" install-agent claude
```

Restart Claude Code to load its lifecycle and permission hooks. Sessions already open during installation appear after a new hook event.

The installer updates `$CLAUDE_CONFIG_DIR/settings.json` or `~/.claude/settings.json`, preserves existing hooks and permission rules, and creates a private backup. A custom configuration directory is supported:

```sh
"$monitor_bin" install-agent claude --home /path/to/claude-config
```

### Cline

```sh
"$monitor_bin" install-agent cline
```

In Cline, turn on **Enable Hooks**, then open or resume a task. Hooks are installed in `~/Documents/Cline/Hooks`; existing hook files from other tools are not overwritten. For a custom hook directory:

```sh
"$monitor_bin" install-agent cline --home /path/to/cline-hooks
```

Cline requests are displayed and generate notifications, but approval happens in Cline. Its public approval API cannot address a specific request, so the monitor does not offer an external **Allow** button.

</details>

## Using the monitor

### Sessions and navigation

Click the robot icon, or use your configured shortcut, to open the menu. Idle sessions are included and have no status icon. Hover over a status icon for its description.

- **Click the session title** to switch to its host. VS Code sessions open their project; terminal sessions activate their terminal app. Terminal and iTerm select the matching tab when it can be identified; Warp currently supports app activation only. macOS may ask for Automation permission to select terminal tabs.
- **Click the chevron** to expand or collapse details.
- **Click the small up-chevron in the left gutter** beside the first displayed message to prepend one earlier agent message. Repeat to read further back. History loads locally for Codex, Claude Code, and Cline; closing the menu resets it.
- **Click outside**, press **Escape**, or use the shortcut again to close the menu.
- Use **gear** in the footer to open settings, or **power** to quit the resident monitor.

Closing the menu clears the filter and collapses the sessions. On the next opening, working sessions and requests needing a response expand again. Quitting the resident monitor does not remove installed hooks. A later hook starts the installed monitor in the background when needed.

Project navigation opens the workspace in VS Code; it does not select a particular chat. If the project folder no longer exists, the monitor switches to VS Code without trying to open the missing path. If the same project is open in several windows, VS Code chooses the window.

### Keyboard controls

| Key | Action |
| --- | --- |
| **↑ / ↓** | Select the previous or next session |
| **Enter** | Open the selected session's project |
| **→** | Expand the selected session |
| **←** | Collapse the selected session |
| **Type text** | Filter session titles |
| **Backspace** | Delete the last filter character |
| **Escape** | Close the menu |

Filtering ignores case and splits the query on spaces. Every word must appear in the title: `api tests` matches titles containing both `api` and `tests`, in any order. The current query appears in the footer.

### Approvals and questions

Expand a pending request to read it and see its available actions beneath the message. Buttons follow the choices offered by the agent. When they fit, they share one row; otherwise they form a column.

For supported Codex requests, actions can include one-time approval, approval for a session, decline, cancellation, or proposed policy changes. Questions can offer answer buttons, a custom response, or a separate form for multiple questions. Secret answers use a secure input field in the form.

The monitor rechecks that the request is still current before sending a response. A successful response refreshes the session and removes its notification without closing the menu. Requests answered in the editor also disappear after the monitor observes the update. Unsupported request types direct you to the editor.

macOS notifications use actions specific to the request. Depending on the notification layout, these may appear in an actions menu. Larger sets of answers use **More answers** or open the full form. macOS controls how many actions and text lines are visible.

**Hook-based approvals:** hooks forward their events to the resident monitor and wait up to 60 seconds for a one-time **Allow**. The monitor owns the request, notification, and response to the agent. The same response handler serves notification buttons and the session menu. Dismissing a notification leaves the request in the monitor; expiry, disconnection, or restarting the monitor returns control without approval. A stale notification cannot approve a later request. Errors never grant permission automatically.

### Codex quota

The footer shows the latest locally reported quota, such as **Codex Weekly · 74% · Nov 4**. The percentage is **remaining**, not used. Hover for the report time and exact reset time.

This is a cached report from Codex sessions, checked every ten seconds, rather than a live account query. Usage on another device appears after Codex reports updated limits locally. After a reported window expires, the monitor shows `—` until another report arrives.

## Settings

Settings use a native preferences toolbar with three sections: **General**, **Applications**, and **Integrations**.

| Section | Setting | Behavior |
| --- | --- | --- |
| **General** | Launch at login | Registers a macOS Login Item; disabled by default |
| **General** | Menu bar animation | Animates green robot eyes while agents work; when disabled, working eyes stay green |
| **Integrations** | Check & set up | Inspect detected agents, hook status and errors, or retry setup |
| **General** | Keyboard shortcut | Records a global shortcut using ⌘, ⌥, or ⌃; **Escape** cancels recording and **×** clears it |
| **Applications** | Application switches | Show or hide that application's sessions in the menu |

The Integrations section shows detected agents with **Needs setup**, **Setting up…**, **Hooks installed**, or **Needs attention**. Status is checked against the current hook configuration; **Hooks installed** does not mean that Codex trust or Cline’s Enable Hooks setting has been confirmed. Required next steps appear beneath each agent. Setup runs in the background without a separate results window.

The application list fills automatically as sessions are discovered. Entries and your choices are retained after applications close and across monitor restarts. Hiding an application affects the session list; notifications and the menu bar indicator still account for its sessions.

macOS **Reduce Motion** also stops the menu bar animation. If macOS requires approval for a Login Item, settings provide a button to open the relevant system controls.

## Troubleshooting

### Notifications are in Notification Center, but no banner appears

Check **System Settings → Notifications → AI Agent Monitor** and allow banners or alerts. Check the current **Focus / Do Not Disturb** mode as well. A successful delivery can still be silent when macOS suppresses banners.

Inspect the notification configuration or send a harmless test:

```sh
"$monitor_bin" notification-settings
"$monitor_bin" test
"$monitor_bin" test --approval
```

The test **Allow** button only checks the response path; it does not grant any agent permissions. To use a specific Codex chat title in the test:

```sh
"$monitor_bin" test --approval --session-id THREAD_ID
```

### A session or approval is missing

Check that the agent runs locally. For Claude Code, restart after hook installation; for Cline, enable hooks and reopen the task. For Codex CLI approvals, review the hook's trust in `/hooks`.

```sh
"$monitor_bin" sessions --json
"$monitor_bin" check-hooks
```

`sessions --json` prints discovered open sessions without opening the menu; application visibility preferences only filter the window. `check-hooks` inspects Codex hook discovery and trust without starting a model. To inspect another Codex executable:

```sh
"$monitor_bin" check-hooks --binary /path/to/codex
```

Codex's local client protocol is version-dependent. If a live request cannot be read or answered, use the agent's own interface. The monitor falls back to logs and hooks when possible. Historical chats are not added as substitutes for unavailable session data.

### Hooks stopped working after moving the app

Run the corresponding `install` or `install-agent` command from the app's **new location**, then restart the agent. Review updated Codex hooks again in `/hooks`.

## Updating and uninstalling

For a downloaded app, quit the monitor using **power**, download the new DMG, and replace the app in Applications. Preferences are retained. When its location is unchanged, existing hooks keep working; setup reconnects agents if the executable path changes.

For a source build, quit the monitor, pull the latest code, rebuild, and replace the installed bundle:

```sh
git pull --ff-only
cargo run --release --locked -- bundle
ditto "build/AI Agent Monitor.app" "$HOME/Applications/AI Agent Monitor.app"
open -g "$HOME/Applications/AI Agent Monitor.app"
```

If the app path is unchanged, existing hooks keep pointing to it. If an integration changes in an update, run its installer again and restart the agent.

To uninstall, turn off **Launch at login**, quit the monitor, and remove its hook entries before deleting the app. For Codex, disable the hook in `/hooks` or remove only the `AIAgentMonitor hook` handler from `hooks.PermissionRequest` in `hooks.json`. For Claude Code, remove this monitor's hook commands from its settings. For Cline, remove only wrappers installed by this project from its Hooks directory. Keep unrelated hooks intact.

## Development

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

`build.rs` increments `build-number.txt` on each Cargo build, including test and check builds. The compiled version, such as `0.1.0+44`, appears in `--version` and the menu footer; its numeric build suffix is stored in the bundle's `CFBundleVersion`. Packaging the same binary preserves its compiled build number.

```text
src/
├── app/         CLI, shared utilities, app bundling, and hook installation
├── agents/      Common session/interaction models and agent providers
│   ├── codex/   Local session discovery, live requests, questions, and usage
│   ├── claude/  Claude Code lifecycle and permission hooks
│   └── cline/   Cline hooks and current-task state
└── macos/       Menu, notifications, settings, shortcuts, and answer forms
```

To create a distributable disk image for the build machine's architecture:

```sh
cargo run --release --locked -- dmg
```

This packages the signed app and an Applications shortcut into `build/release/AI-Agent-Monitor-apple-silicon.dmg` (or `AI-Agent-Monitor-intel.dmg` for an Intel build), alongside `SHA256SUMS`. DMG packaging does not grant a Developer ID signature or notarization.

See [Integration notes](docs/integrations.md) for discovery, response validation, local storage, and provider extension details.
