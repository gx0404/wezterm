# `bell_cooldown_ms`

{{since('nightly')}}

Fork addition, default `100` (milliseconds); `0` disables throttling.

Per-pane bell throttle: after a pane's bell is let through, further
bells from the same pane are suppressed for this long, so bell storms
from busy programs collapse into a one-bell-per-window cadence. The
throttle window only opens when a bell is let through — it is not a
debounce, so a sustained bell stream remains audible at the cadence
instead of being muted forever.

```lua
config.bell_cooldown_ms = 250
```
