# Coverage, traceability and verification report

<a id="coverage-method"></a>
## Method and evidence boundary

Reverse-engineered the source at `aa1601582edcf768767f23b8d3dd960dfc4a9461`, 2026-11-02. Inspected domain structures, monitor transitions, adapter discovery/response projections, frontend builders/event handlers, stylesheet tokens, native notification planning/reconciliation, Telegram worker/browser/formatting, settings and existing assertion bodies. Read prototype CSS, fixture rendering and complete product interaction handlers, plus standalone wrapper embedding/bootstrap. Existing documentation was compared as intent, not treated as proof.

**No application build, restart, preference change, agent response, Telegram send, screenshot capture or live accessibility test was performed for this task.** Existing tests are cited as inspected assertions, not fresh passing results. Documentation validation checks links/anchors, matrix identifiers and file scope only. This prevents a documentation-only change from incrementing builds, modifying resources, exposing a real notification/token, or implying empirical platform certification.

Documentation checks completed:17 specification documents,264 relative links/anchor references resolved,94 unique parity rows, no duplicate explicit section anchors, `git diff --check` clean. The standalone prototype contains exactly one escaped copy of the canonical fragment. The superseded macOS-focused style guide has been removed after comparing its scope with this specification. Its measurements, interaction rules and acceptance cases are covered here; retained design principles and writing intent are explicitly labeled, and conflicting visibility policy remains D-01. Navigation in README.md and docs/ubuntu.md points here. The pre-existing build-number.txt modification is left untouched.

<a id="coverage-sources"></a>
## Source-to-specification inventory

| Source / significant symbols | UI/UX covered | Destination |
|---|---|---|
| [Session](../../src/domain/session.rs) `Session`, `Client`, `Activity`, `project_path`, `working_directory`, keys |Identity,activity,host,path,request |[State](state-models.md#state-session) |
| [Interaction](../../src/domain/interaction.rs) `valid_answers` |Question/action/custom/secret capability and validation |[Requests](components/requests.md) |
| [Preferences](../../src/domain/preferences.rs) `observe`, `enabled`, `list`, defaults |Known apps,stable IDs,visibility,persistence defaults |[Settings](components/settings.md) |
| [Layout](../../src/domain/layout.rs), [usage](../../src/domain/usage.rs), [markdown](../../src/domain/markdown.rs) |Geometry,sticky icons,remaining balance,safe formatting |[Foundations](components/foundations.md),[Sessions](components/sessions.md) |
| [Monitor state](../../src/actors/monitor/state.rs), [commands](../../src/actors/monitor/command.rs), [handle](../../src/actors/monitor/handle.rs), [run](../../src/actors/monitor/mod.rs) |Generation/transition/authority/stale response,history,chat,failures |[Shared](shared.md),[State](state-models.md) |
| [Agent actor](../../src/actors/agents.rs), [provider](../../src/agents/sessions.rs) |2second observations,capability enrichment,delivery evidence |[Shared](shared.md#shared-capabilities) |
| [Codex discovery](../../src/agents/codex/mod.rs) `open`, `read_open`, `read`, activity/transcript helpers |Live file/database membership,title,activity/message fallback |Shared/State |
| [Codex requests](../../src/agents/codex/requests.rs) `interaction`, `decision_label`, `decision_detail`, wire mapping |Real action alternatives/commands,freeform and secret flags |Requests |
| [Codex runtime](../../src/agents/codex/runtime.rs) `views`, `activity`, `body`, `exact_request`, `deliver_for`, `respond_inner`, `send_message` |Live authority,waiting reasons,uncertain send |State/Requests |
| [Async questions](../../src/agents/codex/async_questions.rs) `pending`, `reply_text`, `reply_envelope` |Idle questions,accepted response removal,new-user-input retirement |State/Requests |
| [VS Code membership](../../src/agents/codex/vscode.rs) `live_sessions_cached`, `resolve_live`; [terminal](../../src/macos/terminal.rs) `retain_live_codex`, `focus` |Workspace association,window closure,TTY/tab navigation |Shared/Platform supplements |
| [Claude](../../src/agents/claude/mod.rs), [Cline](../../src/agents/cline/mod.rs), [tracking](../../src/agents/tracking.rs), [history](../../src/agents/history.rs), [pending](../../src/agents/pending.rs) |Experimental capabilities,PID lifetime,transcript/task history,hook request limits |Shared/State |
| [macOS menu](../../src/macos/menu.rs) `build_*`, `render_rows`, `request_context`, `session_metadata`, `receive_sessions`, `filter` |Every popup/header/detail/footer renderer and empty state |Sessions/Requests/Foundations |
| Same file `handle_session_key`, `keyboard_targets`, `focus_answer`, `update_hover_at`, `apply_hover`, `route_scroll`, `begin_disclosure`, geometry extensions |Pointer/focus/key/lifecycle/animation |[Input lifecycle](interactions/input-lifecycle.md) |
| Same file `show_settings`, `refresh_*_settings`, recorder/Telegram handlers; [settings](../../src/macos/settings.rs), [setup helpers](../../src/macos/telegram_setup.rs), [onboarding](../../src/macos/onboarding.rs) |All4panes/defaults/conditional setup/login/shortcuts |Settings |
| [Inline](../../src/macos/inline_questions.rs) `Panel`, `RadioDecoration`, `FieldFocusRing`, `CaretBlink`; [answer window](../../src/macos/answers.rs) `AnswerForm`, `show_with_draft` |Question input states,validation,drafts,caret,normal fallback |Requests/Input |
| [M native](../../src/macos/native.rs), [hotkey](../../src/macos/hotkey.rs), [pointer](../../src/macos/pointer.rs), [motion](../../src/macos/motion.rs) |Native events,shortcuts,single hit target,eyes/disclosure timing |M supplement/Indicator/Input |
| [M robot](../../src/macos/session_robot.rs), [thread](../../src/macos/session_thread.rs), [markdown](../../src/macos/markdown.rs) |Artwork geometry,thread contours,formatted native text |Foundations/Sessions |
| [M notification actor](../../src/actors/human/notifications.rs), [bindings](../../src/macos/notification_actions.rs), [categories](../../src/macos/notification_categories.rs) |Dynamic actions,retry,stale categories,retirement |[Notifications](interactions/notifications.md) |
| [G model](../../src/linux/model.rs) `eligible`, `Disclosure`, `markup`, `empty_text` |Ordering/filter/disclosure/formatting/empty state |Sessions/Input |
| [G UI](../../src/linux/ui.rs) `Desktop`, `Row` |Popup lifecycle/sizing/focus/history/card revalidation/sticky icons/footer |G supplement/Sessions/Input |
| [G requests](../../src/linux/requests.rs) `Card`, `command_projection`, `command_block` |Preset/radio/custom/secret/context/2line commands |Requests |
| [G style](../../src/linux/style.rs), [CSS](../../src/linux/style.css), [widgets](../../src/linux/widgets.rs) |Colors,targets,icons,typography,Markdown,theme |Foundations/G supplement |
| [G settings](../../src/linux/settings.rs), [credentials](../../src/linux/credentials.rs) |All settings,autostart,keyring,setup/cancel |Settings |
| [G notifications](../../src/linux/notifications.rs) `update`, `token`, `close` |Actions,expiry/filtering,one-shot failure,stale token |Notifications |
| [G panel](../../src/linux/panel.rs), [Shell](../../src/linux/gnome-extension/extension.js), [geometry](../../src/linux/gnome-extension/geometry.js), [activation](../../src/linux/gnome-extension/activation.js) |Ambient state,anchor,Wayland focus,shortcut/bus lifecycle |Indicator/G supplement |
| [Telegram worker](../../src/actors/human/telegram.rs) `Worker`, `Pending`, event/poll/flush/answer/setup functions |Pairing,replacement,request routing,dedup,cleanup/errors |[Telegram](interactions/telegram.md) |
| [Browser](../../src/actors/human/telegram/sessions.rs) `Browser::*`; [Text](../../src/actors/human/telegram/text.rs) |Sessions/pages/history/info/selection/new-message vs edit,entities |Telegram |
| [HTML fragment](../../design/previews/monitor-ui-fragment.html) all `.aam-*` CSS + product listeners |Complete implemented prototype state/visual/input behavior |[HTML supplement](platforms/html.md),matrix |
| [HTML standalone](../../design/previews/monitor-ui.html) iframe/data-srcdoc/bootstrap |Embedding,local fallback/library dependencies |HTML supplement |
| [README](../../README.md), [Ubuntu](../ubuntu.md), [integrations](../integrations.md), [architecture](../actor-architecture.md) |Intent/background compared against actual source |Discrepancy register |

The table references behavioral source paths/symbols, not a line-by-line audit certification of every transport helper. Build/packaging/notarization implementation is outside UI/UX except install/launch messages and app identity. Agent protocol internals are included only where they change observable capability, delivery/availability or session membership.

<a id="coverage-assertions"></a>
## Existing assertion evidence inspected (T)

| Assertions | What they support | What they do not prove |
|---|---|---|
| [Monitor tests](../../src/actors/monitor/tests.rs): `finished_sessions_stay_unseen_until_acknowledged`, stale ack, completion connection loss |Completion generations/actions,uncertain source retention |Real visual color/contrast |
| Same: simultaneous answers, stale/expired/invalid, uncertainty, changed context, refreshed notification category |Response safety,single send,late rejection |Actual external-agent protocol compatibility today |
| Same: window_and_telegram_history…,late generations,stopped adapter |History isolation/loading/errors |Real transcript completeness |
| Same: sending_chat_text…,stale_empty_disconnected…,adapter failure |Chat validation/dedup/failure state |Real Telegram network behavior |
| [M menu tests](../../src/macos/menu.rs) button layout,horizontal answer wrap,keyboard order,navigation/filter/counts,active disclosure,close animation,footer geometry |Pure layout/order/selection logic |AppKit event delivery,menu-bar selection,caret |
| [M pointer tests](../../src/macos/pointer.rs) target_at/gutter |Clipping/single pointer target/bounds |Real pointer sampling on all displays |
| [M motion](../../src/macos/motion.rs) disclosure and eyes; [inline questions](../../src/macos/inline_questions.rs) answer/caret tests |Timing arithmetic,valid answers |Perceived animation/native blinking |
| [Notification actor](../../src/actors/human/notifications.rs), [bindings](../../src/macos/notification_actions.rs) |Dynamic actions,changed context,retry invalidation,menu route |Actual OS banner/button display/permissions |
| [Telegram tests](../../src/actors/human/telegram.rs), [browser tests](../../src/actors/human/telegram/sessions.rs), [text tests](../../src/actors/human/telegram/text.rs) |Account/nonce/message authority,selection preservation,replacement,edit/dedup,deletion fallback,entities |Live BotFather/Telegram Desktop/keyring connectivity |
| [G model](../../src/linux/model.rs) behavior_tests; [requests](../../src/linux/requests.rs) command tests |Unicode/natural sort,disclosure,measured2line intent |Every theme and font allocation |
| [G UI ignored widget harness](../../src/linux/ui.rs) `native_widgets_follow_style_guide_and_preserve_request_snapshots` |Real GTK widget assertions for geometry,drafts,focus,history,settings when run |Not run here; not GNOME compositor/Wayland certification |
| [Settings pane harness](../../tests/settings_pane.rs) |Retained AppKit pane replacement lifecycle |Full settings display/cross-tab interaction |
| [GNOME geometry](../../tests/ubuntu_panel_geometry.js), [activation](../../tests/ubuntu_editor_activation.js) |Pure placement and editor-window matching cases |Shell extension lifecycle/compositor activation outcome |
| Prototype |Handlers inspected directly |No dedicated parity/interaction automated test suite found |

<a id="coverage-limits"></a>
## Coverage limits: source unknown versus unimplemented

**Verified structurally:** surfaces,controls,render branches,explicit handlers,tokens,snapshot transitions,validation,persistence keys and conditional copy listed in components. **Present but only statically verified:** AppKit/GTK native keyboard routing,caret/hover,window pinning/animation,OS callbacks,Secret Service/Keychain and Telegram HTTP effects. **Unknown runtime:** OS/menu focus transfer,real notification layout/icon caches/DND,VoiceOver/Orca,IME/full keyboard access,font/contrast scaling,actual Wayland panel anchoring under resize and version-specific agent/terminal behavior.

**Absent in source inspected:** H real backend/discovery/single-question branch/notification/Telegram/full settings/typed session filter; G exact terminal tab selection/AppKit-like menu traversal/native notification text entry; universal accessibility semantics; complete reduced-motion policy; generic multi-select questions; native show-metadata preference; Telegram completion acknowledgment; unified changed same-ID Telegram request refresh. These are not merely “unverified,” and no contract silently supplies them.

Potentially misleading existing prose: architecture model includes fields/identities richer than actual domain and retains historical Codex-hook discussion; integrations says notification body opens editor while current active binding default opens menu; Ubuntu says shared392width but actual GTK outer frame408 for margins; README calls broad automatic setup although G initial check is read-only. New specification records actual paths and [decisions](discrepancies.md), not literal inheritance.

<a id="coverage-review"></a>
## New frontend implementation/review checklist

1. Implement shared state/response safety and persistence capabilities first; do not invent user-visible command success from queueing.
2. Map every parity row to implemented/native exception/missing/decision-needed. Keep the selected platform's intentional OS conventions; do not copy AppKit internals.
3. Follow prototype geometry/color hierarchy as reference while preserving recorded production conflicts as decisions, not assumptions.
4. Exercise component acceptance cases including stale context,concurrent answers,expiry,non-authoritative state,external resolution,connection loss and disabled controls.
5. Run manual native acceptance for pointer/focus/IME/accessibility,real materials/screens/OS notifications and Telegram/keyring. Record OS/theme/device/version/results separately.
6. Update baseline,source symbols,contracts,parity and discrepancy status together. Keep architecture implementation mechanics out of product specification except to explain visible availability and lifecycle.
