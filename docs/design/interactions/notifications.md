# Native notification interaction

<a id="notify-contract"></a>
## Shared purpose and lifecycle

Notifications expose pending human requests while the popup is closed. Notification title is session title; body is request body, without an extra agent-name subtitle. App identity/icon supplied by OS can still show AI Agent Monitor. Neither notification dismissal nor body click means Allow/Deny.

**NT-01 (S/T):** Offered response buttons MUST target the exact displayed request and explicit alternative. Once resolved, changed, or removed, old controls MUST NOT acquire authority over a new request. Successful retirement removes pending/delivered OS item where transport supports it. A stale click may still navigate, but must not authorize.

**NT-02 (S):** OS banner/list/sound presentation is OS-owned. The app requests a notification; it cannot guarantee that Focus/Do Not Disturb, permissions, shell policy or user notification style shows a banner. No settings toggle bypasses these OS policies. Icon-cache refresh behavior is not specified by source.

Sources: [M actor](../../../src/actors/human/notifications.rs) `run`, `present_button`, `DeliveryQueue`; [M plans](../../../src/macos/notification_actions.rs) `Plan`, `Bindings`; [M transport](../../../src/macos/notification_categories.rs) `category`, `deliver`, `post`; [M native](../../../src/macos/native.rs) `NotificationDelegate`, `remove_stale_notifications`; [G](../../../src/linux/notifications.rs) `Notifications`.

<a id="notify-macos"></a>
## macOS action planning (S/P)

1. Include offered actions in order.
2. Exactly one nonsecret question: include each preset as a direct reply; if freeform append Reply… (no choices) or Other answer… (choices), native text input Send/Your answer.
3. Multiple questions or secret question: Answer questions… navigation entry (no secret text entry in notification).
4. If no controls, Answer in editor….
5. More than4total entries: first3 + More answers…; full reply table remains in plan, entry opens local menu instead of silently authorizing omitted choices.

This4-entry budget is an app policy, not a documented promise of visible4buttons in every macOS banner. Actions require authentication; Form/More/Editor also request Foreground. Category requests CustomDismissAction; dismiss callback makes no response. Foreground delegate explicitly requests Banner|List|Sound. Sound default. Text reply must be nonblank; preset supplied with forged text is rejected by plan.

Bindings keyed by approval key retain immutable Session/Plan and fresh unique category `AGENT_DYNAMIC_<pid>_<nonce>`. Unchanged approval+interaction does not redeliver just because latest output/timestamp changed. Changed context/options under same key creates new category and retires old delivered notification/queued plan. Active disappearance removes binding and OS notification. Delivery sequential with one in-flight request, error retry5seconds only if category still current. Late callback for retired request removes item instead of reviving it. Ordinary user dismissal leaves binding known, so identical snapshots do not recreate the banner.

Body click/current Form/More → ShowRequest with `form:false`: open session menu, expand request, focus response. Editor entry → open agent target. Stale body click falls back to stored navigation target. Native delegate queue-full fallback can request normal answer window. No response from one channel is sent through another human channel.

At startup remove legacy CODEX_APPROVAL and dynamic delivered notifications from another process ID. Each posted identifier `session-<approval_key>`; removal covers pending and delivered. Shutdown retires currently known request notices. Passive notices use independent IDs, title/body/target/default sound, no active approval authority.

**Filtering caveat:** M actor iterates all approvals without application enabled, expiry or submission-phase filter. UI may be hidden/sending/expired while a request notification remains known; monitor still rejects unsafe sends. Secret planning hides reply controls but notification **body still uses approval.body**. Thus source does not establish “secret question text never leaves local UI” for macOS notification body.

<a id="notify-gnome"></a>
## GNOME/freedesktop notifications (S/P)

Uses session D-Bus Notifications Notify, bundled AppIcon-robots.png in state directory, desktop-entry hint ai-agent-monitor, default expiry−1 (server policy). Active token includes session key, approval Debug representation and full interaction Debug representation. Only enabled applications with expiry>now considered. Changed token retires previous entry via CloseNotification and sends new. Default Open monitor reveals current session; explicit actions enqueue exact Session snapshot. Response-only send does not immediately close locally; state reconciliation closes when request gone/changed.

Action list: default Open monitor; up to4interaction actions; if one question, up to4choices; add Answer questions… for multiple questions or first freeform/>4choices. This is **not** an aggregate4-button cap; no equivalent More answers for >4action-only alternatives. Notification server determines visible arrangement. No inline freedesktop text reply implementation. Single-secret choices are not filtered by `secret`, unlike M plan. A multi-question default opens GTK request form.

Seen tokens suppress duplicate posting; updates with any submission entry skip new delivery, including Failed. Notify failure is ignored, token remains seen; no M-style retry. If no notification bus connection, no delivery and no visible recovery path. Only ActionInvoked subscribed; NotificationClosed not tracked. Passive notices use separate ID ledger and default Open monitor. Drop unsubscribes and closes stored active requests; restart has no persisted active DBus ID cleanup equivalent to M. Queued callback checks whether token still exists and closes late notification if retired.

<a id="notify-tests"></a>
## Acceptance

- One action,3choices+custom,5choices,2questions,secret,unsupported request: compare actual plan, platform limits and fallback route.
- Answer in local popup or agent: OS request retires after successful observation; dismiss notification alone: request remains pending and no permission granted.
- Change command only under same request ID while OS delivery in flight/retrying: old action rejected, new M category or G token used.
- Deny notification permission/enable DND: requests remain in local window even when no banner; report OS policy, not successful interactive delivery.
- Disconnect notification service: M retry vs G one-shot failure recorded, not assumed equivalent. Secret content exposure requires explicit decision [D-09](../discrepancies.md#d-09).
