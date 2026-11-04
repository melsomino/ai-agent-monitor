# Session list, expanded content, and footer

<a id="sessions-membership"></a>
## Membership, ordering, filtering

**SE-01 (S/T):** Display open discovered sessions including idle. Do not substitute a history archive for live discovery. Filter by enabled application before rendering. Filtering never deletes the underlying session or stops its agent. Refresh automatically from monitor snapshots; no Refresh control in the native popup or current Telegram keyboard.

M/G grouping rank: VS Code, Terminal, identified desktop applications, Unknown. Then application ID, then natural title order, then stable session key. M uses Foundation localizedStandardCompare, G uses lowercase GLib FilenameCollationKey; locale/case equivalence is not guaranteed. H fixtures are fixed within explicitly enumerated VS Code/Terminal/Codex groups, not dynamically sorted.

Each application group begins with a scrolling 24-high heading: actual 24×24 application icon, application name in secondary 11-point semibold, then a horizontal separator at the same vertical center. The heading occupies 26 points/pixels before the first session (24 plus 2 below); later groups add 6 above. The separator continues after the name with a 10-point gap. Icons have transparent backgrounds and do not occupy a session's robot position. All headings scroll with the document; none pin to the viewport or replace another heading at the top. Sessions retain their original per-application ordering and individual robot controls.

**SE-02 (S, H/M/G):** Scrolling or expanding a session must not detach the application heading from its group. Headings are informational, not buttons or keyboard targets. The first session displays its robot exactly like subsequent sessions. Native icon fallback is generic app if unresolved. M resolves bundle ID/file icon; G resolves desktop-entry/theme icons (generic executable for unsupported desktop clients); H embeds fixture assets.

Sources: [M](../../../src/macos/menu.rs) `State::filtered_sessions`, `render_rows`, `client_icon`, `client_group_heading`, `update_group_headings`; [G](../../../src/linux/model.rs) `eligible`, [ui](../../../src/linux/ui.rs) `refresh`, `Row::update`; [shared geometry](../../../src/domain/layout.rs) `GROUP_HEADER_HEIGHT`, `GROUP_GAP`; [H](../../../design/previews/monitor-ui.html) `build`, `syncGutter`.

<a id="sessions-header"></a>
## Session header anatomy and targets

| Element | Visible states | Pointer/activation | Keyboard |
|---|---|---|---|
| Robot |Neutral head; green eyes working; orange eyes request |Separate rounded icon hover; disclosure toggle |Excluded from native custom Up/Down traversal; H normal tab stop |
| Title |One-line, semibold, truncated, filter match emphasis |Measured rounded hover; click opens host |Native title traversal target; Enter opens; Right expands,Left collapses |
| Working status |Green filled circle with small solid rightward play triangle |Informational, tooltip Working |No answer/disclosure action |
| Needs response |Orange filled ! |Informational; does not grant permission |Not an approval target |
| Finished unseen |Blue filled check |Native click acknowledges current completion without closing/expanding; H status span inert |Not in native custom traversal |
| Unknown |M muted questionmark.circle; G muted “?” drawing |Informational |No approval action |
| Chevron |Right collapsed; rotates down expanded |Separate icon hover/click toggle |Not in native custom traversal; H normal tab stop |

**SE-03 (S):** Opening the host is distinct from disclosure. Robot/chevron toggle; title opens. Tooltip is “Switch to <application>” in the native frontends and H. On title hover or title keyboard focus, a rightward arrow followed by the actual application icon appears immediately after the title, inside its existing rounded highlight. These are decorative, not independent targets; activation anywhere in the title still opens the host. Reserve their width even while hidden to prevent title movement. Arrow14, application icon16, gap4, title-to-decoration gap8; native title allocation reserves42. Long titles truncate before the decorations and never displace the status or disclosure controls. Unfocused/unhovered titles hide both decorations. On native keyboard focus the arrow uses selected text ink; the application icon keeps its original colors. Working/request glyphs occupy space before the chevron. Idle has no status glyph. Title hit/highlight ends after measured text plus padding and excludes the right-hand controls. H/M/G differ in exact highlight origins and target size; [geometry](foundations.md#visual-geometry).

**SE-04 (S):** Multiple sessions can be expanded. On opening, native working/request sessions expand. Within an open popup, manual collapse survives unchanged updates. A new request reopens its session. M recognizes new approval ID and first working occurrence per open lifecycle; G recognizes changed approval/interaction token but does not auto-open an already-known idle row merely because it later becomes working. H initially expands fixture `delivery` only. See [D-04](../discrepancies.md#d-04).

<a id="sessions-robot-motion"></a>
## Working robot motion

**SE-ROBOT-01 (S, implemented H/M/G):** A working session keeps its robot at the normal 14×11 artwork size. Only its eyes animate; the head must not breathe, rotate, translate, fade, resize its row, or change its hit target. Application headings scroll independently with their groups and never cover the robot.

**SE-WORKING-01 (S, implemented H/M/G):** The working status is a static green filled disc, diameter15 within an18×18 symbol viewport, with a small solid rightward play triangle centered optically inside. H/M/G render the same6.72×8.32 filled triangle directly within the18×18 viewport. Its centroid coincides with the disc center: base at x−2.24 with endpoints y±4.16, tip at x+4.48. M uses `WorkingStatusView`, G Cairo and H inline SVG, avoiding font/symbol alignment insets. Light appearance uses white triangle ink, dark appearance uses dark green `#10351e`; disc green follows each frontend's established working palette. No outline-only triangle, rotating arrows or status animation. The glyph is informational, with “Working” tooltip/native label, and is not an actionable Play button. It must not enter answer navigation or acquire its own hover/click target. Request/finished state indicators retain their existing semantics and colors.

**SE-ROBOT-02 (S, implemented H/M/G):** Working eyes are green and cycle through left/right brightness pairs `.5/0 → 1/0 → 1/.5 → 1/1 → .5/1 → 0/1 → 0/.5 → 0/0 → repeat`, linearly interpolated across eight equal intervals of a 2.4-second cycle. Zero brightness means invisible eyes, without neutral eyes underneath. This follows the menu-bar eye pattern but uses its own slower cycle; it does not change menu-bar animation or counters. Request eyes remain orange and idle/other eyes remain neutral; these states have static eyes.

**SE-ROBOT-03 (S):** System reduced-motion disables eye animation, leaving both working eyes steadily green. H uses `prefers-reduced-motion`, M uses the workspace accessibility preference, G uses GTK's enable-animations preference. The app's *Menu bar animation* setting does not control session markers. Discovery redraws must not restart eye motion; native frontends use an owner-held monotonic epoch. Artwork, click targets and thread geometry remain fixed; native robot drawing is14×11 centered in the existing button bounds, with no extra vertical offset. The thread starts at the robot's normal contour.

Sources: [shared motion](../../../src/robot_motion.rs) `sample`; [AppKit robot](../../../src/macos/session_robot.rs) `advance`, `draw_robot`, `bottom_contour_in`; [AppKit controller](../../../src/macos/menu.rs) `advance_robot_eyes`; [GTK drawing](../../../src/linux/widgets.rs) `robot`, [GTK owner](../../../src/linux/ui.rs) `robot_epoch`; [HTML](../../../design/previews/monitor-ui.html) `aam-eye-blink`, `status`.

Acceptance: observe at least two cycles; verify green eyes alternate while head size/body opacity and row geometry/hover target remain fixed. Confirm the green status disc and small solid play triangle remain stationary in both themes, including after an approval resolves to working. Refresh discovery mid-cycle without a restart. Change working to request/idle and verify motion stops with correct eye color. Enable reduced motion and verify normal-sized static robot with both green eyes. Scroll across a group boundary and verify the application heading scrolls with its group and never covers a robot. Source/build checks verify M; GTK runtime and browser visual checks remain required.

<a id="sessions-expanded"></a>
## Expanded reading order and messages

Shared native reading order: header → earlier messages/current latest → active request card → folder/branch metadata. Request does not replace current message. Trailing whitespace is trimmed on message display. Native empty message reads **No messages yet**. H supplies populated fixture messages and a fallback previous fixture only.

M builds earlier chronological labels plus an anchor if the previously-latest message changed; when a request is present, includes nonempty latest before the card. G appends current latest to window history; no anchor preservation. Both are selectable native formatted text, and the main list is the sole scrolling region. H uses explicit prewritten HTML (headings, links, lists, code); it has no backend parser or arbitrary-message ingestion.

Markdown native common source: [parser](../../../src/domain/markdown.rs) `parse`. Supports emphasis,strong,strike,headings,ordered/unordered/task lists,quotes,inline/fenced code,hard breaks and rule text. Soft breaks become spaces. Raw HTML is literal text, not executed. Images are not fetched. Links are only HTTP, HTTPS or mailto; local/command URLs stay text. Tables are not enabled as a dedicated layout. There is no native syntax highlighter. M renders attributed NSTextField; G renders Pango markup. G preserves textual list/quote projection but does not reproduce M's paragraph indentation; exact typography is platform-specific.


Sources: [M markdown](../../../src/macos/markdown.rs) `apply`; [G model](../../../src/linux/model.rs) `markup`; [G widgets](../../../src/linux/widgets.rs) `message`; [H CSS/fixtures](../../../design/previews/monitor-ui.html) `.aam-message`, `sessionMarkup`.

<a id="sessions-history"></a>
## Previous message gutter

**SE-05 (S):** Previous loads one earlier agent message and prepends locally; it does not replace or send a new message to the agent. Clicking it acknowledges an idle completion. Separate history cursors protect Telegram/window independence.

M: 16-wide clickable gutter beside first message, arrow10×12 centered vertically over its full message height. Decoration appears only over first-message/gutter region, including scroll-driven pointer resampling. Normal target remains allocated even when decoration hidden. Disabled during loading/exhaustion; tooltips distinguish Loading previous message…, No earlier agent messages, and retryable failure. G:34-wide gutter region,16-wide trailing previous button,2 end margin; pointer sampled each frame over first message event bounds. Button disabled for loading/exhaustion; gutter handler emits click, callback rechecks loading/exhaustion. History error appears in row feedback rather than equivalent per-state tooltips. H:12-wide gutter over first visible fixture message,8px glyph; pointer test uses the message's **vertical y band anywhere within product**, not x containment. Clicking reveals exactly one fixture and hides the button; no loading/exhausted/error state.

Expanded thread: subtle line from robot's contour, interrupted at request icon and filled5px message dots for second and later messages; no ring around robot and no terminal end-dot in current code. Native M line1 at label10%, dots25%, orange request disc15. G line.5 at fg22%, dots25%, request disc16; H actual SVG-stroke mask. First message has no dot. Application headings are separate from the session gutter and do not cover the thread or robot. These decorations are not interaction targets.

<a id="sessions-metadata"></a>
## Metadata

Folder and optional Git branch each occupy one compact line at the end of details, outside request card. Home prefix becomes `~`; M handles exact home as `~`, G currently produces `~/`. Full directory tooltip retained. Text selectable on native platforms, ellipsized if long (G middle; H end). No Directory:/Branch: text prefixes. Folder/branch icons are secondary12px. Branch missing means omit row; unknown branch is not fabricated. Metadata cannot currently be disabled in native settings; H has a Show folder and branch toggle.

Git symbolic branch is preferred; detached checkout displays `HEAD · <short commit>`. Unborn branches can display their symbolic name; missing/non-directory/non-repository omits branch. Codex database title prioritizes nonblank name, then nonblank title, then Untitled; archived rows excluded when schema supports archived. Title display is not guessed from the latest tool command.

The expanded-session footer also includes an optional model row after directory and branch, using the same secondary11-point text,16-point row height and1-point inter-row gap. M uses the `cpu` symbol; G uses `computer-chip-symbolic`; H uses Lucide `cpu`. Text uses `Session::model_label`: the provider-supplied display name is used verbatim, followed by the session-reported reasoning level when available (e.g. `GPT-6.1-Sol Medium`). Codex reads exact slug→display_name entries from its own `models_cache.json` under the configured Codex home during discovery; changes appear on subsequent snapshots. Missing/unreadable/malformed/oversized catalogs, missing entries and blank display names fall back to the raw model identifier. No capitalization, separator replacement or guessed model names. Known effort labels are None, Minimal, Low, Medium, High, Extra High (`xhigh`), Max, Ultra; unknown effort strings remain unchanged. Text is selectable in M and truncated consistently with other metadata. Tooltip: “Model: <identifier>”. Hide the entire row when missing/blank; never show a global/default model as though it belonged to the session. Model changes arrive through ordinary monitor snapshots without altering status, disclosure, answers or focus.

Codex reads `threads.model` when the database schema supports it; null/blank/missing columns fall back to the last nonempty `turn_context.payload.model` found in the existing4MiB rollout tail. Database values take precedence over potentially older journal context. Reasoning effort comes from `threads.reasoning_effort`, falling back to the matching latest model context's `payload.effort`; never use a global/catalog default or carry effort from a different model. Older sessions whose model context is outside that bounded tail can remain unknown. Claude reads main assistant `message.model`, excluding sidechains and `<synthetic>` responses, retaining its last known identifier when later records omit it. Persisted hook records default missing model to unknown. Cline currently has no verified model source and hides the row. The model is also available as nullable `model`, `modelDisplayName`, `reasoningEffort` and `modelLabel` in the local sessions JSON API; this change does not add it to system notifications or Telegram messages.


Model catalog source: [Codex models](../../../src/agents/codex/models.rs) `display_names`, [Codex adapter](../../../src/agents/codex/mod.rs) `open`, [Session](../../../src/domain/session.rs) `model_label`.

Source: [Session::working_directory](../../../src/domain/session.rs), [M](../../../src/macos/menu.rs) `session_metadata`, [G](../../../src/linux/ui.rs) `Row::update`, [git](../../../src/app/git.rs) `branch`.

<a id="sessions-footer"></a>
## Footer and empty/error states

Pinned footer is outside main scroll. Full-width separator, denser opaque background, optional `Filter: <query>`, usage rows, version left, Settings/power right. Power **quits** natively; H toggles a local Pause/Resume state and version text only. Settings M closes the menu then opens settings; G opens settings and may leave popup visible while settings active. H toggles an in-footer overlay.

Quota label `Codex <Window::label()>`: Weekly10080min,Daily1440,integer-hour else minutes. Value is rounded/clamped **remaining** percentage `100−used_percent`, not usage consumed. Reset at/past now produces `—` until new report; missing date `—`. >=day uses local abbreviated month/day; shorter windows local HH:mm. M dates/percent columns right-aligned, fixed12gap and widths measured across rows; G horizontal14gap and independently measured row labels. M unavailable quota: Codex usage + one dash; G two dashes; H static74%,Nov7,Weekly. M tooltip includes full reset, observed_at, expired note; G date tooltip raw reset timestamp/observed string.

Native empty states distinguish **No open sessions**, **No visible sessions**, **No matching sessions**. M prioritizes nonblank title query then hidden sessions; G prioritizes empty underlying list/entirely disabled apps then query. H no empty/filter fixture. G shows discovery/preference error at popup top and open-target errors there; M does not render discovery_error in the session popup. Request/history errors have their own controls.

Acceptance: natural sort Task2/Task10 per locale; hide an app and preserve catalog; scroll a one-session and a multi-session group; load history then receive new output/request; verify latest remains above card and one scroller moves everything. Inspect exact-home path, missing branch, empty quota and expired quota separately. All footer tools must remain outside scroll and last-row hover must have clearance.
