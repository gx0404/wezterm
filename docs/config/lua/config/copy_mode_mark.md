# `copy_mode_mark_bg` / `copy_mode_mark_fg`

{{since('nightly')}}

Fork addition: background/foreground colors of the copy-mode mark
(`m` sets a mark, `'` jumps to it, exchanging the cursor with the mark).
Both default to the copy-mode cursor colors when unset.

```lua
config.colors = {
  copy_mode_mark_bg = '#7aa2f7',
  copy_mode_mark_fg = '#1a1b26',
}
```
