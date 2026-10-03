---
tags:
  - appearance
---
# `win32_frame_follow_colors = false`

{{since('nightly')}}

Fork addition, default `false`.

When enabled on Windows 11 (build 22000 and later), the native window frame
drawn by the Desktop Window Manager follows the colors from
[window_frame](window_frame.md) instead of the system theme:

* the title bar background uses `active_titlebar_bg` while the window is
  focused and `inactive_titlebar_bg` otherwise;
* the title text uses `active_titlebar_fg` / `inactive_titlebar_fg`;
* the one pixel window border uses `border_top_color`; when that is not set
  the system default border color is kept;
* the light/dark style of the caption buttons follows the brightness of
  `active_titlebar_bg` rather than the system light/dark setting, so the
  buttons stay readable on the configured title bar color;
* windows without a native title bar (`window_decorations` of `"NONE"`,
  `"RESIZE"` or anything including `"INTEGRATED_BUTTONS"`) ask for rounded
  corners.

The colors are re-applied when the configuration is reloaded and when the
window gains or loses focus. Turning the option off again restores the
system defaults.

On Windows 10 and on other platforms this option has no effect.

```lua
config.win32_frame_follow_colors = true
config.window_frame = {
  active_titlebar_bg = '#1e1e2e',
  inactive_titlebar_bg = '#181825',
  active_titlebar_fg = '#cdd6f4',
  inactive_titlebar_fg = '#6c7086',
  border_top_color = '#45475a',
}
```
