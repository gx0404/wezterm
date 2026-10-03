# `bell_requests_attention`

{{since('nightly')}}

Fork addition, default `false`.

When a pane rings the bell while its window is unfocused, request the
user's attention from the window manager: X11 urgency hint, or a macOS
dock bounce. Lua code can do the same explicitly via
`window:request_attention()`.

Windows (taskbar flashing) is not implemented yet and is planned: on Windows,
and on Wayland where no standard mechanism exists, enabling this option
currently has no effect.

```lua
config.bell_requests_attention = true
```

See also [bell_notification_handling](bell_notification_handling.md).
