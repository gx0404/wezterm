---
tags:
  - multiplexing
---
# `mux_synchronized_output_timeout_ms = 150`

{{since('nightly')}}

When an application enables synchronized output (`DECSET 2026`,
`ESC [ ? 2026 h`), wezterm holds back everything the application writes until
the matching `ESC [ ? 2026 l` arrives, so that a whole frame is presented at
once without tearing.

If the application crashes, hangs or is killed while a synchronized block is
open, the closing sequence never arrives.  This option bounds how long a block
may hold back output, in milliseconds: once the timeout elapses, wezterm
flushes the pending output as if the block had ended, logs a message at
`warn` level and processes subsequent output normally.

The default is `150` milliseconds, which is long enough for a frame from a
well-behaved TUI program.  Legitimate blocks that stay open for longer than
the timeout are presented in more than one piece.

Set it to `0` to disable the timeout and keep an open block held
indefinitely:

```lua
config.mux_synchronized_output_timeout_ms = 0
```
