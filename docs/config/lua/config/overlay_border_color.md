---
tags:
  - appearance
  - command_palette
  - color
---
# `overlay_border_color`

{{since('nightly')}}

Fork addition: the color of the 1 pixel border drawn around the overlay
boxes that share the command palette styling (command palette, settings,
keybindings and wallpaper overlays, context menus), including the arcs
of their rounded corners (see
[overlay_corner_radius](overlay_corner_radius.md)).

When unset (the default), the border is
[command_palette_fg_color](command_palette_fg_color.md) blended at 12%
over [command_palette_bg_color](command_palette_bg_color.md): a subtle
outline that follows the palette colors. The same color is used for the
separator lines of the context menus.

```lua
config.overlay_border_color = '#45475a'
```
