# `bell_notification_handling`

{{since('nightly')}}

Fork addition: controls when a pane's bell produces a notification side
effect (audible bell, visual bell, attention request). The `bell` Lua
event is emitted regardless of this setting.

Possible values:

* `"AlwaysShow"` - bells from any pane are shown (the default)
* `"NeverShow"` - never show bell side effects
* `"SuppressFromFocusedWindow"` - suppress while the bell's window is focused
* `"SuppressFromFocusedTab"` - suppress while the bell's tab is the active tab
* `"SuppressFromFocusedPane"` - suppress while the bell's pane is the active pane

```lua
config.bell_notification_handling = 'SuppressFromFocusedPane'
```

See also [bell_requests_attention](bell_requests_attention.md),
[bell_cooldown_ms](bell_cooldown_ms.md) and
[audible_bell](audible_bell.md).
