# Settings and setup

<a id="settings-shell"></a>
## Window, navigation, and sizing

M: one retained normal titled/closable settings window, title **AI Agent Monitor Settings**, floating level, does not hide on deactivate. Native preferences toolbar icon+label tabs: General(gearshape), Applications(square.stack.3d.up), Integrations(puzzlepiece.extension), Telegram(paperplane). Fixed content width460; each pane requests its own height; resize preserves top edge and selected pane updates immediately. Close retains window; selection persists in the retained controller but is not explicitly persisted to disk. Opening activates settings, unlike opening the session menu.

G: normal GTK Window560 initial width, toolbar ToolButtons (system/app-grid/addon/mail-send symbols), nonhomogeneous Stack of four same-named pages;18page padding,10vertical spacing. Page change/snapshot triggers preferred-height resize, preserves position. Window manager can affect actual decoration/resize behavior. Close hides, retains controls. G button toolbar is not AppKit preference navigation; no browser settings replica exists beyond H's miniature overlay.

H: absolute overlay245max width, bottom40/right12, padding12/radius10. General label + checked Launch at login/Menu bar animation/Show folder and branch. Only metadata toggle has a product-visible handler; other checkboxes change local checked state. It has no Applications/Integrations/Telegram/shortcut pane, login service, or persistence. H gear aria-expanded tracks visibility; no outside-click/escape handler.

Sources: [M menu](../../../src/macos/menu.rs) `show_settings`, `PreferencesTabs::did_select`, `size_settings_panel`, `resize_preferences_window`, refresh methods; [pane replacement](../../../src/macos/settings_pane.rs) `replace`; [G settings](../../../src/linux/settings.rs) `Settings::new`, `sync`, `resize`.

<a id="settings-general"></a>
## General

| Control | Default/storage | Inputs and effects | Conditional/error state |
|---|---|---|---|
| Launch at login |Disabled in shared default; actual OS status used |M checkbox→SMAppService register/unregister; G CheckButton→autostart desktop file |M RequiresApproval stays checked, note asks system permission, Open Login Items Settings… appears; Unavailable note; preference error red. G failure sets Unavailable and inline global error |
| Menu bar animation |true when unset |M checkbox saves AnimateMenuBar, G sends SetAnimation/save |Controls ambient eye animation, **not all UI motion**; no separate speed control |
| Keyboard shortcut |None |Click record, then key; clear removes; toggles session menu |Modifier required: Cmd/Option/Control M; Ctrl/Alt/Super G. Shift alone insufficient |

M General content460×244,24horizontal margins; checkbox row24; support11pt; separators; recorder140×28, clear×28 with accessible Clear keyboard shortcut. Text “Open the sessions menu from any app.” Recording title Press shortcut…; Escape cancels, Tab exits recording and proceeds, unmodified Backspace/Delete clears, repeated key ignored. On losing key window, recording ends/restores prior registration. Valid combination attempts registration before committing preference; conflict remains recording with inline error. Menu-open Carbon registration released and hidden native Close key equivalent handles the same shortcut; reinstall on close. Loaded preferences applied even if they arrive after UI startup. Source: [hotkey](../../../src/macos/hotkey.rs) `Shortcut`, `Registration`, `MenuKeyboard`; menu `record_shortcut_key`, `configure_menu_shortcut`.

G recorder grabs focus; Escape restores label, modifierless key shows Use Control, Alt, or Super. GTK accelerator text stored; Shell extension registers only in normal shell mode, reports conflict “This shortcut is already in use. Choose another combination.” Explicit Clear button. General checkboxes/record control initialized once; `sync` does not update every general widget from later snapshots, so cross-process/external preference changes are not guaranteed to repaint controls. No shared requirement for identical physical key codes across OS.

<a id="settings-apps"></a>
## Applications

Purpose: choose whose sessions are included without disabling agent adapters. Catalog is learned from discovery and retained; unknown clients appear **Other apps**. New apps enabled by default, seen app name/icon updated without resetting enabled flag. Sort lowercase name then stable ID. No add/remove/search control. Disabled/closed apps remain listed.

M title Show sessions in the menu;22icon + name13 + native switch right; rows36; list viewport max216 (six rows), pane height viewport+52. Empty: Applications appear as sessions are discovered. Scrollbar vertical auto, no horizontal. Real icon from stored existing path else generic app. Switch tooltip Show sessions from <name>, accessible label name.

G icon24 + label + GtkSwitch, row spacing inherited from page10, full list no dedicated internal scroller. No equivalent Applications empty hint. Icon resolves from currently matching session Client; a closed known app can fall back to Unknown generic despite catalog icon_path. No explicit per-switch ATK naming beyond widget structure. Both send per-ID visibility change and persist; live discovered state remains unchanged. Ambient/request delivery scope differs; [D-01](../discrepancies.md#d-01).

<a id="settings-integrations"></a>
## Integrations and startup

Header Detected agents and hook setup. Each detected provider row: name, status, explanatory next step. Shared phases/status strings:

| Phase | Text | M presentation |
|---|---|---|
| Detected |Needs setup |secondary circle |
| Installing |Setting up… |secondary circular-arrows symbol |
| Installed Codex |No setup required |green filled check |
| Installed other |Hooks installed |green filled check |
| Failed |Needs attention |orange filled warning triangle |

M status starts12after measured provider name, status icon6after measured status; detail wraps11pt, tooltip also directory. Dynamic list max240high; pane viewport+88. No providers: No supported agents found. Install Codex, Claude Code, or Cline, then click Check & set up. Button disabled/title Setting up… while busy; notes support2lines with tooltip. M automatic resident startup checks/installs needed non-Codex integrations idempotently, then asks OS notification authorization; signatures/errors stored. Check & set up forces retry. Notification denial/failure note appears here. Translocated/disk-image app startup is blocked with **Move AI Agent Monitor to Applications** alert, OK; no install-through-DMG execution.

G startup calls `check_integrations(...,false)` (check only). User Check & set up runs installation. Rows use ✓ for Installed, ! for every other phase, secondary text rather than M color vocabulary; no no-provider explanation. During check busy button disabled; final note Setup complete. Restart agents that need hooks. Background worker reports aggregate after work rather than per-row Installing transitions. Codex direct connection/no hooks; Claude restart; Cline enable hooks then reopen/resume, approval stays in Cline.

Sources: [integrations domain](../../../src/domain/integrations.rs) `Row::status`; [M onboarding](../../../src/macos/onboarding.rs) `Setup::begin`, `installed`, `permission`; [G](../../../src/linux/settings.rs) `check_integrations`; [setup](../../../src/app/setup.rs) `discover`, `Integration::install`.

<a id="settings-telegram"></a>
## Telegram pane: state-specific structure

**SET-01 (S/T):** Ready/connected management requires `connected && paired && !setting_up`, not merely a valid token. Replacement remains in the same settings window. Change bot MUST keep current connection active until new candidate activation and credential/config commit succeed. Cancel replacement MUST retain the previous bot. Pairing is still implemented despite earlier discussion of trusting a personal bot.

### Connected management

Title Telegram is connected. Next row `@username`, fixed gap, Test, Open, vertically centered. Bottom left Change bot then Unlink bot. Test sends harmless bot connection notice; Open opens bot; Change enters setup; Unlink disconnects/removes credential/config, without a confirmation dialog. Status errors/test result below name when meaningful. M suppresses status strings starting@, uses height116 with no note/160with note; name width capped268,12gap then62Test+8gap+62Open. Test enabled by `status.enabled`. G horizontal8gap row, omits only exact routine `@username · Connected`; Test has no equivalent enabled binding. No separate Send questions checkbox.

### Initial/replacement setup

M pane494high, title15bold, support11, margins24/content412:

1. Set up Telegram integration; “Set up your personal Telegram bot in a few simple steps.”
2. Privacy paragraph: personal bot connects directly to Mac; no monitor servers, only Telegram. **No Why your own bot? heading.**
3. Already have a bot? Just paste its token below.
4. **1. Open BotFather and send /newbot.** with Open button. M attempts Telegram share URL containing /newbot; fallback HTTPS. G opens BotFather HTTPS without prefilled command.
5. **2. Choose your bot’s name and username**; suggested Bot name AI Agent Monitor (read-only selectable); suggested Bot username generated from full real name, transliterated Latin/stripped diacritics, lowercase ASCII, collapsed underscores, prefix `aiagmon_`, suffix `_bot`, length≤32. Blank fallback `my`; regenerate inserts4nonce chars before `_bot`. Copy controls; regenerate symbol with tooltip. G uses GLib real_name/to_ascii; name and username are editable, so generated values can differ by locale/transliteration.
6. **3. Copy the token from BotFather and paste it below.** “Find it in BotFather’s last message.” Secure single-line token input; Paste aligned with prior Copy. No realistic token example in current code. “Your bot token will be stored securely in macOS Keychain.” G keyring wording, token Entry max256; no name/username row freeze.
7. Status area for Checking token…, invalid/token/network errors or Start instructions; initially blank. Connect bottom left; optional Open bot after link exists; Cancel conditional. No additional setup window.

M token field width342, right Paste62, suggested name254 (ends at same x366), username220+regen28. Connect104×28 at x24; bottom Open bot132 at136; Cancel124 at312. `AutoOpen` consumes a changed start link once only after explicit Connect; Paste copies plain clipboard text and focuses field, while Connect clears the input immediately and the worker trims the token. Cmd+V supported through the installed main Edit menu's responder-chain key equivalents (`menu::start`). G asynchronous clipboard Paste, Connect/Cancel bottom row; no extra manual Open bot in setup, auto-open upon link change; input is not explicitly cleared on Connect. G privacy says computer instead of Mac. G page preferred-height recomputed, no fixed494.

M Cancel hidden when neither connected nor setting_up. For paired replacement it sends CancelSetup; otherwise Disconnect, which removes initial unpaired token. G Cancel always displayed and always CancelSetup; initial token/pairing may remain. M Open bot hidden until `status.link` exists; connected Open uses fallback username link when no start link. Text “Your current bot stays connected…” generated only for existing paired replacement state. Remaining pairing workflow/errors: [Telegram](../interactions/telegram.md#tg-setup).

<a id="settings-storage"></a>
## Defaults and persistence ledger

M NSUserDefaults: KnownApplications(JSON string), AnimateMenuBar(defaulttrue), MenuShortcut(optional encoded), LaunchAtLoginEnabled plus actual SMAppService status. Legacy domain migration copies missing values once but not OS notification authorization/first-launch results. Setup keeps IntegrationSetup.<agent>, error strings, notification error, FirstLaunchSetupCompleted. Tokens via Security.framework Keychain. Shared domain default loginDisabled, animationtrue, shortcutNone, empty catalog.

G preferences.json under state directory: applications,animation,shortcut; corrupt/missing file falls back to defaults. Login determined by existence of `$XDG_CONFIG_HOME/autostart/ai-agent-monitor.desktop` (default~/.config), created/removed by control. Token via `secret-tool` Secret Service, never JSON; missing/locked keyring yields explicit error. Telegram nonsecret config in state `telegram.json`. State home M `~/Library/Application Support/AI Agent Monitor`; G `$XDG_STATE_HOME/ai-agent-monitor` or `~/.local/state/ai-agent-monitor`. UI updates do not all wait for successful persistence; errors can leave optimistic state (G preference error shown; M app-catalog persistence has no reported failure result).

Sources: [M settings](../../../src/macos/settings.rs), [M token helpers](../../../src/macos/telegram_setup.rs), [G settings](../../../src/linux/settings.rs), [M Keychain](../../../src/macos/keychain.rs), [G credentials](../../../src/linux/credentials.rs), [domain preferences](../../../src/domain/preferences.rs).

Acceptance: empty and populated catalog; off app reopened; all four integration phases; first setup vs replacement; Cancel each before/after Connect; token invalid/webhook/conflict/keyring failure; new bot Start succeeds and old remains until then. Change panes while resizing and return without blank content. No token should appear in JSON, status text, clipboard except explicit user Copy/Paste, or command arguments. Native name suggestions and pairing are independent from actual BotFather naming operations.
