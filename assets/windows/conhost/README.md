# Console Host

This directory contains a copy of built artifacts from the Microsoft
Terminal project which is provided by Microsoft under the terms
of the MIT license.

Why are they here?  At the time of writing, the conpty implementation
that ships with windows is lacking support for mouse reporting but
that support is available in the opensource project so it is desirable
to point to that so that we can enable mouse reporting in wezterm.

It looks like we'll eventually be able to drop this once Windows
and/or the build for the terminal project make some more progress.

https://github.com/wezterm/wezterm/issues/1927

## 当前版本与来源（gx0404 fork）

- 来源：微软官方 NuGet 包 `Microsoft.Windows.Console.ConPTY`，版本
  `1.24.261001001`（nuspec 声明：Authors=Microsoft，License=MIT，
  项目主页 https://github.com/microsoft/terminal）。
- 下载 URL：
  `https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/1.24.261001001/microsoft.windows.console.conpty.1.24.261001001.nupkg`
  （nupkg SHA256 `4D6AADDC1D2385C9F5897DF28F33879F699F8F2783315D5204CF3D8C3616AC5F`）。
- 包内路径（仅 x64）：
  - `build/native/runtimes/x64/OpenConsole.exe`
  - `runtimes/win-x64/native/conpty.dll`
- 两个文件均带微软 Authenticode 签名（签名状态 Valid），
  `FileVersion` 均为 `1.24.2610.01001`（ProductVersion `1.24.261001001`）。
- 替换日期：2026-10-03；此前为 `1.22.2502.04002`（自行从 ms-terminal
  仓 `bcz rel` 构建的产物）。
- 升级动机：规避 pwsh 退出全屏 TUI 时 conhost 触发 FailFast 导致终端
  闪退的上游问题（1.24 系列已修复；原计划版本 1.24.260402001 在 NuGet 上
  不存在，改用 1.24 系列最新稳定版；实机效果待验证）。

| 文件 | 大小（字节） | SHA256 |
|---|---|---|
| `OpenConsole.exe` | 1066808 | `6F8E68DEC4E8E5E15A54ECC8AFA127B0E62139EDC004FFBB4892125FF01D5737` |
| `conpty.dll` | 109920 | `5AFEA6C9480E2C7DFD195376DD10996B0AE25CDC6F014E6247FD8AEDC8C34852` |

## 升级注意事项

- `OpenConsole.exe` 与 `conpty.dll` 必须**成对**替换，且来自同一个包版本；
  版本错配会导致 conpty 握手失败或行为异常。
- `wezterm-gui/build.rs` 仅在目标文件缺失时才复制这两个文件。升级后本地
  构建前需先删除 `target/<profile>/conpty.dll` 与
  `target/<profile>/OpenConsole.exe`，否则仍会沿用旧副本。
- 运行时二者与 `wezterm-gui.exe` 同目录（平铺），由 `pty` crate 优先
  侧载 `conpty.dll`；打包由 `scripts/gx_package.py` 与
  `ci/windows-installer.iss` 引用，路径不变。
- 以前文档提到的 VC++ 运行时支持包（可能需要）：
  https://www.microsoft.com/en-us/download/details.aspx?id=53175

## 许可证

NuGet 包不含独立 LICENSE 文件，nuspec 声明 `license type="expression"`
为 `MIT`，版权为 `© Microsoft Corporation`。以下为 microsoft/terminal
仓库 LICENSE 全文：

```
Copyright (c) Microsoft Corporation. All rights reserved.

MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED *AS IS*, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
