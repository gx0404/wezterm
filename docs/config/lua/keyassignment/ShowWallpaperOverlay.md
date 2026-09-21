# ``ShowWallpaperOverlay``

{{since('nightly')}}

Opens the wallpaper manager overlay (a fork addition): lists the images
in the `backdrops/` directory next to your config (file name, pixel
dimensions, file size, and a ✓ marker on the current one).

* ↑↓/j/k/n/p or hover: move and **live-preview** (swaps only the window
  background layers, no config reload)
* `Enter`: apply and persist to `gui-settings.json` (survives restarts;
  the lua `backdrops` module reads the choice back at startup)
* `r`: random preview
* `a`: add — a path input (Tab completion, `~` expansion, paste)
  validated as a decodable image and copied into the backdrops directory
* `d`: delete (requires a `y` confirmation; only files inside the
  backdrops directory can be removed)
* `Esc` or clicking outside: discard unconfirmed previews

```lua
config.keys = {
  { key = 'w', mods = 'LEADER', action = wezterm.action.ShowWallpaperOverlay },
}
```
