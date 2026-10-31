<div align="center">
  <img src="assets/AppIcon-robots.png" width="128" alt="AI Agent Monitor — two robots">

  <h1>AI Agent Monitor</h1>
  <!-- Add a real demo GIF here once recorded: open sessions → permission request → Allow → project. -->

  <p><strong>Your coding agents, one menu away.</strong></p>
  <p>A native macOS menu bar app for your Codex sessions.<br>See what’s working, answer approval requests, and jump back to the right project.<br>Experimental support for Claude Code and Cline.</p>

  <p>
    <img src="https://img.shields.io/badge/macOS-13%2B-111827?style=flat-square&logo=apple&logoColor=white" alt="macOS 13 or later">
    <img src="https://img.shields.io/badge/built_with-Rust-dea584?style=flat-square&logo=rust&logoColor=white" alt="Built with Rust">
    <img src="https://img.shields.io/badge/UI-native_AppKit-147EFB?style=flat-square" alt="Native AppKit UI">
  </p>

  <p><a href="https://github.com/melsomino/ai-agent-monitor/releases/latest/download/AI-Agent-Monitor-apple-silicon.dmg"><strong>Download for Mac · Apple Silicon</strong></a></p>
  <p>Free to use · Open source under <a href="LICENSE">Apache-2.0</a></p>

  <p><a href="#features">Features</a> · <a href="#installation">Installation</a> · <a href="#privacy--permissions">Privacy</a> · <a href="#connect-your-agents">Agent setup</a> · <a href="#using-the-monitor">Usage</a> · <a href="#troubleshooting">Troubleshooting</a></p>
</div>

## Features

- **All open sessions in one place.** See your Codex sessions across VS Code, terminal apps, and the desktop client, grouped by application and sorted by name.
- **Status at a glance.** A green number counts working sessions, green robot eyes show activity, and an amber counter beside the icon counts requests waiting for a response.
- **Answer from the menu or a notification.** Read approval requests and questions, choose an offered answer, or enter a custom reply without switching windows.
- **Return to the right project.** Jump to a VS Code workspace or the session’s terminal app, with matching-tab selection in Terminal and iTerm.
- **Take questions to Telegram.** Connect your own bot, answer remotely, or browse open sessions with the **Sessions** button. Resolved question messages are removed automatically.

One Rust executable, native AppKit controls, and macOS notifications. No Python or Swift runtime is needed.

## Supported agents

| Agent | Session discovery | Respond from the monitor | Setup |
| --- | --- | --- | --- |
| **Codex** | Local CLI, VS Code, and desktop sessions | Approval actions, supported questions, custom answers | Automatic local discovery; optional approval hook for CLI requests |
| **Claude Code** *(experimental)* | Lifecycle hooks in terminal and VS Code | One-time **Allow** for permission requests | Install hooks and restart Claude Code |
| **Cline** *(experimental)* | Lifecycle hooks and the current task's messages | Open the project and respond in Cline | Install hooks and enable them in Cline |

**Claude Code and Cline integrations are experimental.** Discovery, activity tracking, and hook behavior may vary with agent and extension versions. For Cline, requests must be answered in Cline; external approval is not supported.

The monitor runs on the Mac that hosts the agents. SSH, containers, and remote development environments need a separate delivery mechanism. Linux is not supported yet.

## Telegram notifications

Use your own Telegram bot to receive agent questions and answer them away from the Mac. No hosted relay is needed; the resident monitor connects directly to Telegram using long polling.

1. Open [@BotFather](https://t.me/BotFather) in Telegram, send `/newbot`, and create a dedicated bot.
2. In **Settings → Telegram**, paste the bot token and click **Connect**. The token is stored in macOS Keychain.
3. Click **Open pairing link**, then press **Start** in your bot. The one-time pairing link expires after 10 minutes.
4. Click **Send test** to verify delivery. **Send questions to Telegram** pauses or resumes the integration; **Disconnect** removes the saved token and pairing.

The bot's **Menu** button lists `/sessions`, `/status`, `/info`, and `/help`. **Sessions** (or `/sessions`) opens an interactive list of currently open sessions, including idle ones, using the same application filters and ordering as the menu. Each session button shows its application symbol before the title and its status before the disclosure arrow. Select a session to view its latest message and current question with answer buttons. The header shows only the application symbol, session title, and active status. **Info** (or `/info`) shows the working folder and Git branch in a separate view. New agent output is posted at the bottom of the chat, with the previous session message kept and its buttons removed. Status changes update the current message. **Previous message** steps back through agent messages; **Latest message** returns to the live view. Plain chat text is sent to the selected session through the monitor. This currently requires a live Codex connection (VS Code or desktop); unsupported integrations report an error. Use Reply on a question message to enter a custom answer. Use **‹ Sessions** to return to the list; open views update automatically. Large lists are paginated. `/status` shows the monitor version and notification state. Command replies also include a button keyboard. The session list and status commands remain available while Telegram question notifications are paused.

Agent messages and question bodies render Markdown with Telegram text entities: headings, bold and italic text, lists, links, quotations, and inline or fenced code. This also applies to previous messages.

Approval messages use the agent's offered answer buttons. For a question that accepts a custom answer, reply to its bot message. Multiple questions are presented one at a time and submitted together. Questions marked as secret must be answered in the agent app.

Only the paired private Telegram account can reply. The monitor rechecks the live request before forwarding an answer, and deletes its bot message when a request is resolved, including answers made in the editor or monitor. If Telegram cannot delete the message, the monitor replaces its text and removes the answer buttons. Temporary cleanup failures are retried. Your Mac must be awake and the monitor running to deliver notifications or process replies. Session titles and question text, and viewed messages when browsing sessions, and working folder and Git branch when opening Info, are sent through Telegram; this integration is opt-in. Use a separate bot for each monitor instance: Telegram polling cannot be shared with another bot client or an active webhook.

## Installation

### Download the app — no terminal required

1. Download the [latest DMG for Apple Silicon](https://github.com/melsomino/ai-agent-monitor/releases/latest/download/AI-Agent-Monitor-apple-silicon.dmg) from [GitHub Releases](https://github.com/melsomino/ai-agent-monitor/releases/latest).
2. Open the DMG and drag **AI Agent Monitor** to **Applications**.
3. Open the app from Applications. It connects detected agents automatically and requests notification permission.
4. Open **Settings → Integrations** to check status and next steps: restart the connected agents, review Codex's hook in `/hooks`, or enable Cline hooks if needed.

Requires **macOS 13+** and **Apple Silicon** for the prebuilt DMG. Agents are installed separately. Rust, Xcode, and terminal commands are not needed to run the downloaded app.

Official release DMGs are signed with Developer ID and notarized by Apple. Download the DMG from this repository’s GitHub Releases; source builds signed ad hoc are intended for local development.

The robot icon appears in the menu bar. To start automatically next time, enable **Settings → General → Launch at login**. Setup can be rerun from **Settings → Integrations → Check & set up**, including after installing another agent. Existing unrelated hooks are preserved.

### Build from source

For development or a native Intel build, install Rust/Cargo with support for the **2024 edition** and Xcode Command Line Tools, then:

```sh
git clone https://github.com/melsomino/ai-agent-monitor.git
cd ai-agent-monitor

AI_AGENT_MONITOR_SIGN_IDENTITY=- cargo run --release --locked -- bundle

mkdir -p "$HOME/Applications"
ditto "build/AI Agent Monitor.app" "$HOME/Applications/AI Agent Monitor.app"
```

This command creates an ad-hoc signed app bundle for local development. Install it at its final location **before** configuring hooks: hooks store the absolute executable path.

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

## Privacy & permissions

- **Local session data.** Discovery, message history, Git metadata, and Codex quota are read on your Mac. Passive monitoring does not start model turns. Answers are sent to the existing agent only after you explicitly choose or submit one.
- **Hook configuration.** First-launch setup connects detected agents by updating their hook configuration. Modified Codex and Claude configuration is backed up; unrelated hooks are preserved, and existing third-party Cline hook files are not overwritten. Review the result in **Settings → Integrations**.
- **macOS permissions.** Notification permission enables banners and answer buttons. macOS may request Automation permission to select tabs in Terminal or iTerm. Launch at login is optional and controlled in settings.
- **Optional Telegram delivery.** The monitor connects directly to your bot; there is no project-hosted relay. Once paired and enabled, session titles, question text, and answer choices are sent to Telegram, and replies return through the bot. The bot token is stored in macOS Keychain. Pausing question delivery still allows explicitly requested `/sessions`, `/status`, and `/help` replies to the paired account.

## Price & license

AI Agent Monitor is **free to download and use**. The source code is available under the **Apache License 2.0**; see [LICENSE](LICENSE) for the full terms. Agent services and Telegram are separate from the monitor.

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

### Claude Code (experimental)

```sh
"$monitor_bin" install-agent claude
```

Restart Claude Code to load its lifecycle and permission hooks. Sessions already open during installation appear after a new hook event.

The installer updates `$CLAUDE_CONFIG_DIR/settings.json` or `~/.claude/settings.json`, preserves existing hooks and permission rules, and creates a private backup. A custom configuration directory is supported:

```sh
"$monitor_bin" install-agent claude --home /path/to/claude-config
```

### Cline (experimental)

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

The menu includes working, waiting, and idle sessions; closed chat history stays out of the list. Sessions refresh in the background every two seconds. The window follows its content up to 80% of the screen height, then scrolls.

Sessions use a flat list with application icons in the left gutter instead of group headings. Each icon stays pinned while its application's sessions scroll past, then gives way to the next application. Subtle separators appear only between applications. Session titles are semibold; messages use neutral text, and folder/branch metadata stays at the bottom of the expanded session.

Expand a session to read its latest message, working directory, and Git branch. Pending questions appear in a separate panel below the messages. Working sessions and requests needing a response expand automatically when you open the menu. The green working count disappears when no sessions are working; it remains visible alongside the red request badge when both states are present.

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
| **↑ / ↓** | Move between sessions and enabled answer buttons in expanded sessions |
| **Enter** | Open the selected session's editor or terminal, or activate the selected answer |
| **Space on a preset answer** | Choose it, just like Enter; Space in a custom-answer field inserts a space |
| **→** | Expand the selected session; on a horizontal answer row, select the next answer (wraps) |
| **←** | Collapse the selected session; on a horizontal answer row, select the previous answer (wraps) |
| **Type text** | Filter session titles |
| **Backspace** | Delete the last filter character |
| **Escape** | Close the menu |

When the menu opens with pending requests, the first enabled answer button is selected and scrolled into view.

In a custom-answer field, **↑ / ↓** finish editing and continue menu navigation without discarding the text. **← / →** move the text cursor.
Custom-answer fields use a system focus ring. **Send answer** remains in the navigation order when answers are incomplete; sending is enabled only when every required answer is valid.

Filtering ignores case and splits the query on spaces. Every word must appear in the title: `api tests` matches titles containing both `api` and `tests`, in any order. The current query appears in the footer.

### Approvals and questions

Expand a pending request to read it and see its available actions beneath the message. Buttons follow the choices offered by the agent. When they fit, they share one row; otherwise they form a column.

For supported Codex requests, actions can include one-time approval, approval for a session, decline, cancellation, or proposed policy changes. A single question lists answer buttons vertically, followed by an optional **Your own answer** field with **Send answer** beside it. Clicking a preset sends it immediately; the button beside the field sends only the entered text. Multiple questions appear directly in the menu, each with radio choices and an optional **Your own answer** field; **Send answer** submits all answers together. Selections and text survive menu refreshes. Secret answers use secure input fields. Answer actions in macOS notifications open the request in the sessions menu.

The monitor rechecks that the request is still current before sending a response. A successful response refreshes the session and removes its notification without closing the menu. Requests answered in the editor also disappear after the monitor observes the update. Unsupported request types direct you to the editor.

macOS notifications use actions specific to the request. Depending on the notification layout, these may appear in an actions menu. **Answer questions** and **More answers** open the sessions menu with the request expanded. macOS controls how many actions and text lines are visible.

**Hook-based approvals:** hooks forward their events to the resident monitor and wait up to 60 seconds for a one-time **Allow**. The monitor owns the request, notification, and response to the agent. The same response handler serves notification buttons and the session menu. Dismissing a notification leaves the request in the monitor; expiry, disconnection, or restarting the monitor returns control without approval. A stale notification cannot approve a later request. Errors never grant permission automatically.

### Codex quota

The footer shows the latest locally reported quota, such as **Codex Weekly · 74% · Nov 4**. The percentage is **remaining**, not used. Hover for the report time and exact reset time.

This is a cached report from Codex sessions, checked every ten seconds, rather than a live account query. Usage on another device appears after Codex reports updated limits locally. After a reported window expires, the monitor shows `—` until another report arrives.

## Settings

Settings use a native preferences toolbar with four sections: **General**, **Applications**, **Integrations**, and **Telegram**.

| Section | Setting | Behavior |
| --- | --- | --- |
| **General** | Launch at login | Registers a macOS Login Item; disabled by default |
| **General** | Menu bar animation | Animates green robot eyes while agents work; when disabled, working eyes stay green |
| **Integrations** | Check & set up | Inspect detected agents, hook status and errors, or retry setup |
| **General** | Keyboard shortcut | Records a global shortcut using ⌘, ⌥, or ⌃; **Escape** cancels recording and **×** clears it |
| **Applications** | Application switches | Show or hide that application's sessions in the menu |
| **Telegram** | Personal bot | Connect and pair your bot, pause or resume questions, send a test, or disconnect |

The Integrations section shows detected agents with **Needs setup**, **Setting up…**, **Hooks installed**, or **Needs attention**. Status is checked against the current hook configuration; **Hooks installed** does not mean that Codex trust or Cline’s Enable Hooks setting has been confirmed. Required next steps appear beneath each agent. Setup runs in the background without a separate results window.

The application list fills automatically as sessions are discovered. Entries and your choices are retained after applications close and across monitor restarts. Hiding an application removes its sessions from the list and the green working count. Notifications and the red pending-request badge still include its requests. Typing a title filter only changes the list.

macOS **Reduce Motion** also stops the menu bar animation. If macOS requires approval for a Login Item, settings provide a button to open the relevant system controls.

## Troubleshooting

### Notifications are in Notification Center, but no banner appears

Check **System Settings → Notifications → AI Agent Monitor** and allow banners or alerts. Check the current **Focus / Do Not Disturb** mode as well. A successful delivery can still be silent when macOS suppresses banners.

### A session or approval is missing

Check that the agent runs locally. For Claude Code, restart after hook installation; for Cline, enable hooks and reopen the task. For Codex CLI approvals, review the hook's trust in `/hooks`.

Open **Settings → Integrations** to inspect agent status and errors, then use **Check & set up** to retry setup. No terminal commands are required.

Codex's local client protocol is version-dependent. If a live request cannot be read or answered, use the agent's own interface. The monitor falls back to logs and hooks when possible. Historical chats are not added as substitutes for unavailable session data. If an agent adapter stops, its sessions are marked unavailable and replies are disabled; restart the monitor to reconnect.

### Hooks stopped working after moving the app

Open the app from its **new location**, then use **Settings → Integrations → Check & set up** and restart the agent. Review updated Codex hooks again in `/hooks`.

## Updating and uninstalling

For a downloaded app, quit the monitor using **power**, download the new DMG, and replace the app in Applications. Preferences are retained. When its location is unchanged, existing hooks keep working; setup reconnects agents if the executable path changes.

For a source build, quit the monitor, pull the latest code, rebuild, and replace the installed bundle:

```sh
git pull --ff-only
AI_AGENT_MONITOR_SIGN_IDENTITY=- cargo run --release --locked -- bundle
ditto "build/AI Agent Monitor.app" "$HOME/Applications/AI Agent Monitor.app"
open -g "$HOME/Applications/AI Agent Monitor.app"
```

If the app path is unchanged, existing hooks keep pointing to it. If an integration changes in an update, use **Settings → Integrations → Check & set up** and restart the agent.

To uninstall, turn off **Launch at login**, quit the monitor, and remove its hook entries before deleting the app. For Codex, disable the hook in `/hooks` or remove only the `AIAgentMonitor hook` handler from `hooks.PermissionRequest` in `hooks.json`. For Claude Code, remove this monitor's hook commands from its settings. For Cline, remove only wrappers installed by this project from its Hooks directory. Keep unrelated hooks intact.

## Development

```sh
cargo +nightly fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Nightly is used for formatting; ordinary builds do not require a nightly toolchain.

`build.rs` increments `build-number.txt` on each Cargo build, including test and check builds. The compiled version, such as `0.1.0+44`, appears in `--version` and the menu footer; its numeric build suffix is stored in the bundle's `CFBundleVersion`. Packaging the same binary preserves its compiled build number.

```text
src/
├── domain/      Pure session, interaction, preference, and status models
├── actors/      Monitor state owner, agent adapters, and human interactors
├── app/         Runtime bootstrap, IPC, CLI, bundling, and hook installation
├── agents/      Provider protocols, discovery, and owned caches
│   ├── codex/   Local session discovery, live requests, questions, and usage
│   ├── claude/  Claude Code lifecycle and permission hooks
│   └── cline/   Cline hooks and current-task state
└── macos/       Menu, notifications, settings, shortcuts, and answer forms
```

To create an ad-hoc signed disk image for local testing on the build machine’s architecture:

```sh
AI_AGENT_MONITOR_SIGN_IDENTITY=- cargo run --release --locked -- dmg
```

This packages the app and an Applications shortcut into `build/release/AI-Agent-Monitor-apple-silicon.dmg` (or `AI-Agent-Monitor-intel.dmg` for an Intel build), alongside `SHA256SUMS`. The command above uses ad-hoc signing and does not notarize the app. For public distribution, use the signed and notarized release script below.

### Signed and notarized releases

Run the complete release pipeline from the repository root:

```sh
./scripts/release-macos.sh
```

The script lists valid **Developer ID Application** signing identities from Keychain
and asks you to select one by number, even when only one is available. It then
builds the current architecture, signs the app and DMG with the selected identity,
submits the DMG to Apple, requires an `Accepted` result, staples the ticket to the
DMG, and checks the signature, ticket, disk image, and Gatekeeper assessment.
`SHA256SUMS` is regenerated only after these checks, because signing and stapling
change the DMG. Upload the resulting DMG and `SHA256SUMS` from `build/release` to
GitHub Releases; the script does not publish or tag anything. The app inside the
DMG is covered by notarization; the offline ticket is attached to the DMG.

Once per build Mac, save notarization credentials in Keychain (the command prompts
for an Apple app-specific password; do not put passwords in scripts):

```sh
xcrun notarytool store-credentials ai-agent-monitor-notary \
  --apple-id YOUR_EMAIL --team-id YOUR_TEAM_ID
```

The default profile is `ai-agent-monitor-notary`. Override it with
`AI_AGENT_MONITOR_NOTARY_PROFILE`, and override the signing certificate with
`AI_AGENT_MONITOR_SIGN_IDENTITY` for non-interactive certificate selection. The
override must match the SHA-1 or full name of an available Developer ID Application
identity. Ad-hoc signing is rejected. Submission
receipts and rejection logs are retained in `build/notarization` for diagnostics.
If any step fails, do not publish that artifact. Fix the problem and rerun the
script. Each new build needs notarization; do not rebuild, re-sign, or modify a
completed release artifact.

The resident uses Tokio actors with one-way commands and state subscriptions. Window controllers, macOS notifications, and Telegram share the Monitor state; no application state is stored in global variables. See the [actor architecture](docs/actor-architecture.md) for the design.

See [Integration notes](docs/integrations.md) for discovery, response validation, local storage, and provider extension details.
