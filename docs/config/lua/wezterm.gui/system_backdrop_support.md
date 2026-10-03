# `wezterm.gui.system_backdrop_support()`

{{since('nightly')}}

Returns which of the system backdrop materials that can be selected with
[win32_system_backdrop](../config/win32_system_backdrop.md) can actually be
rendered on this system.

A material that the system doesn't support is silently ignored: for example,
asking for `"Mica"` on Windows 10 together with a reduced
[window_background_opacity](../config/window_background_opacity.md) leaves a
see-through window with no blur behind it. Checking this first lets your
configuration choose a different look instead.

The following example was typed into the [Debug
Overlay](../keyassignment/ShowDebugOverlay.md) (by default: press
`CTRL-SHIFT-L`) on Windows 11:

```
> wezterm.gui.system_backdrop_support()
{
    "acrylic": true,
    "mica": true,
    "tabbed": true,
}
```

The return value is a table with the following boolean keys:

* `mica` - `true` if `win32_system_backdrop = "Mica"` is supported
* `acrylic` - `true` if `win32_system_backdrop = "Acrylic"` is supported
* `tabbed` - `true` if `win32_system_backdrop = "Tabbed"` is supported

The answer depends on the version of the operating system:

* Windows 11 build 22621 (22H2) and later: all three are supported.
* Windows 11 build 22000 (21H2): `mica` and `acrylic` are supported.
* Windows 10 version 1803 (build 17134) and later: only `acrylic` is supported.
* Earlier versions of Windows: none are supported.
* Other operating systems: all three are `false`, as `win32_system_backdrop`
  has no effect there.

This function doesn't depend on any window having been created, so it is safe
to call while the configuration is being evaluated.

This example uses *Mica* where it is supported, and otherwise keeps an opaque
window:

```lua
local wezterm = require 'wezterm'
local config = wezterm.config_builder()

-- wezterm.gui is not available to the mux server
local gui = wezterm.gui
if gui and gui.system_backdrop_support().mica then
  config.win32_system_backdrop = 'Mica'
  config.window_background_opacity = 0
end

return config
```
