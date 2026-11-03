# Discrepancies, intended-only behavior, and open decisions

<a id="decisions-policy"></a>
## How to use this register

No application changes are proposed or made here. Each entry preserves actual differing behavior and identifies the decision/verification needed. “Conflict” does not necessarily mean defect: a deliberate platform exception can resolve it. Source-verified assertions are distinct from runtime failure reports. These entries are not an implementation backlog automatically authorized by this documentation task.

<a id="d-01"></a>
## D-01 — Hidden applications and attention scope

M `State::working_count`/`finished_count` filter enabled apps; `update_badge(snapshot.pending_count())` does not filter pending. M notification actor iterates all approvals. G `Panel::publish` filters all counts; notification `update` filters enabled apps+unexpired approval. Telegram browser filters apps, standalone `Worker::sync_sessions` does not. Earlier macOS design guidance required hidden apps to be excluded from all native counts, while [integration documentation](../integrations.md) explicitly exempts pending badge/notifications. That earlier intent is retained here as an unresolved conflict, not a shared requirement. H no app settings.

**Decision:** Does application visibility govern list only, all attention delivery, or specific channels? Until decided, preserve documented platform behavior. Sources: [M menu](../../src/macos/menu.rs), [State::pending_count](../../src/actors/monitor/state.rs), [M notifications](../../src/actors/human/notifications.rs), [G panel](../../src/linux/panel.rs), [G notices](../../src/linux/notifications.rs), [Telegram](../../src/actors/human/telegram.rs).

<a id="d-02"></a>
## D-02 — Indicator eye precedence and counter shape

M only animates eyes when no pending requests; G whenever working>0. Different tick140/150ms, G retains phase across pauses; H static. M/H14-high capsule counters, G≥18circles with uncapped totals, M99+cap. **Decision:** Which differences are deliberate legibility/native choices and which are parity requirements? [Indicator evidence](components/indicator.md).

<a id="d-03"></a>
## D-03 — Hover, focus, symbols and geometry

H/M14%contrast hover, G6%; H robot26×24/footer25 vs native28square. H stage fixed blue shadows root AccentColor support. M/G native selection colors appropriate to OS but H cannot claim system-accent correctness. Request/finished ink dark M/H versus white G; disc15/16. M unknown questionmark vs H non-idle fallback request. **Decision:** Establish exact visual exceptions without equating toolkit tokens to same pixels. [Foundation ledger](components/foundations.md).

<a id="d-04"></a>
## D-04 — Autoexpand and changed requests

M tracks new approval keys and first working occurrence during open. G request token includes full interaction, but a known Idle→Working row's existing false disclosure entry is not reopened. M same approval ID changed context does not trigger new-request expansion key; G does. H initially only delivery expanded. **Decision:** Define triggers for repeated work and changed same-ID request, and when manual collapse wins. Sources `State::expand_active_sessions`, `Disclosure::observe`, H sessionMarkup.

<a id="d-05"></a>
## D-05 — Closing, drafts, scroll and history lifetime

M `reset_disclosures` clears local history/contexts but active `question_panels` cache retained; drafts can survive close. G `hide` removes cards/drafts but monitor Window history retained; no explicit scroll reset. H saves expansion under widget state. docs/ubuntu says close clears drafts, correctly G but not generalizable to M. **Decision:** What should close/reopen retain for native channels? Sources `MenuController::close_panel`, `State::reset_disclosures`, `Desktop::hide`, `Row::update`, H applyExpanded.

<a id="d-06"></a>
## D-06 — History anchor and gutter hover

M preserves history-start latest anchor; G does not. H one fixture and y-only hover, G/M actual region hit-testing. Distinct loading/exhaustion/error/tooltips. **Decision:** Fix common visible history contract versus prototype simplification. [History](components/sessions.md#sessions-history).

<a id="d-07"></a>
## D-07 — Approval layout and command details

H allows partial-row wrap and all contexts before actions; native chooses entire row or full column and per-action details adjacent. H obsolete Future commands starting with text and no two-line command expander. Command fill uses hover rather than opaque darker token. **Decision:** Update design intent or declare prototype lag; no automatic choice here. Sources H `.aam-actions`, `sessionMarkup`; M `answer_button_layout`,`request_context`; G `Card::new`,`command_projection`.

<a id="d-08"></a>
## D-08 — Form validation and keyboard

Up/Down clamps M/H, wraps G. M disabled incomplete Send remains traversal target; G sensitive submit silently rejects; H validity bubble. Title Space filters M but clicks G/H. Custom focus selects M/G multi but not H; G single custom Enter lacks equivalent hook. M descriptions inline vs G tooltip, mixed requests use different form route. **Decision:** Specify which are native convention and which must converge. [Input table](interactions/input-lifecycle.md#input-keyboard), [forms](components/requests.md).

<a id="d-09"></a>
## D-09 — Secret questions across notifications

M Plan suppresses secret presets/text reply, but posts approval.body unredacted. G includes single-question secret choices/body without guard. Telegram suppresses current secret body/options, local inputs masked. Existing broad prose about private secrets must not be interpreted as source-verified universal redaction. **Decision:** Define channel content redaction separately from secure input handling. Sources `Plan::new`, native actor Notice construction, G Notifications::update, Pending::body_content. No secret fixtures executed in this review.

<a id="d-10"></a>
## D-10 — Telegram same-ID request changes

Native notifications/inline cards refresh on full interaction equality changes. Telegram `sync_sessions` only adds new approval keys; `answer` rejects differing current interaction, leaving stale remote request until retired/replaced. **Decision:** Desired refresh/reset policy for changed body/command/options under same ID. Current rejection is safe but UX not synchronized. Source `Worker::sync_sessions`, `Worker::answer`.

<a id="d-11"></a>
## D-11 — Unsupported/expired requests and transport feedback

M unsupported requests offer Answer in editor; G empty-capability card lacks dedicated route (title still opens). G expired card explicit error/disabled; M central reject with no main-card expiry gate. M ordinary actions don't show question-panel submission status/error. G notification no retry/failure UI, >4action-only controls truncated without More. **Decision:** Shared unavailable/error visibility and alternatives; no behavior invented. [Requests](components/requests.md#request-failures), [notifications](interactions/notifications.md).

<a id="d-12"></a>
## D-12 — Prototype settings and power

H power Pause/Resume local only, native Quit terminates. H Show folder and branch setting absent native; login/animation checkboxes do not configure services. H has no full settings tabs, remote channels, backend state or lifecycle close. **Decision:** Label simulation versus intended product behavior, and whether prototype should eventually cover these surfaces. Source H build/click/change handlers, native settings/quit. Do not use “99%” as a verified coverage metric.

<a id="d-13"></a>
## D-13 — Reduced motion and accessibility intent

H disables disclosure transitions on prefers-reduced-motion. M guards ambient eyes only; G guards Revealer only, eyes depend animation preference and list spinner always advances while visible. Custom app/robot/drawing semantics incomplete for screen readers. **Decision:** Scope of reduced-motion and semantic focus/expanded-state parity. Real VoiceOver/Orca/highcontrast verification required; no conformance claim. [Foundations](components/foundations.md#visual-motion).

<a id="d-14"></a>
## D-14 — Integration setup and error observability

M automatic install/non-Codex hooks on launch; G check-only until manual button. G no empty-agent explanation; row phase symbol/color simplified. General GTK widgets created once may not reflect later preference snapshots. M popup ignores discovery_error; G shows it. **Decision:** Expected automatic setup per platform and authoritative error surface. Existing README generalized automatic setup is M-specific. [Settings](components/settings.md#settings-integrations).

<a id="d-15"></a>
## D-15 — Initial Telegram Cancel and setup controls

M initial unpaired Cancel sends Disconnect, removes token; paired replacement CancelSetup retains old. G alwaysCancelSetup, potentially retains initial token/pairing. G fields editable, M suggested fields readonly; G BotFather link does not prefill /newbot; no setup manual Open bot. **Decision:** Initial Cancel contract, editability, recovery controls. [Settings](components/settings.md#settings-telegram).

<a id="d-16"></a>
## D-16 — Host navigation and session lifetime

M Terminal/iTerm exact tab and Warp app focus; G terminal adapter target navigates project/editor with no terminal-tab integration. M removes daemon-only terminal sessions; G lacks this guard. M/G native icon resolution differs for desktop/closed known apps. **Decision:** Mark unsupported platform capability explicitly rather than promising same-app focus. [Platform supplements](platforms/gnome.md#gnome-navigation).

<a id="d-17"></a>
## D-17 — Fallback form context freshness

M ordinary inline panel matches approval+interaction before reuse. Separate AnswerForm cache/reconcile matches approval identity/equality without full interaction equality; stale form safe central rejection but may stay visible. **Decision:** Is fallback form intended to remain a supported surface with its own draft refresh semantics? Source `AnswerForm::reconcile`, `show_with_draft`.

<a id="unknowns"></a>
## Unresolved runtime questions (U)

1. Does current AppKit status selection stay visible throughout every native tracking path and support neighboring third-party menu extras? OS behavior cannot be proven from setMenu alone.
2. Do header/footer extensions repaint without transient strips under all display/theme/resize conditions? Arithmetic/source is not screenshot proof.
3. Does GNOME re-anchor correctly after content changes while open, especially fractional scaling/multiple monitors? Extension map/toggle handlers alone do not certify it.
4. What are effective native Tab traversal and modifier/IME interactions in menu tracking and GTK radio groups? Custom arrow handlers don't establish all toolkit-default behavior.
5. Are all text fields/carets visible in non-key tracking windows and with alternate input sources? Does VoiceOver/Orca announce role, title, selection, expanded and disabled state correctly?
6. Do unusual GTK themes enlarge circular hit targets or alter contrast? Is text scale constrained before screen edges? Native minimum requisitions need allocated-widget review.
7. How do notification Focus modes, installed app identity/icon caches, notification daemon availability and action layouts vary by OS version?
8. Does Codex protocol-version change affect discovery ownership, async question retirement and request alternatives? Current adapters/test fixtures describe baseline only.
9. On hook-only requests, how quickly does disappearance/reconciliation converge under disconnect/network/load? There is no universal2second end-to-end UI guarantee.
10. Should standalone Telegram requests follow app-visibility policy, and should bot list expose unseen completions? Neither is a decided shared rule.

No unsupported behavior has been filled in with a new design. These questions intentionally prevent a new frontend developer from guessing at unimplemented policy.
