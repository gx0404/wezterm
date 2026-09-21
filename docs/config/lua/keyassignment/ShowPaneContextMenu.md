# ``ShowPaneContextMenu``

{{since('nightly')}}

Opens the pane context menu (a fork addition): split right/down, toggle
pane zoom, copy, paste, scroll to top/bottom and close pane.

When triggered from a mouse binding the menu opens at the click position;
when triggered from the keyboard it opens centered in the window. The
default mouse binding (registered when
[mouse_right_click_menu](../config/mouse_right_click_menu.md) is enabled)
is a plain right click in the pane area:

```lua
config.mouse_bindings = {
  -- equivalent to the built-in default:
  {
    event = { Down = { streak = 1, button = 'Right' } },
    mods = 'NONE',
    mouse_reporting = false,
    region = 'Pane',
    action = wezterm.action.ShowPaneContextMenu,
  },
}
```

Because the menu is a regular mouse binding, your own `mouse_bindings`
and `bypass_mouse_reporting_modifiers` apply: with the terminal
application holding the mouse, a plain right click goes to the
application and Shift+right-click opens the menu.
