# Session list, expanded content, and footer

<a id="sessions-membership"></a>
## Membership, ordering, filtering

**SE-01 (S/T):** Display open discovered sessions including idle. Do not substitute a history archive for live discovery. Filter by enabled application before rendering. Filtering never deletes the underlying session or stops its agent. Refresh automatically from monitor snapshots; no Refresh control in the native popup or current Telegram keyboard.

M/G grouping rank: VS Code, Terminal, identified desktop applications, Unknown. Then application ID, then natural title order, then stable session key. M uses Foundation localizedStandardCompare, G uses lowercase GLib FilenameCollationKey; locale/case equivalence is not guaranteed. H fixtures are fixed within explicitly enumerated VS Code/Terminal/Codex groups, not dynamically sorted.

No application-name heading row in the popup. An actual application icon floats in the left gutter as the group identifier. Native icon fallback is generic app if unresolved. M resolves bundle ID and file icon; G resolves desktop-entry/theme icons. G desktop clients beyond VS Code/Terminal fall back to a generic executable icon. H embeds fixture app icons, not OS resolution. The icon may cover a first-row robot because it has the higher drawing layer; each session still has its own robot control underneath.

**SE-02 — Sticky group mark (S/T):** The icon tracks the first header until it reaches the viewport top, stays there while its group remains visible, then is pushed by the group end. It MUST NOT escape its own group. Native shared geometry is `max(start,viewport_top).min(max(start,end−36))`. H uses CSS sticky grid overlay. Native icon decoration is click-through; it does not open the host independently.

Sources: [M](../../../src/macos/menu.rs) `State::filtered_sessions`, `render_rows`, `client_icon`, `update_sticky_icons`; [G](../../../src/linux/model.rs) `eligible`, [ui](../../../src/linux/ui.rs) `refresh`, `draw_application_icons`; [shared geometry](../../../src/domain/layout.rs) `sticky_y`; [H](../../../design/previews/monitor-ui-fragment.html) `build`, `syncGutter`.

<a id="sessions-header"></a>
## Session header anatomy and targets

| Element | Visible states | Pointer/activation | Keyboard |
|---|---|---|---|
| Robot |Neutral head; green eyes working; orange eyes request |Separate rounded icon hover; disclosure toggle |Excluded from native custom Up/Down traversal; H normal tab stop |
| Title |One-line, semibold, truncated, filter match emphasis |Measured rounded hover; click opens host |Native title traversal target; Enter opens; Right expands,Left collapses |
| Working status |Green circular arrows |Informational, tooltip Working |No answer/disclosure action |
| Needs response |Orange filled ! |Informational; does not grant permission |Not an approval target |
| Finished unseen |Blue filled check |Native click acknowledges current completion without closing/expanding; H status span inert |Not in native custom traversal |
| Unknown |M muted questionmark.circle; G muted “?” drawing |Informational |No approval action |
| Chevron |Right collapsed; rotates down expanded |Separate icon hover/click toggle |Not in native custom traversal; H normal tab stop |

**SE-03 (S):** Opening the host is distinct from disclosure. Robot/chevron toggle; title opens. Tooltip is “Switch to <application>” in the native frontends and H. Working/request glyphs occupy space before the chevron. Idle has no status glyph. Title hit/highlight ends after measured text plus padding and excludes the right-hand controls. H/M/G differ in exact highlight origins and target size; [geometry](foundations.md#visual-geometry).

**SE-04 (S):** Multiple sessions can be expanded. On opening, native working/request sessions expand. Within an open popup, manual collapse survives unchanged updates. A new request reopens its session. M recognizes new approval ID and first working occurrence per open lifecycle; G recognizes changed approval/interaction token but does not auto-open an already-known idle row merely because it later becomes working. H initially expands fixture `delivery` only. See [D-04](../discrepancies.md#d-04).

<a id="sessions-expanded"></a>
## Expanded reading order and messages

Shared native reading order: header → earlier messages/current latest → active request card → folder/branch metadata. Request does not replace current message. Trailing whitespace is trimmed on message display. Native empty message reads **No messages yet**. H supplies populated fixture messages and a fallback previous fixture only.

M builds earlier chronological labels plus an anchor if the previously-latest message changed; when a request is present, includes nonempty latest before the card. G appends current latest to window history; no anchor preservation. Both are selectable native formatted text, and the main list is the sole scrolling region. H uses explicit prewritten HTML (headings, links, lists, code); it has no backend parser or arbitrary-message ingestion.

Markdown native common source: [parser](../../../src/domain/markdown.rs) `parse`. Supports emphasis,strong,strike,headings,ordered/unordered/task lists,quotes,inline/fenced code,hard breaks and rule text. Soft breaks become spaces. Raw HTML is literal text, not executed. Images are not fetched. Links are only HTTP, HTTPS or mailto; local/command URLs stay text. Tables are not enabled as a dedicated layout. There is no native syntax highlighter. M renders attributed NSTextField; G renders Pango markup. G preserves textual list/quote projection but does not reproduce M's paragraph indentation; exact typography is platform-specific.

Sources: [M markdown](../../../src/macos/markdown.rs) `apply`; [G model](../../../src/linux/model.rs) `markup`; [G widgets](../../../src/linux/widgets.rs) `message`; [H CSS/fixtures](../../../design/previews/monitor-ui-fragment.html) `.aam-message`, `sessionMarkup`.

<a id="sessions-history"></a>
## Previous message gutter

**SE-05 (S):** Previous loads one earlier agent message and prepends locally; it does not replace or send a new message to the agent. Clicking it acknowledges an idle completion. Separate history cursors protect Telegram/window independence.

M: 16-wide clickable gutter beside first message, arrow10×12 centered vertically over its full message height. Decoration appears only over first-message/gutter region, including scroll-driven pointer resampling. Normal target remains allocated even when decoration hidden. Disabled during loading/exhaustion; tooltips distinguish Loading previous message…, No earlier agent messages, and retryable failure. G:34-wide gutter region,16-wide trailing previous button,2 end margin; pointer sampled each frame over first message event bounds. Button disabled for loading/exhaustion; gutter handler emits click, callback rechecks loading/exhaustion. History error appears in row feedback rather than equivalent per-state tooltips. H:12-wide gutter over first visible fixture message,8px glyph; pointer test uses the message's **vertical y band anywhere within product**, not x containment. Clicking reveals exactly one fixture and hides the button; no loading/exhausted/error state.

Expanded thread: subtle line from robot's contour, interrupted at request icon and filled5px message dots for second and later messages; no ring around robot and no terminal end-dot in current code. Native M line1 at label10%, dots25%, orange request disc15. G line.5 at fg22%, dots25%, request disc16; H actual SVG-stroke mask. First message has no dot. The floating app icon may cover thread/robot at the top of group. These decorations are not interaction targets.

<a id="sessions-metadata"></a>
## Metadata

Folder and optional Git branch each occupy one compact line at the end of details, outside request card. Home prefix becomes `~`; M handles exact home as `~`, G currently produces `~/`. Full directory tooltip retained. Text selectable on native platforms, ellipsized if long (G middle; H end). No Directory:/Branch: text prefixes. Folder/branch icons are secondary12px. Branch missing means omit row; unknown branch is not fabricated. Metadata cannot currently be disabled in native settings; H has a Show folder and branch toggle.

Git symbolic branch is preferred; detached checkout displays `HEAD · <short commit>`. Unborn branches can display their symbolic name; missing/non-directory/non-repository omits branch. Codex database title prioritizes nonblank name, then nonblank title, then Untitled; archived rows excluded when schema supports archived. Title display is not guessed from the latest tool command.

Source: [Session::working_directory](../../../src/domain/session.rs), [M](../../../src/macos/menu.rs) `session_metadata`, [G](../../../src/linux/ui.rs) `Row::update`, [git](../../../src/app/git.rs) `branch`.

<a id="sessions-footer"></a>
## Footer and empty/error states

Pinned footer is outside main scroll. Full-width separator, denser opaque background, optional `Filter: <query>`, usage rows, version left, Settings/power right. Power **quits** natively; H toggles a local Pause/Resume state and version text only. Settings M closes the menu then opens settings; G opens settings and may leave popup visible while settings active. H toggles an in-footer overlay.

Quota label `Codex <Window::label()>`: Weekly10080min,Daily1440,integer-hour else minutes. Value is rounded/clamped **remaining** percentage `100−used_percent`, not usage consumed. Reset at/past now produces `—` until new report; missing date `—`. >=day uses local abbreviated month/day; shorter windows local HH:mm. M dates/percent columns right-aligned, fixed12gap and widths measured across rows; G horizontal14gap and independently measured row labels. M unavailable quota: Codex usage + one dash; G two dashes; H static74%,Nov7,Weekly. M tooltip includes full reset, observed_at, expired note; G date tooltip raw reset timestamp/observed string.

Native empty states distinguish **No open sessions**, **No visible sessions**, **No matching sessions**. M prioritizes nonblank title query then hidden sessions; G prioritizes empty underlying list/entirely disabled apps then query. H no empty/filter fixture. G shows discovery/preference error at popup top and open-target errors there; M does not render discovery_error in the session popup. Request/history errors have their own controls.

Acceptance: natural sort Task2/Task10 per locale; hide an app and preserve catalog; scroll a one-session and a multi-session group; load history then receive new output/request; verify latest remains above card and one scroller moves everything. Inspect exact-home path, missing branch, empty quota and expired quota separately. All footer tools must remain outside scroll and last-row hover must have clearance.
