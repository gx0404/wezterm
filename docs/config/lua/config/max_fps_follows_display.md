---
tags:
  - tuning
---
# `max_fps_follows_display = false`

{{since('nightly')}}

Fork addition, default `false`.

When enabled, the frame rate cap used to pace repaints follows the refresh
rate of the monitor that the window is currently on, instead of the fixed
[max_fps](max_fps.md) value. A window on a 165 Hz display is paced at
165 frames per second, and a window dragged onto a 60 Hz display is paced at
60 frames per second; the value is re-read when the window moves between
monitors and when the display mode changes.

[max_fps](max_fps.md) is still used as the fallback whenever the refresh
rate cannot be determined (for example in some remote desktop sessions, or
when the driver reports a placeholder rate of 0 or 1 Hz). Whatever the
source, the effective rate is clamped to the range `1..=1000`.

This option is currently implemented on Windows; on other platforms it has
no effect and [max_fps](max_fps.md) applies as before.

```lua
config.max_fps_follows_display = true
```
