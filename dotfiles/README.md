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
| `install.sh` | Linux 用户级安装器（幂等、无 sudo、`--check` 干跑；换入配置时保留原 `gui-settings.json`；默认不触碰 `~/.zshrc`——cursor-mode 块归 oh-my-zsh gx 层，无 gx 层的机器用 `--zshrc` 显式追加） | — |
| `install.ps1` | Windows 用户级部署脚本（换入配置时保留原 `gui-settings.json`） | `%LOCALAPPDATA%`、`%APPDATA%` |

Windows 注意：配置装到 `%USERPROFILE%\.config\wezterm`（源码确认与 Linux 同一查找
序）；插件装到 `%APPDATA%\wezterm\plugins`（Windows 数据目录是 Roaming AppData）。

## 原生安装包（推荐）

GitHub Actions 手动运行 `gx-release`，可下载完整的 Windows x64 安装 EXE 和
兼容 Ubuntu 20.04 与 24.04 的同一个 amd64 deb。Windows 默认安装到当前用户的
`%LOCALAPPDATA%\Programs\WezTerm GX`，也可选择所有用户；Linux 使用
`sudo apt install ./wezterm-gx_X.Y.Z_amd64.deb`。构建命令、发版入口及校验方式
见 [发布说明](../docs/RELEASE.md)。

安装包带齐配置、插件、字体和壁纸。原生启动器在普通用户首次启动时初始化资源：
改过的配置与 `gui-settings.json`（含壁纸选择）保留；包管理的四个插件备份后升级，保留 `state/`，其他插件
不受影响。插件暂存、备份和版本标记在数据目录的 `wezterm-gx/` 下，位于插件扫描
目录之外。卸载程序保留用户配置与会话。

已有配置按文件升级，每个载荷版本做一次（结果记在数据目录的 `wezterm-gx/config-version`；
启动器自带的已发布文件表变了，或日志里的根因文件改过，会再做一次）：

- 与任一已发布版本内容相同的文件（文本 CRLF 视同 LF）先备份到
  `wezterm-gx/backups/<时间戳>/wezterm-config/`，再换成新版；
- 新增的文件补上（`wezterm.lua` 不是 GX 发布的版本时，只补新代码 require 的模块）；
  发布过、被删掉的文件（如在壁纸浮层删除的壁纸）不补回，除非新代码 require 它；
- 改过的文件、只读文件和符号链接保留，原因与载荷里的对应路径写进
  `wezterm-gx/config-migration.log`，供手动合并；不再随包的文件原样留下；
- `gui-settings.json`（设置页保存的语言、默认 Shell、壁纸等）永不改动；
- 文件与它 `require` 的模块一起升级。只改了各发布版都没变过的文件（如 `wezterm.lua`
  的日期格式、`colors/custom.lua`）不挡任何升级；改了本次也更新的文件（如
  `config/bindings.lua`），它 require 的已更新模块（`config/plugins.lua`、
  `utils/backdrops.lua`）和依赖这些模块的文件（`config/appearance.lua` 等）一起留在旧版，
  日志写 `held back because config/bindings.lua is modified locally`；
- 相互 require 的一组文件全部换新或全部不动；写入失败时整组回滚，最多尝试 3 次
  （每次启动一次），备份都写进同一目录。

例如改过的 `config/launch.lua` 不会被替换（设置页的 Shell 分区因此没有可选项），依赖它的
`events/new-tab-button.lua` 也留在旧版，新增的 `utils/shells.lua` 等模块照常补上。把
`config/launch.lua` 换成日志给出的载荷版后，下次启动自动补齐；想保留自己的改动，就对照
载荷版合并，再从同一载荷目录复制 `events/new-tab-button.lua`。

## 旧版 bundle 与源码安装

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
git clone --recurse-submodules -b feature/gx_wezterm https://github.com/gx0404/wezterm.git
cd wezterm && make gx-install       # get-deps(需 sudo) → release 构建 → 部署
```

**3. Windows 旧版 zip 部署**

旧版 `gx-windows-build` 已归档，新安装包通过 `gx-release` 获取。
需要兼容旧安装脚本且已有 `wezterm-windows-*.zip` 时，与 `dotfiles/` 放同一目录后：

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
须更新 PROVENANCE.md 的 pin 记录。设置页写的 `gui-settings.json` 是用户数据：sync 不对比
也不收回它，原生安装包（`gx_package.py`）也不收录。

## 快捷键（Linux 与 Windows 相同）

两个平台用同一套 Ubuntu 终端习惯的 `Ctrl+Shift` 键位，不占用裸 `Alt`：
`Alt+f` / `Alt+b` / `Alt+d` / `Alt+.` / `Alt+Backspace` 等 readline 与 zsh 标准键
原样交给 shell（GX-10）。`Ctrl+C`（中断）、`Ctrl+V`（应用内图片粘贴）、
`Ctrl+B`（Herdr 前缀）、`Ctrl+_`（即 `Ctrl+Shift+-`，readline/zsh/emacs/nano 的撤销）
从不绑定。宿主 leader 是 `Ctrl+Shift+Space`（1 秒超时）。

| 快捷键 | 功能 |
|---|---|
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | 复制 / 粘贴 |
| `Ctrl+Shift+F` | 搜索 |
| `Ctrl+Alt+Shift+U` | 快速选择并打开 URL |
| `Ctrl+Shift+T` | 新标签（默认 Shell） |
| `Ctrl+Shift+W` | 关闭当前窗格（先确认） |
| `Ctrl+Alt+Shift+W` | 关闭当前标签（先确认） |
| `Ctrl+Shift+[` / `]`，`Ctrl+PageUp` / `Ctrl+PageDown` | 上一个 / 下一个标签 |
| `Ctrl+Alt+Shift+[` / `]` | 标签左移 / 右移 |
| `Leader 1..9` | 跳到第 1–9 个标签 |
| `Ctrl+Shift+0` / `Ctrl+Alt+Shift+0` | 重命名标签 / 恢复自动标题 |
| `Ctrl+Shift+9` | 显示或隐藏标签栏 |
| `Ctrl+Shift+N` | 新窗口 |
| `Leader -` / `Leader =` / `Ctrl+Alt+Shift+Enter` | 窗口缩小 / 放大 / 最大化 |
| `Ctrl+=` 或 `Ctrl++`（`Ctrl+Shift+=`、小键盘 `+`）/ `Ctrl+-` / `Ctrl+0`，`Ctrl+滚轮`，`Leader f` | 字号放大 / 缩小 / 复位（`Leader f` 后用 k/j/r） |
| `Ctrl+Shift+\` / `Ctrl+Alt+Shift+\` | 上下 / 左右分屏 |
| `Ctrl+Shift+Enter` | 缩放当前窗格 |
| `Ctrl+Alt+Shift+H/J/K/L` | 切到左 / 下 / 上 / 右窗格 |
| `Ctrl+Alt+Shift+P` | 选择窗格并交换 |
| `Leader p` | 调整窗格大小（h/j/k/l，Esc 或 q 退出） |
| `Ctrl+Shift+U` / `Ctrl+Shift+D` | 上 / 下滚 5 行 |
| `Shift+PageUp` / `Shift+PageDown` | 翻页；前台是 alt-screen 应用（herdr/vim/Claude Code）时透传给应用 |
| `Ctrl+Shift+S` | 智能 workspace 切换 |
| `Ctrl+Alt+Shift+S` / `Ctrl+Alt+Shift+R` | 保存 / 恢复会话（resurrect 首次按下时才加载） |
| `F1` / `F2` / `F3` / `F4` / `F5` | 复制模式 / 命令面板 / 启动菜单 / 标签列表 / workspace 列表 |
| `F8` | 发送 `Ctrl+R`（Atuin 历史菜单） |
| `F11` / `F12` | 全屏 / 调试浮层 |
| `Alt+Shift+S` / `Alt+Shift+V` | 仅 Linux：Flameshot 截图 / `~/.local/bin/ai-image-paste` 附加剪贴板图片 |

裸 `PageUp` / `PageDown` 留给 less、vim 等程序。WezTerm 在 Windows 与 X11 上把
`Ctrl+Shift+[` 报成 `{`，用户键位又不会自动合成 Shift 变体，所以带 Shift 的标点/数字
键位同时绑定美式布局的 Shift 字符（`{ } | ( )`）。

关闭窗格或标签时，窗格里只剩空闲 Shell 就直接关闭，不弹确认
（`skip_close_confirmation_for_processes_named`；Windows 上另补了 `zsh.exe`、`bash.exe`
等带 `.exe` 的 Shell 进程名、MSYS2 的 `env.exe` 与 GX Zsh 常驻的 gitstatusd）；herdr 等
其他程序在运行时仍先确认。

macOS：`Cmd` 代替 `Ctrl+Shift`，`Cmd+Ctrl` 代替 `Ctrl+Alt+Shift`（复制、粘贴、新标签与
Leader 仍是 `Ctrl+Shift+C/V/T/Space`）；翻页不需要 Shift；
保留 `Cmd+←/→/Backspace` 行首 / 行尾 / 清行；没有 `Ctrl+PageUp/PageDown`、`Ctrl+=/+/-/0`
与 `Leader 1..9`。与 Linux/Windows 相同的改动也作用于 macOS：关闭标签先确认、窗口
缩放在 `Leader -` / `Leader =`、截图与 AI 图片粘贴键只在 Linux 上有；启动菜单顺序也变了，
见下文「默认 Shell」。

Leader 层（壁纸、窗口缩放与浮层入口，herdr 抓鼠标时键盘仍可达）：

| 快捷键 | 功能 |
|---|---|
| `Leader .` / `Leader ,` | 下一张 / 上一张壁纸 |
| `Leader /` | 随机壁纸 |
| `Leader i` | 打开壁纸选择器 |
| `Leader b` | 切换纯色专注模式与壁纸 |
| `Leader w` | 壁纸管理浮层 |
| `Leader -` / `Leader =` | 窗口缩小 / 放大 |
| `Leader m` / `Leader s` / `Leader k` | 主菜单 / 设置 / 快捷键速查 |

修改键位后同步到 Oh My Zsh 仓库的 `gx/wezterm/config/bindings.lua`（两份逐字节一致），
并运行 `tests/pure_fn_test.lua` 的键位断言。

## 已知边界

- `config/bindings.lua` 的 Alt+Shift+V（仅 Linux）依赖 `~/.local/bin/ai-image-paste`，
  快照机器上该脚本本就缺失，触发时显示失败 toast（行为与本机一致）。
- 插件只从数据目录的 `plugins/` 加载（安装包与启动器预置），目录缺失时相关键位降级为
  空操作，不会在配置求值时联网 clone。
- 配置假定输入法为 fcitx（`xim_im_name='fcitx'` + desktop entry 注入 fcitx 环境
  变量）；用 ibus 的机器装完后可 `./install.sh --im ibus` 重装 desktop entry，
  并自行调整 general.lua。
- tabline.wez 插件目录含一段未上游化的 cpu.lua 本地补丁（见 PROVENANCE.md），
  重新从上游拉取会丢失，以本快照为准。

## Herdr 终端工作台

宿主 leader 统一为 `Ctrl+Shift+Space`，保留 `Ctrl+B` 给 Herdr。普通左键拖选由开启
鼠标协议的应用处理；Shift+拖选固定使用宿主选择并复制，Ctrl+左键仅在应用未捕获
鼠标时直接打开链接。正文采用 Regular 字重，TUI 标题和选中项自行强调。

## 默认 Shell

`utils/shells.lua` 在配置求值时只检查文件是否存在，不启动任何子进程，找到的 Shell
进入启动菜单（`F3`、右键 `+` 按钮）：

| Shell（`default_shell` 取值） | 查找位置 |
|---|---|
| GX Zsh（`gx-zsh`） | GX Shell 安装包内（`utils/gx-shell.lua` 按 `wezterm.executable_dir` 定位） |
| PowerShell 7（`pwsh`） | PATH（含应用商店别名）、`%ProgramFiles%\PowerShell\7`（及 7-preview）、scoop |
| Windows PowerShell 5.1（`powershell`）/ cmd（`cmd`） | `%SystemRoot%\System32` / `%ComSpec%` |
| Git Bash（`git-bash`） | PATH 上的 `git.exe` 反推的安装目录、Program Files、`%LOCALAPPDATA%\Programs\Git`、scoop |
| MSYS2 UCRT64（`msys2-ucrt64`） | `C:\msys64`、`C:\tools\msys64`、scoop、PATH 上的 MSYS2 `usr\bin`；GX 私有运行时不算 |
| Nushell（`nu`） | PATH、`%ProgramFiles%\nu\bin`、scoop |
| WSL 发行版（`wsl:<发行版>`） | `wezterm.default_wsl_domains()`（`utils/wsl.lua`，见下） |
| Zsh / Bash（`zsh` / `bash`，Linux） | PATH |
| Fish / Bash / Nushell / Zsh（`fish` / `bash` / `nu` / `zsh`，macOS） | 固定列表，不检查是否存在（Fish、Nushell 在 `/opt/homebrew/bin`） |

System32 与 WindowsApps 下的 `bash.exe` 是 WSL 启动器，不当作 Git Bash 或 MSYS2。除
WindowsApps 下的应用执行别名外，打不开的文件（失效的 PATH 条目、没插盘的驱动器）一律
视为不存在。WSL 发行版列表由 `utils/wsl.lua` 获取（`config/domains.lua` 与
`config/launch.lua` 共用，所以在 `domains.lua` 里加 SSH 域不会挡住启动菜单的升级）：
找到发行版后整个 GUI 进程只运行一次 `wsl.exe`；列表为空或 `wsl.exe` 失败时 5 分钟后
的配置求值会再试。WSL 使用真实发行版的默认用户和登录 shell，不假定 Windows 用户名
等于 Linux 用户名。

在设置浮层（`Leader s`，或主菜单 / 标签栏右键菜单的「默认 Shell…」、右键 `+` 列表末尾
的「设为默认 Shell…」，后两者直接打开 Shell 分区）选择默认 Shell，结果写入
`gui-settings.json` 的 `default_shell` 键
（选 GX Zsh 时删除该键）。该文件与 `wezterm.lua` 同目录；配置是 `~/.wezterm.lua` 时在
`$XDG_CONFIG_HOME/wezterm`（默认 `~/.config/wezterm`），Lua 侧（`utils/gui-settings.lua`）
与 WezTerm 按同一规则定位。未选择、或所选 Shell 已不存在（此时在日志里警告）时依次回退
GX Zsh → PowerShell 7 → PowerShell 5.1（Linux：GX Zsh → Zsh；macOS：Fish）；此时生效的
就是启动菜单里第一个 Shell，所以 macOS 的 Fish 现在排在 Bash 前面。
选中 WSL 发行版时，新标签默认进入该发行版。启动菜单每一项都固定自己的 domain，
所以在 WSL 标签里打开 PowerShell 仍然是本机 PowerShell。

作为 GX Shell 安装包运行时，启动菜单最前面是「GX Zsh」「herdr」，切换默认 Shell
后还会调用 `bin\herdr.exe --gx-set-default-shell <Shell 的绝对路径>`，让 herdr 的新窗格
跟随（已打开的窗格保持原 Shell），结果用 toast 提示。herdr 只接受一个可执行文件，
做不到的部分会在 toast 里写明：选 WSL 发行版时 herdr 进入 WSL 默认发行版；选 MSYS2
UCRT64 时 herdr 得到 MSYS 环境的 bash；Linux 上选系统 zsh 时 herdr 设为 GX Zsh（herdr
服务端带着 GX 的 zsh 环境，系统 zsh 在它的窗格里同样会加载 GX 配置）。herdr 配置不是
GX 管理的（手改过，或设置了 `HERDR_CONFIG_PATH`）时保持不变并提示。

`wezterm.lua` 将自身目录放到 Lua 模块搜索路径前端，因此 `--config-file` 的隔离
验证不会混入 `~/.config/wezterm` 的旧模块。联调先用独立配置验证，再备份并定向
更新本机文件；不要全量覆盖用户自己的插件和配置。
