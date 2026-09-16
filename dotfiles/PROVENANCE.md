# 快照溯源（PROVENANCE）

首次收录：2026-09-16，来源机器 = 本仓库日常开发机（Ubuntu 22.04, x86_64）。
收录命令：`rsync -a --exclude='.git'`（保留未提交工作区状态，剔除嵌套 .git）。

## wezterm-config/

- 来源：`~/.config/wezterm`（本身是 git 仓库：origin
  `https://github.com/gx0404/wezterm.git`，branch master，HEAD `9b60228`，共 2 个
  commit；基于 [KevinSilvester/wezterm-config](https://github.com/KevinSilvester/wezterm-config)
  模板二次修改）。
- 快照取的是**工作区现状**（当时有 11 个已修改文件 + 未跟踪 `backups/`、
  `config/appearance.lua.bak`、`events/status.lua`——status.lua 是生效中的状态栏
  核心，未纳入原仓库版本控制；快照全部包含）。
- 收录时相对本机工作区的**有意改动**（仅改仓库副本，本机 `~/.config/wezterm`
  未动，因此 `make gx-sync` 会持续显示这两处差异）：
  - `config/launch.lua`：Git Bash 路径 `C:\Users\kevin\scoop\...` 改为
    `wezterm.home_dir` 动态拼接；
  - `config/domains.lua`：WSL 域写死的 `username='kevin'` / `/home/kevin` 改为
    `os.getenv('USERNAME')` 动态取当前账户；
  - `config/general.lua`（2026-09-16，R1）：新增 `language = 'zh-CN'`
    （本 fork 的界面文案语言配置项，见根 CHANGELOG 0.2.0；本机尚未同步）。
- 本机存在 `~/.config/wezterm.bak-20260713`（38MB 旧快照），未收录。
- 2026-09-16 增量同步：`config/bindings.lua` 恢复 Linux 壁纸控制的
  `Alt+.` / `Alt+,` / `Alt+/` / `Ctrl+Alt+/` / `Alt+b`，与本机配置及
  Oh My Zsh 仓库 `gx/wezterm/` 同步；常用终端功能继续使用 `Ctrl+Shift`。

## plugins/

目录名 = wezterm 插件加载器转义格式（`lua-api-crates/plugin/src/lib.rs::compute_repo_dir`：
`/`→`sZs`、`:`→`sCs`、`.`→`sDs`）。各插件的 `.git` 以 `gitdir/` 名义随快照入库
（避免嵌套 git 仓库），`install.sh` 部署时还原为 `.git`——`wezterm.plugin.list()`
经 `load_from_dir` 要求每个插件目录是带 remote 的合法 git 仓库，缺 `.git` 会导致
整条 `config/plugins.lua` require 链报错。插件机制只 clone 一次、不自动 pull。
**升级插件 = 重新快照（含 gitdir/）并更新本表。**

| 快照目录 | 上游 | pin（HEAD） | 本地改动 |
|---|---|---|---|
| `httpssCssZssZsgithubsDscomsZschrisgvesZsdevsDswezterm` | github.com/chrisgve/dev.wezterm | `1b5d9e0` | 无（当前配置未引用，闲置） |
| `httpssCssZssZsgithubsDscomsZsmichaelbrusegardsZstablinesDswez` | github.com/michaelbrusegard/tabline.wez | `5e148f0`（v1.6.0-14） | `plugin/tabline/components/window/cpu.lua` 有未上游化本地补丁（+27/-1）；当前配置未引用 |
| `httpssCssZssZsgithubsDscomsZsMLFlexersZsresurrectsDswezterm` | github.com/MLFlexer/resurrect.wezterm | `47ce553`（v1.0.0-254） | 无；state/ 为空会话，无历史数据 |
| `httpssCssZssZsgithubsDscomsZsMLFlexersZssmart_workspace_switchersDswezterm` | github.com/MLFlexer/smart_workspace_switcher.wezterm | `40228a0`（1.2.0-18） | 无 |

## fonts/

全量字体 363MB 不入库，按配置实际引用精选（`config/fonts.lua`：主字体
JetBrainsMono Nerd Font DemiBold + 回退 Noto Sans CJK SC Bold；fontconfig 中
DemiBold 权重由 SemiBold 命名文件提供）：

| 文件 | 来源 |
|---|---|
| `JetBrainsMonoNerdFont-{Regular,Bold,Italic,BoldItalic,SemiBold,SemiBoldItalic}.ttf` | `~/.local/share/fonts/JetBrainsMonoNerd/`（用户手动安装的 Nerd Fonts） |
| `NotoSansCJK-{Regular,Bold}.ttc` | `/usr/share/fonts/opentype/noto/`（Ubuntu `fonts-noto-cjk` 包） |

## assets/ 与 templates/

- `assets/org.wezfurlong.wezterm.png`：取自本机 AppImage
  `~/.local/opt/wezterm-nightly/squashfs-root/usr/share/icons/hicolor/128x128/apps/`。
- `templates/org.wezfurlong.wezterm.desktop.template`：基于本机
  `~/.local/share/applications/org.wezfurlong.wezterm.desktop`（保留 fcitx 环境变量
  与 `start --cwd .`），绝对路径改占位符。
- `templates/wezterm-wrapper.sh.template`：基于本机 `~/.local/bin/wezterm`。
- `templates/zshrc-wezterm.sh`：本机 `~/.zshrc` 的光标模式键位段，补全
  autoload/zle 定义后改为带标记自包含块。

## 不收录项

- `~/.cache/wezterm/`（64M 运行时缓存，可再生）。
- `~/.local/share/wezterm/check_update`（GitHub release 元数据缓存）。
- resurrect 已保存会话（当时为空）。
- 快照机器当时在用的官方 nightly AppImage 二进制（`~/.local/opt/wezterm-nightly/`）：
  目标机改用本分支自建二进制。
