# Ubuntu desktop integration

The Ubuntu interface is implemented in Rust with GTK 3. It runs in the same executable as the Tokio monitor and communicates through typed handles and snapshots. There is no Python interpreter, helper process, JSON UI protocol, web view, or remote image renderer.

A small JavaScript GNOME Shell extension owns the top-panel button, symbolic robot artwork, working-eye animation, global shortcut, compositor placement, and existing editor-window activation. GNOME Shell extensions require JavaScript; session rendering and application behavior remain in Rust.

## GNOME compatibility

Ubuntu support is experimental. The companion extension declares compatibility with **GNOME Shell 45, 46, 47, 48, 49, and 50** in [`metadata.json`](../src/linux/gnome-extension/metadata.json). This is the declared compatibility range, not confirmation that every version has been tested end to end. Versions outside that range are not declared supported by the extension.

Check your installed version with `gnome-shell --version`. The extension is required for the top-panel icon, global shortcut, Wayland positioning, and existing VS Code window activation. The GTK window can run without it, but those integrations will be unavailable. Other Linux desktops are not currently supported.

## Build and install

Install Rust using [rustup](https://rustup.rs/) if it is not already installed. The toolchain must support the 2024 edition.

```sh
sudo apt-get install git build-essential pkg-config libgtk-3-dev libsqlite3-dev \
  libsecret-tools lsof curl xdg-utils zip
cargo build --release --locked
./scripts/install-ubuntu.sh
```

Rust must support the 2024 edition. Run as your regular user in Ubuntu GNOME. The installer places the executable in `~/.local/bin`, installs the robot app icon and desktop launcher, and installs the companion panel extension without changing other enabled extensions.

GNOME may require one logout/login after installing or updating a local extension. The installer queues first-time enablement for the next login. Launch the monitor again after logging in. The current extension supplies panel placement, colored badges, eye animation, persistent open-window highlighting, shortcut registration, and existing editor-window activation. Previously loaded extension code remains active until the next login.

For development:

```sh
cargo run --locked
./scripts/install-gnome-panel.sh
```

The app also runs as a normal panel window if the GNOME extension is unavailable. Ubuntu GNOME is the supported Linux desktop; a generic AppIndicator menu is no longer used.

## UI/UX specification and interactions

The [UI/UX Specification](design/README.md) is the authoritative cross-platform reference. It compares the interactive HTML prototype, macOS and GNOME without silently resolving their differences. See the [GNOME supplement](design/platforms/gnome.md) and [three-way parity matrix](design/parity.md) for native behavior and known gaps.

The native panel uses the shared 392-pixel width, 36-pixel headers, 24-pixel sticky application icons, and 326-pixel detail region. Sessions are grouped by application identity and naturally sorted by title. The footer remains outside the main scroller; window height follows content up to the screen-dependent limit. The compositor anchors the panel beneath the robot and confines it to the monitor's usable area.

Click the robot to toggle the panel. Escape or clicking away dismisses it. Closing clears transient focus, filtering, drafts, and disclosure. Working sessions and requests expand when the panel opens; a new request can reveal a manually collapsed session. Closing alone does not clear unseen completions.

Robot and chevron clicks toggle disclosure. Title clicks bring the existing VS Code window for the session workspace to the foreground, including windows on other workspaces. The extension accepts activation calls only from the connected monitor. Unmatched or ambiguous windows fall back to opening the session/project URI with the GTK desktop activation context. The blue completion button clears only its displayed generation. Folder and branch text are selectable, with full paths in tooltips. Previous-message controls appear in the first message's gutter on hover; history is prepended and uses its own monitor cursor. One scroller handles expanded content.

Type outside an input to filter titles; all words must match. Backspace removes one composed Unicode character. Up/Down traverses session titles and request controls, Left/Right on a title changes disclosure, and Enter opens or activates the target. Input editing takes precedence over filtering. Native focus uses the current system accent.

Single-question presets submit directly. Multiple questions have independent radio groups and one shared submit. Freeform fields appear only when offered. Permission actions retain the agent's exact captions and policy context. Commands collapse to two **rendered** lines using Pango measurements, with a separate pointer-only expander. Widgets retain the exact session snapshot that created them; stale controls, duplicate sends, expired requests, and changed command context are rejected by the monitor.

Markdown uses the shared parser and native Pango text, including heading sizes, code, emphasis, lists, quotes, and safe links. The GTK theme supplies native foreground, selection, focus, and disabled styles; request, command, footer, and status colors follow the guide. Reduced animation follows GTK's system setting. GNOME controls symbolic panel tinting.

## Settings and integrations

One normal settings window contains icon toolbar sections for General, Applications, Integrations, and Telegram, and resizes to the active pane.

General supports launch at login, menu-bar animation, and shortcut recording/clearing. Control, Alt, or Super is required for a shortcut; the GNOME extension registers it and reports conflicts inline. Application visibility persists across restarts and excludes hidden applications from both the list and panel counts without stopping agents.

Integrations shows detected providers and actual hook state. **Check & set up** installs the required hooks. Codex uses automatic local discovery; Claude Code and Cline remain experimental. Restart Claude Code after setup and enable Cline's hooks before reopening its task. Cline approvals remain in Cline.

`CODEX_HOME` is honored. VS Code uses `$XDG_CONFIG_HOME/Code` (default `~/.config/Code`). Discovery relies on live session evidence rather than historical transcripts. Terminal-tab selection is not implemented on Linux; navigation uses supported project/editor targets.

## Telegram and notifications

Telegram reuses the existing Rust actor. The setup pane provides BotFather navigation, local bot-name/username suggestions, Copy/Paste, a secure single-line token field, activation status, and replacement/cancel controls. Connected management provides Test, Open, Change bot, and Unlink bot. The existing bot remains connected until a replacement activates.

Tokens are stored in Ubuntu's Secret Service keyring through `secret-tool` (`libsecret-tools`), passed on stdin rather than argv. They are never saved in preferences. Telegram delivery still requires the computer awake and the monitor running; Telegram messages are not end-to-end encrypted.

Freedesktop desktop notifications display actual request text and supported actions, using the bundled robot app icon. Actions go through the same monitor validation as the window. Resolved requests are withdrawn; late notification replies and request reconciliation do not grant stale permissions or create repeated announcements. Exact notification action layout is controlled by GNOME.

## State, checks, and remaining platform limits

State is under `$XDG_STATE_HOME/ai-agent-monitor` (default `~/.local/state/ai-agent-monitor`). The directory is private (0700), and the Unix socket is private (0600). Linux `SO_PEERCRED` authenticates both ends as the current user. `AI_AGENT_MONITOR_SOCKET` overrides the socket path for development; its parent must already exist.

```sh
AIAgentMonitor sessions --json
cargo test --locked
# Optional real-widget check in an isolated virtual desktop:
dbus-run-session -- xvfb-run -a cargo test native_widgets_follow_style_guide \
  -- --ignored --test-threads=1
gjs -m tests/ubuntu_panel_geometry.js
gjs -m tests/ubuntu_editor_activation.js
```

The real-widget test covers native geometry, sorting, disclosure, direct answers, request snapshots, preserved live drafts, filtering, Settings, and close/reopen state. Unit tests do not certify GNOME placement, increased contrast, reduced transparency, or every end-to-end agent/Telegram interaction; use the guide's manual acceptance checklist on the actual desktop.

AppKit menu materials and system menu ownership are platform-specific. Ubuntu uses a GTK panel and a GNOME shell button; AppKit blur and neighboring macOS menu navigation are not emulated. Mac bundle, DMG, and notarization commands remain macOS-only. No prebuilt Ubuntu package is published by these scripts.
