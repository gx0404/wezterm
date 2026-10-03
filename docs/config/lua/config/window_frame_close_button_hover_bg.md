---
tags:
  - appearance
  - tab_bar
  - color
---
# `window_frame.close_button_hover_bg`

{{since('nightly')}}

Fork addition to [window_frame](window_frame.md): the background of the
integrated close button while the mouse hovers over it, when
[window_decorations](window_decorations.md) includes `INTEGRATED_BUTTONS`
and [integrated_title_button_style](integrated_title_button_style.md) is
`"Windows"`.

When unset (the default), it is `#c42b1c`, the close button color of
Windows 11. The cross is drawn in white while hovered.

```lua
config.window_frame = {
  close_button_hover_bg = '#e81123',
}
```
