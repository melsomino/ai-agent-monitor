# Ambient system indicator

<a id="indicator-structure"></a>
## Purpose and structure

The ambient indicator provides a compact summary without opening the session list: two-robot identity artwork, green working number, filled blue unseen-finished counter, filled orange request counter, in that order. Zero values are omitted. It is not a task progress percentage. Counter meanings are separate from Codex quota in the footer.

**IN-01 (S):** Working/finished totals MUST reflect application visibility, and MUST NOT change just because the popup title query changes. H has no application filtering and calculates fixture counts only. **Pending filtering is unresolved:** M uses `State::pending_count()` on all sessions, G filters applications first. See [D-01](../discrepancies.md#d-01).

Sources: [M menu](../../../src/macos/menu.rs) `build_status_item`, `update_badge`, `State::working_count`, `finished_count`; [G panel](../../../src/linux/panel.rs) `Panel::publish`; [Shell](../../../src/linux/gnome-extension/extension.js) `enable`, `_update`, `_animateEyes`; [H](../../../design/previews/monitor-ui-fragment.html) `recount`, `.aam-bar`.

<a id="indicator-rendering"></a>
## Rendering by frontend

| Property | H | M | G |
|---|---|---|---|
| Base artwork |Raster mask21×18 |Template PDF24×17.6 in26-wide canvas; fallback rectangle.stack |Symbolic SVG square22 canvas preserves artwork ratio |
| Working number |11 regular, #35E466 in blue preview bar |12 regular systemGreenColor |11 normal #33D17A |
| Filled counters |14-high/min14wide,font10 |14high,width14/20/26 for1/2/3+digits,font10 regular |Dynamic circular diameter ceil(max18,textwidth+8,textheight+4)),font10 normal |
| Large totals |No cap |Finished/request cap99+; working uncapped |Uncapped text; disc grows |
| Blue ink |white light/#122534 dark |pale RGB(.45,.78,1),dark ink(.07,.15,.21) |#73C7FF/#122534 |
| Orange ink |surface-dependent |system orange/black |#FF9F0A/#202020 |
| Selected background |Always drawn, not tied to popup lifetime |Native attached NSMenu selection |Shell checked pseudo-class from Visible property |
| Activation |Status span has no toggle handler |Native status click/global shortcut |Primary click or Return/Space + accelerator toggles |
| Tooltip/accessibility |ARIA labels, static preview |“Agent Sessions” + Working / Pending requests / Unseen finished nonzero clauses |Accessible name includes all three numeric counts |

Native M status width is computed from text widths and OS-added button margins; it is deliberately not a fixed full-width capsule. G extension hides its button when the resident bus owner disappears; its availability is distinct from whether any sessions exist. H includes fake time and an appearance switch; neither belongs in native product chrome.

<a id="indicator-animation"></a>
## Working eyes and precedence

M eye activation: **pending total=0 AND visible working total>0**. Working number still appears when a request exists, but the animated working base/eyes are inactive. G eyes activate for working>0 regardless of pending. Both use brightness pairs `(left,right)`:

`(.5,0) → (1,0) → (1,.5) → (1,1) → (.5,1) → (0,1) → (0,.5) → (0,0)`.

With animation preference off, an active working base keeps both eyes fully green. With no active working state, return to ordinary base artwork. M repeated snapshots do not reset phase; a new active period starts phase0. G retains its phase when work stops; updates may redraw without advancing. H has no corresponding eye animation.

<a id="indicator-acceptance"></a>
## Acceptance

- Zero counts: artwork only; no zero placeholders.
- Add one working, one finished and one pending session: order remains green, blue, orange; numbers are regular weight.
- Apply a title query: counts unchanged. Disable an app: working/finished removed; pending obeys each platform's documented current policy.
- Open/close the native popup: M relies on system tracking selection; G checked style follows Visible. Verify persistent native selection on a real desktop (U, not certified by code).
- Disable animation while working: eyes steady when eligible, status labels unchanged. Notification answers never originate from clicking this summary.
