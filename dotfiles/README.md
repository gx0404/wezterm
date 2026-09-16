# dotfiles/：本机 wezterm 环境快照与一键安装

本目录是 gx0404 机器 wezterm 用户环境的**仓库真源快照**（首次收录 2026-09-16，
溯源见 [PROVENANCE.md](PROVENANCE.md)），配合 `make gx-bundle / gx-install / gx-sync`
实现向全新 Ubuntu 20.04/24.04 或 Windows 机器的无缝移植。

## 目录结构

| 路径 | 内容 | 安装目标（Linux） |
|---|---|---|
| `wezterm-config/` | `~/.config/wezterm` 完整工作区快照（含壁纸、事件脚本） | `~/.config/wezterm/` |
| `plugins/` | 4 个插件快照（目录名为 wezterm 插件加载器的转义格式；各插件 `.git` 以 `gitdir/` 入库，安装时还原——`wezterm.plugin.list()` 要求插件目录是带 remote 的 git 仓库） | `~/.local/share/wezterm/plugins/` |
| `fonts/` | JetBrainsMono Nerd Font 6 字重 + Noto Sans CJK Regular/Bold | `~/.local/share/fonts/wezterm-gx/` |
| `templates/` | desktop entry / wrapper / zshrc 片段模板（占位符渲染） | 见 install.sh |
| `assets/` | 图标 | `~/.local/share/icons/wezterm-gx/` |
| `install.sh` | Linux 用户级安装器（幂等、无 sudo、`--check` 干跑） | — |
| `install.ps1` | Windows 用户级部署脚本 | `%LOCALAPPDATA%`、`%APPDATA%` |

Windows 注意：配置装到 `%USERPROFILE%\.config\wezterm`（源码确认与 Linux 同一查找
序）；插件装到 `%APPDATA%\wezterm\plugins`（Windows 数据目录是 Roaming AppData）。

## 三种用法

**1. 离线 bundle（全新 Ubuntu 工控机，无需网络/工具链）**

```bash
# 本机（或任何有 docker 的机器）打包：
make gx-bundle                      # 产物在 dist/wezterm-gx-*-linux-amd64.tar.xz
# 传输后目标机解压执行：
tar xf wezterm-gx-*-linux-amd64.tar.xz && cd wezterm-gx-*
./install.sh --check                # 干跑预检
./install.sh                        # 正式安装
```

**2. 源码构建（目标机有网 + sudo）**

```bash
git clone -b feature/gx_wezterm https://github.com/gx0404/wezterm.git
cd wezterm && make gx-install       # get-deps(需 sudo) → release 构建 → 部署
```

**3. Windows**

分支推送后在 GitHub Actions 手动触发 `gx-windows-build` workflow 取得
`wezterm-windows-*.zip`，与 `dotfiles/` 放同一目录后：

```powershell
powershell -ExecutionPolicy Bypass -File dotfiles\install.ps1 -ZipPath <zip>
```

## 日常维护（改配置回收入分支）

本机 `~/.config/wezterm` 保持独立（未 symlink 到仓库）。在本机改完配置后：

```bash
make gx-sync                        # 只读对比，列出差异
GX_SYNC_WRITE=1 make gx-sync        # 把本机改动收回 dotfiles/
```

收录时提交进仓库的插件/字体不随 sync 变动；升级插件属于重新快照，
须更新 PROVENANCE.md 的 pin 记录。

## 已知边界

- `config/bindings.lua` 的 Alt+Shift+V 依赖 `~/.local/bin/ai-image-paste`，
  快照机器上该脚本本就缺失，触发时显示失败 toast（行为与本机一致）。
- 配置假定输入法为 fcitx（`xim_im_name='fcitx'` + desktop entry 注入 fcitx 环境
  变量）；用 ibus 的机器装完后可 `./install.sh --im ibus` 重装 desktop entry，
  并自行调整 general.lua。
- tabline.wez 插件目录含一段未上游化的 cpu.lua 本地补丁（见 PROVENANCE.md），
  重新从上游拉取会丢失，以本快照为准。
