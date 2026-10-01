---
tags:
  - clipboard
---

# `clipboard_image_paste`

{{since('nightly')}}

Fork addition, default `"Inline"`.

Controls what the `Paste` action does when the clipboard holds an image
rather than (only) text:

|value|meaning|
|-----|-------|
|`"Inline"`|The image is decoded (PNG as-is; `CF_DIB`/`CF_DIBV5` transcoded to PNG, downsampled when the longest edge exceeds 2048 pixels) and inserted into the pane as an inline image ([imgcat](../../../cli/imgcat.md)-style OSC 1337), without writing anything to the program's input|
|`"Path"`|The image is written to a temporary PNG file and its file path is pasted as text|
|`"None"`|The clipboard is never treated as an image; pasting always sends text|

When no image is available, the image cannot be decoded, or the mode is
`"None"`, pasting falls back to the historical text behavior.

Reading image data from the clipboard is currently implemented on
Windows; on other platforms this option has no effect and pasting is
text-only.

```lua
config.clipboard_image_paste = "Path"
```
