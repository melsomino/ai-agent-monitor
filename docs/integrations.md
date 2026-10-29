# Integration notes

This document describes the current implementation. For installation and everyday use, see the [README](../README.md).

## Native application

AI Agent Monitor uses Rust with AppKit, UserNotifications, and the system SQLite library. The runtime is a single executable inside a locally signed `.app` bundle. Swift is only used by the optional menu bar asset generator; it is not needed for ordinary builds or use.

The session menu is an `NSMenu` with custom content. AppKit owns its presentation and dismissal. The main list is the only scroll region; expanded messages do not introduce nested scrolling. Pointer sampling runs in common run-loop modes so hover stays independent of the first responder and native menu tracking. Keyboard selection and pointer hover are separate states.

The bundle identifier remains `local.codex.agent-monitor.notify` to preserve existing notification authorization and preferences. The bundle command writes the compiled version and build number into `Info.plist`. It creates `build/AIAgentMonitor.previous` once when replacing an existing bundled executable.

## First-launch setup and disk images

Only the resident app invokes onboarding. Notification senders, permission hooks, and diagnostic commands never install integrations. Onboarding discovers existing Codex/Claude configuration or known VS Code extensions and Cline storage, then runs the existing idempotent installers off the UI thread. It does not install agent software, enable login items, override Codex trust, or toggle Cline's own hook setting.

Successful setup is remembered per agent, configuration directory, and executable path. Failures remain retryable; unrelated agents can still complete setup. New agents can be connected on a later launch or using **Set up integrations…** in General settings. The initial summary explains remaining agent-side steps, and macOS handles its own notification authorization prompt.

Hooks from another copy of this app bundle are migrated to the current executable path. Commands outside the named app bundle, extra arguments, and unrelated hooks are preserved. Configuration backups and existing installer validation still apply.

Launching directly from `/Volumes` or App Translocation displays installation instructions and exits before acquiring the resident lock or writing hooks. The user must drag the bundle to a stable location such as Applications first.

The `dmg` command packages the current executable, ad-hoc signs the bundle, creates a compressed read-only disk image with an Applications shortcut, and writes a SHA-256 checksum. Asset names identify the native architecture; no universal-binary claim is made.

## Codex discovery

The provider finds rollout files open for writing by the current user's Codex processes, reads metadata from the local database in read-only mode, and reads transcript state. Titles come from the exact session ID, with renamed titles taking priority. It does not substitute another chat's title.

Transcript tails are limited to the latest 4 MiB for message reading. If lifecycle events lie outside the tail, the provider scans backward to resolve the status. File identity and boundary checks invalidate cached data after rewrites or rotation. A session can be open without currently running a turn.

### VS Code windows and workspace identity

A shared Codex process may retain a loaded transcript after a VS Code window closes. Open transcript handles alone therefore do not establish that a window is still open.

The provider combines `Codex.log` thread ownership records, the current extension-host generation, `exthost.log`, and `workspaceStorage/workspace.json`. It accepts live log handles held by Code Helper and records belonging to that host's current lifetime. Closed windows are excluded on refresh; reopening restores discovery. If window discovery is unavailable, transcript-based discovery remains a conservative fallback.

Workspace mapping is separate from live-window membership: a deleted folder does not hide a session in an otherwise live window. Recent unambiguous mappings take priority. Commands executed in other directories do not redefine the editor's project. Multi-root workspaces use the original session working directory for the session details.

Navigation uses the VS Code URL handler. It opens a project, not a particular chat. If a suitable path no longer exists, navigation activates VS Code without opening the missing folder.

### Live requests and questions

The provider observes the local Codex client socket at `$CODEX_HOME/ipc/ipc.sock`. It validates socket ownership and permissions, discovers the owning client, and subscribes as a follower. It does not advertise ownership or accept agent operations.

Snapshots and revisioned patches maintain request state. Periodic resubscription obtains fresh state; invalid revisions and reconnects invalidate cached observations. The protocol is internal and version-dependent. Unsupported versions are ignored, with logs and hooks retained as fallbacks.

Native requests can expose command/file approval choices, proposed policy amendments, scoped permissions, and `item/tool/requestUserInput`. When `availableDecisions` exists, it is authoritative. Unknown request types and option pickers without an addressed response route open the editor instead.

Asynchronous questions are separate: `request_user_input_async` records questions in agent-message history, and a turn can be idle while a question still needs an answer. The provider retains a compact history projection with question metadata and user replies, rather than complete agent text and tool output. Answered questions and questions superseded by new user input are removed.

After an explicit user choice, an asynchronous answer is encoded with the original `questionItemId` in the native reply format. It is sent as a user message in a new turn or as input steering the active turn, inheriting the thread's settings. Passive observation does not start a turn or invoke a model.

## Responses and notifications

The provider creates opaque, process-local tickets bound to the session, owner, and full pending request. Before responding, it rediscovers the same owner, obtains fresh state, and checks that the exact request still exists, including its thread, turn, and parameters. A stale ticket fails closed. Responses are not automatically retried after a timeout.

The shared `Interaction` / `Response` model represents offered actions, answer choices, custom text, and multiple questions. The UI does not construct provider-specific permission decisions. Forms require an explicit answer; secret inputs use native secure text fields.

Each live notification receives its own immutable action category. A response must match the request key and exact category. Up to four actions are registered; larger sets route through a full form or expanded monitor view. The visible banner layout and number of directly visible actions belong to macOS.

Category registration is serialized across the resident process and short-lived hook senders, preserving unrelated active categories. Registration is acknowledged before posting. Resolved requests remove their notification bindings and categories; cancellation is rechecked around asynchronous delivery. After a text-response failure, the full form can reopen with the draft preserved.

### Permission hooks

Codex CLI and Claude Code permission hooks wait for a one-time response for up to 60 seconds. Only explicit **Allow** emits the corresponding permission decision. Timeout, dismissal, failure, or opening the editor returns control to the agent's normal permission UI.

Each pending file-backed response uses a separate user-owned temporary directory with mode `0700`. Responses are consumed once; symlink substitution is rejected. The directory is removed after the response or timeout. An expired notification cannot authorize a subsequent request.

For Codex hooks, the native permission dialog appears after the hook finishes waiting. Other independently installed hooks may run in parallel. If native notification delivery is unavailable, a plain `osascript` fallback has no approval button or click navigation and returns control to Codex immediately.

## Claude Code and Cline

Claude Code installs lifecycle hooks and `PermissionRequest` in its settings. Existing hook groups and permission rules are preserved, with a private configuration backup before changes.

Cline installs executable lifecycle wrappers into its Hooks directory. Existing third-party hook files are not overwritten. Current task state is read from `ui_messages.json` in standard VS Code storage or `~/.cline/data/tasks`. Only the latest task for each host is tracked, rather than scanning task history. `SessionShutdown`, when available, closes task state; host termination also removes its sessions.

Snapshots are stored under `~/Library/Application Support/AI Agent Monitor/agents` with access restricted to the current user. They are bound to the agent or extension-host PID and process start time, so closed processes and reused PIDs do not retain sessions.

Cline's public `pressPrimaryButton()` API does not identify a specific task or request. The monitor therefore routes approval back to Cline instead of risking approval of a different request. This does not change Cline's built-in permission or auto-approval settings.

## Preferences and quota

The settings window uses an `NSTabViewController` with a native preferences toolbar. General preferences, the global shortcut, and the observed application catalog are saved in the app's `NSUserDefaults` domain. Application IDs are independent of display names; discovery updates names and icons without resetting visibility. Closed and disabled applications remain in the catalog.

Application switches filter the window's sessions, alongside the title query. They do not delete session state, disable providers, or suppress notifications and the menu bar activity indicator.

Codex quota is derived from the newest valid `token_count.rate_limits` report in recent local transcripts, cached for ten seconds. It displays `100 − used_percent`, with no network account query. Expired windows display `—` until a new report arrives. Reading quota or session state does not invoke a model.

## Adding an agent provider

Implement `SessionProvider` and register it in `src/agents/mod.rs`. Common types live in `agents/sessions.rs` and `agents/interaction.rs`.

A provider supplies open sessions, activity, navigation targets, and supported interactions. UI keys include the provider identity, so identical session IDs from different agents do not collide. `Approval.id` is an opaque token to the UI, not a response-file path.

Discovery must never authorize actions. A response implementation must verify that the request still belongs to the intended session and remains current before sending an explicit user decision.
