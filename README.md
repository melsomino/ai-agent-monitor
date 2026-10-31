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

  <p><a href="#features">Features</a> · <a href="#installation">Installation</a> · <a href="#privacy--permissions">Privacy</a> · <a href="#connect-your-agents">Agent setup</a> · <a href="#using-the-monitor">Usage</a> · <a href="#telegram">Telegram</a> · <a href="#troubleshooting">Troubleshooting</a></p>
</div>

## Features

- **All open sessions in one place.** See your Codex sessions across VS Code, terminal apps, and the desktop client, arranged by application and sorted by name, with pinned application icons.
- **Status at a glance.** A green number counts working sessions, green robot eyes show activity, and an orange counter beside the icon counts requests waiting for a response.
- **Answer from the menu or a notification.** Read approval requests and questions, choose an offered answer, or enter a custom reply without switching windows.
- **Return to the right project.** Jump to a VS Code workspace or the session’s terminal app, with matching-tab selection in Terminal and iTerm.
- **Take questions to Telegram.** Connect your own bot, browse sessions and message history, answer questions, and send messages to a live Codex session. Resolved standalone question notifications are removed automatically.

One Rust executable, native AppKit controls, and macOS notifications. No Python or Swift runtime is needed.

## Supported agents

| Agent | Session discovery | Respond from the monitor | Setup |
| --- | --- | --- | --- |
| **Codex** | Local CLI, VS Code, and desktop sessions | Approval actions, supported questions, custom answers | Automatic local discovery and direct client connection; no hooks |
| **Claude Code** *(experimental)* | Lifecycle hooks in terminal and VS Code | One-time **Allow** for permission requests | Install hooks and restart Claude Code |
| **Cline** *(experimental)* | Lifecycle hooks and the current task's messages | Open the project and respond in Cline | Install hooks and enable them in Cline |

**Claude Code and Cline integrations are experimental.** Discovery, activity tracking, and hook behavior may vary with agent and extension versions. For Cline, requests must be answered in Cline; external approval is not supported.

The monitor runs on the Mac that hosts the agents. SSH, containers, and remote development environments need a separate delivery mechanism. Linux is not supported yet.

## Installation

### Download the app — no terminal required

1. Download the [latest DMG for Apple Silicon](https://github.com/melsomino/ai-agent-monitor/releases/latest/download/AI-Agent-Monitor-apple-silicon.dmg) from [GitHub Releases](https://github.com/melsomino/ai-agent-monitor/releases/latest).
2. Open the DMG and drag **AI Agent Monitor** to **Applications**.
3. Open the app from Applications. It connects detected agents automatically and requests notification permission.
4. Open **Settings → Integrations** to check status and next steps: restart Claude Code or enable Cline hooks if needed. Codex connects directly without hooks.

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

- **Local session data.** Discovery, message history, Git metadata, and Codex quota are read on your Mac. Passive monitoring does not start model turns. Answers and chat messages are sent to the existing agent only after you explicitly choose or submit them.
- **Hook configuration.** Codex connects directly without modifying its hook configuration. First-launch setup connects Claude Code and Cline through hooks. Modified Claude configuration is backed up; unrelated hooks are preserved, and existing third-party Cline hook files are not overwritten. Review the result in **Settings → Integrations**.
- **macOS permissions.** Notification permission enables banners and answer buttons. macOS may request Automation permission to select tabs in Terminal or iTerm. Launch at login is optional and controlled in settings.
- **Optional Telegram delivery.** The monitor connects directly to your bot; there is no project-hosted relay. Once paired and enabled, session titles, question text, approval commands, permission scopes, and answer choices are sent to Telegram, and replies return through the bot. The bot token is stored in macOS Keychain. Browsing sessions and opening **Info** also sends the requested message history or folder/branch metadata. Pausing question delivery still allows read-only commands from the paired account.

## Price & license

AI Agent Monitor is **free to download and use**. The source code is available under the **Apache License 2.0**; see [LICENSE](LICENSE) for the full terms. Agent services and Telegram are separate from the monitor.

## Connect your agents

**First-launch setup does this automatically for detected agents.** The commands below are optional: use them for custom configuration directories, manual setup, or diagnostics. For the DMG installation, the executable is `/Applications/AI Agent Monitor.app/Contents/MacOS/AIAgentMonitor`; set `monitor_bin` to that path before using these examples.

<details>
<summary><strong>Manual agent setup and custom paths</strong></summary>

### Codex

Open local Codex sessions are discovered automatically from local state and process activity. Live requests, approval choices, and supported questions are read and answered through Codex's local client connection. No permission hook, installation command, or hook trust prompt is required.

By default, the monitor uses `$CODEX_HOME` or `~/.codex`. To monitor another configuration directory, launch the monitor with the same `CODEX_HOME` as Codex. The client protocol is version-dependent; if a request is unavailable through it, answer in Codex itself.

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

Sessions use a flat list with application icons in the left gutter instead of group headings. Each application icon stays pinned while its sessions scroll underneath, then gives way to the next application. Small robot markers identify individual sessions in the same gutter; their eyes are green while working, orange while waiting for a response, and neutral while idle. Subtle separators appear only between applications. Session titles are semibold; messages use neutral text, and folder/branch metadata stays at the bottom of the expanded session.

Expand a session to read its latest message, working directory, and Git branch. Pending questions appear in a separate panel below the messages. Working sessions and requests needing a response expand automatically when you open the menu. The green working count disappears when no sessions are working; it remains visible alongside the orange request counter when both states are present. Request indicators in the menu bar, session header, robot eyes, and question panel all use the same system orange.

Click the robot icon, or use your configured shortcut, to open the menu. Idle sessions are included and have no status icon beside the disclosure arrow. Hover over a status icon for its description.

- **Click the session title** to switch to its host. VS Code sessions open their project; terminal sessions activate their terminal app. Terminal and iTerm select the matching tab when it can be identified; Warp currently supports app activation only. macOS may ask for Automation permission to select terminal tabs.
- **Click the robot or chevron** to expand or collapse details. Title hover and keyboard focus share a rounded highlight ending just after the title. The robot marker has its own rounded hover, like the disclosure button; status and disclosure remain outside the title highlight.
- **Click the small up-chevron in the left gutter** beside the first displayed message to prepend one earlier agent message. Repeat to read further back. History loads locally for Codex, Claude Code, and Cline; closing the menu resets it.
- A thin gutter line connects each expanded session to its content. An orange indicator marks the pending question panel; small filled dots mark the second and subsequent messages in the displayed history.
- **Click outside**, press **Escape**, or use the shortcut again to close the menu.
- Use **gear** in the footer to open settings, or **power** to quit the resident monitor.

Closing the menu clears the filter and collapses the sessions. On the next opening, working sessions and requests needing a response expand again. Quitting the resident monitor does not remove installed hooks. A later hook starts the installed monitor in the background when needed.

Project navigation opens the workspace in VS Code; it does not select a particular chat. If the project folder no longer exists, the monitor switches to VS Code without trying to open the missing path. If the same project is open in several windows, VS Code chooses the window.

### Messages and history

Agent output is rendered as native formatted text, including headings, bold and italic text, lists, links, quotations, and inline or fenced code. Markdown is applied to previous messages as well. Reading history does not start a new agent turn.

Folder and Git branch details stay below the messages and pending question panel. Home-directory paths use `~` to keep long paths compact. The question panel supplements the latest message rather than replacing it.

### Keyboard controls

| Key | Action |
| --- | --- |
| **↑ / ↓** | Move between session headers, answer choices, custom-answer fields, and buttons |
| **Enter** | Open the selected session's editor or terminal, or activate the selected answer |
| **Space on a preset answer** | Choose it, just like Enter; Space in a custom-answer field inserts a space |
| **→** | On a session header, expand it; on a horizontal answer row, select the next answer (wraps) |
| **←** | On a session header, collapse it; on a horizontal answer row, select the previous answer (wraps) |
| **Type text** | Filter session titles |
| **Backspace** | Delete the last filter character |
| **Escape** | Close the menu |

When the menu opens with pending requests, the first enabled answer button is selected and scrolled into view.

In a custom-answer field, **↑ / ↓** finish editing and continue menu navigation without discarding the text. **← / →** move the text cursor.
**Enter** in a custom-answer field selects that answer and keeps the menu open; it does not submit the form. Only one element has keyboard focus at a time. Buttons and answer choices use the system accent color; custom-answer fields use a system focus ring. **Send answer** remains in the navigation order when answers are incomplete; sending is enabled only when every required answer is valid.

Filtering ignores case and splits the query on spaces. Every word must appear in the title: `api tests` matches titles containing both `api` and `tests`, in any order. The current query appears in the footer.

### Approvals and questions

Command approvals show the command in a selectable monospaced block below the question. When Codex offers **Allow similar commands**, its allowed command prefix is shown separately from the full command. Telegram includes the command and readable permission scope too.

Expand a pending request to read it and see its available actions beneath the message. Buttons follow the choices offered by the agent. When they fit, they share one row; otherwise they form a column.

For supported Codex requests, actions can include one-time approval, approval for a session, decline, cancellation, or proposed policy changes. A single question lists answer buttons vertically, followed by an optional **Your own answer** field with **Send answer** beside it. Clicking a preset sends it immediately; the button beside the field sends only the entered text. Multiple questions appear directly in the menu, each with radio choices and an optional **Your own answer** field; **Send answer** submits all answers together. Selections and text survive menu refreshes. Secret answers use secure input fields. Answer actions in macOS notifications open the request in the sessions menu.

The monitor rechecks that the request is still current before sending a response. A successful response refreshes the session and removes its notification without closing the menu. When the focused answer disappears, keyboard focus clears instead of moving to the session title. Requests answered in the editor also disappear after the monitor observes the update. Unsupported request types direct you to the editor.

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

The Integrations section shows Codex as **No setup required**. Claude Code and Cline show **Needs setup**, **Setting up…**, **Hooks installed**, or **Needs attention**. Hook status is checked against the current configuration; **Hooks installed** does not confirm Cline’s Enable Hooks setting. Required next steps appear beneath each agent. Setup runs in the background without a separate results window.

The application list fills automatically as sessions are discovered. Entries and your choices are retained after applications close and across monitor restarts. Hiding an application removes its sessions from the list and the green working count. Notifications and the orange pending-request counter still include its requests. Typing a title filter only changes the list.

macOS **Reduce Motion** also stops the menu bar animation. If macOS requires approval for a Login Item, settings provide a button to open the relevant system controls.

## Telegram

Connect your own bot to receive questions, browse sessions, and reply away from the Mac. The resident monitor connects directly to Telegram using long polling; no hosted relay is required. Your Mac must be awake and the monitor running.

### Connect your bot

1. Open [@BotFather](https://t.me/BotFather), send `/newbot`, and create a dedicated bot.
2. In **Settings → Telegram**, paste its token and click **Connect**. The token is stored in macOS Keychain.
3. Click **Open pairing link**, then press **Start** in your bot. The one-time link expires after 10 minutes.
4. Click **Send test** to verify delivery.

**Send questions to Telegram** pauses or resumes question delivery. **Disconnect** removes the saved token and pairing. Use a separate bot for each monitor instance: another polling client or an active webhook cannot share the bot connection.

### Browse and chat

Press **Sessions** to open the interactive session list. It includes idle sessions and follows the same application filters and ordering as the Mac menu. Each row shows an application symbol, session title, status, and disclosure arrow. Telegram uses symbols rather than the Mac's actual application images.

Select a session to read its latest message and pending request. Available controls include:

- **Answer buttons** — use the choices offered by the agent.
- **Previous message / Latest message** — read earlier output or return to the live view.
- **Info / Message** — show the working folder and Git branch, or return to the conversation.
- **‹ Sessions** — return to the list. Large lists are paginated.

Views update automatically; there are no Refresh buttons. When new agent output or a new request arrives in the selected session, it is posted at the bottom of the chat. The previous message stays in the conversation with its buttons removed. Status changes update the current message. A request already displayed in the selected session is not sent again as a standalone notification; an earlier standalone copy is removed when the request is shown there.

The selected session is stored separately from notification messages and restored after a monitor restart. A request from another session does not change the destination for chat text. Opening **Sessions** or returning to the list clears the destination; text sent without a selection produces a hint instead of being silently ignored.

**Type in chat to send a message to the selected agent session.** This requires a live Codex connection in VS Code or the desktop client; unsupported integrations report an error. To answer a question with your own text instead, use Telegram's **Reply** on that question message. Multiple questions are presented one at a time and submitted together. Questions marked as secret must be answered in the agent app.

Messages, history, and question bodies support formatted headings, bold and italic text, lists, links, quotations, and inline or fenced code.

### Commands

The persistent button keyboard contains **Sessions** and **Help**, without slash prefixes. Telegram's **Menu** lists these slash commands:

| Command | Action |
| --- | --- |
| `/sessions` | Open the active session list |
| `/info` | Show the selected session's working folder and Git branch |
| `/help` | Show available commands and usage |

`/status` is also accepted when typed manually and reports the monitor version and notification state; it is not listed in the main menu. Read-only commands remain available while question delivery is paused. After an update, press **Help** to replace a previously saved button keyboard.

### Answers and cleanup

Only the paired private Telegram account can reply. Every answer goes through the monitor's shared request validation, just like an answer from the Mac menu or a macOS notification.

When a request is resolved, including in the editor, the monitor deletes its standalone question notification. If deletion is unavailable, it replaces the text and removes the answer buttons; temporary cleanup failures are retried. Session conversation messages are retained, with their controls updated or removed as appropriate.

Telegram is opt-in. Session titles, questions, and answer choices are sent through your bot; browsing sends the requested messages, and **Info** sends the working folder and Git branch. See [Privacy & permissions](#privacy--permissions).

## Troubleshooting

### Notifications are in Notification Center, but no banner appears

Check **System Settings → Notifications → AI Agent Monitor** and allow banners or alerts. Check the current **Focus / Do Not Disturb** mode as well. A successful delivery can still be silent when macOS suppresses banners.

### The notification icon stays outdated

The app now uses `com.melsomino.ai-agent-monitor` as its macOS bundle identifier. When upgrading from the original notification identity, it imports existing preferences and requests notification permission again. Telegram pairing and hook paths remain unchanged; macOS may ask to allow access to the existing Keychain token again. Review **Launch at login** and Automation permissions after the migration, since these are managed by macOS separately from app preferences. An old entry can remain in System Settings; use the newly authorized entry for current notifications.

### A session or approval is missing

Check that the agent runs locally. For Claude Code, restart after hook installation; for Cline, enable hooks and reopen the task. Codex uses a direct local connection; restart its client if live requests are unavailable.

Open **Settings → Integrations** to inspect agent status and errors, then use **Check & set up** to retry setup. No terminal commands are required.

Codex's local client protocol is version-dependent. If a live request cannot be read or answered, use the agent's own interface. Logs can still supply session activity and messages; approvals require the direct connection. Historical chats are not added as substitutes for unavailable session data. If an agent adapter stops, its sessions are marked unavailable and replies are disabled; restart the monitor to reconnect.

### Hooks stopped working after moving the app

Open the app from its **new location**, then use **Settings → Integrations → Check & set up** and restart the agent. Codex requires no hook configuration.

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

To uninstall, turn off **Launch at login**, quit the monitor, and remove its hook entries before deleting the app. Codex requires no hook cleanup. For Claude Code, remove this monitor's hook commands from its settings. For Cline, remove only wrappers installed by this project from its Hooks directory. Keep unrelated hooks intact.

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

Notarization is an explicit release step, separate from ordinary builds, signing, packaging, commits, and pushes. Run it only when preparing a release for publication.

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
