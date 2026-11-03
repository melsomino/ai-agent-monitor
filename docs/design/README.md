# AI Agent Monitor UI/UX specification

<a id="spec-index"></a>
## Authority, scope, and evidence

This is the living, platform-independent product specification, reverse-engineered from the repository. It describes existing behavior, **not a redesign**. Its baseline is commit `aa1601582edcf768767f23b8d3dd960dfc4a9461`, inspected on 2026-11-02. The pre-existing local `build-number.txt` change is outside this specification. Application code, resources, preferences, and tests were not changed for this work. Copy maintenance on 2026-11-03 translated the architecture document and HTML prototype to English; prototype event logic and native implementations were unchanged. Corresponding copy descriptions below reflect that update.

Three frontend implementations are compared throughout:

1. **H — HTML:** [standalone interactive prototype](../../design/previews/monitor-ui.html). Primary reference for visual design and demonstrated interaction intent, including its JavaScript, not just its appearance.
2. **M — macOS:** Rust/AppKit. Primary reference for established native behavior, with OS-specific constraints kept separate.
3. **G — GNOME:** Rust/GTK 3 plus the GNOME Shell extension, including Wayland positioning and activation.

Neither H nor M automatically wins a disagreement. Shared requirements below are only those supportable by the shared implementation or consistent native behavior. Differences are recorded in the [parity matrix](parity.md) and [discrepancy register](discrepancies.md). Prototype intent alone is not a claim of production support.

**Evidence vocabulary:**

| Label | Meaning |
|---|---|
| **S** | Verified by reading the rendering/event/state code at this baseline. Not a claim of execution or visual QA. |
| **T** | A relevant existing automated assertion was inspected. Tests were not executed in this documentation task. |
| **P** | Platform-specific implementation or OS-owned behavior; stated for that platform only. |
| **I** | Explicit intent in existing documentation or prototype, absent or incompletely implemented in production. |
| **U** | Cannot be established confidently from source; needs runtime, OS, accessibility, or product verification. |
| **D** | Implementations conflict, or a documented contract is not matched by code. |

Normative **MUST / MUST NOT** statements are observable contracts backed by the implementation. They do not authorize fixes or turn discrepancies into decided requirements. Acceptance criteria are reproducible checks, **not results of a fresh test run**. An OS-owned action is qualified where source cannot guarantee its appearance or completion.

<a id="spec-navigation"></a>
## Reading map

| Document | Responsibility |
|---|---|
| [Shared specification](shared.md) | Product boundaries, surfaces, cross-channel invariants, capability model |
| [State models](state-models.md) | Activity, completion, requests, delivery, history, persistence |
| [Visual foundations](components/foundations.md) | Geometry, typography, colors, icons, motion, theme comparison |
| [Ambient indicator](components/indicator.md) | Menu bar/panel counts, eyes, selection, tooltip |
| [Sessions and content](components/sessions.md) | Membership, grouping, sticky icons, header targets, expanded content, footer |
| [Requests and forms](components/requests.md) | Approvals, command details, single/multiple questions, custom/secret input |
| [Settings](components/settings.md) | Every native pane, defaults, conditions, persistence, setup states |
| [Input and lifecycle](interactions/input-lifecycle.md) | Opening, dismissal, geometry, scroll, pointer, focus, keyboard/filter |
| [Notifications](interactions/notifications.md) | Native action planning, binding, replacement, dismissal, stale responses |
| [Telegram](interactions/telegram.md) | Setup/pairing, browser, messages, request response and transport errors |
| [macOS supplement](platforms/macos.md) | AppKit-specific implementation and constraints |
| [GNOME supplement](platforms/gnome.md) | GTK/Shell/Wayland-specific implementation and constraints |
| [HTML supplement](platforms/html.md) | Prototype state machine, fixtures, embedding, responsive behavior and limits |
| [Three-way parity matrix](parity.md) | Coverage and differing behavior per frontend |
| [Discrepancies and questions](discrepancies.md) | Undecided conflicts, intended-only behavior, unknowns |
| [Coverage and traceability](coverage.md) | Source inventory, assertion inventory, verification limits, maintenance procedure |

Architecture remains in [actor architecture](../actor-architecture.md); integration internals remain in [agent integrations](../integrations.md) and [Ubuntu documentation](../ubuntu.md). Those documents are not duplicated here. This specification replaces the earlier macOS-focused style guide. Its visual tokens, interaction rules, settings, external channels and manual acceptance cases are covered by the component documents and platform supplements. Earlier design intent is distinguished from verified implementation; unresolved conflicts remain in the discrepancy register.

<a id="spec-maintenance"></a>
## Maintaining this specification

Preserve explicit anchor IDs and contract IDs when reorganizing prose. A behavioral change should update its component, acceptance criteria, parity row, and any affected discrepancy. Record the inspected revision and evidence scope. Do not mark a runtime issue verified merely because it was fixed in source. Resolve a conflict only with an explicit decision and evidence; retain a short historical note if it explains platform divergence. Relative source links plus symbols are preferred to volatile line numbers. A developer implementing a new frontend should read shared/state documents first, component contracts next, then platform exceptions and unresolved decisions.
