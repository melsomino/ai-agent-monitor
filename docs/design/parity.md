# Three-way UI/UX parity matrix

<a id="parity-legend"></a>
## Classification

**C** shared and consistent in observable intent; **P** differing for known native/OS reasons; **M** missing or simulated in one or more implementations; **D** conflicting behavior requiring a decision. A row may have more than one classification. H=HTML, M=macOS, G=GNOME. All entries are source-verified, not fresh runtime results. “Native” below does not imply tested end to end. Links lead to component/source evidence rather than duplicate every measurement.

<a id="parity-table"></a>
## Ambient surface and popup

| ID | Behavior | H | M | G | Class / evidence |
|---|---|---|---|---|---|
| PA-01 |Robot identity |Embedded raster mask |Template PDF |Symbolic SVG |P,[indicator](components/indicator.md) |
| PA-02 |Counter order/zero |Working,finished,request;hide0 |Same |Same |C |
| PA-03 |Application visibility in counts |Not implemented |Working/finished only |All three |M/D,[D-01](discrepancies.md#d-01) |
| PA-04 |Counter geometry/cap |14high,no cap |14high,99+request/finished |≥18circle,no cap |D,indicator geometry |
| PA-05 |Eye animation |Static |8steps150ms,only no pending |8steps140ms,also pending |M/D,[D-02](discrepancies.md#d-02) |
| PA-06 |Open highlight |Always drawn |NSMenu system tracking |Visible→Shell checked |P/M |
| PA-07 |Tooltip/name |Static ARIA |Dynamic tooltip counts |Dynamic accessible name |P |
| PA-08 |Indicator toggles popup |No |Yes |Yes click/Return/Space |M |
| PA-09 |Popup lifetime |Always visible |Native menu,nonactivating |Activated GTK Utility |P/M,[lifecycle](interactions/input-lifecycle.md) |
| PA-10 |Outside/Escape |Absent |OS menu |Delayed focus-out/Escape |P/M |
| PA-11 |Neighbor menu keys |Absent |Pass Left/Right with no local target |Absent |P/M |
| PA-12 |Position work area |Embedded product |Native screen visibleFrame |Compositor workarea anchor |P/M |
| PA-13 |Max content height |List360 |80%/usable-screen cap |Same cap plus GTK frame |P |
| PA-14 |Material |Opaque gradient |System blur+65%tint |Opaque theme bg |P |
| PA-15 |Closing transient cleanup |No close |Clears query/history,retains cached draft possible |Clears query/cards,retains shared history |D,[D-05](discrepancies.md#d-05) |
| PA-16 |Single list scroll/footer |Yes |Yes |Yes |C |
| PA-17 |Elastic overscroll |Contain CSS |Explicitly off |GTK policy |P |
| PA-18 |Responsive/coarse pointer |≤420/44min targets |No mobile layout |No mobile layout |P/M |

## Sessions and content

| ID | Behavior | H | M | G | Class / evidence |
|---|---|---|---|---|---|
| PS-01 |Discovery/live membership |Fixtures |Provider+TTY/window evidence |Provider,without Mac TTY filter |M/P,[shared capabilities](shared.md#shared-capabilities) |
| PS-02 |Grouping rank |Fixed VSCode,Terminal,Codex |Client rank+ID |Same rank+ID |C/M |
| PS-03 |Title ordering |Fixture order |Foundation natural |GLib natural lowercase |M/P,locale outcome U |
| PS-04 |Application group identification |Scrolling icon/name/separator header |Same document header |Same GTK group child header |C; 24 icon, 26 reserved header height, no sticky overlay |
| PS-05 |Robot title/status/chevron |Yes |Yes |Yes |C,[sessions](components/sessions.md) |
| PS-ROBOT |Working robot fixed size/eyes alternate |2.4s CSS; fixed14×11; 8 interpolated eye phases |Same shared motion, native drawing |Same shared motion, GTK frame callback |C; system reduced-motion yields normal size/both green; runtime GTK/H not verified |
| PS-06 |Title opens /robot toggles |Simulation/toggle; hover/focus shows → + app icon |Host open/toggle; same destination hint |Host open/toggle; same destination hint |C/M; H host action simulated |
| PS-07 |Title hover measured width |Yes |Yes |Natural button |C/P |
| PS-08 |Icon circular target |26×24robot,25footer |28square normalized |28minimum+theme |D,foundations |
| PS-MODEL |Expanded-session model metadata |Optional fixture row, cpu icon |Optional reported model, cpu symbol |Optional reported model, chip symbol |C/P; hide unknown; Codex/Claude supported, Cline unknown |
| PS-09 |Working/request/idle |Static filled green play/filled!/none |Same, direct triangle drawing |Same, Cairo solid triangle |C/P; Working indicator is informational |
| PS-10 |Unknown |No fixture branch;defaults request |Muted questionmark |Muted? |M/D |
| PS-11 |Finished click |Inert disc |Ack generation |Ack generation |M |
| PS-12 |Finished action ack |Title/robot/chevron/history click |Same plus check;keyboard disclosure |Same plus check |C/M,keyboard H omission |
| PS-13 |Autoexpand active on open |Only delivery fixture |Working+requests |Working+requests |M |
| PS-14 |Live autoexpand/manual collapse |No live state |New approval ID/first working |Changed request token;not later work of old row |D,[D-04](discrepancies.md#d-04) |
| PS-15 |Older→latest→request→metadata |Fixture |Yes |Yes |C |
| PS-16 |Previous history |One simulated paragraph |Prepend,anchor,loading,error |Prepend,loading,error,no anchor |M/D |
| PS-17 |Previous arrow hover |Yband anywhere product |First message/gutter bounds |First message event bounds |D,[D-06](discrepancies.md#d-06) |
| PS-18 |Message thread/dots |CSS masked SVG contours |Cairo-equivalent native line15request |Cairo line16request |C/P |
| PS-19 |Markdown |Fixture HTML only |Shared parser→attributed text |Shared parser→Pango |M/P |
| PS-20 |Metadata |Home-like fixture,hide toggle |Selectable~+branch |Selectable~/+branch |C/D,exact home / toggle |
| PS-21 |Empty list states |Absent |3states |3states,different priority |M/D |
| PS-22 |Quota |Static |Remaining/date/expiry/tooltips |Same values,different alignment/tooltips |C/M/P |
| PS-23 |Filter row |Absent |Footer query |Footer query |M |
| PS-24 |Footer power |Pause simulation |Quit |Quit |D,[D-12](discrepancies.md#d-12) |

## Requests and input

| ID | Behavior | H | M | G | Class / evidence |
|---|---|---|---|---|---|
| PR-01 |Solid framed question card |Yes |Yes |Yes |C,[requests](components/requests.md) |
| PR-02 |Agent action mapping |Allow/deny simulated |Offered IDs |Offered IDs |C/M |
| PR-03 |Action layout |Partial flex wrap |All-fit row else column |All-fit row else column |D,[D-07](discrepancies.md#d-07) |
| PR-04 |Action-specific context |All context before buttons |Immediately before action |Immediately before action |D |
| PR-05 |Command two-line expander |Absent |Measured2lines,pointer-only |Measured2lines,pointer-only |M |
| PR-06 |Command dark bg |Hover token |Opaque darker token |Same |D |
| PR-07 |Single-question presets |No single branch |Direct submit |Direct submit |M |
| PR-08 |Multiple independent radios |Yes |Yes |Yes |C |
| PR-09 |Default selected preset |First |First |First toolkit radio |C |
| PR-10 |Choice descriptions |No dynamic option descriptions |Inline text |Tooltip |M/D |
| PR-11 |Custom field only if offered |Always in fixture |Conditional |Conditional |M |
| PR-12 |Custom selects on focus |No,input event only |Yes editing start |Yes multi focus |D |
| PR-13 |Secret input |Absent |Secure native |Visibility=false |M/P |
| PR-14 |Incomplete submit |Validity bubble on click |Disabled but traversal target |Sensitive,silent click return |D,[D-08](discrepancies.md#d-08) |
| PR-15 |Return in custom |Select custom,stay |Select custom,stay |Multi select;single no explicit hook |D |
| PR-16 |Sending/reconciling/failure |Immediate local result |State-driven,inline/ordinary differ |State-driven,whole-card/error |M/D |
| PR-17 |Stale response authority |No backend |Exact snapshot rejected |Exact snapshot rejected |C/M |
| PR-18 |Expiry unavailable rendering |Absent |No explicit main-card gate |Explicit disabled/error |M/D |
| PR-19 |Unsupported request route |Absent |Open in editor |No card fallback action |M/D,[D-11](discrepancies.md#d-11) |
| PR-20 |Multiple+actions mixed |Absent |Fallback normal window |Inline combined |P/D |
| PK-01 |Single keyboard target |DOM activeElement |One logical target/editor |One GTK focus |C/P,[input](interactions/input-lifecycle.md#input-keyboard) |
| PK-02 |Hover vs key color |Hover14%,fixedblue |Hover14%,systemaccent |Hover6%,systemtheme |D,[D-03](discrepancies.md#d-03) |
| PK-03 |Up/Down ends |Clamp |Clamp |Wrap |D |
| PK-04 |Horizontal answer cycle |Absent |Same-y enabled controls |Horizontal box siblings |M/P |
| PK-05 |Title Left/Right disclosure |Yes |Yes |Yes |C |
| PK-06 |Preset Space |Browser radio |Explicit multi-preset select |Native toolkit |C/P |
| PK-07 |Title Space |Simulated open |Filter space |Native button activation |D |
| PK-08 |Type filter/AND words |Absent |Yes |Yes+IME |M |
| PK-09 |Filter highlight |Absent |Merged UTF16 text emphasis |Underline |M/P |
| PK-10 |Focus-on-open/reveal answer |Absent |Yes |Yes |M |
| PK-11 |Resolve clears key focus |Blur within form |Removed control clears |Removed card clears |C/P |
| PK-12 |Caret blink |Browser |Tracking-time custom fallback |GTK entry |P |
| PK-13 |Reduced motion |CSS transitions and working robot off |Menu-bar eyes and working robot off |Reveal and working robot off (panel eye pref separate) |D,[D-13](discrepancies.md#d-13) |

## Settings and external channels

| ID | Behavior | H | M | G | Class / evidence |
|---|---|---|---|---|---|
| PX-01 |Settings shell |Tiny overlay |4native toolbar panes460 |4GTK toolbar panes560 |M/P,[settings](components/settings.md) |
| PX-02 |Login |Local checked only |SMAppService |XDG autostart |M/P |
| PX-03 |Animation setting |Local checked only |Persisted |Persisted |M |
| PX-04 |Global shortcut |Absent |Carbon/native menu |GNOME extension |M/P |
| PX-05 |Application catalog |Absent |Learned switches/empty hint |Learned switches/no empty hint |M/P |
| PX-06 |Integrations |Absent |Automatic install+retry |Startup check/manual install |M/D,[D-14](discrepancies.md#d-14) |
| PX-07 |Telegram setup/connected |Absent |Same actor+native pane |Same actor+GTK pane |M/P |
| PX-08 |BotFather/newbot |Absent |Prefilled command attempt |BotFather link only |M/P |
| PX-09 |Username suggestion |Absent |Foundation transliteration,readonly |GLib transliteration,editable |M/P |
| PX-10 |Setup Cancel |Absent |InitialDisconnect/replacementCancel |AlwaysCancelSetup |D,[D-15](discrepancies.md#d-15) |
| PX-11 |Replacement preserves old |Absent |Yes |Yes shared worker |C/M |
| PX-12 |Token store |Absent |Keychain |Secret Service |P/M |
| PX-13 |Notification action plan |Absent |4budget + More/text reply |4per actions/choices + Open, no text |P/M/D,[notifications](interactions/notifications.md) |
| PX-14 |Notification filtering |Absent |All approvals |Visible/unexpired only |D |
| PX-15 |Notification changed context |Absent |Rebind category/retry |New token,no retry |P/D |
| PX-16 |Secret notification policy |Absent |No presets/text,body not redacted |Presets/body not redacted |D,[D-09](discrepancies.md#d-09) |
| PX-17 |Telegram browser/answers |Absent |Shared worker |Shared worker |C/M,[Telegram](interactions/telegram.md) |
| PX-18 |Telegram same-ID change |Absent |Reject stale,doesn't refresh Pending |Same shared limitation |D,[D-10](discrepancies.md#d-10) |
| PX-19 |Accessibility |ARIA/browser |AppKit labels,custom draws |GTK/ATK,custom draws |P/U,[coverage](coverage.md#coverage-limits) |

| TG-IMAGES |Codex embedded images |No backend media fixture |History placeholder; no native preview |History placeholder; no native preview |Telegram sends media from monitor state; [TG-IMAGE-01–03](interactions/telegram.md#tg-images); H/M/G native preview missing |

External-channel cells are M because H has no implementation, not because desktop OS notifications should be emulated in a sandboxed prototype. The shared Rust Telegram channel is identical logic on M/G except host collation, credentials and native setup; duplication in the matrix intentionally makes its absence in H explicit.
