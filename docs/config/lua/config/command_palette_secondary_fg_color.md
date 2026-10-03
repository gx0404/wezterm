---
tags:
  - appearance
  - command_palette
  - color
---
# `command_palette_secondary_fg_color`

{{since('nightly')}}

Fork addition: the color of secondary text in the overlays that share the
command palette styling: footer hints, the inactive section tabs of the
settings overlay and the key labels shown next to commands.

When unset (the default), it is
[command_palette_fg_color](command_palette_fg_color.md) blended at 60%
over [command_palette_bg_color](command_palette_bg_color.md).

```lua
config.command_palette_secondary_fg_color = '#a6adc8'
```
