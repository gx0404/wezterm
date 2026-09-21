# ``OpenSettings``

{{since('nightly')}}

Opens the settings overlay (a fork addition): four sections — Language
(中文/English), Appearance (1001 built-in color schemes with fuzzy
filtering, live preview on move/hover, Enter to apply, Esc to revert),
Interaction (toggles for the right-click menu, scroll bar, audible bell
and close confirmation) and Font (font size step/reset).

Applying a value writes `gui-settings.json` next to the effective
`wezterm.lua` and reloads the configuration so it applies globally and
survives restarts. Previews are window-local and volatile; they are
reverted when the overlay is dismissed by any path.

```lua
config.keys = {
  { key = ',', mods = 'CTRL|SHIFT', action = wezterm.action.OpenSettings },
}
```
