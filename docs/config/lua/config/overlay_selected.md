---
tags:
  - appearance
  - color
---
# `overlay_selected_bg` / `overlay_selected_fg`

{{since('nightly')}}

Fork addition: colors of the selected row in the text overlays — the
[Launcher](../keyassignment/ShowLauncherArgs.md),
[InputSelector](../keyassignment/InputSelector.md) and the
[Confirmation](../keyassignment/Confirmation.md) buttons.

When either color is set, the selected row is painted with the
`overlay_selected_bg` background across the full width of the overlay,
and a `▌` accent bar in the `overlay_selected_fg` color is drawn at its
left edge. A color that is left unset falls back to the terminal default.

When neither color is set, the selected row keeps the historical
reverse video style.

These overlays also render their title line in bold, followed by a
dimmed separator line.

```lua
config.colors = {
  overlay_selected_bg = '#313244',
  overlay_selected_fg = '#89b4fa',
}
```
