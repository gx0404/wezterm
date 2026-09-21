# `language`

{{since('nightly')}}

Fork addition: the interface language for the GUI (command palette,
menus, overlays, copy mode status line) and the localized CLI `--help`
output.

Possible values:

* `"zh-CN"` (the default in this fork)
* `"en"`

The [WEZTERM_LANG](../environment.md) environment variable takes
precedence over this option; the value can also be changed at runtime
from the settings overlay ([OpenSettings](../keyassignment/OpenSettings.md)),
which persists it to `gui-settings.json`.

```lua
config.language = 'zh-CN'
```
