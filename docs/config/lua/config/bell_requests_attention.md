# `bell_requests_attention`

{{since('nightly')}}

Fork addition, default `false`.

When a pane rings the bell while its window is unfocused, request the
user's attention from the window manager: X11 urgency hint, a macOS
dock bounce, or on Windows a flashing taskbar button that stays highlighted
until the window is activated. Lua code can do the same explicitly via
`window:request_attention()`.

On Wayland, where no standard mechanism exists, enabling this option
currently has no effect.

```lua
config.bell_requests_attention = true
```

See also [bell_notification_handling](bell_notification_handling.md).
