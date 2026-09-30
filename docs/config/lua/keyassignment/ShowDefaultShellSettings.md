# ``ShowDefaultShellSettings``

{{since('nightly')}}

Opens the settings overlay (a fork addition) directly on its Shell section,
where the default shell is chosen. This is what the "Default Shell…" entries
of the main menu and of the tab bar context menu do.

The section lists the [launch_menu](../config/launch_menu.md) entries whose
`set_environment_variables` carry a `GX_SHELL_ID`; entries without it are not
offered. The check mark is on the saved choice while it is still offered,
otherwise on the `gx-zsh` entry, otherwise on the first one. Choosing an entry
writes its id to the `default_shell` key of `gui-settings.json` next to the
effective `wezterm.lua` (choosing `gx-zsh` removes the key) and reloads the
configuration; once both succeeded it emits the `gx-default-shell-changed`
window event, otherwise the overlay shows the error. The configuration itself
decides what the saved id means.

```lua
config.keys = {
  {
    key = 'd',
    mods = 'CTRL|SHIFT|ALT',
    action = wezterm.action.ShowDefaultShellSettings,
  },
}
```

See also [OpenSettings](OpenSettings.md) and [ShowMainMenu](ShowMainMenu.md).
