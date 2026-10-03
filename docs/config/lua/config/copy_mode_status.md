---
tags:
  - appearance
  - color
---
# `copy_mode_status_bg` / `copy_mode_status_fg`

{{since('nightly')}}

Fork addition: colors of the status line shown by
[CopyMode](../keyassignment/ActivateCopyMode.md) while searching
(`Search: ... (n/m matches ...)`) and by
[QuickSelect](../keyassignment/QuickSelect.md) (`Select: ...`).

When either color is set, the status line uses it; a color that is left
unset falls back to the terminal default. When neither color is set, the
status line keeps the historical reverse video style.

Like the other copy mode colors, the values are color specs that accept
either an `AnsiColor` name or a `Color` string:

```lua
config.colors = {
  copy_mode_status_bg = { Color = '#313244' },
  copy_mode_status_fg = { AnsiColor = 'Yellow' },
}
```
