---
tags:
  - appearance
  - scroll_bar
---
# `scroll_bar_thumb_width = "3pt"`

{{since('nightly')}}

Fork addition: controls the width of the scroll bar "thumb" shown when
[enable_scroll_bar](enable_scroll_bar.md) is `true`.

The thumb is drawn as a slim bar with rounded ends, placed near the right
edge of the right window padding, and is drawn in a lighter shade while
the mouse hovers over it or drags it. The area that responds to the mouse
still spans the whole width of the right padding.

The default of `"3pt"` is 4 pixels at 96 dpi and scales with the dpi of
the display.

The value can be a number to specify the number of pixels, or a string
with a unit suffix:

* `"4px"` - pixels
* `"3pt"` - points; there are `72` points in `1 inch`
* `"0.5cell"` - a fraction of the terminal cell width
* `"50%"` - a fraction of the width of the right padding; `"100%"` fills
  the padding, like the historical full-width thumb

The value must be greater than zero, and the thumb is never wider than
the right padding.

```lua
config.scroll_bar_thumb_width = '6px'
```
