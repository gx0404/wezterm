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
    （本 fork 的界面文案语言配置项，见根 CHANGELOG 0.2.0；**已于 2026-09-20
    的 gx-upgrade 部署到本机**，不再是 sync 差异项）。
- 2026-09-21 更正（WEZ-HYG-03）：早前记录的「本机存在
  `~/.config/wezterm.bak-20260713`（38MB 旧快照）」已不存在，删除该条；
  字体段落同步为现状（正文 Regular 字重，见下）。
- 2026-09-16 增量同步：`config/bindings.lua` 恢复 Linux 壁纸控制的
  `Alt+.` / `Alt+,` / `Alt+/` / `Ctrl+Alt+/` / `Alt+b`，与本机配置及
  Oh My Zsh 仓库 `gx/wezterm/` 同步；常用终端功能继续使用 `Ctrl+Shift`。
- 2026-09-21 GX-10：`config/bindings.lua` 壁纸五键（随机/上一张/下一张/
  选择器/专注模式）迁入 leader 层（`Ctrl+Shift+Space` 前缀），分屏 `Alt+\`
  系并入 `Ctrl+Shift(+Alt)+\`，标签直达 `Alt+1..9` 改 `Leader 1..9`，
  `Alt+w` 关闭 pane 改 `confirm=true`——裸 Alt 组合抢走 readline 标准键
  （`Alt+.` 末参数、`Alt+b` 退词）且无确认销毁 pane。
- 2026-09-21 批 8（WZ-16/WEZ-UX-01）：`config/bindings.lua` leader 层新增
  浮层入口三键 `Leader m` 主菜单 / `Leader s` 设置 / `Leader k` 快捷键
  速查（fork 的 `disable_default_key_bindings=true` 使内建默认键
  `Ctrl+Shift+M/,//` 不生效，需显式绑定）。
- 2026-09-21 批 12（WEZ-CFG-04）：`config/bindings.lua` 的
  `Shift+PageUp/Down` 改为 alt-screen 感知——alt screen 应用（herdr/
  Claude Code/vim）里透传 `\x1b[5;2~`/`\x1b[6;2~`，普通 scrollback
  下照旧 `ScrollByPage`；原先宿主无条件滚动，alt screen 下静默空操作。
- 2026-09-21 批 12（WEZ-HYG-01）：快照清理——删除两个死模块
  （`events/left-status.lua`、`utils/gpu-adapter.lua`，全树无 require）、
  `backups/` 目录（4 个 2026-07 的 pre-flicker .bak）与
  `config/appearance.lua.bak`；这些死代码/备份此前随每次安装分发。
- 2026-09-21 批 13：壁纸管理浮层键位 `Leader w`（`ShowWallpaperOverlay`）；
  `utils/backdrops.lua` 新增 `set_default_from_sidecar()`（启动/重载时
  按 `gui-settings.json` 的 `wallpaper` 键覆盖默认壁纸），wezterm.lua
  在 `set_default` 后链式调用。
- 2026-09-28 GX Shell 单仓：新增 `utils/gx-shell.lua`，`config/launch.lua`
  在 GX Shell 安装包内运行时默认进入包内 GX Zsh，并把「GX Zsh」「herdr」放在
  启动菜单最前面；独立安装不受影响。仅改仓库副本，本机 `~/.config/wezterm`
  未同步，`make gx-sync` 会显示这两处差异。
- 2026-09-30 GX Shell 0.2.0：`backdrops/` 16 张壁纸从原图等比缩到不超过
  1920×1080（不放大，文件名与格式不变），并去掉元数据。壁纸与字形共用 GPU 纹理
  atlas，图片长边超过 2046 像素就要 4096²、超过 4094 像素要 8192²：原图
  （2560×1600～5760×3630）都要 4096² 以上，angry-samurai、final-showdown、
  frieren、voyage 要 8192²；缩后全部（含默认的 nord-space.png）只需 2048²。
  总量从 19,390,659 字节降到 6,256,115 字节。做法：Pillow 12.3 Lanczos 缩放；
  JPEG 用 baseline、优化 Huffman 表、质量 85（源图是标准量化表且质量低于 85 的
  沿用源质量：cherry-lava、house 为 75，sunset 为 80），色度采样与源图一致；
  PNG 无损，zlib 9 级。JPEG 只留 JFIF 头，PNG 只留 IHDR/IDAT/IEND。去掉的是
  ffmpeg 注释和 nord-space.png 的 GIMP 内置 sRGB ICC、EXIF、XMP 等块，都不含
  作者或版权信息；画面里的署名和水印保持原样。原图在提交 `b0f5d696e` 及更早的
  历史里。Oh My Zsh `gx/wezterm/backdrops/` 与本目录逐字节相同。本机
  `~/.config/wezterm/backdrops` 还是原图时，`make gx-sync` 会列出这 16 个文件：
  应把仓库版部署到本机，不要用 `GX_SYNC_WRITE=1` 回收，否则会把原图写回仓库。
- 2026-09-30 GX Shell 0.2.0（Lua 配置）：以下都是仓库侧的有意改动，本机
  `~/.config/wezterm` 未同步。本机还是旧配置时，`make gx-sync` 会把这些文件列为
  CHANGED（新增的三个 `utils/*.lua` 列为 repo only），`make framework-check` 因此失败：
  应先 `make gx-upgrade` 把仓库版部署到本机，不要用 `GX_SYNC_WRITE=1` 回收，否则旧配置
  会覆盖这些改动。
  - 默认 Shell：新增 `utils/shells.lua`（只用 `io.open` 探测已安装的 Shell，生成
    `launch_menu` / `default_prog` / `default_domain` 与 herdr 同步所需信息）、
    `utils/gui-settings.lua`（按 WezTerm 的规则定位设置页写的 `gui-settings.json`）、
    `utils/wsl.lua`（`wezterm.default_wsl_domains()` 结果存 `wezterm.GLOBAL`：找到发行版后
    每个 GUI 进程只跑一次 `wsl.exe`，空结果或失败 5 分钟后重试）。`config/launch.lua` 改为
    调用它们，默认 Shell 取 `gui-settings.json` 的 `default_shell`，`gx-default-shell-changed`
    事件让安装包内 herdr 的新窗格跟随；`config/domains.lua` 的 WSL 域改取 `utils/wsl.lua`。
    没有选择时回退到的默认 Shell 总在启动菜单第一位，macOS 的顺序因此变为 Fish、Bash、
    Nushell、Zsh。
  - `events/new-tab-button.lua`：`+` 右键菜单在点击时按 `window:effective_config()` 生成
    （启动菜单项、SSH/Unix 域，末项直达设置页 Shell 分区），不再 require `config.domains`。
  - `config/bindings.lua`：Windows 改用与 Linux 相同的 `Ctrl+Shift` 方案（原为裸 `Alt`）；
    带 Shift 的 `[ ] \ 0 9` 键位补绑 `{ } | ) (`；窗口缩放从 `SUPER+-` / `SUPER+=` 移到
    `Leader -` / `Leader =`（macOS 也一样），`Ctrl+Shift+=` 放大字号；关闭标签先确认；
    `Alt+Shift+S/V` 只在 Linux 绑定；resurrect 在保存/恢复键第一次按下时才加载。
  - `config/plugins.lua`：插件目录不存在就不调用 `wezterm.plugin.require`（它会在配置
    求值里同步 git clone）；resurrect 的 `init.lua` 还会 require dev.wezterm，两者都在
    才绑定 resurrect 键位。
  - `events/status.lua`：前台进程只在单标签的 herdr 应用模式下探测，每窗口至多 2 秒一次；
    电池信息缓存 60 秒；时钟回拨时两个缓存都作废。
  - `config/appearance.lua`：`front_end` 按平台取值（目前三个平台都是 OpenGL）；光标闪烁
    改 `Constant` 缓动、`animation_fps = 10`，空闲窗口不再持续重绘。
  - `config/general.lua`：`check_for_updates = false`；Windows 补齐免关闭确认的进程名
    （带 `.exe` 的 shell、MSYS2 `env.exe`、`gitstatusd-msys_nt-10.0-x86_64`）。
  - `config/fonts.lua`：Windows 在字体文件存在时追加 Microsoft YaHei、Segoe UI Emoji 回退。
  - `utils/backdrops.lua`：`gui-settings.json` 的位置改由 `utils/gui-settings.lua` 解析。
  - `tests/pure_fn_test.lua` 补上述行为的断言。Oh My Zsh `gx/wezterm/` 除 `backdrops/`
    外与本目录逐字节相同。

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
| `httpssCssZssZsgithubsDscomsZsMLFlexersZsresurrectsDswezterm` | github.com/MLFlexer/resurrect.wezterm | `47ce553`（v1.0.0-254） | GX：`utils.ensure_folder_exists` 先检查目录，再经 `run_child_process` 建目录并检查结果，修复 Windows 闪窗与转义；`state_manager` 在不可 yield 的 require 阶段延后创建，保存前再次检查；不收录 state/ |
| `httpssCssZssZsgithubsDscomsZsMLFlexersZssmart_workspace_switchersDswezterm` | github.com/MLFlexer/smart_workspace_switcher.wezterm | `40228a0`（1.2.0-18） | 无 |

## fonts/

全量字体 363MB 不入库，按配置实际引用精选（`config/fonts.lua`：主字体
JetBrainsMono Nerd Font **Regular**（2026-09-18 起由 DemiBold 改为常规字重）
+ 回退 Noto Sans CJK SC；fontconfig 中字重命名与文件名的对应关系以
`fc-match` 实测为准）：

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
  autoload/zle 定义后改为带标记自包含块。**2026-09-21 起 `~/.zshrc` 归
  oh-my-zsh gx 层真源**：`install.sh` 默认不再追加该块（`--zshrc` 显式
  追加供无 gx 层的机器），检测到 `~/.oh-my-zsh/.gx-managed` 时清理历史
  追加块。

## 不收录项

- `~/.cache/wezterm/`（64M 运行时缓存，可再生）。
- `~/.local/share/wezterm/check_update`（GitHub release 元数据缓存）。
- resurrect 已保存会话（当时为空）。
- 快照机器当时在用的官方 nightly AppImage 二进制（`~/.local/opt/wezterm-nightly/`）：
  目标机改用本分支自建二进制。

## Herdr 工作台联动（2026-09-18）

本轮主动调整：绑定前缀统一、明确 Shift 拖选归属、Windows shell 存在性检查、
WSL 自动发现、Regular 正文字重与 CJK 比例、独立配置模块加载路径。未更新插件
提交或字体资产。Oh My Zsh 的 `gx/wezterm` 保留相同配置改动；其原有未提交壁纸
快捷键另行保留，不作为本轮提交内容。
