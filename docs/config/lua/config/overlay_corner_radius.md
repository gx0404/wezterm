---
tags:
  - appearance
  - command_palette
---
# `overlay_corner_radius = "0.25cell"`

{{since('nightly')}}

Fork addition: the corner radius of the overlay boxes that share the
command palette styling: the
[command palette](../keyassignment/ActivateCommandPalette.md), the
settings, keybindings and wallpaper overlays, and the context menus.

The 1 pixel border of those boxes (see
[overlay_border_color](overlay_border_color.md)) follows the rounded
corners as a thin arc. The selected row is rounded with half this radius.

The value can be a number to specify the number of pixels, or a string
with a unit suffix:

* `"8px"` - pixels; gives a true circular corner
* `"6pt"` - points; there are `72` points in `1 inch`
* `"0.25cell"` - a fraction of the cell width horizontally and of the
  cell height vertically, which gives an elliptical corner

The default of `"0.25cell"` is the radius the command palette always had.
`"0px"` gives square corners; negative values are rejected.

```lua
config.overlay_corner_radius = '8px'
```
