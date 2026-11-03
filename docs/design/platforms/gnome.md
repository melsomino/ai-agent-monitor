# Ubuntu GNOME / Wayland supplement

<a id="gnome-split"></a>
## Desktop composition

Production session/settings UI uses Rust **GTK 3**, not HTML or Python. A small GNOME Shell JavaScript extension supplies top-panel button, count badges, eyes, shortcut, compositor anchoring and existing editor-window activation. This split is a native platform constraint, not an alternative session state model.

Source: [Linux root](../../../src/linux/mod.rs) `run`, `open_session`; [GTK desktop](../../../src/linux/ui.rs) `Desktop`; [bridge](../../../src/linux/panel.rs) `Panel`; [Shell](../../../src/linux/gnome-extension/extension.js) `MonitorPanel`; [geometry](../../../src/linux/gnome-extension/geometry.js); [activation](../../../src/linux/gnome-extension/activation.js).

GTK popup is undecorated/nonresizable Utility, skip-taskbar, transparent outer canvas with manually drawn shadow/rounded opaque theme surface. It is **not** a GNOME Shell PopupMenu and therefore does not automatically inherit all shell-menu behavior. Shell checked style follows published Visible; GNOME button hides when monitor bus owner vanishes. Extension disconnect cancels polls/shortcut, removes checked state; GTK can still run without extension but lacks panel anchor/shortcut and exact editor activation.

<a id="gnome-focus"></a>
## Focus and placement constraints

Window is presented/activated; focus-out waits100ms and hides only when neither popup nor retained settings is active. Settings can remain open alongside popup. Escape explicitly hides even from input. System show-desktop/outside dismissal follows resulting focus state rather than dedicated events. No neighboring panel-menu Left/Right navigation is implemented. Global accelerator available in Shell normal mode; collision sent inline to preferences. With extension not loaded, no accelerator registration alternative.

Compositor maps matching PID/title window beneath indicator and clamps to workarea. Coordinates logical incl fractional scaling. Extension positions on map/toggle; GTK resizes with content. Native Wayland forbids client absolute placement; `window.move_` calls in settings do not guarantee compositor positioning. Larger-than-workarea edge cases and dynamic popup resize anchoring need real compositor QA; no automatic copy of AppKit blur is expected.

<a id="gnome-navigation"></a>
## Editor and terminal behavior

Extension accepts ActivateEditor only from connected resident D-Bus owner. Match supported VS Code desktop IDs/wmClass, decode vscode://file path, first unique full-path segment in title, then unique basename/workspace name in MRU list. Ambiguous duplicates return false; no arbitrary MRU guess. Main.activateWindow can switch workspaces. URI fallback uses GTK app launch context with timestamp/activation token to avoid focus prevention. Unsupported URI schemes rejected. Open errors appear atop popup and keep it visible.

There is **no Linux terminal-tab selection**, and no macOS `retain_live_codex` terminal lifetime filter. Terminal sessions can navigate to project/editor URI via adapter target. This is a documented capability difference, not evidence of a terminal-app focus implementation. Codex desktop Client still has a macOS path in shared adapter; generic icon/URI fallback may be observed. Do not present this as complete support for every Linux editor/terminal.

<a id="gnome-native"></a>
## Theme, settings, notifications and access

Theme colors/selected ink from GTK, with shared request/footer/message tokens. Dark detection sums background RGB<1.5; theme/prefer-dark property changes reload CSS. Native GTK requisitions and installed theme affect actual metrics/hover; CSS6% hover differs from prototype/macOS14%. Native radio/input semantics intentionally use GTK, but application Up/Down wrapping/modifier precedence is a current conflict rather than required convention.

Settings width560; pages size naturally with content. Startup **checks** integrations but does not install until Check & set up. Autostart uses XDG desktop entry; credentials use Secret Service/secret-tool with stdin. Notification transport freedesktop; actions/format controlled by GNOME, no inline text reply, no aggregate4-control budget. Missing notification bus/failure lacks visible retry. See [notification parity](../interactions/notifications.md#notify-gnome).

Accessibility uses real GTK Button/Entry/RadioButton/Label/ATK and explicit icon/entry names where set. Application group headings are ordinary scrolling icon/label/separator rows; session threads are drawing-layer decorations without a full semantic group object; Settings switches lack explicit names; native toolkit semantics require Orca review. GTK animation setting disables Reveal and session robot eye cycling; ambient/list animations do not all respect it. Working robots use the shared 2.4-second motion sampler and a GTK frame callback; the desktop owner keeps the epoch stable across snapshot redraws. High contrast, font scaling and Wayland IME need runtime validation.

Acceptance supplement: installed vs unavailable Shell extension, D-Bus disconnect/reconnect; repeated open with selected checked style; fractional scaling/multiple workspaces; unique vs ambiguous VS Code window names; native notification server capability failure; keyring locked/missing; loading settings pane at several text scales. Relevant ignored widget harness does not certify real compositor behavior.

Group headings and title destination hints follow [SE-02/SE-03](../components/sessions.md#sessions-membership). Headings live inside group containers rather than a viewport overlay. Title destination opacity is zero until the title is hovered/focused, preserving layout; GTK draws the arrow and a 16px application icon.
