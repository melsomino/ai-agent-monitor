# Session activity model

The implemented session content model lives in [`session_activity.rs`](../src/domain/session_activity.rs). [`Session`](../src/domain/session.rs) contains `VecDeque<SessionActivity>`, session identity/metadata, the current execution `Activity` (Working / Idle / Confirmation / Unknown), and a separate `ConnectionState`. Execution status is not an entry in the content timeline; unseen completion remains a monitor-owned overlay.

## Records and identity

`SessionActivity` has three variants with their own structures and no generic block wrapper:

- `Message(SessionMessage)`: `id`, `started_at`, `updated_at`, `role` (User / Agent), text and image payloads.
- `Action(SessionAction)`: the same identity/times, action kind (Command / Tool / Reasoning / Other), public caption, execution status (Running / Completed / Failed), and `current` eligibility for the live caption. Completion alone does not erase a caption; a new item, message or turn supersedes it. Raw private reasoning and tool output are not activity captions.
- `Request(SessionRequest)`: the same identity/times, opaque provider `Approval` delivery descriptor, neutral `Interaction`, and Pending / Answered / Superseded / Resolved / Expired state. The activity ID identifies the timeline record; the approval ID routes a response and must still be revalidated by its provider.

IDs are unique within one session, including across variants. Two equal texts, consecutive commands of the same kind, and events with equal timestamps remain distinct if their IDs differ. Reusing an ID updates that event; an ID cannot change variant. The original `started_at` survives updates, while older `updated_at` payloads are rejected. Replaying an unchanged pending request cannot reopen an answered/resolved request. A genuinely edited delivery descriptor/context can replace it when its update is newer.

The total order is **`(started_at, id)`**, ascending, using UTC instants and Rust string order for ties. `updated_at` never determines list position. There is no sequence counter. Source timestamps are used when available. Runtime events without source start times use first observation time; a backfilled item may use its turn start as the available lower-resolution source time. No exact within-turn timing is inferred. In the prototype fixture boundary times are ISO UTC strings.

## Bounds and synchronization

The normal recent tail has soft limits of **30 ordinary activities** and **2 MiB of ordinary textual content per session**. Eviction removes only a chronological prefix. Pending requests are non-evictable barriers: once the oldest remaining entry is pending, eviction stops and retains every newer activity, even if either soft limit is exceeded. Pending requests do not consume the ordinary tail budget; provider request-count and transport limits remain in force (Codex observers retain at most 128 addressed pending requests per transport view). Once a boundary request becomes terminal, eviction resumes from the oldest entry until both limits are satisfied or the next pending boundary is reached. Old message images are cleared when the retained image-byte budget would exceed **40 MiB**; their text/identity remain. Cloned image data uses shared `Arc` storage. This is not a complete conversation archive.

`Session::merge_previous` merges provider snapshots with the monitor's previous tail by ID. Missing pending requests become terminal only with authoritative live evidence; explicit replies/superseding prompts can also settle read-only transcript question records; disconnects preserve their presentation while disabling replies. A confirmed response settles that exact request, not every request in the session. Settled records remain in the tail until eviction. Session closure removes its in-memory tail. There is no new persistent activity database.

A connected observer and a writable message route are separate capabilities. Direct owner IPC permits message sending. The shared Codex App Server also permits explicit text messages to already loaded idle terminal sessions, with fresh loaded/status validation before submission. Claude/Cline hooks do not implement the composer route. A visible disk transcript does not establish either live capability. The composer and monitor command admission check `can_send_message`, while approval admission checks live request authority.

## Adapter ingress

- **Codex rollout:** bootstrap from the last 4 MiB of complete JSONL records, then consume only appended bytes using in-memory cursors. Preserve parser state across polls; incomplete records wait for a newline, with at most 64 KiB buffered in RAM before spilling to a private unlinked temporary file. Reset on truncation, file replacement or failed prefix guards; monitor restart bootstraps again. Dedicated async-question tool calls also supply read-only question visibility when no owner is reachable; these `transcript:` records offer editor navigation, never answer authority. Message source ID when provided, otherwise absolute byte offset; roles remain distinct. Canonical `response_item` assistant messages supersede legacy mirrored `event_msg.agent_message` representation. Same-kind repeated messages are preserved. Function/custom tool calls and public reasoning summaries become actions; matching call outputs complete the action without retaining their bodies. Existing image discovery attaches bounded published images to the latest agent message.
- **Codex owner stream:** projected items retain their source turn/item identities, public captions and lifecycle status. `recent_events` preserves distinct available items, even if several complete before one observer notification. Current caption selection remains separate from execution completion. Addressed requests use source method/turn/request identity and the existing opaque validated delivery ticket. Multiple pending envelopes are retained; waiting flags alone do not manufacture requests.
- **Shared App Server:** item start/completion and public summary deltas update the corresponding item activity; requests retain exact JSON request IDs and per-connection response tickets. Passive observation still uses `excludeTurns: true` and does not hydrate full history. Only events observed by that connection are available through this path; rollout history remains the fallback. Thread/turn/message lifecycle supersedes the current caption.
- **Claude:** bounded transcript tail, public user/assistant text only, source UUID or absolute byte offset. Source RFC3339 times take precedence. Existing hook records can supply a fallback last-message observation if no transcript records are available; their hook update identity does not prove an independent conversational message when the source only republishes its snapshot.
- **Cline:** bounded selected-task UI document, stable timestamp plus array-position identity for public text/completion entries. Partial updates replace the same position; equal text at another position remains separate. Cline requests remain navigation-only because its public API has no addressed approval route.

No snapshot-based observer can guarantee capture of an event that the source never exposes or already discarded. Missing source IDs/timestamps and mirror conventions are adapter limitations, not permission to deduplicate arbitrary same-type content.

## Presentation and diagnostics

Native AppKit, GTK, Telegram and notifications obtain the latest agent message, current caption and oldest pending request through projections of the timeline. Flat `last_message`, `images`, `progress`, `approval`, `interaction_data`, and `authoritative` storage fields have been removed from `Session`. Getter names retained for consumers are computed projections. Existing previous-message navigation still pages source history on demand; this refactor does not expand every retained event into a permanently visible UI row or replace archival history cursors.

The HTML preview host supplies typed `activities`; its renderer derives presentation fields before keyed reconciliation. Mock submissions settle the matching request activity and append a new message. It remains an offline prototype with no native bridge.

`sessions --json` includes activities with IDs, UTC times, typed content/status and connection capabilities. Image diagnostics contain ID/MIME/size only, never binary data. Legacy latest-message/progress/approval JSON properties remain derived compatibility projections.

## Acceptance criteria

1. Inserting identical texts with different IDs retains both; same timestamp ties order by ID.
2. Updating an existing event preserves its start and position, and changes only its payload/update time.
3. A stale update cannot overwrite newer content or reopen a terminal request.
4. Exceeding either ordinary-history limit evicts only the oldest prefix, stopping at the first pending request and preserving its entire newer tail. Resolving it resumes eviction up to the next pending boundary or until both limits are satisfied.
5. Disconnecting preserves an active question and blocks responses. Reconnection requires fresh provider validation.
6. Answering the first of two pending requests leaves the second pending.
7. Consecutive identical commands with different source item IDs remain separate activities.
8. The UI's current-caption projection, latest message, composer and existing paged history continue to work without parallel mutable content fields.

Tests: `domain::session_activity::tests`, `actors::monitor::tests`, adapter tests, and the native visual frontend harness. GNOME runtime behavior requires Linux/desktop verification; macOS compilation does not certify cfg-gated GTK code.
