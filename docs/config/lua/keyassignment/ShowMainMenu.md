# ``ShowMainMenu``

{{since('nightly')}}

Opens the main menu overlay (a fork addition), anchored at the tab bar's
☰ button position: command palette, keybinding cheat sheet, settings,
reload configuration, hide/minimize window and quit.

```lua
config.keys = {
  { key = 'm', mods = 'CTRL|SHIFT', action = wezterm.action.ShowMainMenu },
}
```

See also [ShowKeybinds](ShowKeybinds.md), [OpenSettings](OpenSettings.md)
and [show_menu_button_in_tab_bar](../config/show_menu_button_in_tab_bar.md).
