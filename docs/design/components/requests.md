# Requests, approvals, and questions

Current H/M request, command and action surface colors are specified in the [glass surface revision](glass-surfaces.md#glass-surfaces). Request behavior below is unchanged.

<a id="request-structure"></a>
## Card structure and capability selection

An active request is a neutral solid card in the expanded message column, with an orange marker in the gutter. Its content is a question/request body, optional explanation, literal command/context, offered controls, and transient delivery feedback. Directory and branch are below/outside. It does not replace the latest message. The card has11 inset,12radius,1border; nested command block8radius. [Foundation tokens](foundations.md#visual-geometry).

**RQ-01 (S):** Each response MUST retain its offered label and map to its own response ID/value. A choice is never inferred from button position after the request changes. H only simulates this; backend authority belongs to native monitor validation.

M chooses an inline `Panel` for nonempty questions **and empty actions**. Otherwise it builds action controls, direct single-question choices, optional Answer… fallback and Open in editor. G renders actions and questions together, potentially including both. H has separate hardcoded approval/multiple-question fixtures. Mixed actions+questions are therefore not a shared visual contract.

Sources: [M menu](../../../src/macos/menu.rs) `request_controls`, `render_rows`, `request_context`; [inline questions](../../../src/macos/inline_questions.rs) `Panel::create`, `sync`, `submit`; [G requests](../../../src/linux/requests.rs) `Card::new`, `command_block`; [H](../../../design/previews/monitor-ui.html) `questionForm`, `sessionMarkup`, `finish`.

<a id="request-approval"></a>
## Permission actions and command scope

Codex adapter maps current protocol decisions to captions:

| Decision | Caption / context |
|---|---|
| accept |Allow once |
| acceptForSession on file changes |Allow all edits |
| acceptForSession elsewhere |Allow this conversation |
| decline |Deny |
| cancel, with decline also offered |Cancel turn |
| cancel, without decline |Deny |
| execpolicy amendment |Allow similar; shell-quoted argument prefix as detail |
| network amendment |Always allow / Always deny; Host: value |
| explicit permission object |Allow once / Allow this conversation / Deny; permission JSON details |

Available decisions govern alternatives; defaults are synthesized **by the adapter** where Codex omits `availableDecisions`, not by frontends. Unsupported decisions are omitted. Command approval default is Allow once,Deny plus valid amendments/session alternative if available; file approval includes Allow all edits. Option picker can be shown as body but has no implemented decision translation. Claude ordinary permission capability is Allow only. Cline has no remote approval action. See [requests.rs](../../../src/agents/codex/requests.rs) `decisions`, `decision_label`, `decision_detail`, `interaction`; [runtime](../../../src/agents/codex/runtime.rs) `body`.

**RQ-01a — Scoped action disclosure (M/H implemented).** Actions with a nonempty detail (for example **Allow similar**) render as a natural-width split pill with a trailing 28-point disclosure segment, a thin internal vertical divider and a continuous shared surface. There is no gap or detached circular button between the action and disclosure. The detail starts hidden. Clicking the disclosure MUST reveal the literal, selectable monospace prefix/host/JSON below that row; clicking it again MUST hide it. There is no duplicate action-label caption. Revealing details MUST NOT submit an approval. The action remains independently clickable while details are hidden or visible. Disclosure is excluded from custom keyboard traversal and does not take keyboard focus on pointer activation. Expansion is keyed by the request and action, survives unchanged-request redraws, and resets when the popup closes (M) or the renderer is remounted (H). Expanding native details recalculates the popup document height. Revealed scope text is fully shown rather than requiring a second expander. M/H decisions use natural caption widths rather than equal stretched columns. Immediate decisions precede scoped decisions in one wrapping row with a 10 pt gap. A scoped button and its disclosure form an indivisible block for wrapping. **Allow similar** has no checkmark; Allow once retains its checkmark and Deny its cross. Expanded scope text occupies a full-width row below all decision buttons. Expanding/collapsing scope MUST NOT change decision button positions or widths. On insufficient width, wrap only between decision blocks. G retains its existing detailed-action layout; parity is outstanding. Sources: `menu.rs` `request_context`, `request_action_layout`, `respond_to_request`; `monitor.js` `requestMarkup` and scope click handler; `monitor.css` `.aam-scope-row`, `.aam-scope-toggle`.

**RQ-02 — Main command disclosure (M/H implemented).** For a command approval offering **Allow once** (or **Allow**), the main command MUST start fully hidden. The primary decision is a split pill: its text segment submits the existing approval action, and a separate trailing 28-point disclosure segment toggles command visibility. A thin vertical divider separates the segments inside the same continuous surface. There is no separate **Command** caption or command button above the decisions. The disclosure MUST NOT submit, select or change the approval decision, and MUST remain outside custom keyboard traversal without taking focus on pointer activation. Native tooltips/accessibility labels are **Show command** / **Hide command**.

Expanding shows the full literal, selectable monospace command in a full-width row below all decision buttons. Neither explicit newlines nor soft wrapping are clamped after expansion. Collapsing hides the entire command. The combined approval/disclosure width is reserved before wrapping: the segments MUST stay together, and opening or closing details MUST NOT change decision positions or widths. Expansion remains keyed by approval/request context, survives unchanged-request redraws and resets on popup close (M) or renderer remount (H). Command scope details remain independent under RQ-01a, and can be opened alongside the main command. Sources: M `command_action`, `action_disclosure`, `request_context`, `request_action_layout`, `respond_to_request`; H `requestMarkup`, command click handler and `.aam-command-split`.

M/H command panels retain 10 pt/px secondary-colored monospaced text, 6 pt/px vertical and 8 pt/px horizontal padding and no Copy button. Native command background is #DADADA/#2A2A2A (light/dark), darker than the card. For requests lacking a matching Allow decision, M retains its caption-row two-rendered-line expander fallback; H offers a standalone **Command** disclosure pill. This fallback is a platform discrepancy. GNOME retains its existing command presentation and has not adopted the split primary action. No generic “Future commands with prefix:” belongs to current native detail formatting.

<a id="request-single"></a>
## One question

**RQ-03 (S):** A native single-question preset is a submit button; clicking/Enter submits that choice directly. It does not require an additional Send for a preset. Choices preserve order. A freeform field appears only when `freeform=true`; secret uses a secure input. When no freeform is offered, no custom input is invented. With no choices, adapter sets freeform for Codex questions.

M displays title then vertically stacked natural-width preset buttons, description under each if provided. Freeform field and Send answer share a final row; Send enabled only for a valid nonblank custom response. Enter in active custom field selects/commits custom answer **without submitting or dismissing**. M field receives native editing/copy/paste behavior within menu tracking. G displays preset action buttons and final GTK Entry+Send row, descriptions as tooltips. Its one-question Entry has no explicit activate handler; Return uses GTK entry behavior, not a guaranteed identical custom-selection action. G shows presets even for a `secret` single question; M native notification policy differs from G. H fixture only demonstrates multiple-question radio forms; no real single-question form branch.

<a id="request-multiple"></a>
## Multiple questions

**RQ-04 (S/T):** Render independent radio groups in request order, each with title, ordered choices, and custom row only if offered. Choosing a row changes only that question's draft. Selected preset changes radio state; label text remains normal unless focused/hovered. Shared Send answer submits a complete answer map. Empty custom values MUST NOT be sent. Defaults select first preset when present. A custom-only question remains incomplete until entered.

M multi radio visual:16outer,6white inner dot,accent selected circle; text area highlighted to measured caption width rather than full row. Focus uses system accent with selected ink; hover same geometry with normal hover fill. G uses GTK RadioButton group styling and label-only hover/focus. H native HTML radio14 with7gap; first choice checked; both custom radio and input are DOM targets. M custom radio/button serves as focus proxy for editing; G custom radio not focusable, Entry is.

Custom focus/editing selects own-answer option in M and G. M text change selects it even if empty; G changed handler selects on nonempty, and focus-in/activate selects. H only input event selects custom; focusing alone does not. M Enter/Space activates a focused preset; Enter in input chooses custom without send; G standard GTK radio/Button keyboard semantics plus global Up/Down routing; H Enter radio selects, Space browser-default selects. **Selection is not keyboard focus.** A selected radio may coexist with a different focused answer, but only one keyboard-target highlight is allowed natively.

Native M Send disabled if incomplete, yet included in custom traversal as a disabled submit target; activation checks `isEnabled`. G Send stays sensitive while incomplete and quietly returns without sending; its traversal includes sensitive targets. H uses browser custom validity on submit and validation string “Enter your own answer or select an option.” No H Sending/Failed/reconciling presentation.

<a id="request-focus"></a>
## Focus and draft lifetime

**RQ-05 (S/T):** One logical keyboard target across entire popup: title, action/submit, input or preset. Focus must not imply selected answer or send. Request removal clears response focus rather than transferring it to a title. On opening an actionable request, native popup expands it and focuses/reveals first available response; command expanders are excluded. See full [keyboard table](../interactions/input-lifecycle.md#input-keyboard).

M preserves exact request drafts by panel cache matching session key+approval+interaction. Re-render suspends/restores editing and selection; editing target synchronizes global focus. Changed request rebuilds panel, so stale drafts are not reused. Popup close stops editing/disclosure/history; panel cache can remain for active requests, so drafts **can survive close/reopen**. G unchanged card token preserves drafts during snapshots/filtering; hide removes cards and clears token, so drafts reset on reopen. H expansion can persist through widget state, but form drafts do not have serialized persistence.

M caret: native editor when key window; custom blink drawing for non-key NSMenu tracking. `CaretBlink::visible` uses500ms cadence and resets on selection movement; ring extends6outside field. This is implementation-specific, not a requirement for custom painting elsewhere. Actual caret/native focus visibility and IME behavior need runtime QA.

<a id="request-failures"></a>
## Disabled, unavailable, and failed requests

| Condition | M inline/native controls | G | H |
|---|---|---|---|
| Sending |Disable answer controls, “Sending…” for inline |Whole card insensitive, “Sending answer…” |Immediate local success; no waiting |
| Reconciling |Disable, “Checking delivery…” |Insensitive, same Sending answer… |Absent |
| Failed |Inline error + valid retry |Row error + reenabled card |Absent |
| Non-authoritative |Disable |Whole card insensitive |Absent |
| Expired but retained |Monitor rejects; main M controls have no explicit expiry gate/copy |Insensitive, explicit expired/open-agent error |Absent |
| Invalid custom |Submit disabled/validation error |Click silently does nothing |Browser validity bubble |
| Enqueue error |Tooltip on ordinary response; inline status string |Most callbacks ignore enqueue result |Absent |
| No supported actions/questions |Open in editor fallback |Body card, no explicit open-editor request control |Not modeled |

M ordinary action failures do not have the same inline status renderer as question panels; do not promise universal error copy. A fallback `AnswerForm` normal window remains for mixed/unusual routes and mailbox fallback:520wide, title=session title, floating, scroll bounded80% usable height, dropdown “Choose an answer…” then optional secure/text field per question, Open in editor and Send answer. Custom nonblank overrides dropdown; no default preset. Form closes when approval disappears/changes; reconciliation matches approval but not full interaction context (monitor still rejects stale submission). Standard Return default sends there, unlike inline Enter. H/G have no equivalent fallback window.

<a id="request-acceptance"></a>
## Acceptance cases

1. Test0/1/3/5 choices; one vs two questions; freeform on/off per question; custom-only and secret; multiline captions and descriptions. Do not assume async chat questions expose `freeform=false`—that path sets true; use adapter fixture for capability coverage.
2. Review a long command containing explicit newlines and soft wraps. Native collapsed text ≤2 rendered lines; expander visible only if needed and skipped by Up/Down.
3. Preset selection in a multiple form changes radio only; Space/Enter picks it. Input Enter leaves menu open; Send creates one response only if complete.
4. Change only command/options under same approval ID: controls rebuilt, old controls rejected. Repeat unrelated snapshots: draft/caret/focus remain where source supports it.
5. Resolve in agent or Telegram while local field focused: request and old focus disappear after reconciliation; message stays. Test sending, uncertain, failed, expired and unsupported separately.
