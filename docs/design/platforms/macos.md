# macOS AppKit supplement

<a id="macos-shell"></a>
## Native shell contract (S/P; runtime outcome U where noted)

Resident is an accessory NSApplication, no normal Dock/task-switcher app window for the session popup. NSStatusItem owns an attached NSMenu; menu view is a custom enabled item, autoenable off. Native menu tracking owns opening/dismissal/selected menu-bar background, rather than a manually activated standalone NSPanel. `performClick` opens it; `cancelTracking` dismisses. Native horizontal navigation passes through only when no session target owns Left/Right. Whether other apps' menu extras participate and exactly when selection clears remains an OS-runtime check.

AppKit material is retained; a click-through MenuTintView/PanelView draws windowBackgroundColor at65% alpha over complete popup, with dynamic appearance and top extension. Footer opaque fill covers bottom OS padding. Correct before `viewWillDraw` and frame updates, rather than visible post-open jump. Work-area clamping and max-height described in [lifecycle](../interactions/input-lifecycle.md#life-open); OS corners/shadow not hardcoded to prototype18px.

Scroll elasticity remains disabled. Application headings scroll with the document and are repositioned from their first session after disclosure layout changes. Reveal clip is not an inner scroller; outer list routes wheel. Main-thread pointer polling in common run-loop modes handles non-first-responder hover, button clipping and editing focus. Inline input uses field editor plus custom focus/caret decoration because menu tracking may not make its window key. This is not a recommended mechanism for future platforms—only the visible single-focus/editing result is shared.

Sources: [native](../../../src/macos/native.rs) `run`, `MonitorApplication`, `NotificationDelegate`; [menu](../../../src/macos/menu.rs) `build_status_item`, `build_menu`, `PanelView`, `MenuTintView`, `OverlayView`, `close_panel`, `handle_session_key`; [pointer](../../../src/macos/pointer.rs); [hotkey](../../../src/macos/hotkey.rs); [inline fields](../../../src/macos/inline_questions.rs).

<a id="macos-navigation"></a>
## Host navigation and permission prompts

VS Code target uses pinned workspace association, not latest tool cwd. M `open_target` first delegates terminal custom targets; otherwise it requires a parsable VS Code path. For a missing project path, it activates running VS Code or opens the VS Code application rather than requesting the missing path. For an existing path it opens the URI through NSWorkspace. A generic “activate the named Desktop client” route is not implemented here; Codex desktop sessions also receive the adapter's project/editor target. Terminal target encodes agent/session/cwd rather than interpolating commands. Terminal host resolved through live process ancestry/TTY; resume/session identity preferred to cwd, ambiguous same-directory sessions not guessed.

Terminal.app AppleScript selects matching tab by TTY and activates window; iTerm finds matching session/tab. Warp/other hosts without tab script focus application only, not guaranteed tab. Automation permission may be needed; no custom permission bypass/settings control. A process retained only by daemon rollout must not keep a closed terminal session visible. Navigation no-ops/ambiguous fallbacks can occur; source does not guarantee every terminal vendor opens exact tab.

Sources: [terminal](../../../src/macos/terminal.rs) `target`, `retain_live_codex`, `choose`, `focus`, `TERMINAL_SCRIPT`, `ITERM_SCRIPT`; [native](../../../src/macos/native.rs) `open_target`; [Codex VSCode](../../../src/agents/codex/vscode.rs).

<a id="macos-settings-access"></a>
## Settings, notifications, accessibility and system controls

Settings normal floating window intentionally activates, stays above ordinary windows and does not hide when menu clicked. General uses native checkboxes; Applications native switches. Shortcut uses Carbon registration while menu closed, native menu key equivalent while open. Launch at login uses SMAppService, can require separate approval in macOS settings.

Notifications use UserNotifications with dynamic request categories; OS policy controls actual banner action density. Notification permission asked after onboarding setup; denial note in Integrations. New bundle identity `com.melsomino.ai-agent-monitor` and legacy preference migration exist, but icon cache invalidation is OS-owned and cannot be guaranteed by these source paths.

Accessibility source evidence: SF Symbol accessibility descriptions, button tooltips/labels, title captions, explicit application switch/shortcut-clear labels, system text controls; decorative hitTest None. There is no complete custom NSAccessibility outline/list/expanded-state implementation in these custom rows. Robot/chevron tooltips update; a generic Session details symbol description is not proof of VoiceOver announcement parity. Tab order, VoiceOver navigation, increased contrast, reduced transparency, full keyboard access, IME composition and high zoom require manual verification. Native system colors do not by themselves prove contrast compliance.

Reduced-motion accessibility checked for ambient and session robot eyes; disclosure still animates. Dark/light dynamic provider covers tinted menu, cards, footer, message and hover. No user-facing theme selector in native preferences.

Acceptance supplement: actual side/bottom Dock and offset monitor; popup open without stealing previous app focus; native checked background throughout tracking; no header strip during resize; one hover when pointer moves/exits; field caret and cmd+V; automation denied; notification permission/DND; alternate accent/theme. These are not certified by arithmetic/unit tests.

Session robot motion follows [SE-ROBOT-01–03](../components/sessions.md#sessions-robot-motion): the existing popup tracking timer invalidates working robot drawings, sampling a controller-owned monotonic epoch. Reduced motion is read from NSWorkspace. Robot artwork and hit/hover geometry stay fixed; only eye brightness changes.

Application headings are click-through 24-point document views with icon/name/separator; the first session reserves26 points above itself. `update_group_headings` uses document coordinates, never viewport pinning. Title destination hints share the title open target and hover/focus lifecycle; hidden hints reserve42 points, protecting status/disclosure controls. See [SE-02/SE-03](../components/sessions.md#sessions-header).
