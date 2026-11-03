# Personal Telegram bot UX

<a id="tg-scope"></a>
## Channel and prerequisites

Telegram is the same shared Rust human-interactor implementation on M/G; frontend differences concern setup/native link opening/credential storage. H has no Telegram surface. Bot API calls go directly from resident computer to Telegram over HTTPS; no monitor relay backend. Computer awake, network available, monitor running are required. Telegram UI controls alignment, native app notifications and message appearance; inline keyboards are centered by Telegram, with no left-alignment setting in this implementation.

Source: [actor](../../../src/actors/human/telegram.rs) `Worker`, `Pending`; [browser](../../../src/actors/human/telegram/sessions.rs) `Browser`; [formatted text](../../../src/actors/human/telegram/text.rs) `Text`; [settings](../components/settings.md#settings-telegram).

<a id="tg-setup"></a>
## Setup, pairing and replacement state machine

**TG-01 (S/T):** Only the paired private account can answer, browse and send to sessions. A valid bot token alone is not a complete connection. Connect validates format (≤256bytes,numeric ID colon ≥20allowed secret characters), getMe identity and no existing webhook. Bot must be dedicated: another poller/webhook can conflict.

Initial Connect saves token to credential store, resets new config, creates random128-bit `/start` payload valid600seconds and opens bot link once through native setup. Press Start with exact unexpired payload from positive user ID equal chat ID and `chat.type=private` pairs/enables. Group chats and wrong/expired payload ignored. Existing user may be recognized from valid configuration; new token generally still requires activation. Expired setup status asks reconnect; no old nonce can authorize. Setup and connected management are in one native settings window, never a browser-hosted configuration form.

Change bot enters setting_up without disconnecting old token/user. Candidate token separately polled,600sdeadline, replacement pairing constrained to existing private owner. New network/keyring/config failure retains old connection; Cancel drops candidate and resumes old presentation. Successful install commits candidate token/config, retires old request messages, announces bot change, resets commands/poll generation, sends Connected greeting. Old poll completions ignored by generation. Credential persistence attempts rollback if new config save fails.

Unlink: retire remote active controls; delete credential/config; clear pairing/outbox/token. If credential deletion fails, disable notifications and report error rather than claiming clean unlink. Cancel differs initially M/G as listed in [settings](../components/settings.md#settings-telegram). Commands available while notifications disabled if already paired; response authorization additionally requires enabled.

<a id="tg-navigation"></a>
## Commands and session browser

Persistent reply keyboard: **Sessions**, **Help**, captions without slash. Bot command menu registers sessions/info/help; `/status` is accepted but not registered as a visible menu item. `/start` after pairing acts Help. Recognizes `/command@bot_username` case-insensitive target; wrong bot ignored. Text “Sessions”,“Status”,“Info”,“Help” is command only outside explicit reply (slash commands take precedence even inside reply). No Refresh buttons; legacy refresh callback recognized but not generated.

`/sessions`: new bounded browser, retires previous browser message, clears selected session and persists selection. Header Sessions(n), Select a session to expand it; pagination30per page, Previous/Next. Empty No open agent sessions. Filter applications, group ordering by native host platform natural collator, no group-heading buttons. Each session is one inline keyboard row: host emoji, normalized/truncated title (64 UTF-16 budget), status orange/green, right chevron. No blue completion marker in bot. Host emoji VSCode💻,Terminal⌨️,Codex desktop🤖,other🖥,Unknown◻️. Button captions bounded79/80units. Native app icons cannot be embedded as images inside Telegram inline-button text by this implementation.

Click title row selects that stable session ID, resets detail/history state. Changes in sort order do not retarget old key indices. Session view header **emoji + bold title + optional status**; no written VS Code subtitle, no type-a-message guidance, no success Message sent line. Latest Markdown below, active question below latest, keyboard below body. Previous message row, optional Latest message, then ‹ Sessions and Info. Info replaces body with folder `~`-shortened path and optional branch; ‹ Message returns. A closed/hidden selected session shows **This session is closed or hidden in Applications settings.** with navigation; plain-text sends then fail against monitor if unavailable. Pagination/back clears selection. Selection is persisted separately from last rendered browser card, so unrelated requests do not steal chat destination.

**TG-02 (S/T):** Ordinary nonblank chat text while a session is selected MUST send to that selected agent session, even after another session's request arrives/is answered. Explicit Reply to a pending request instead addresses custom answer. A stale/unmatched explicit reply does not fall through to a new agent chat message. No session → instruct choose in Sessions. Send identity is telegram:chat:messageID, validated centrally; not shell execution.

<a id="tg-messages"></a>
## Automatic updates and history

**TG-03 (S/T):** Status/request changes edit the **current message** rather than delete/recreate it. New agent text (same selected session, changed nonempty latest) first removes old message's inline keyboard, then posts a new message with current controls. Old history text remains without active controls. New status alone does not append duplicate agent text. On failure to strip old controls, do not advance; retry. Different display states clear staged request authority until delivered.

Current browser stores last rendered payload; unchanged content makes no API edit. Successful send/edit commits request/agent content authority; failed send/edit does not count as delivered. New latest resets info/earlier view, preserves outgoing send tracking. Browser session output limited1700units, request portion1500, whole text3900; truncation adds ellipsis and clips entities. There is no document attachment for full overflow.

Previous message requests Telegram-specific history generation; display one selected earlier message with Earlier message label, not all accumulated history. Repeated Previous moves cursor earlier; Latest resets. Loading… caption while awaiting,10s interrupted note, no earlier messages exhaustion. Returning to Sessions resets detail. History unavailable/error shown in body. Local popup history unaffected.

<a id="tg-requests"></a>
## Requests, questions and deduplication

Pending standalone message starts bold session title then newline/body, **no agent-name line or blank line after title**. Uses Markdown projection, literal preformatted command, optional action-detail code block near action caption, one inline answer button per row. Command bounded1600units,bash language; action detail300units,bash only Allow similar commands. Approval body2200; choice description300. Wire JSON decisions not displayed as raw button captions.

Structured questions delivered sequentially: Question i of n, current title, choices + optional descriptions; freeform instructs Reply to this message. Up to90choices/actions rendered. Preset picks value; custom trim/nonblank≤4096characters and only if freeform. Each intermediate choice advances question and rotates nonce; no partial monitor send. Complete set validated and sent once. Secret current question shows private-answer instruction, no body/choices/secret answer controls; may still retain separately rendered latest agent message in browser. No general promise of redacting unrelated sensitive content.

**TG-04 (S/T):** If current browser successfully shows a request, no duplicate standalone request should be sent. Previously sent standalone is retired when browser adopts it. Failed browser update cannot mark it delivered. A request from another session is standalone and MUST preserve selected_session. Embedded resolved request disappears by editing current session message; standalone resolved message is **deleted**, falling back to replacement text with no buttons if deletion fails.

Pending delivery capped128requests, command outbox128. New questions take priority; paced1second between successful work,15second retry for transport/cleanup/edits. Polling separately from outgoing updates (loop50ms); a long incoming poll must not block notification delivery. Shutdown/restart retires persisted active message IDs; old callback authority is not reconstructed from IDs. Cleanup/delete already-not-found and edit-not-modified treated success. Offline removal remains queued for retry, so visible old Telegram text may persist while response authority is gone.

<a id="tg-errors"></a>
## Answer validation and error presentation

Validate paired sender/chat/private type; message ID; current request nonce and allowed index; current approval equality, interaction equality, authoritative session, not already Sending/Reconciling; then submit via monitor. Central monitor additionally validates expiry. Changed same-ID interaction is **not** automatically replaced in `sync_sessions` (it only adds absent keys); stale pending can remain and answer reports **The question changed. Answer it in the agent app.** This differs from native rebinding; [D-10](../discrepancies.md#d-10).

Sending removes answer keyboard + appends Sending answer…; Reconciling removes it + Checking whether the agent received the answer…; Failed includes error and can retain choices. Callback feedback is small Telegram toast (Updated., Answer queued., unavailable/account error), not a new persistent success message. Chat sends show Sending message… while in flight, no Sent line after success; failure appended to session body.

Transport errors are short status notes: cannot reach Telegram/check connection;401token rejected;409another client/webhook;429rate limit retry;403unblock bot; generic incomplete request. No automatic permission retry from uncertain delivery. Test sends one harmless connection message; API failure note shown in settings. Private-token input and command payloads kept out of logs/argv; format examples in documentation are not real credentials.

<a id="tg-acceptance"></a>
## Acceptance

Pair only correct private account; wrong nonce/group/user ignored. Cancel a staged replacement on network/keyring failure and confirm old bot works. Open session A, receive and answer B, send plain text: A remains destination. Reply to B explicitly: answer B, no plain message to A. Exercise sequential two questions including no-custom/secret. Submit externally and verify embedded edit/standalone retirement. Change only status: current ID stays; change latest text: old buttons removed then one new message. Offline transport must not fabricate delivery, steal selection, or send an answer twice. Compare Markdown entities with emoji/Unicode, long command truncation and safe links.
