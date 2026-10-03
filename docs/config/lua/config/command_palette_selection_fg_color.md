---
tags:
  - appearance
  - command_palette
  - color
---
# `command_palette_selection_fg_color`

{{since('nightly')}}

Fork addition: the text color of the selected row in the overlays that
share the command palette styling (command palette, settings, keybindings
and wallpaper overlays, context menus).

When unset (the default), the selected row uses
[command_palette_bg_color](command_palette_bg_color.md) as its text color,
which is the historical reverse video selection. See also
[command_palette_selection_bg_color](command_palette_selection_bg_color.md).

```lua
config.command_palette_selection_fg_color = '#cdd6f4'
```
