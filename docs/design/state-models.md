# Observable state models

<a id="state-session"></a>
## Session identity and presentation data

Source: [domain/session.rs](../../src/domain/session.rs) `Session`, `Client`, `Activity`, `Approval`.

| Field/concept | Observable interpretation |
|---|---|
| `agent` + `id`, `Session::key()` | Stable session identity (`agent:id`); equal native IDs from different adapters remain distinct |
| `title` | Primary one-line session heading, natural-sort and title-filter input |
| `client` | Host application, distinct from agent; determines grouping, application visibility and navigation label |
| `cwd`, `workspace`, `git_branch` | Location metadata; workspace takes precedence for project navigation |
| `working_directory()` | Workspace path unless it is a `.code-workspace` file, in which case use cwd; prevents treating a multi-root descriptor as a directory |
| `last_message` | Latest assistant output, independent of active request |
| `authoritative` | Current live evidence sufficient for sending; false does not itself hide the session |
| `approval` | Optional active request: opaque ID, body, expiry, legacy `can_allow` capability |
| `interaction_data` | Optional literal command, action list and/or question list; not implied by `Activity::Confirmation` alone |
| `target` | Adapter-produced navigation target; not calculated from display text |

`Client` values are Terminal, VSCode, Desktop{name, app_path}, Unknown. Application preference IDs are `terminal`, `com.microsoft.VSCode`, `desktop:<app_path>`, `other`. Terminal grouping does not necessarily expose the actual terminal vendor as a distinct group.

<a id="state-activity"></a>
## Execution activity and completion overlay

Execution values: **Working**, **Idle**, **Confirmation** (“Needs response”), **Unknown** (“Unknown status”). Codex active flags `waitingOnApproval` or `waitingOnUserInput` yield Confirmation; `systemError` yields Unknown. Pending async questions can persist after a turn becomes idle; the adapter exposes them as a request, and discovery gives request activity precedence.

Codex currently exposes at most one active request envelope per session: first recognized runtime request (command/file/permissions/user input/option picker/MCP elicitation), otherwise aggregated async questions. If runtime says waiting but no recognized request exists, a non-actionable fallback approval body says Codex is waiting for your confirmation or input. Direct tickets expire at `u64::MAX` and live ownership/current-envelope validation supplies lifetime. A new envelope normally produces a new opaque ticket; same-ID mutation checks remain necessary for other sources. Async questions clear when their explicit accepted answer IDs appear or later ordinary user input supersedes them; rejected/pending steering input does not resolve them. Source: `Runtime::views`, `async_questions::pending`.

**ST-01 (S/T):** An authoritative Working → authoritative Idle observation for the same session creates an unseen completion generation. An initial Idle session is not newly finished. Confirmation → Idle does not by itself create one. Source: [monitor/state.rs](../../src/actors/monitor/state.rs) `State::reconcile_finished`.

| Following observation/action | Completion result |
|---|---|
| Session gone | Marker removed |
| Authoritative Working or Confirmation | Marker removed |
| Idle, or non-authoritative observation including Unknown | Existing marker retained |
| Authoritative Unknown | Existing marker retained by the implemented predicate |
| `FinishedSeen` with matching generation and current Idle | Marker removed |
| Old generation acknowledgment / current non-Idle | No removal |
| Popup merely opened, row visible, popup closed | No acknowledgment |

Local native actions acknowledge completion when opening the host, expanding/collapsing, loading previous output, or clicking finished status. Acknowledgment does not change agent execution state. Telegram has no completion marker/acknowledgment implementation; H clears its fixture's finished state on title/disclosure/history clicks, but the finished disc itself is not clickable.

<a id="state-request"></a>
## Human interaction model

Source: [domain/interaction.rs](../../src/domain/interaction.rs). An `Interaction` has `command: Option<String>`, actions `{id,label,detail}`, and questions `{id,title,choices,freeform,secret}`. Choice `{label,description}` uses the label as the submitted value. Questions do not have an independent explanation field; descriptions belong to choices and a request body may serve as explanation. The older architecture document's richer model is not the exact implemented schema.

**ST-02 — Valid answer set (S/T):** At least one question, exactly one nonblank string per question ID, and exactly the number of IDs offered. If `freeform=false`, that string must equal an offered choice label. Secret affects presentation/channel policy, not answer validation. No implemented multi-select answer model, despite wire values being vectors.

**ST-03 — First in-flight response (S/T):** For a current unexpired authoritative request and valid response, submission enters Sending if the adapter queue can accept it. Another Sending/Reconciling submission for that key does nothing. A full queue produces Failed (“Agent queue is busy. Try again.”). A stale/invalid/expired `Submit` is ignored at this boundary; no universal visible failure is guaranteed.

| Delivery evidence | Observable state and permitted next action |
|---|---|
| Confirmed | Approval and interaction suppressed from subsequent snapshots for the same approval; submission removed |
| NotSent | Failed: “Could not send the answer. The request may have changed.”; valid retry can be offered |
| Uncertain | Reconciling; repeated submission blocked |
| External resolution observed | Request absent; submission removed, all surfaces reconcile |
| Discovery error | Keep last successful source sessions; expose `discovery_error` |
| Non-authoritative session with no approval in new observation | Copy previous approval and interaction; preserve request, sending remains disallowed |
| Adapter stopped | Retain source sessions as non-authoritative Unknown; active submissions/history/message sends receive failure state |

Source: [monitor/mod.rs](../../src/actors/monitor/mod.rs) `run`, `valid_response`; [Codex runtime](../../src/agents/codex/runtime.rs) `deliver_for`, `respond_inner`, `request_resolved`. Confirmed suppression removes approval/interaction, but does **not** forcibly set `activity=Idle`; activity may briefly remain Confirmation until the adapter reports its next state. Do not infer readiness from a single missing icon or request field.

Hook requests can overlay/create sessions until expiry or hook loss. Legacy hook approval waits are time-limited (60 seconds in core); direct Codex requests use their own current-request evidence. These are not a shared frontend timeout. Closing a popup or dismissing a notification never returns Allow.

<a id="state-history"></a>
## History, sends, and local presentation

`Session::images` carries bounded current-turn Codex tool images; history `Message::images` carries images on an individual earlier page. Buffers are shared, not copied into every snapshot, and contain no external file/URL fetch instructions. See [Telegram media contracts](interactions/telegram.md#tg-images).

`History` stores messages, cursor, loading, exhausted, error, generation per `(session, Window|Telegram)`. Starting with no cursor resets that consumer's history; same-generation duplicate commands do not reread. A matching completed read prepends one message, `None` means exhausted, an error leaves retry possible. Late/wrong-generation completion is ignored; session removal prunes history. macOS additionally maintains an anchor to retain a message that was latest when history began; GNOME composes fetched history with current latest without that anchor.

Chat message sends are separate from answers: IDs deduplicate commands, text must be 1–16000 characters after nonblank validation, session must exist, be visible by application setting, and authoritative. Up to 256 retained send entries, evicting completed entries first. States Sending → Sent or Failed; unsupported adapters return a capability error. Codex uncertain send says to check the agent before sending again. No local popup composer is implemented; this is observable in Telegram.

Presentation state is channel-local: hover, focus, typed query, disclosure, draft, command expansion, scroll and history view. The monitor snapshot is the common outcome source, not the current pointer or widget state. See [input lifecycle](interactions/input-lifecycle.md).

<a id="state-persistence"></a>
## Persistence contract and limits

| Item | Retained across restart? | Evidence |
|---|---|---|
| Known apps + visibility | Yes, including closed/disabled apps | `Applications::observe`, native settings persistence |
| Animation, shortcut, login setup | Yes, platform storage/service | [settings](components/settings.md#settings-storage) |
| Telegram token | OS credential store, not settings JSON | macOS Keychain / Linux Secret Service |
| Telegram username/user/chat/offset/message IDs/selected session | Yes in private `telegram.json` | `Config`, `persist`, `remember_selection` |
| Active response authority/category/nonce | Not restored as authority | native bindings, Telegram pending lifecycle |
| Completion markers | No explicit disk persistence | Monitor State created default |
| Local title query, open rows, local message drafts | Transient; close behavior differs for history/drafts | [lifecycle](interactions/input-lifecycle.md#life-close) |
| Discovery data | Reobserved; historical files are inputs, not a saved visible UI list | adapter discovery |

Acceptance: compare a new authoritative idle session, a confirmed working→idle transition, a transient unknown observation and a restarted process. Only the actual transition creates a completion; stale acknowledgments cannot clear a later one. Complete answers in another channel and verify eventual retirement without replacing the latest message with the question text.
