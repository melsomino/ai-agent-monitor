# Popup lifecycle, pointer, keyboard, and scrolling

<a id="life-open"></a>
## Opening and placement

Native entry points: system status button, configured shortcut, CLI/IPC show request, supported notification/body action. M `show()` avoids starting another tracking loop when already open; `menuWillOpen` refreshes/filters, starts pointer/keyboard tracking and answer focus. G `Desktop::show` presents/focuses a GTK window and first request after idle layout. The monitor remains resident when popup is closed. H popup is always present and status item is not a toggle.

**LC-01 (S):** On a new native open lifecycle, reset transient title query/disclosure traversal state as documented; expand currently working/request sessions, focus first response if available and scroll it into view. No available response means no forced response target. Do not grant permission, launch an agent or clear completion by opening.

M positioning belongs to native NSMenu tracking, corrected before draw/resize to usable screen. Window top aligned to visibleFrame top (menu-bar bottom); side/bottom margin4. Custom content392; max height `min(screen.height*.8−20,visible.height−24)`, lower bound100; actual `min(max(content+footer+8,64+footer),maximum)`. Scroll viewport subtracts footer+8. Geometry uses actual status-item screen with main-screen fallback. Header/footer extension views cover native menu padding; this source intent still needs native runtime verification for flashes/gaps.

G undecorated Utility window408, root inset8 horizontal/4top/12bottom, clipped18radius. Max height same80%−20/workarea−24, minimum max120; content+footer+error+16 clamped footer+52..max. Shell compositor positions center under indicator, clamps x to workarea+4..right−width−4 and y to max(workarea.y,min(anchor.bottom,workarea.bottom−height−4)); rounded logical coordinates. It positions on map/toggle, not a custom per-frame reposition during every GTK size change. Wayland placement without extension is unavailable; GTK cannot impose absolute compositor coordinates. Do not promise macOS-style neighboring menu traversal on G.

Sources: [M menu](../../../src/macos/menu.rs) `show`, `will_open`, `resize_content`, `constrain_menu_to_work_area`, `sync_header_extension`; [G ui](../../../src/linux/ui.rs) `show`, `resize_to_content`; [Shell geometry](../../../src/linux/gnome-extension/geometry.js) `popupPosition`; [H](../../../design/previews/monitor-ui-fragment.html) `.aam-list`, `.aam-menu`.

<a id="life-close"></a>
## Closing and lifecycle cleanup

| Event | M | G | H |
|---|---|---|---|
| Status click/global shortcut while open |Native tracking/toggle close |Toggle hides |No status handler |
| Escape |Native menu dismissal/editor delegation |Explicit hide before editor routing |No handler |
| Click outside/switch app |OS NSMenu tracking |Focus-out delayed100ms; hide unless popup or settings active |No dismissal |
| Show desktop |OS native behavior, runtime-dependent |Compositor focus-out behavior; not explicitly handled |N/A |
| Open host title |Dismiss then command open |Command open; hide after successful activation/launch, error keeps window |Update version text only |
| Settings |Dismiss then floating settings window |Open settings; active settings exempts focus-out hide |Toggle internal overlay |
| Power |Shutdown monitor |gtk main quit then monitor shutdown |Toggle demo pause, no sessions stopped |

**LC-02 (S):** Closing alone MUST NOT approve/deny or acknowledge completion. M clears keyboard focus, filter, disclosures, reveal/command expansion, local history and scroll-to-top; stops editing and timers. Active question-panel cache not explicitly emptied, so draft retention differs from documented “clear drafts” intent. G clears query/disclosure/focus and removes request cards/drafts, but shared Window history remains and scroll adjustment is not explicitly reset. H saved expansion uses widget state; no close lifecycle.

<a id="input-pointer"></a>
## Pointer, hover, and click targeting

M continuously samples pointer in common menu-tracking run-loop modes. `pointer::target_at` considers current window bounds, clipped viewport, toolbar/buttons and measured title headers. At most one **pointer** target is chosen per sample; hidden/disabled controls excluded. Leaving window clears hover. Keyboard target is separate and may remain while another item is hovered; this does not mean two keyboard foci. A field editor takes keyboard-target precedence. Foreground title hover, robot hover, finished/check/chevron hover and action hover are independent hit regions. Decorative views ignore hit-testing. G uses GTK widget hover and standard single focused_widget; H CSS `:hover` can affect ancestors simultaneously, so global single-hover exclusivity is not established there.

**IP-01 (S, native):** Title hit geometry MUST exclude status/disclosure controls. Robot and chevron MUST toggle identically. A completion status click MUST acknowledge without toggling or closing. Hover geometry should follow the actual allocated control bounds, not oversized drawing visibility. H finished disc has no click implementation; parity gap.

Previous-message pointer visibility is separately specified in [sessions](../components/sessions.md#sessions-history). H y-only first-message test can show history arrows in another column of the product; do not call it equivalent to native hit-testing.

<a id="input-keyboard"></a>
## Keyboard routing table

| Input/context | M | G | H |
|---|---|---|---|
| Up/Down outside settings |Titles then controls of expanded sessions; clamp ends; no focus→first/last |Same order but wraps ends; skips insensitive widgets |Titles,actions,radio/custom inputs visible/enabled; clamp ends |
| Up/Down in custom Entry |Commit editor, navigate preserving field position |Navigate before editor, even with modifiers |Navigate targets; custom radio and Entry are separate stops |
| Left/Right on title |Collapse/expand, no repeats; no focus→pass to native menu bar |Collapse/expand |Collapse/expand |
| Left/Right on same-row action buttons |Cycle enabled answers when all relevant controls share y center (≤.5 tolerance) |Cycle targets sharing horizontal GTK Box; radio uses toolkit |No action-specific cycle |
| Left/Right on vertical answers |Consume without disclosing parent |Toolkit/proceed |No special handler |
| Left/Right within input |Native cursor editing |GTK editing |Browser editing |
| Enter on title |Open app, acknowledge completion, close |Open via title clicked |Simulate navigation in version label |
| Enter on answer/button |Activate once, no auto-repeat |GTK button emit clicked |Browser button or explicit radio selection |
| Space on multi preset |Select once |Native radio/Button behavior |Browser radio behavior |
| Space on title |Printable query character; no disclosure |Native Button behavior may click title |Browser button click opens simulation |
| Enter in input |Select custom, leave open, no send |Multi activate selects custom; single no explicit handler |Select custom, prevent submit |
| Tab |Not in custom ordered traversal; native menu/editor behavior needs QA |GTK default traversal in addition to custom arrows |DOM order includes robot,chevron,history,settings |
| Printable outside editor |Accumulate title filter |IMMulticontext commit or unicode char accumulates filter |No title filter implementation |
| Backspace outside editor |Remove last composed NSString character; always consume |Pop unicode grapheme |Browser defaults |
| Cmd/Ctrl/Alt-modified |M Command/Control/Option bypass navigation after editor handling |Up/Down handled before modifier guard |No global modifier guard on custom arrow handler |

M filtering ANDs whitespace-separated case-insensitive title substrings; highlights all matching spans with UTF-16-safe ranges, merged overlaps. G same matching rule, markup underline per matching character; overlapping-match enumeration differs (G `match_indices` is nonoverlapping). Query visible in footer, not search input. M typing a query can establish first matching title focus; G clears focus on text commit. Input editing takes precedence over query. M cmd+A/X/C/V explicitly routed to field editor; G/browser standard edit handling.

M `keyboard_targets` excludes ContextToggle/robot/chevron/history/footer; includes disabled inline Send target for focus continuity but not activation. G `targets` sensitive titles/card widgets only, while GTK Tab may reach other focusable controls. H filter tests DOM visibility and disabled flags and uses `.aam-answer input`, including custom radio and text. These are differences, not one universal key map.

<a id="input-scroll"></a>
## Scrolling, reveal and motion

**IP-02 (S):** Expanded content MUST use the list's single vertical scroll region, not trap the wheel in an inner message. Footer stays pinned. When keyboard target changes, reveal its bounds with minimal movement. Opening a pending response similarly reveals its first control. M routes margin/root/reveal wheel to main scroll, prevents recursive routing, disables both elasticities. G uses GTK ScrolledWindow and adjustment clamp_page; pinned icons repaint on adjustment/allocation. H `overflow-y:auto`,max360,overscroll contain,scroll-padding8,scrollIntoView nearest.

M supports interrupted cubic240ms disclosure; redraw resizes all rows/footer and rotates chevron in same timeline. G Revealer240ms, toolkit animation disabled→0; H180ms ease/reduced-motion none. G list working spinner ticks even if panel animation preference is off; M spinner also independent. Neither should be documented as stopped by “Menu bar animation.”

Acceptance: use keyboard from no focus; traverse beginning/end; compare clamp/wrap without declaring them equal. Edit a custom response, use Left/Right/Up/Down/Enter/Space and copy/paste, verify only current keyboard target highlighted. Scroll with pointer on margins, message, request and last-row hover; footer stays fixed. Open near side/bottom Dock or on offset/scaled displays; verify real OS placement and non-jumping resizing separately from arithmetic tests.
