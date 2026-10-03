---
tags:
  - appearance
  - char_select
  - color
---
# `char_select_border_color`

{{since('nightly')}}

Fork addition: specifies the color of the 1 pixel border drawn around the
[CharSelect](../keyassignment/CharSelect.md) box.

When unset (the default), the border uses
[char_select_bg_color](char_select_bg_color.md) and is therefore
invisible, matching the historical appearance.

The box has rounded corners with a fixed 6 pixel radius.

```lua
config.char_select_border_color = '#45475a'
```
