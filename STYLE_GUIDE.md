# AI Agent Monitor — UI & UX Style Guide

This guide defines the visual language, interaction rules, and user-facing behavior of AI Agent Monitor. It covers the native macOS interface, system notifications, and the personal Telegram bot.

Baseline: the AppKit implementation on 2026-11-02. Dimensions are logical macOS points, not screenshot pixels. The interface remains native AppKit; the HTML preview is a design reference, not the production UI.

**How to read this document:** component measurements describe the current implementation. Interaction rules describe the behavior to preserve. The manual acceptance checklist is a verification requirement, not a claim that every macOS interaction has already been visually verified.

## 1. Product principles

1. **Make agent activity easy to scan.** Session titles are the strongest text in the list. Working, waiting, and newly finished states are recognizable without expanding a session.
2. **Keep the user in control.** Opening a panel, hovering, reading a message, or focusing an answer must never grant permission. Sending an answer is an explicit action.
3. **Show the agent's actual request.** Preserve its question, command, response captions, and alternatives. Do not replace specific choices with a generic Allow when the agent offers more detail.
4. **Use progressive disclosure.** Show titles and status first; messages, questions, directory, and branch appear when expanded. Rarely needed metadata stays at the bottom.
5. **Behave like a macOS menu.** Opening, dismissal, keyboard behavior, appearance, and menu-bar selection should feel native. Avoid activation changes, delayed repositioning, and bespoke window behavior where AppKit provides the expected interaction.
6. **Treat all response surfaces as peers.** The menu, notifications, Telegram, and the agent's own UI reflect the same request. Resolving it in one place removes the actionable request elsewhere.
7. **Keep updates calm.** Refresh automatically. Avoid repeated notifications, duplicate Telegram messages, unexpected focus changes, and decorative movement unrelated to agent activity.

## 2. Information architecture

| Surface | Primary purpose | Main content |
| --- | --- | --- |
| Menu-bar item | Ambient activity and attention summary | Robot artwork, working/request/finished counts |
| Sessions menu | Inspect and act on active sessions | Application groups, session headers, expanded messages and requests, metadata |
| Menu footer | Persistent utility information | Codex quota, version, Settings, Quit |
| Settings window | Configure the monitor | General, Applications, Integrations, Telegram |
| macOS notifications | Timely requests outside the menu | Session title, request text, supported actions |
| Telegram bot | Remote reading and interaction | Session list, messages, requests, history, session information |

There is no dashboard heading or search box above the native session list. Settings is one window with toolbar sections. Telegram setup and replacement stay inside that window.

## 3. Visual foundations

### 3.1 Native materials and appearance

- Support both Aqua and Dark Aqua. Use semantic AppKit colors for labels, focus, links, disabled controls, and native backgrounds.
- The sessions menu retains the system menu material and its blur. A **65%-opacity `windowBackgroundColor` overlay** reduces distracting contrast from the desktop underneath; this attenuates the visible background rather than changing the operating system's blur radius.
- Cover the complete menu surface, including native top padding. Do not leave an untreated strip at the top or an exposed gap below the footer.
- The footer is a quieter, opaque surface: light `#E7E7E7`, dark `#2C2C2C`.
- Request cards are opaque neutral panels: light `#E7E7E7`, dark `#373737`. They are more solid than the menu material.
- Command blocks are darker than their containing request card: light `#DADADA`, dark `#2A2A2A`.
- Preserve the system window outline, shadow, and outer corner treatment. Do not simulate another window border inside the menu.

### 3.2 Color roles

| Role | Light appearance | Dark appearance | Usage |
| --- | --- | --- | --- |
| Primary text | `labelColor` | `labelColor` | Titles, questions, answer captions |
| Secondary text | `secondaryLabelColor` | `secondaryLabelColor` | Metadata, descriptions, supporting copy |
| Lowest emphasis | `tertiaryLabelColor` | `tertiaryLabelColor` | Version and unknown-state treatment where applicable |
| Message text | `#414141` | `#CFCFCF` | Expanded agent messages |
| Request text | `#202020` | `#F1F1F1` | Readable request content inside cards |
| Working, in the list | `#007637` | `#30D158` | Spinner and robot eyes |
| Attention | `systemOrangeColor` | `systemOrangeColor` | Pending request markers and request count |
| Finished, in the list | `#0066BE` | `#73C7FF` | Filled completion disc |
| Hover fill | Black at 14% alpha | White at 14% alpha | Pointer feedback, resolved for the current appearance |
| Keyboard selection | `controlAccentColor` | `controlAccentColor` | The single active keyboard target |
| Selected foreground | `selectedMenuItemTextColor` | `selectedMenuItemTextColor` | Text on a focused selection |
| Links | `linkColor` | `linkColor` | Native attributed-text links |

Text actions with an opaque resting surface use a more distinct hover: light `#E1E1E1`, dark `#646464`. Keyboard focus remains the system accent color.

The menu-bar artwork sits on a different material from the list. Its working count/eyes use system green; its finished count uses a pale blue filled disc with dark ink. Do not copy the darker list green into the menu bar without checking contrast there.

Color supplements shape: rotating arrows mean working, an exclamation mark means a request, and a checkmark means finished. Idle has no right-hand status icon.

### 3.3 Typography

Use system fonts and native text measurement. Do not size text by character count.

| Element | Size / treatment |
| --- | --- |
| Session title | 13 pt system font, weight 0.3; one-line header |
| Agent message | 13 pt system font; Markdown styles applied as attributed text |
| Question heading | 13 pt, stronger than answer text |
| Answer / action caption | 12 pt; medium emphasis for action buttons |
| Freeform input | 12 pt system font |
| Directory and branch | 11 pt secondary text |
| Code / command | 12 pt monospaced system font |
| Supporting descriptions | Usually 11 pt secondary text |
| Version | 10 pt, tertiary emphasis |
| Markdown headings | 16 / 15 / 14 pt for heading levels 1 / 2 / 3 |

Markdown paragraph line spacing is 3 pt. Code receives a subtle secondary-label background at 10% alpha. Bold, italic, strikethrough, lists, quotes, and links are native attributed text; do not introduce a web view or remote image loading to render messages.

### 3.4 Geometry tokens

The menu's custom content is 392 pt wide; its list document is 384 pt wide. Coordinates below are local component coordinates, not global screen positions.

| Token | Value | Meaning |
| --- | ---: | --- |
| Header row height | 36 pt | Collapsed session and expanded session header |
| List top inset | 6 pt | Initial document inset |
| Application-group gap | 13 pt | Separation between different applications |
| Application icon | 24 × 24 pt | Real application icon |
| Icon column left | 8 pt | Icon placement within the list |
| Session text left | 40 pt | Title/message text alignment |
| Detail/card width | 326 pt | Standard expanded detail region |
| Header-to-message gap | 4 pt | Expanded content starts close to its header |
| List bottom padding | 8 pt | Prevent the final hover from being clipped |
| Icon-button target | 28 × 28 pt | Shared circular hover size |
| Icon-button hover radius | 14 pt | A circle, not a flattened capsule |
| Text-button horizontal inset | 18 pt | Each side of an action caption |
| Text-button vertical inset | 4 pt | Each side of a single-line caption |
| Text-button minimum height | 24 pt | Grow when wrapped text requires it |
| Adjacent text-action gap | 6 pt | When actions can share a row |
| Request-card radius | 12 pt | Neutral request container |
| Command-block radius | 8 pt | Nested code container |
| Metadata row | 16 pt | Directory and branch row height |
| Metadata row gap | 1 pt | Compact directory/branch pair |

Application icons, robots, status symbols, and controls must be optically centered against the title's visible text, not just aligned to the text field's rectangular bounds. Account for cap height and native cell insets. Increasing a hover target must not enlarge or shift its glyph.

## 4. Menu-bar item

### 4.1 Identity and artwork

- The application icon uses the two-robot artwork. The menu-bar icon uses the matching outlined template PDF with ears, round eyes, and no mouths.
- Use the real bundled app icon for system notifications. Do not retain legacy icon resources or a separate outdated notification identity.
- Template artwork follows native menu-bar tinting. Keep its apparent size compact; do not enlarge its clickable pill to accommodate empty space.
- Counters use regular system weight rather than bold.

### 4.2 Counts and working animation

Display only nonzero counts, in this order: **working → unseen finished → requests**. Hidden applications must not contribute to native-menu activity counts. The transient title filter does not change the global counts.

Working animation is the robot-eye sequence, not a rotating badge. Idle eyes disappear in the working artwork; green eyes cycle through independent 0%, 50%, and 100% intensities over eight phases. Respect the Menu bar animation preference. Do not animate when no agent is working.

### 4.3 Native tracking contract

The status item owns an attached `NSMenu`. Its selected menu-bar background must remain visible for the entire time the menu is open and clear when it closes. Avoid replacing that relationship with a standalone popup and manually simulating a pressed button.

With no keyboard target inside the panel, Left/Right are passed to AppKit for menu-bar navigation. Once the user is navigating a session or a request, those keys have the local meanings documented below. Switching to another menu closes this one and transfers the system selection normally.

This contract must be checked on macOS: unit tests do not prove native selected-background persistence or navigation between separate applications' menu-bar items.

## 5. Sessions menu

### 5.1 Opening, positioning, and dismissal

- Open by clicking the status item, using the configured global shortcut, or following a supported notification action.
- Use native menu tracking rather than activating the monitor as a normal foreground application just to inspect sessions.
- Align the window's top edge with the bottom of the menu bar; no additional top gap.
- Keep the window inside the current display's usable area. Respect a Dock on the left, right, or bottom, including displays with negative coordinates. The work-area confinement uses a 4 pt side/bottom margin.
- Resolve placement before painting. Do not first appear over the Dock and then jump sideways.
- Height follows content until the screen-dependent limit. The current maximum is the smaller of 80% of screen height minus 20 pt and usable screen height minus 24 pt; long content then scrolls.
- Recalculate geometry during disclosure and state updates. The background, native padding extensions, scroll area, and footer must update within the same painted frame.
- Clicking outside, Escape, switching away, and normal system menu dismissal must behave like a macOS menu. The global shortcut toggles the menu.
- Closing clears transient keyboard focus and typed filtering, stops editing, collapses sessions, and resets disclosure state. Closing alone does not acknowledge finished sessions.

### 5.2 Session membership, grouping, and order

Show open sessions, including idle sessions. Do not treat historical transcripts as proof of a live session. Remove sessions when their hosting application/window/process is no longer available according to the adapter's live-session evidence.

Respect the Applications preference before constructing the visible list. Group by application without separate application-name heading rows. The current native order is VS Code, Terminal, other identified desktop applications, then unknown clients; application identity separates groups. Inside each application, sort by localized natural title order with a stable identity tie-breaker.

Use the application's actual icon obtained through macOS. Use a generic app symbol only when an actual icon cannot be resolved. One sticky application icon identifies the group; it stays at the top of the viewport as its group scrolls and is pushed away by the next group. It must not drift during rubber-band scrolling or overlap the following group.

### 5.3 Session header anatomy

The header contains a small robot in the left gutter, a title, an optional status symbol, and a disclosure chevron. The sticky application icon can occupy the same gutter above the scrolling robot; this overlap expresses group identity without adding a group-heading row.

| Target | Pointer action | Feedback |
| --- | --- | --- |
| Session title | Switch to the session's hosting application/window | Rounded title hover; tooltip “Switch to <application>” |
| Robot | Expand/collapse the session | Circular icon hover; disclosure tooltip |
| Chevron | Expand/collapse the session | Same circular hover size as the robot |
| Finished checkmark | Mark the completion seen | Circular hover; tooltip “Mark as seen” |
| Working/request status | Informational status | Status tooltip; do not turn it into an implicit approval |

The title's hover and keyboard highlight follow the title's measured width plus padding. They exclude the robot, right-side status, and chevron. Give the right-side controls their own space and targets; do not allow a long title to cover them. Long titles remain single-line and use native truncation when needed.

All icon buttons share a square target and circular background. Center the glyph inside it. The button target, hover target, and intended click region must agree. Previous-message gutter interaction is a deliberate exception: its target is the message's gutter area, described below.

### 5.4 Status vocabulary and transitions

| State | Right-side indicator | Robot treatment | Meaning |
| --- | --- | --- | --- |
| Idle | None | Neutral eyes | Open session, no current work or request |
| Working | Rotating circular arrows, green | Green eyes | Agent is working |
| Needs response | Filled orange disc with exclamation mark | Orange eyes | Active human interaction, including permissions and questions |
| Finished, unseen | Filled blue disc with checkmark | Neutral eyes | Session transitioned from working to authoritative idle and has not been acknowledged |
| Unknown | Muted question-mark symbol | Neutral eyes | Activity cannot currently be established |

Finished is an attention marker layered over idle, not a separate agent execution state. A confirmed working-to-idle transition creates the marker and increments the finished count. Merely seeing the row does not clear it. Acknowledge it when the user:

- Expands or collapses that session.
- Opens it in its application.
- Loads a previous message.
- Clicks its finished checkmark.

Acknowledgment is scoped to that completion generation. It must not consume a newer completion. Starting work or receiving a new request supersedes the old marker; temporary unknown/non-authoritative observations must not silently erase it. Acknowledging finished must not stop work, answer a request, close the menu, or expand the row by itself.

### 5.5 Expanded session and message history

Use this reading order:

1. Session header.
2. Visible messages, chronological from older to newer.
3. Active request panel, if present.
4. Working directory and Git branch.

The request panel must not replace the latest agent message. Trim trailing whitespace when displaying messages. Show “No messages yet” when no message is available. Render Markdown as native attributed text and measure the formatted result, not the raw Markdown source.

Load previous messages by prepending them. The small previous-message control belongs in the gutter beside the first visible message and is vertically centered within that message's gutter. It appears only while the pointer is over the first message or its gutter; the whole gutter is clickable. Keep a small horizontal separation from message content. Loading history must not reset live messages or corrupt another surface's history cursor: the native window and Telegram have separate history state.

The expanded gutter uses a subtle vertical thread descending from the robot's contour, filled dots for later messages, and the orange request marker beside a request card. The line must meet, not cross, indicator contours. These are secondary reading guides, never stronger than the text. Do not add a decorative ring around the robot.

Use one main vertical scroller. Expanded content and window margins route wheel input to that scroller; hovering a long expanded message must not trap scrolling in an inner document. Maintain bottom padding so the last row's hover remains visible above the footer.

### 5.6 Metadata

Directory and Git branch belong at the end of expanded content, each on its own compact line. Use the folder and branch symbols instead of “Directory:” and “Branch:” prefixes. Shorten the home directory to `~`; keep the full directory available in the tooltip. Metadata text is selectable. Align icons to visible text height and keep their color secondary.

Do not infer the hosting VS Code workspace from the most recent command's working directory. Commands can run in other directories without changing the editor project.

## 6. Requests, approvals, and answers

### 6.1 Request card

Use a neutral, opaque card with a subtle outline, 12 pt corners, and balanced internal spacing. Keep its left alignment consistent with the message column. Attention is conveyed by the orange indicator, not by making the entire card bright orange.

The question is first. Optional explanation and command context follow it. Buttons belong below the relevant text/context. Metadata stays outside the card. Do not leave an unexplained empty region after the final action.

When the menu opens, sessions with requests and sessions working are expanded automatically. Within an already open menu, respect a user's manual collapse unless a new interaction needs to be revealed. Focus the first available answer control on opening a request and scroll it into view if necessary.

### 6.2 Permission actions

Preserve response captions supplied by the agent, such as **Allow once**, **Allow similar commands**, and **Deny**. Do not relabel Deny as Cancel turn unless that is the actual offered action. Keep the real alternatives available; do not synthesize permission policies.

Buttons use natural caption width, pill-shaped corners, and visible separation. If all buttons fit, lay them out left-to-right, aligned with the question. Otherwise use a left-aligned column of naturally sized buttons; do not stretch them to card width. Context tied to a specific action stays beside that action in reading order, which can require a vertical layout even when captions alone would fit.

An unfocused action is neutral. Hover uses the normal subtle fill. Only the keyboard-focused button uses the accent selection. Readability takes precedence over fixed height: never split “Allow” across lines just to fit a too-small button.

### 6.3 Commands and policy prefixes

- Render commands in darker monospaced blocks inside the card. Preserve content and whitespace needed to understand the operation.
- Collapse to **two rendered lines**, counting both explicit line breaks and wrapping. This is not a two-newline limit.
- Put a small expand/collapse chevron to the right of the command caption. Show it only when the content overflows.
- The command expander is pointer-accessible but excluded from the answer keyboard traversal.
- Keep a reusable command prefix near the policy action it explains. Show the prefix itself, without verbose scaffolding such as “Future commands with prefix:”.
- Never expose raw internal response JSON as the user-facing policy explanation.

### 6.4 One question

Show the question followed by its ready-made answers as direct-action buttons. Choosing a ready answer submits it. When freeform input is offered, show the input with **Send answer** alongside it. Do not show a custom-answer field when the agent disallows it.

### 6.5 Multiple questions

Present each question as its own radio group, with presets followed by an optional custom-answer row. One choice is selected per question. A shared **Send answer** button submits the combined response.

- Selected answers use the radio's accent-colored selected mark. Do not color all selected answer text blue.
- Keyboard focus highlights the answer's text region with the same system accent selection used elsewhere. Selection and focus are different concepts.
- Hover has the same shape as focus, using the normal hover color.
- Typing a custom answer selects that question's custom radio choice.
- Enter in a custom field selects/commits that custom choice and keeps the menu open; it must not activate a default action that closes the menu.
- The focused field has a system-style focus ring and a visible blinking insertion caret. Do not leave a simultaneous answer/button highlight active.
- Long question and answer text wraps naturally, with enough leading and spacing for reading. Do not reserve invisible width that causes premature wrapping or clip the right edge.
- A shared submit control is included in keyboard traversal, has a visible caption in every state, and follows all the question groups.

Complex requests can use a dedicated native answer form as a fallback where the inline renderer is not applicable. Preserve the same answer semantics and request validation; do not create a second, independent request state.

### 6.6 Submission lifecycle

All answers are commands to the monitor. The monitor validates the current request identity, sends the response to the agent, and publishes the resulting state. A click is not proof that the agent accepted an answer.

Prevent duplicate submissions while a response is pending. Keep failures readable and recoverable rather than showing unconditional success. If the request changes or disappears, retire stale controls. Once an answer resolves the request, remove it from every surface, including the system notification. Clear the answer's keyboard focus; do not silently transfer it to the session title.

## 7. Hover, keyboard focus, and navigation

### 7.1 Two independent concepts

There is **at most one hovered target** and **at most one keyboard-focused target** in the menu. They can be different targets: the pointer and keyboard are independent input methods. Neither state is an answer selection.

Determine hover from the current pointer position in a shared coordinate system. Recalculate when the pointer moves or layout/scrolling changes. Controls take precedence over titles; clipped or hidden content is not interactive. Leaving the window clears hover. Do not rely on whichever view last became first responder to update pointer feedback.

The single keyboard target is one of: session title, action button, answer option, or input field. Native field editing and the menu's target model must agree; a field ring and an unrelated blue answer/button must never appear simultaneously.

### 7.2 Key map

| Input | Behavior |
| --- | --- |
| Configured global shortcut | Open the menu; toggle it closed if already open |
| Escape | Dismiss using native menu behavior |
| Up / Down | Traverse visible session titles and interactive request controls; reveal the target by scrolling |
| Left / Right on a session title | Collapse / expand that session |
| Left / Right on horizontally arranged answer buttons | Move through that question's buttons, wrapping around |
| Left / Right with no internal target | Leave handling to AppKit's menu-bar navigation |
| Enter on a session title | Switch to its application, as with a title click |
| Enter on an answer/action | Activate that control |
| Space on a preset answer | Select/activate it as Enter does; do not toggle session disclosure |
| Enter in a custom-answer field | Commit/select the custom answer and keep the menu open |
| Printable text outside an input | Accumulate the title filter |
| Backspace outside an input | Remove the last composed character from the filter |
| Standard editing shortcuts inside an input | Native editing, including Command-V paste |

Field editing takes precedence over filtering. Do not interpret a space typed into an input as a preset activation. Modified keys and cursor movement must retain native editing behavior. Tab is not a substitute for the documented Up/Down session traversal; retain native control behavior where applicable.

### 7.3 Type-to-filter

No permanent search box is shown. Outside text fields, typed text builds a temporary filter. Split it into space-separated words; every word must occur in the session title for that session to remain visible. Highlight matches in the title. Backspace removes a composed Unicode character, not an arbitrary byte. An empty filter restores the full eligible list; closing clears it.

Provide distinct empty states for “No matching sessions”, “No visible sessions” after application filtering, and “No open sessions”. Never present a blank unexplained list.

## 8. Footer

The footer stays outside the scrolling list. A full-width separator divides it from the content. Its fill extends through native bottom padding; no transparent strip should appear below it.

Show Codex quota rows with a service-and-window label such as **Codex Weekly**, the remaining percentage, and reset date. Percentage and date align to the right, separated by a fixed gap rather than a growing spacer. Tooltip detail can include the full reset timestamp and observation time. Unknown quota values use an em dash; do not invent a remaining balance after a reset.

The bottom row shows `v<version>+<build>` at the left. Settings (`gearshape`) and Quit (`power`) are circular icon buttons at the right. Do not add a second Quit text row or a Refresh button. Discover and update sessions automatically.

## 9. Settings window

### 9.1 Common window behavior

Use one standard titled window: **AI Agent Monitor Settings**. Finder-style toolbar sections use an icon above their caption: **General**, **Applications**, **Integrations**, **Telegram**.

Keep sections compact and sized to actual content. Preserve the top edge when resizing and constrain the window to the usable screen. Switching tabs, beginning bot replacement, canceling it, and completing setup must update the actual tab content and window size immediately. No blank tab, clipped explanatory text, stale hidden pane, or large artificial footer.

Settings is a normal editable window rather than the transient sessions menu. It must not disappear behind other windows merely because the user clicks a menu-bar item. Native window controls and text-editing shortcuts remain available.

### 9.2 General

- **Launch at login:** checkbox-style preference with supporting copy. Surface approval-required or unavailable login-item states where necessary.
- **Menu bar animation:** checkbox-style preference controlling working-eye animation.
- **Keyboard shortcut:** recorder and clear control. Show the recorded combination; explain recording/errors inline. Require an appropriate Command, Option, or Control modifier. Escape cancels recording, and clearing removes the shortcut.

Use compact checkbox rows for the top preferences, not large trailing switches. Avoid oversized separators and unexplained empty blocks between settings.

### 9.3 Applications

Show known applications with actual icons, readable names, and trailing visibility switches. The list grows from detected applications and retains the user's choices across restarts. Turning an application off excludes its sessions from the native menu and its activity counters; it does not close or stop the agent sessions.

Give application rows enough separation for independent scanning. Do not add an empty fixed-height footer below a short list.

### 9.4 Integrations

Show detected agent providers with status next to the provider name and a short explanation underneath. Keep status text near its provider, not against an unrelated far edge. Use a status symbol as well as color.

Codex uses a direct local connection and needs no approval hook. Claude Code and Cline integrations are experimental; show their actual installation/capability state. **Check & set up** performs integration detection/setup. Do not imply that every provider supports every response type. There is no separate integration-setup window.

### 9.5 Telegram: connected state

Show **Telegram is connected**, then the bot username. **Test** and **Open** follow the username on the same row with a fixed gap and vertical centering. Below, left-align **Change bot** and **Unlink bot**.

Keep the connected state distinct from setup. Do not show token entry, onboarding steps, or inactive connection controls merely because the user opened this section. Test feedback appears inline without changing the connected bot.

### 9.6 Telegram: initial setup and replacement

Use the title **Set up Telegram integration** and a brief introduction. Retain the privacy explanation, without a separate “Why your own bot?” heading:

> We chose personal Telegram bots over shared ones to protect your privacy. Your bot connects directly to the app running on your Mac, so your conversations never pass through our servers — only Telegram's.

Mention that an existing bot's token can be pasted directly. Then show three steps:

1. **Open BotFather and send `/newbot`.** Provide an Open button.
2. **Choose the bot's name and username.** Compact aligned rows: Bot name + field + Copy; Bot username + field + Regenerate + Copy.
3. **Copy the token from BotFather and paste it below.** Explain where to find it in BotFather's last message.

Default bot name: **AI Agent Monitor**. Suggest `aiagmon_<canonical_full_name>_bot`: transliterate the user's full name to Latin, lowercase it, normalize separators, and respect Telegram's length/character constraints. Regenerate offers a variant if a username is taken. Suggestions never rename an already connected bot.

The token is a secure **single-line** field. Its right edge aligns with the bot-name field; Paste occupies the aligned action column. Support Command-V. Put **Connect** at the bottom left, aligned on the same row as Cancel. Show “Your bot token will be stored securely in macOS Keychain.” without truncation.

Use inline operational status such as “Checking token…”, an actionable error, or waiting for activation. Do not add redundant “Not connected” copy while the user is simply filling in the form. Open the bot after a successful token check so the user can press Start; hide Open bot until that step is available.

During replacement only, explain that the current bot stays connected until the replacement is activated. Do not unlink it when Change bot is first clicked. Cancel restores the connected management pane when there is an existing bot, or the appropriate disconnected/setup state otherwise; it must never leave an empty section.

## 10. macOS notifications

The notification's session title identifies the conversation. Do not prepend a redundant Codex heading or repeat the application/service name in the message body. System-owned application attribution is separate from session content.

Use the current bundled robot icon. Permission captions come from the agent. Supported single questions may offer choices and text reply; multiple questions open the menu through **Answer questions…**. Overflow choices retain an entry such as **More answers…** rather than silently dropping alternatives. The current notification plan reserves up to four actions for the system's detailed presentation; macOS decides their visible layout.

Answer actions send commands to the monitor, exactly like native-menu controls. Opening a notification or opening the editor is not approval. Opening a multi-question request should reveal the sessions menu and the relevant interaction rather than an unrelated conversation.

Remove delivered/pending notifications when their requests resolve, including when answered in the editor or from another surface. Do not notify again merely because an already submitted request briefly reappears during reconciliation. Notification banners depend on macOS notification settings and Focus; do not interpret an entry in Notification Center as proof that a banner appeared.

## 11. Telegram interaction design

### 11.1 Navigation and session list

Provide command buttons with readable captions such as **Sessions** and **Help**, without a leading slash. Slash commands remain supported (`/sessions`, `/status`, `/info`, `/help`).

The session list is interactive and updates automatically. Do not add application-name heading buttons or Refresh buttons. Session buttons identify the application on the left and put any status before the navigation chevron. Telegram controls the alignment and rendering of inline keyboard labels; do not promise AppKit-style left alignment or real macOS app artwork inside a text-only Telegram button.

Opening a session establishes a stable selected-session context. Text sent to the bot then targets that supported live agent session. A request arriving from another session must not steal or clear that selection. Explicit navigation, not whichever message was last sent, determines the target.

### 11.2 Session detail

The heading contains application marker, session title, and state. Do not add redundant “VS Code” text after the marker. Render supported rich text using Telegram formatting; use proper preformatted command blocks and a language hint when available.

Provide **Previous message**, **Sessions**, and **Info** navigation as applicable. Directory and branch appear in Info, not below every message. Avoid repeated helper text such as “Type a message to send it to this session” and success messages such as “Message sent” for routine sends.

New agent messages become new Telegram messages, and controls move to the newest relevant message. Changes to status or the current request update the existing message rather than creating duplicate copies of identical agent text. Older history stays readable without leaving stale actionable buttons attached.

### 11.3 Requests and reconciliation

If the selected session detail already displays a request, suppress a second general notification message for that same request. Requests from another session may be delivered independently while preserving the selected session.

Use the actual offered answers and readable command/policy context. Revalidate callback identity and request state before submission. A queued answer is not the same as agent execution success. Remove resolved standalone request messages where possible; retire stale controls and give an understandable fallback if deletion cannot complete. Do not expose internal response serialization in button captions.

Telegram setup uses a personal bot and stores its token in Keychain. The monitor runs on the user's Mac; the Mac must be awake and the app running. Explain that session content goes through Telegram, without claiming end-to-end encryption or a separate monitor-hosted relay.

## 12. Motion and live updates

Disclosure uses a 240 ms cubic ease-out transition. Rotate the chevron with the reveal progress. Interrupted transitions continue from their sampled position rather than restarting with a jump. Keep header controls pinned while content height changes.

Spinner rotation is about the glyph's center, with no orbiting or side-to-side movement. The menu-bar eye animation and list spinner are separate expressions of working state; disabling menu-bar animation does not imply the agent is idle.

Background fill must be present on every animation frame, including native menu padding. Sticky application icons stay constrained to their groups during normal scrolling and overscroll. Auto-refresh must not reset the current filter, disclosure choice, selected Telegram session, text being entered, or request identity unnecessarily.

## 13. Writing, errors, and accessibility

- Production interface captions are English. Agent-authored messages, titles, questions, and options retain their original language.
- Prefer short, concrete action captions: Open, Test, Copy, Paste, Connect, Change bot, Unlink bot, Send answer.
- Use tooltips for icon actions and full truncated metadata. Include application context in the title's navigation tooltip.
- Keep text selectable where the user may need to copy paths, branches, commands, or message content.
- Provide a readable disabled/error state. Explain what failed and which action is possible next; do not leave a permanently loading request after a delivery failure.
- Do not expose raw protocol payloads, tokens, or implementation details as ordinary UI copy.
- Use semantic accessibility descriptions for meaningful symbols and labels for inputs. Decorative thread lines and click-through icon artwork must not steal input from their controls.
- Check system accent variations, increased contrast, Reduce Transparency, and Reduce Motion manually. These are acceptance requirements; the presence of semantic colors alone does not certify complete accessibility support.

## 14. Manual acceptance checklist

Run this checklist after changes to menu ownership, geometry, focus, appearance, or rendering. Do not equate compilation/unit-test success with visual acceptance.

### Menu and layout

- Open by click and shortcut: one menu, no focus theft, no stray launch window.
- Menu-bar selection persists while open; clears after close. Check neighboring-menu navigation in macOS with no internal keyboard focus.
- No gap above the menu, no unfilled top strip, no footer-bottom gap.
- Dock left/right/bottom: no overlap or visible sideways correction. Repeat on another display.
- Expand/collapse repeatedly below and at maximum height: no flash of unfilled native padding, no content jump.
- One short session, many sessions, a one-session application group, long titles, and overscroll all keep icons/controls aligned.
- Robot, chevron, completion, utility, and command icon-button hovers are circular and equal in size; glyph sizes remain independent.
- The final row's hover is fully visible above the footer.

### States and input

- Exercise idle → working → request → working → finished → acknowledged.
- View/close without acting: finished remains. Click its checkmark: only that completion clears, menu remains open.
- Answer in the editor: the request disappears in the menu, notification, and Telegram.
- Check hover after pointer movement, scrolling, disclosure, and leaving the window; no stale highlight.
- Traverse a session, answer option, field, and submit button: exactly one keyboard target, correct system accent.
- Type in a custom field: native ring and caret, custom radio selected, Enter does not close the menu.
- Test multiword filtering, Unicode Backspace, no matches, and closing/reopening.

### Requests and external surfaces

- One question with/without freeform; multiple questions; long answer captions; a request with no supported reply.
- Allow once / policy / Deny remain distinct. Long commands collapse at two visual lines; expansion is outside answer traversal.
- Click an answer twice, resolve elsewhere, then click an old notification/Telegram button: no duplicate or stale permission is granted.
- Telegram: selected session survives an unrelated request; status changes edit instead of duplicating message text.
- Notifications: current icon, session title, actual response captions, and appropriate menu-opening action.

### Settings and appearance

- Light and dark backgrounds, busy wallpaper, different accents: labels, spinner, counts, and controls remain readable.
- Every settings section sizes to content immediately. Change bot → Cancel restores a populated pane.
- Long bot username, token paste, invalid token, activation wait, successful replacement, and unlink are understandable and remain in one settings window.
- Check accessible labels, increased contrast, and system transparency/motion preferences.

## 15. Source map and maintenance

| Responsibility | Source |
| --- | --- |
| Native menu ownership, layout, hover, settings, footer | [`src/macos/menu.rs`](src/macos/menu.rs) |
| Shared list geometry | [`src/macos/session_layout.rs`](src/macos/session_layout.rs) |
| Pointer target precedence and clipping | [`src/macos/pointer.rs`](src/macos/pointer.rs) |
| Question forms and editing/focus | [`src/macos/inline_questions.rs`](src/macos/inline_questions.rs) |
| Robot and thread drawing | [`src/macos/session_robot.rs`](src/macos/session_robot.rs), [`session_thread.rs`](src/macos/session_thread.rs) |
| Disclosure and eye animation | [`src/macos/motion.rs`](src/macos/motion.rs) |
| Native Markdown | [`src/macos/markdown.rs`](src/macos/markdown.rs) |
| Settings pane replacement/resizing | [`src/macos/settings_pane.rs`](src/macos/settings_pane.rs) |
| Notification actions | [`src/macos/notification_actions.rs`](src/macos/notification_actions.rs) |
| Telegram setup suggestions | [`src/macos/telegram_setup.rs`](src/macos/telegram_setup.rs) |
| Telegram interaction and browser | [`src/actors/human/telegram.rs`](src/actors/human/telegram.rs), [`telegram/sessions.rs`](src/actors/human/telegram/sessions.rs) |
| Completion/history state and request commands | [`src/actors/monitor`](src/actors/monitor) |
| HTML design reference | [`design/previews/monitor-ui.html`](design/previews/monitor-ui.html) |
| Menu-bar artwork specification | [`assets/menubar/README.md`](assets/menubar/README.md) |

For a visual change, update the native implementation and relevant HTML reference together, then update this guide's tokens and behavior if the intended design changes. A mockup must not silently become a promise of functionality absent from the native app. Keep adapter/protocol details in the architecture and integration documentation; keep this document focused on what users see and do.
