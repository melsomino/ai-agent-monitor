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
| `activities` | Bounded chronological `SessionActivity` tail: Message / Action / Request, each with its own ID and start/update times |
| `last_message()`, `images()` | Latest agent message and images projected from the tail, independent of active request |
| `connection`, `authoritative()`, `can_send_message()` | Live request authority and plain-text message capability are separate; disconnected history remains visible |
| `pending_request()`, `approval()` | Oldest pending request projected from the tail; opaque delivery descriptor is separate from activity identity |
| `interaction()` | Pending request’s literal command, actions and questions; not implied by `Activity::Confirmation` alone |
| `target` | Adapter-produced navigation target; not calculated from display text |

`Client` values are Terminal, VSCode, Desktop{name, app_path}, Unknown. Application preference IDs are `terminal`, `com.microsoft.VSCode`, `desktop:<app_path>`, `other`. Terminal grouping does not necessarily expose the actual terminal vendor as a distinct group.

<a id="state-content-timeline"></a>
## Bounded content timeline

**ST-00 (S/T):** `SessionActivity::{Message,Action,Request}` each owns a separate structure with `id`, `started_at` and `updated_at`. Sort by `(started_at,id)` ascending. Update by identity, preserve the original start and position, reject older payloads. Never merge distinct events merely because their types/text match. The soft limits are 30 ordinary activities and 2 MiB ordinary text. Evict only an oldest chronological prefix; a pending request at the eviction boundary stops cleanup and preserves the entire newer tail even above these limits. Resolving that request resumes cleanup until the next pending boundary or until both limits are satisfied. Bound retained message images to 40 MiB. Disconnect is not evidence of request resolution. Answered/superseded/resolved/expired requests remain historical records until eviction, and cannot regain authority from unchanged delayed snapshots.

`Session::last_message()`, `images()`, `progress()`, `approval()` and `interaction()` are derived projections, not parallel content storage. Native surfaces keep their established latest-output/paged-history presentation; retaining activities does not automatically show every event as a row. The offline HTML host also supplies typed activities and derives presentation fields in its renderer. [The activity model and ingress contract](../session-activities.md) specifies source identities, missing-time fallbacks, transport capabilities and acceptance criteria; architecture/mechanism details belong there rather than in visual component rules.

<a id="state-activity"></a>
## Execution activity and completion overlay

Execution values: **Working**, **Idle**, **Confirmation** (“Needs response”), **Unknown** (“Unknown status”). Codex active flags `waitingOnApproval` or `waitingOnUserInput` yield Confirmation; `systemError` yields Unknown. Pending async questions can persist after a turn becomes idle; the adapter exposes them as a request, and discovery gives request activity precedence.

Codex retains multiple addressed request activities per session (command/file/permissions/user input/option picker/MCP elicitation plus aggregated async questions). Current frontend controls expose the oldest pending request; responding to it does not resolve its siblings. A waiting runtime flag without a recognized addressed request retains Confirmation activity but creates no approval envelope, card, ticket or request notification. Real unsupported addressed requests may still offer navigation to the editor. Direct tickets expire at `u64::MAX` and live ownership/current-envelope validation supplies lifetime. An unchanged envelope retains its opaque ticket through a temporary observer disconnect; a missing transport view does not prove resolution. Sending stays disabled until fresh authoritative state returns. Session removal, a different owner, or a resolved/edited envelope invalidates the ticket. A new envelope normally produces a new opaque ticket; same-ID mutation checks remain necessary for other sources. Async questions clear when their explicit accepted answer IDs appear or later ordinary user input supersedes them; rejected/pending steering input does not resolve them. Source: `Runtime::views`, `async_questions::pending`.

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

Source: [monitor/mod.rs](../../src/actors/monitor/mod.rs) `run`, `valid_response`; [Codex runtime](../../src/agents/codex/runtime.rs) `deliver_for`, `respond_inner`, `request_resolved`. Confirmed delivery settles the exact request immediately. If its previous status was Confirmation and no other pending request remains, the monitor presents Working while awaiting the adapter’s next execution state. A delayed snapshot of that same terminal request cannot restore its waiting indicator. The confirmed-resolution guard also spans separate request-removal and runtime-flag patches: an unchanged waiting flag does not restore orange while the agent is resuming. The guard ends when a fresh non-waiting execution state or a different pending request arrives. That different request retains Confirmation; independent unaddressed waiting indications outside this resumption guard remain Confirmation. Uncertain/failed delivery does not clear waiting status, and a fresh Idle/Unknown observation takes precedence.

Hook requests can overlay/create sessions until expiry or hook loss. Legacy hook approval waits are time-limited (60 seconds in core); direct Codex requests use their own current-request evidence. These are not a shared frontend timeout. The direct Codex adapter allows up to10 seconds for a fresh owner snapshot before replying and up to10 seconds to observe resolution after acknowledgement (`SNAPSHOT_TIMEOUT`); a slow image-heavy history must not be treated as a changed request merely because it exceeds the former2-second budget. The same fresh-snapshot budget applies to chat-message admission. The displayed request/owner must still match exactly, and an acknowledgement alone does not prove resolution. Closing a popup or dismissing a notification never returns Allow.

**ST-04 — Incremental Codex synchronization:** A healthy owner subscription updates from revisioned patches; periodic owner-liveness checks alone must not reset request authority, change request identity, resend notifications or reload full history. A revision gap, transport reset or owner change invalidates the cached live observation and requests replacement state. A fresh authoritative snapshot restores answer availability; a stale owner cannot restore a resolved request. An explicit response still requires fresh exact-request validation and evidence of resolution. Source: `codex/runtime.rs` `watch`, `needs_snapshot`, `apply_change`, `respond_inner`; selective frame decoding: `codex/projection.rs`. This transport optimization changes no frontend permission choices or history-navigation contract.

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

### ST-05 — Terminal Codex App Server synchronization

For independently discovered live terminal sessions, the shared App Server adapter supplies authoritative status and at most the first currently pending supported request to the existing monitor snapshot. Joining a loaded thread replays pending requests without history hydration. Command/file/permission approvals and structured answers use the same UI contracts as owner IPC. An external response removes the request; a locally submitted answer becomes Confirmed only after a matching session/request resolution event. Same-ID edited requests, removed sessions and reconnects invalidate old local answer tokens. A disconnected adapter contributes no authoritative observation. Ambiguous delivery is Uncertain and must not be automatically retried. This addition changes no UI layout, default choice, secret-input or notification-binding rule.

Source: [App Server adapter](../../src/agents/codex/app_server.rs) `Runtime::observe`, `Runtime::deliver`, `Connection::event`, `Connection::submit`, `watch`; [transport details and limitations](../integrations.md#terminal-codex-and-the-shared-app-server).

### ST-06 — Prompt publication without discovery delay

Codex IPC and App Server observers wake the adapter actor when live state changes. Existing session metadata is retained while authoritative activity, request and interaction data are refreshed from those local observations. A request arrival, external resolution or post-answer status update MUST NOT intentionally wait for the two-second filesystem/process discovery cadence. A confirmed delivery triggers an immediate local-live refresh as well. No optimistic approval or fabricated Idle transition is introduced; transport authority and matching resolution evidence still govern request removal. Unchanged local-live state produces no duplicate Observe snapshot. A discovery refresh already in progress can still delay processing; native macOS consumes changed monitor snapshots on its existing100ms timer. Sources: `SessionProvider::live_updates`, `refresh_live`; `actors/agents.rs` `run`, `publish_live`; Codex observer publishers. Regression: `live_requests_and_confirmed_status_publish_without_another_discovery_scan`.

<a id="state-readonly-questions"></a>
## Read-only questions during an owner outage

A dedicated async-question tool call found in the bounded Codex rollout tail can produce a navigation-only request while `connection` remains disconnected. Display its question text and allow opening the host application; do not enable responses, choices or Allow actions. Canonical reply envelopes resolve matching question IDs, and a subsequent real user prompt supersedes preceding questions. Injected environment/AGENTS context does not cancel them. These positively observed transcript updates can settle their own fallback records without implying that the live agent connection recovered. An arbitrary command/tool invocation is not evidence of a permission request. This source fallback does not recover questions outside the tail or grant a send capability. Source: `codex/rollout_questions.rs::Reader`, `Session::merge_previous`, native navigation-control enablement.

Owner discovery is multiplexed with live stream consumption; one unresolved owner lookup must not delay another session’s snapshot or request patch. Timeout does not prove closure of an otherwise live owner stream. Source: `codex/runtime.rs::watch`, `Connection::begin_request`; regression: `a_silent_owner_does_not_delay_other_subscriptions_or_request_patches`.

<a id="state-versions"></a>
## Version and update state

`State::versions` is monitor-owned and independent of session/agent state. Its `current`, optional `available` release and last successful `checked_at` timestamp are projected to native consumers, IPC Sessions JSON and the web snapshot. Only the monitor schedules checks and persists the latest stable release metadata. Failed checks preserve the last successful values; successful checks may clear availability when a running build is already current. See the [release interaction contract](components/foundations.md#foundation-release-announcement).
