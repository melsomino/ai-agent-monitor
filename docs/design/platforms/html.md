# Interactive HTML frontend supplement

<a id="html-authority"></a>
## Implementation and scope

The prototype is an interactive third frontend and primary visual reference. The single source is [monitor-ui.html](../../../design/previews/monitor-ui.html), a standalone document containing the product markup, styles and interaction handlers directly. The former fragment and iframe wrapper were consolidated on 2026-11-03. Floating UI supplies tooltips and Lucide supplies icons; both load from pinned CDN URLs. Generic tabs/carousel runtimes and iframe host messaging are removed.

The source demonstrates the session list/detail/question form and palette well; it does **not** implement approximately99% of all current product behavior when including settings, discovery, agent delivery, notifications and Telegram. This is an observed coverage limit, not dismissal of its design authority. Geometry and interactions are specified as they exist, including divergences from production.

Source symbols (single closure): `setPreviewTheme`, `applyExpanded`, `rememberExpanded`, `status`, `questionForm`, `sessionMarkup`, `build`, `syncGutter`, `updatePreviousHover`, `recount`, `acknowledgeFinished`, `finish`, `tune`; product click/input/change/submit/keydown listeners.

<a id="html-state"></a>
## Fixture state machine

- One balanced stage currently rendered; old compact/multiple-variant history is not an active selector. `sessions` is fixture data, cloned per stage; groups are fixed VS Code,Terminal,Codex in fixed fixture order, no backend discovery/sorting.
- Initial expanded fixture ID `delivery`; other opens/collapses independent. `aria-expanded` updated on robot/chevron, clicked control's label changed; programmatic Left/Right only updates expanded attributes/classes and does not call finished acknowledgment/persistence.
- `recount` derives working,finished,request from local status strings; hides zero counters. No automatic async updates or source uncertainty.
- Title click simulates navigation by changing footer version text. It does not open editor, window or project.
- Disclosure/title/history click acknowledges finished by setting local status idle and removing its status icon. Direct finished-disc click has no handler.
- Permission data-action allow → working, deny → idle. Hide question; overwrite latest content with fixed English response/cancel text; blur focus only if inside question. No queued/Sending/Reconciling/Failed/backend stale snapshot.
- Form first choice checked; typing selects custom; submit validates all fieldsets then same local finish(true). Enter text selects custom and prevents send; Enter radio selects and dispatches change. Input focus alone not select-custom.
- Previous reveals one fixture history paragraph, hides previous control permanently for that fixture. No request/loading/error/cursor state.
- Settings gear toggles local overlay. Show metadata control hides metadata. Launch/animation checkboxes have no persistence or actor side effects. Power toggles local paused/aria-pressed and footer text, does not quit or suspend fixtures.
- Disclosure clicks save expansion to browser localStorage under `aam-preview-expansion`, using `privateContent.expanded[layout]`; startup restores it when storage is available. Storage/JSON errors are ignored, leaving fixture defaults usable. No migration of the former host-specific storage key. Other form/filter/settings state is not serialized.

<a id="html-events"></a>
## Visual mechanics, keyboard and responsive behavior

Grid detail uses0fr→1fr/180ms ease, clip visibility, chevron rotate90°. Session thread pseudo-element uses measured header height and SVG-stroke sample mask around request marker/dots. MutationObserver watches icon replacement/children; ResizeObserver watches session groups; requestAnimationFrame initial sync. Application headings scroll with their group: 24px icon and secondary 11px/600 name followed by a separator, 26px reserved above the first session, 6px gap between groups. No sticky positioning or icon-over-robot overlay. Title-highlight right bound measured button right+12; left29. The button reserves an 8px gap then a 14px right arrow, 4px gap and 16px actual application icon; decorations become visible on hover/focus without reflow. There is no pressed-state custom CSS for main controls (browser default may apply); disabled behavior mostly generic HTML and test visibility filtering, no stateful transport disable branch.

`updatePreviousHover` measures first visible message and makes arrow span its whole height, positioned left−15. Pointer tracked only y, so any pointer x inside product at first-message vertical band shows it. Scroll reevaluates pointer. Pointerleave clears. Arrow glyph8px, button12wide. This is not equivalent to actual first-message/gutter x containment.

Custom Up/Down list titles,actions,answer inputs; clamped ends, scrollIntoView nearest. Browser Tab also includes robot/chevron/history/settings/power/theme buttons; Left/Right only title disclosure, not answer cycling. No global filter, Escape, close or shortcut recorder. Browser native radio group arrows may interact with bubbled custom navigation; source-only inspection does not establish exact per-browser ordering. H allows simultaneous hover and keyboard focus, standard DOM single activeElement.

Product fluid≤410; list max360; footer pinned outside. At viewport≤420 stage horizontal padding removed and max-width100%; no native screen/Dock bounds. Coarse pointer raises row/action/toggle/open/footer tool minima44 (group heading stays24) and input min36/font16; radio rows min44. Working status is a static15px green disc in an18px viewport; solid6.72×8.32 rightward play path with its centroid at the18×18 SVG viewport center, white in light appearance and dark green in dark appearance. Working robots retain their normal size and opacity; only their eyes animate through eight brightness pairs (left/right): .5/0, 1/0, 1/.5, 1/1, .5/1, 0/1, 0/.5, 0/0, interpolated linearly over2.4 seconds. Unlit eyes disappear; reduced-motion leaves both steadily green and removes detail/chevron transitions. No robot scaling or working-status rotation. Theme switch Light/Dark left of fake menu bar, aria-pressed, initial system preference, colorScheme root. The blue bar remains blue in both themes. Optional global Tweak sliders row28–42 step2,message12–15,metadata toggle are preview tools, **not native settings**.

<a id="html-content"></a>
## Content, security and accessibility limits

Fixtures include trusted preformatted HTML, embedded icon assets and Lucide replacement. No shared Markdown parser is invoked in H. No permission/network/secret input branch, expired state, hidden application catalog, login actual state, structured backend response, or actual notification/Telegram rendering. ARIA labels exist for controls and groups, icons aria-hidden, radio fieldset legends, version aria-live; English fixture copy. Does not establish WCAG/VoiceOver/Orca conformance. Browser keyboard/scroll/focus and CDN library loading require runtime verification.

Acceptance supplement: real browser light/dark and browser zoom; long title while working/request; application headings scrolling with their groups; interrupted CSS disclosure; first-message band hover; keyboard in radios/custom; invalid empty custom submit; reload expansion restore; unavailable-localStorage fallback. These tests are independent from production native parity; do not use simulated `finish` to claim response delivery.
