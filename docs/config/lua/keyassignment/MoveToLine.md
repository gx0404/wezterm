# ``MoveToLine``

{{since('nightly')}}

`MoveToLine(line)` (a fork addition) moves the copy-mode cursor to the
given scrollback line. Positive values count from the top of the
scrollback (0-based); negative values count back from the last row
(-1 = last row).

Intended for copy-mode key tables:

```lua
config.key_tables = {
  copy_mode = {
    { key = 'g', mods = 'NONE', action = wezterm.action.MoveToLine(0) },
    { key = 'G', mods = 'SHIFT', action = wezterm.action.MoveToLine(-1) },
  },
}
```

See also the fork-added paragraph motions `MoveToStartOfParagraph` /
`MoveToEndOfParagraph` (`{` / `}` in the default copy-mode table).
