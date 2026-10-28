# Menu bar icon

`robots-template.pdf` is the selected vector template icon: two outlined robot
heads, circular eyes, external ears, no mouths. Outline thickness: 1.6 pt at the sides, 3 pt at the top and bottom.
Canvas: 30 × 22 pt. Load with `NSImage.isTemplate = true` for native appearance tinting.

`robots-template.png` is a transparent 4× preview. `preview.png` shows enlarged and
actual-size versions on light and dark backgrounds. The application embeds the PDF and uses it as its menu bar template image.

Regenerate on macOS:

```sh
swift assets/menubar/generate.swift
```

The Swift script is an asset generator, not an application dependency.

The `eyes-left.pdf`, `eyes-both.pdf`, and `eyes-right.pdf` template masks share the
base icon's geometry and clipping. The application overlays them in system green
for the eight-step work animation with independent 0%, 50% and 100% brightness, with no rotation
or separate activity badge. `preview-working.png` shows the sequence.

`robots-working.pdf` has no normal eyes while working. Green eye masks
remain round. The idle icon retains round eyes.
