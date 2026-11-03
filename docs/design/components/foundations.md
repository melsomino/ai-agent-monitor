# Visual foundations and component tokens

<a id="visual-units"></a>
## Units, references, and hierarchy

All measurements below are logical units: CSS px in H, AppKit pt in M, GTK logical px in G. They are not physical pixels and do not establish screenshot equality under scaling. H is the primary design reference; implemented deviations are preserved here. Native font metrics, OS materials and themes can change final rendering. Titles have greater weight than messages; metadata/version have lower contrast and smaller type. Request importance is primarily orange iconography, not an orange-filled form.

Sources: [H CSS](../../../design/previews/monitor-ui.html) `.aam-*`; [shared layout](../../../src/domain/layout.rs); [M menu](../../../src/macos/menu.rs) constants, `render_rows`, `action_button`; [G CSS](../../../src/linux/style.css), [style](../../../src/linux/style.rs), [widgets](../../../src/linux/widgets.rs).

<a id="visual-geometry"></a>
## Geometry ledger (S)

| Component/token | H | M | G |
|---|---|---|---|
| Outer width | Fluid ≤410 | Custom content392; list384 | Window408, inset surface392; list requests384 |
| Outer corners |18, explicit border/shadow | System menu outline/corners/shadow |18 Cairo clipped surface, shadow + 1px outline |
| Header |36 minimum |36 |36 minimum |
| List inset |6 left/right/top,10 bottom |top6, bottom8; viewport x4 |6/6/8 CSS |
| Group boundary |26 heading reserve, 6 gap before later groups |Same |Same |
| App heading |24 icon, secondary11/600 name, centered trailing separator; scrolls |24 icon x8, secondary11 semibold name, centered separator; scrolls |24 icon in heading box, secondary11/600 name and centered separator; scrolls |
| Robot art |14×11, top1 |14×11, custom optical offset |14×11 custom Cairo |
| Robot hit target |26×24, radius12 |Normalized28×28, radius14 |28×28 minimum,50% radius |
| Chevron hit target |28, margin0 3 |28×28 normalized |28×28 minimum |
| Title start |Derived from flex gutter; inset14 and markers |x40 in document |Robot28 + title padding6 (within list inset) |
| Title highlight |Measured button right +12; left29 |x32, measured title +42 reserved destination +12; height36 radius16 |Natural title button with6 horizontal padding,radius16;42 reserved destination |
| Expanded detail |padding2 top,12 bottom, left37 at default inset |width326,x40, top gap4 |Messages326 after34 gutter; card/meta start34,end12; top4,bottom8 |
| Request card |padding11,radius12,border1 |width326,padding11,radius12,border1 |padding11,radius12,border1 |
| Command |padding8 vertical/10 horizontal,radius8 |same padding/radius |same padding/radius |
| Action |min24 high,padding3/18,border1,pill |min24,caption+36+4 width,4 vertical inset; radius½height |min24,padding4/18,border1,radius16 |
| Ordinary actions gap |6, flex wrap |6; all fit one row else one column |6; preferred total≤302 and no details → row else column |
| Multiple-question groups |14 |14 |14 |
| Question→answers |10 |10 |10 in multi; box8 in single |
| Answer row |min27,padding3 vertical |min24+3 gap multi / +8 single |min21+6 padding; card layout applies |
| Custom input |27,radius6,padding4/7 |27 plus expanded focus decoration |min27,radius6,GTK padding |
| Metadata |row line-height1.4,gap1 |16 high +1 gap;12px symbol |min16+1gap;12px symbol |
| Footer |padding10/14/8 |24 per usage/filter row+12+32 utility strip |padding10/14/8,boxes6gap |

No absolute cross-platform pixel requirement is implied for system decorations or GTK theme-added borders/minima. Native widgets can have extra theme requisitions; review allocated bounds rather than CSS alone.

M ordinary request `action_button` initially uses background alpha0.5 (`base_alpha`); Deny/Cancel turn switch to0. Hover/focus paints full alpha. Consequently an RGB fill token is not proof that every resting action surface is opaque. Inline question buttons have their own decoration state. No custom universal pressed-state animation exists: M NSButton tracking, GTK theme active state and browser defaults apply. Disabled M captions use disabledControlTextColor; GTK native insensitive styling and HTML native disabled attributes are toolkit behavior, not a shared hardcoded opacity.

<a id="visual-type"></a>
## Typography and text treatment (S)

H: `-apple-system,BlinkMacSystemFont,Segoe UI,sans-serif`; product13/1.4; title13 weight600; message13/1.48; request13 weight500; action12 weight500/1.3; answers12; secondary/meta11; version10; code12/1.5 system monospace. H heading example `h3`14/1.4 weight600; paragraph gap8, message margin-bottom10.

M: system title13 weight0.3 (semibold-like native weight),22-high text frame centered optically using font metrics; messages13; questions13; actions12 weight0.23; descriptions/meta11; version10. Attributed Markdown uses3pt line spacing, headings16/15/14 for levels1/2/3, otherwise13; code12 monospaced. Insets account for native cell drawing bounds. See `center_label`, `RadioDecoration`, [markdown renderer](../../../src/macos/markdown.rs) `apply`.

G: base sans-serif13; title13 weight600; message13; question13 weight500; action12 weight500; answer12; metadata11; version10. Pango headings16/15/14 (all heading levels≥3 use14), code12; markup widths use WordChar. Bold/italic/list appearance comes from parser runs and Pango/GTK metrics, not a browser engine.

Long titles remain one line with truncation. Expanded prose wraps; commands use character wrapping when collapsed to measured lines. Text measurement must use the rendered font/content, not a guessed character count. H fixture strings are trusted static HTML, **not** evidence of a safe arbitrary Markdown renderer.

<a id="visual-colors"></a>
## Theme/color ledger

| Role | H light / dark | M light / dark | G light / dark |
|---|---|---|---|
| Primary |#202020 / #F1F1F1 |semantic label / label |theme_fg_color |
| Secondary |#686868 / #ADADAD |secondaryLabelColor |fg55% for secondary/meta |
| Tertiary |#747474 / #969696 |tertiaryLabelColor |version fg42% |
| Message |#414141 / #CFCFCF |same |same |
| Request text |primary |#202020 / #F1F1F1 |theme foreground |
| Popup surface |gradient #F6F6F6→#F1F1F1 / #373737→#303030 |system menu + windowBackgroundColor65% wash |opaque theme_bg_color |
| Footer |#E7E7E7 / #2C2C2C |same, opaque |same |
| Request card |#E7E7E7 / #373737 |same, opaque |same |
| Command |uses hover token black/white14% |#DADADA / #2A2A2A opaque |same as M |
| Working list |#168044 / #4BD177 |#007637 / #30D158 |#007637 / #30D158 |
| Request |#FF9500 / #FF9F0A |systemOrangeColor |Cairo(1,.584,0) |
| Finished |#0066BE / #73C7FF |same |same |
| Hover |black/white14% |black/white14% |theme fg6% |
| Action rest |white / #484848 |white / #484848 for opaque captions |theme_base_color |
| Opaque action hover |#E1E1E1 / #646464 |same |theme fg6% |
| Keyboard selection |local #0069DB/#0A84FF |controlAccentColor |theme_selected_bg_color |
| Focus ink |white |selectedMenuItemTextColor |theme_selected_fg_color |

H contains an `@supports AccentColor` declaration on the root, but `.aam-stage` defines its own `--aam-blue`. The stage-local token wins; do not claim verified system-accent parity. H field ring is #007AFF70/#409CFF85; M uses native keyboard-focus indicator and custom tracking-time decoration; G uses GTK entry styling.

Filled request/finished discs use dark request ink in H/M; G draws white foreground. M status viewport18 with disc15; G18 with disc16. This is a visual difference, not an instruction to change either.

<a id="visual-motion"></a>
## Motion and theme changes

| Motion | H | M | G |
|---|---|---|---|
| Disclosure & chevron |180ms ease, CSS grid0fr→1fr,rotate90° |240ms cubic ease-out, interrupted motion continues from sampled position |GTK Revealer240ms; chevron follows actual revealed fraction |
| Working status |Static green15px disc /18px viewport, solid6.72×8.32 play, centroid centered |Static green15pt disc /18pt viewport, same direct triangle geometry |Static green15px disc /18px viewport, same direct triangle geometry |
| Session robot |Fixed14×11 body; eight interpolated eye phases over2.4s |Same shared eye sampler and native draw |Same shared eye sampler and GTK draw |
| Indicator eyes |Static raster-mask logo, no eye sequence |8steps,150ms tick |same8levels,140ms tick |
| Reduced motion |Disclosure transitions and eye animation disabled |Menu-bar eyes and session robot disabled; disclosure lacks equivalent guard |Revealer and session robot follow GTK animations; panel eye loop lacks system-reduction guard |
| Theme |Light/Dark preview switch, initial media preference |Dynamic Aqua/Dark Aqua providers + native appearance |Theme name/prefer-dark notification reloads CSS and refreshes markup |

Acceptance: inspect both themes with long title, multi-line answer, nested command and last-row hover. Compare artwork size separately from target size. Test reduced-motion per row above; do not report parity if only the menu-eye preference was disabled.

<a id="visual-writing"></a>
## Writing and information hierarchy (I; retained design intent)

Production interface captions are English; agent-authored titles, messages, questions and answer options retain their original language. This is a writing convention, not a claim of a complete localization subsystem. The prototype uses English fixtures as documented in the [HTML supplement](../platforms/html.md#html-content); Unicode test data remains intentionally multilingual.

Prefer short, concrete action captions such as Open, Test, Copy, Paste, Connect, Change bot, Unlink bot and Send answer. Preserve the response alternatives projected by the adapter rather than imposing those generic captions on an agent request. Icon-only actions need contextual tooltips/accessibility names; truncated paths should retain their full tooltip. Keep useful messages, paths, branches and command context copyable where the native control supports it. Do not present internal response serialization or credentials as ordinary UI copy.

Errors should identify the failed operation and available next action; disabled/loading states must not imply success. Actual missing error feedback, accessibility semantics and secret-content exposure remain explicit in [request failures](requests.md#request-failures), [platform supplements](../README.md#spec-navigation) and the [discrepancy register](../discrepancies.md). Acceptance includes light/dark backgrounds, alternate accents, increased contrast, reduced transparency/motion, selectable text and accessible names; this is a manual review obligation, not certification from semantic colors alone.
