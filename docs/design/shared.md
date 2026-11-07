# Shared observable specification

<a id="shared-product"></a>
## Product and surfaces

AI Agent Monitor observes existing local coding-agent sessions. It does not start, schedule, or manage their implementation tasks. A session may be working, idle, awaiting a human response, or have unknown activity. A completion marker is additional attention state, not an execution state. The monitor offers navigation, message inspection, and only those responses supported by the agent adapter.

The user surfaces are an ambient system indicator; a session popup with persistent footer; a separate settings window; OS notifications; and an optional personal Telegram bot. A normal native answer window also remains reachable on macOS as a fallback. H simulates the indicator/popup and a small settings panel; it does not implement the OS or remote channels.

**SH-01 — No implicit authorization (S).** Opening, reading, hovering, selecting a radio, expanding command details, and navigating MUST NOT authorize a permission request. An action button or submission gesture must explicitly initiate a response. Native multi-question radio selection only edits a draft. A single-question preset is an explicit submission button.

**SH-02 — Request-specific response (S/T).** A response MUST address the displayed session, current approval snapshot, and current interaction options/context. The monitor MUST reject a mismatching snapshot, expired approval, non-authoritative session, invalid option, incomplete answer set, or duplicate in-flight send. Rejection need not produce a visible error on every frontend; see [request failures](components/requests.md#request-failures).

**SH-03 — Eventual convergence (S/T).** A confirmed reply or an authoritative observation that the agent resolved a request MUST remove that active request from downstream surfaces after their next successful reconciliation. Clicking Send is not itself evidence of resolution. Network/OS delivery may delay remote removal. A frontend MUST NOT manufacture success from queue admission.

**SH-04 — Capability preservation (S).** Action IDs and captions are separate. A frontend MUST preserve offered alternatives and submit their IDs/answer values, not infer a policy from a caption. Codex captions are adapter translations of protocol decisions, not generally literal strings supplied by Codex. An unsupported operation has no invented Allow action. Known GNOME fallback omissions are recorded separately.

**SH-05 — Safe uncertainty (S/T).** Temporary discovery failure MUST NOT be interpreted as successful empty discovery or approval. A retained request may be visible while sending is unavailable. Uncertain delivery MUST NOT be automatically repeated; it enters reconciliation.

**SH-06 — Scope of application visibility (S, D).** Application visibility MUST filter the local session list and Telegram browser. It MUST NOT stop agents or delete their discovered state. Working/finished ambient counts ignore the transient title filter. Pending-count and notification filtering differ across platforms and are not a settled shared requirement; [D-01](discrepancies.md#d-01).

**SH-07 — History isolation (S/T).** Window and Telegram history MUST have separate cursors/loading generations. Loading earlier output on one channel MUST NOT move the other channel's history position.

<a id="shared-principles"></a>
## Retained product principles (I)

These principles preserve explicitly documented design intent from the superseded macOS guide. They guide review; they do not certify missing behavior or override the platform differences recorded below.

- Make activity scannable: titles lead, with distinct working, response-needed and unseen-completion states.
- Keep authorization explicit and preserve the agent's actual question, context and available alternatives; see SH-01–SH-05.
- Use progressive disclosure: show titles/status first, messages and requests when expanded, rarely needed metadata last.
- Preserve native platform opening, dismissal, focus and presentation conventions. macOS menu mechanics are not a universal implementation requirement.
- Treat human-response channels as peers, reconciling resolved requests without assuming that a queued answer succeeded.
- Keep updates calm: automatic refresh should avoid duplicate messages/notifications, unnecessary focus changes, and decorative motion unrelated to activity. Actual reconciliation and motion differences remain documented in their component specifications.

<a id="shared-capabilities"></a>
## Agent-visible capabilities

| Adapter | Observed sessions | Activity/messages | Responses | Navigation/history constraints |
|---|---|---|---|---|
| Codex | Local writable rollout handles + database/transcript + live owner/VS Code evidence | Runtime idle/active/wait flags, latest transcript message, embedded current-turn tool images, async questions | Command/file/permission decisions, structured questions, custom input where offered; live chat messages | Workspace-aware VS Code target; macOS terminal routing; transcript history |
| Claude Code, experimental | Hook record tied to PID and process birth time | Lifecycle hooks and assistant transcript; custom title | One-time Allow through permission hook/pending request | Project pinned at SessionStart; transcript history; new chat-message sending unsupported |
| Cline, experimental | Latest tracked task per live extension host | Lifecycle + freshest task `ui_messages.json` | External approval unsupported; request directs user to Cline | Selected task only, task history; new chat-message sending unsupported |

Sources: [provider boundary](../../src/agents/sessions.rs) `Registry::sessions`, `SessionProvider`; [Codex](../../src/agents/codex/mod.rs) `open`, `read_open`, `enrich_cached`; [runtime](../../src/agents/codex/runtime.rs) `activity`, `body`, `deliver_for`; [VS Code](../../src/agents/codex/vscode.rs) `resolve_live`; [tracking](../../src/agents/tracking.rs) `live`, `snapshot`; [Claude](../../src/agents/claude/mod.rs) `hook`, `enrich`; [Cline](../../src/agents/cline/mod.rs) `parse`, `sessions`.

An open transcript alone is insufficient proof of a live terminal on macOS. That platform additionally matches live CLI/TTY/process evidence. Linux has no equivalent `retain_live_codex` call at this baseline; do not promise identical terminal closure detection. If VS Code live-log evidence is unavailable, discovery has a fallback rather than a universal guarantee of exact window membership. Neither frontend lists the full historical chat archive.

<a id="shared-contract-tests"></a>
## Cross-channel acceptance scenarios

1. With one idle and one working open session, both are eligible for listing. Closing an authoritative host removes its session on a successful discovery update; simulate adapter failure separately and verify retained state.
2. Open a request in two channels. Merely focus an answer in each: neither sends. Submit one: the other is blocked during sending/reconciliation and retires after confirmed resolution.
3. Change command/options under the same request ID. A stale UI/notification/Telegram button must not authorize the changed request. Whether the surface refreshes its controls is a separate parity check.
4. Make delivery uncertain: no success announcement, no automatic repeat; show the implemented checking/unavailable state.
5. Hide an application: list/browser entries disappear, state and agent process remain. Test the unresolved ambient-notification policy per platform rather than assuming parity.
6. Load history in Telegram, then the local popup: each cursor starts independently.
7. Restart the monitor with an old remote button still visible: it cannot obtain response authority from its previous nonce/category alone.

These contracts concern observable behavior. Actor construction, mailbox topology and transport protocols are architecture, except where latency, queue failure, or retained state affects the user.

Per-session model metadata is optional (`Session::model`), reflects session-specific observations, and is independent of activity/interaction state. See [session footer model rules](components/sessions.md#sessions-metadata) for sources, precedence and unavailable behavior.
