# `mouse_right_click_menu`

{{since('nightly')}}

Fork addition, default `true`.

When enabled, a plain right click in the terminal pane area opens the
pane context menu ([ShowPaneContextMenu](../keyassignment/ShowPaneContextMenu.md))
via the default mouse binding. When disabled, the default binding is not
registered at all and right clicks go to the pane's application (or
nowhere).

```lua
config.mouse_right_click_menu = false
```
