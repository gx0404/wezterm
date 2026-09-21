# `bell_requests_attention`

{{since('nightly')}}

Fork addition, default `true`.

When a pane rings the bell while its window is unfocused, request the
user's attention from the window manager: X11 urgency hint, or a macOS
dock bounce. Lua code can do the same explicitly via
`window:request_attention()`.

```lua
config.bell_requests_attention = false
```

See also [bell_notification_handling](bell_notification_handling.md).
