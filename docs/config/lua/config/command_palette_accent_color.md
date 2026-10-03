---
tags:
  - appearance
  - command_palette
  - color
---
# `command_palette_accent_color`

{{since('nightly')}}

Fork addition: the accent color of the overlays that share the command
palette styling. It paints the 2 pixel bar at the left edge of the
selected row and the underline of the active section tab in the settings
overlay.

When unset (the default), it is the cursor background color
(`cursor_bg`) of the active color scheme.

```lua
config.command_palette_accent_color = '#89b4fa'
```
