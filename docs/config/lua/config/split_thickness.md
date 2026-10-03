---
tags:
  - appearance
---
# `split_thickness`

{{since('nightly')}}

Fork addition: controls the thickness of the line drawn between split
panes. Its color is set by `split` in the
[colors](../../appearance.md#defining-your-own-colors) section.

When unset (the default), the line uses the underline thickness (see
[underline_thickness](underline_thickness.md)), as it historically did.
Setting this option decouples the split line from the underline.

The value can be a number to specify the number of pixels, or a string
with a unit suffix such as `"2px"`, `"1pt"` or `"0.1cell"`. A `cell` or
`%` value is relative to the cell width for a line between left and right
panes, and to the cell height for a line between top and bottom panes.
The value must be greater than zero, and the line is never thicker than
one cell. The line stays centered where the default line is drawn.

```lua
config.split_thickness = 2
```
