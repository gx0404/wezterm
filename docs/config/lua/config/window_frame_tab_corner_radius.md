---
tags:
  - appearance
  - tab_bar
---
# `window_frame.tab_corner_radius = "0.5cell"`

{{since('nightly')}}

Fork addition to [window_frame](window_frame.md): the corner radius of
the tabs in the fancy tab bar (see
[use_fancy_tab_bar](use_fancy_tab_bar.md)). The top two corners of each
tab are rounded, or the bottom two when
[tab_bar_at_bottom](tab_bar_at_bottom.md) is `true`.

The value can be a number to specify the number of pixels, or a string
with a unit suffix such as `"8px"`, `"6pt"` or `"0.5cell"`. Pixel and
point values give circular corners; `cell` values follow the cell width
horizontally and the cell height vertically and give elliptical corners.

The default of `"0.5cell"` is the radius the fancy tab bar always used.
`"0px"` gives square corners; negative values are rejected.

```lua
config.window_frame = {
  tab_corner_radius = '8px',
}
```
