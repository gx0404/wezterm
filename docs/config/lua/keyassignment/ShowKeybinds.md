# ``ShowKeybinds``

{{since('nightly')}}

Opens the keybinding cheat sheet overlay (a fork addition): all commands
in command-palette grouping order together with their currently effective
key assignments (reflecting your custom `keys`). Read-only: ↑↓/j/k or the
mouse wheel to scroll, hover to highlight, Esc to close.

```lua
config.keys = {
  { key = '/', mods = 'CTRL|SHIFT', action = wezterm.action.ShowKeybinds },
}
```
