# ``PipeSelection``

{{since('nightly')}}

`PipeSelection(cmd)` (a fork addition) pipes the current selection text
to the standard input of the command described by the
[SpawnCommand](../SpawnCommand.md) `cmd`. With no active selection it
pipes the word at the copy-mode cursor.

The command runs on a detached helper thread with a bounded write and a
5-second wait timeout, so a child that never reads its stdin cannot
freeze the GUI.

```lua
config.keys = {
  {
    key = 's',
    mods = 'CTRL|SHIFT|ALT',
    action = wezterm.action.PipeSelection {
      args = { 'xclip', '-in', '-selection', 'clipboard' },
    },
  },
}
```
