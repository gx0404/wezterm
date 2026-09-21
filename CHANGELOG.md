# Changelog（gx0404/wezterm fork）

本文件记录 fork 层面已实现的可观察变更（AI 协作框架、开发流程、定制功能）。
上游 WezTerm 的产品级变更见 `docs/changelog.md`（随上游同步，不在本文件重复维护，
避免同步上游时产生冲突）。

版本真源：本文件 `## X.Y.Z(日期|TBD)` 标题中的最大 SemVer（`scripts/version.py`，
`make version` 只读查询）。WezTerm 产品自身的版本号由 `wezterm-version/build.rs`
按 git 提交时间与哈希生成，两套体系互不干扰（见 docs/RELEASE.md）。

## 0.2.0(TBD)

- `fix(dotfiles)`：统一 Herdr/WezTerm 前缀和鼠标归属，修复独立配置误载旧模块；
  Windows shell 自动回退、WSL 使用发行版默认用户，正文改用常规字重。


### Added
- 壁纸管理浮层（批 13，`wezterm-gui/src/termwindow/wallpaper.rs`，入口：
  主菜单项与 `KeyAssignment::ShowWallpaperOverlay`，dotfiles 绑
  `Leader w`）：列出壁纸目录图片（文件名/尺寸/大小/当前标记），
  ↑↓/j/k/n/p 移动即实时预览（只替换窗口背景层栈——克隆进入时层
  定义、第一层换图、遮罩层原样保留，不重载 Lua，与批 5 预览同一
  口径）；Enter 应用并持久化到 `gui-settings.json` 的 `wallpaper`
  键（重启生效：Lua 侧 `utils/backdrops.lua::set_default_from_sidecar`
  启动/重载时读回，只认 basename 与目录内条目）；`r` 随机预览；
  `a` 添加——路径输入（Tab 补全/`~` 展开/剪贴板粘贴），校验可解码
  图片（读头，不解码整图）后复制进壁纸目录（同名同内容复用、
  不同内容哈希后缀）；`d` 删除（`y` 二次确认，只删目录内条目，
  删当前壁纸时清 sidecar 并回退预览/快照）；空态提示与行内错误
  不关浮层；Esc/点外/被顶掉经 `Modal::on_dismissed` 还原未确认
  预览。`gui_settings` 新增 `delete_key` 与 `GUI_OWNED_KEYS`
  （fork 自有键静默跳过，不再每次加载告警）；`load_background_layer`
  crate 内公开、`ImageFileSourceWrap` 补 Rust 构造器。大图沿用
  backdrops 现有惰性解码路径（EncodedFile 首次渲染时解码），
  未引入额外线程。6 条纯函数单测（扫描/校验/补全/复制去重/
  预览层栈/空目录）+ Xvfb 沙箱 9 张场景截图读回。
- CLI 帮助全面 zh-CN 汉化（wezterm 与 wezterm-gui 两二进制的全部子命令
  树）：clap derive 的帮助文本来自 doc 注释，采取**运行时本地化**——
  parse 前经 `wezterm-gui-subcommands::localize_clap` 遍历命令树
  （先 `build()` 以覆盖自动生成的 help 子命令与 `--help/--version`
  参数），about/long_about/参数 help 全量过译表（新增 `tr_str`/
  `init_cli_early`：WEZTERM_LANG > gui-settings.json 的 language 键
  （免执行 Lua 的 JSON 探针）> zh-CN 默认）；`WEZTERM_LANG=en` 整体
  回退英文。shell 补全生成钉死英文保持派生文件字节稳定
  （`generated-check` 无环境依赖，补全零 diff）。遍历校验器
  （`.ui-evidence/cli_walk.py` 一次性工具）确认两二进制全部帮助
  缺译键为 0。
- herdr 式主菜单与 ☰ 按钮：tab 栏右端新增 `☰` 主菜单按钮（fancy 与
  retro 两套 tab bar 均渲染，`show_menu_button_in_tab_bar` 默认开），
  左键/右键点击在按钮下方弹出主菜单——命令面板 / 快捷键 / 设置 /
  重载配置 / 隐藏窗口 / 退出（文案与命令面板术语一致）；新增
  `ShowMainMenu`、`ShowKeybinds` 键位/面板命令（默认未绑键，可在
  key_bindings 绑定）；鼠标绑定 region 新增 `MenuButton` 可定向绑定。
- 快捷键速查浮层（`wezterm-gui/src/termwindow/keybinds.rs`）：按命令
  面板分组列出全部命令与当前实际生效键位（含用户自定义），键帽渲染
  与面板共用（`commands::format_key_label` 抽取共享），↑↓/j/k 滚动、
  悬停高亮、Esc 关闭。
- herdr 式设置浮层（`wezterm-gui/src/termwindow/settings.rs`，新
  `OpenSettings` 键位/面板命令，macOS 菜单栏归入 WezTerm 组）：四个分区
  ——语言（中文/English，应用即全 UI 切换）、外观（1001 个内置配色方案，
  支持输入过滤，移动/悬停即预览、Enter 应用、Esc 还原）、交互（右键
  菜单/滚动条开关、响铃、关闭确认的二值切换，即选即生效）、字体
  （字号 0.5 步进增减与重置）。预览走窗口级临时调色板（易失，见下方
  Fixed 段的 WZ-02/WZ-03），应用写入 `gui-settings.json` 并
  `config::reload()` 全局生效、跨重启持久化；Tab 切换分区、↑↓ 选择、
  Enter 应用、鼠标悬停/点击与命令面板共用 Modal 通道，配置重载
  （含语言切换）即时重绘浮层文案。
- GUI 界面文案全面 zh-CN 汉化（311 条译表，`config/src/i18n/zh_cn.rs`，
  由 `scripts/gen_zh_table.py` 生成保序）：命令面板全部命令的标题/描述/
  分组名（模糊搜索与精确匹配同步走中文）、右键菜单、关闭确认与
  [Y]是/[N]否 按钮（CJK 双宽按实测单元格宽度排布）、复制模式/搜索
  状态行、快速选择提示、启动器全部条目与帮助行、调试浮层横幅、
  Emoji/字符选择分组、配置错误窗标题、致命 toast 标题。翻译策略：
  英文原文即 key，未命中回退英文（上游新增文案零成本降级）；
  动态模板（序数/方向/数量插值）构造点经 `fill(&tr(..))` 本地化，
  序数词 zh 输出「第 N 个」；命令面板 frecency 仍以英文标题为稳定
  key，切换语言不丢排序记忆。wezterm-gui 新增译文覆盖守门测试
  （默认命令表静态 brief/doc 缺译即 fail）。
- 界面文案 i18n 基建（`config::i18n`）：英文原文为 key 的 zh-CN 译表
  （有序静态表 + binary_search，未命中回退英文，上游新增文案不炸构建）；
  新增 `language` 配置项（`"zh-CN"` 默认 / `"en"`，严格校验），
  `WEZTERM_LANG` 环境变量优先级最高（运维钉死）；语言状态为进程内
  全局原子，配置加载漏斗（`load_with_overrides` 成功路径）统一落地，
  reload/每窗口 override 路径自然收敛。
- GUI 设置持久化通道 `gui-settings.json`（`config::gui_settings`）：
  位于实际生效的 wezterm.lua 同目录（HOME 时退回 XDG 目录），加载
  优先级 wezterm.lua < gui-settings.json < `--config` CLI 覆盖 <
  每窗口 `set_config_overrides`；该文件为 GUI 设置页独占（临时文件 +
  rename 原子写入），无效键告警跳过而不拖垮整个配置加载；
  `peek_language()` 供 CLI 进程免执行 Lua 快速取语言。

- 收录本机 wezterm 用户环境快照为仓库真源 `dotfiles/`：配置工作区
  （含壁纸与生效中的事件脚本，`config/launch.lua`、`config/domains.lua`
  的 Windows 死路径改动态拼接）、4 个插件按 wezterm 插件加载器转义目录名
  快照（pin 见 `dotfiles/PROVENANCE.md`）、精选字体（JetBrainsMono Nerd
  Font 6 字重 + Noto Sans CJK Regular/Bold）、desktop entry / wrapper /
  zshrc 模板。
- 跨机一键安装链：`dotfiles/install.sh`（Linux 用户级、幂等、`--check`
  干跑）与 `dotfiles/install.ps1`（Windows 用户级）；`make gx-bundle`
  经 docker ubuntu:20.04 容器构建 glibc≤2.31 兼容二进制并组装自包含离线
  包（Ubuntu 20.04/24.04 通吃）；`make gx-install` 提供在线源码构建路径；
  `make gx-sync` 把本机配置改动收回仓库（默认只读对比）。
- Windows 分支构建 workflow `.github/workflows/gx-windows-build.yml`
  （workflow_dispatch / gx-v* tag 触发，产出四件套 zip 供 gx-bundle 组装）。
- 领域规则 `docs/AGENT_RULES/dotfiles.md` + 路由；`dist/`、`target-gx-*/`
  入 `.gitignore` fork 段。
- `gx-sync --check` 接入 `make framework-check` 守门（WEZ-CFG-02）：
  本机配置/插件与仓库快照出现非登记差异时退出码 1，提示两个方向的
  补救命令（部署 `make gx-upgrade` / 回收 `GX_SYNC_WRITE=1 make gx-sync`）；
  PROVENANCE 登记的有意改动（launch.lua/domains.lua）不计入差异，
  也不再被 `--write` 误写回（此前会被本机旧版覆盖，抹掉有意改动）。
  PROVENANCE 同步现场（WEZ-HYG-03）：general.lua 的 `language` 键已随
  09-20 gx-upgrade 部署、失效的 .bak-20260713 记录删除、字体段更正为
  Regular 字重现状、zshrc 模板段记录归属变更。

- UI 交互增强（源自对 herdr 与 tmux 的交互调研）：鼠标绑定新增
  `region` 维度（可按 TabBar/Tab/NewTabButton/LeftStatus/RightStatus/
  Split/ScrollThumb 等区域定向绑定；未指定 region 的绑定语义不变）；
  copy mode 新增 mark 标记（`m` 设标记、`'` 交换式跳转）、段落移动
  （`{`/`}`）、`MoveToLine` 行跳转、无选区时复制光标处词、
  `PipeSelection` 管道赋值与 `copy_mode_mark_bg/fg` 配色；tmux 域
  pane 的 copy mode 搜索默认可用；bell 治理三件套：
  `bell_notification_handling` 聚焦抑制、`bell_requests_attention`
  失焦 WM 提醒（X11 urgency / macOS dock bounce）与 Lua
  `window:request_attention()`、`bell_cooldown_ms` 每 pane 节流。
- 命令面板鼠标交互：悬停行即选中、左键点击直接执行（与 Enter 共用
  激活路径并记录 frecency）；`Modal::mouse_event` 通道接通，行矩形经
  hit map（`UIItemType::Modal`）路由，region 绑定新增 `Modal` 区域。
- `make gx-upgrade`：一条命令完成本机替换——docker ubuntu:20.04 容器
  构建 release 四件套（无 sudo，get-deps 唯一真源）→ `dotfiles/install.sh`
  用户级部署（配置/插件/字体随快照，全带时间戳备份）→ 版本验证；
  消除手工两步安装尾巴。
- 开箱即用的 tmux/herdr 式鼠标交互（默认启用，`mouse_right_click_menu`
  可关）：终端区右键弹 pane 菜单（分屏/缩放/复制/粘贴/滚动/关闭），
  tab 与 tab 栏空白右键弹各自菜单；菜单内悬停即选中、左键执行、
  点击菜单外或 Esc 关闭；tab 支持按住左键拖拽重排（MoveTab）。
  菜单为 `ContextMenu` Modal（`UIItemType::Modal` hit map 路由），
  与命令面板共用 Modal 鼠标通道。

### Fixed

- 浮层定位的 `tab_bar_pixel_height().unwrap()` 全部改走
  `tab_bar_pixel_height_lossy()`（W8）：fancy tab bar 标题字体解析
  失败时，设置/快捷键/命令面板/壁纸等浮层在 GUI 事件循环里 unwrap
  一个 Result 直接 panic 整个进程；回退到单元格高度并告警（渲染
  主路径本就走 `?`/容错，不受影响）。
- 设置页枚举分支新增 key 不再 panic（WZ-21）：`next_enum_value` /
  `enum_display` 的兜底 `unreachable!()` 改为告警并回退（Null 哨兵
  由 `activate` 守卫不落盘、显示占位「?」），漏加分支从 GUI 崩溃
  降级为一次日志。
- 修复全是分隔符的上下文菜单按方向键死循环冻结 GUI（WZ-17）：
  `ContextMenu::move_selection` 的「跳到可选行」循环无上限，整圈
  无可选行时永不退出；改为最多扫一遍（len 步），全分隔符菜单
  停在原地。新增单测（纯分隔符不挂、部分可选仍跳转）。
- 设置页分区 tab 可辨识可点击（WZ-09）与打开定位当前值（WZ-18）：
  tab 行从单一字符串拆为每分区一个子 Element，当前分区用数据行
  选中态同款反显；分区点击经 `MODAL_SECTION_BASE` 哨兵区间路由
  （加 const 断言与数据行/chrome 哨兵不重叠），点击当前分区不重置
  浏览状态。打开与切换分区时选中行落到当前生效值（语言/配色），
  滚动窗口收敛时给末行留一行余量——行预算缓存来自上一分区
  （chrome 行数不同），贴末行的当前值会被下一帧裁掉。
- retro 标签栏 ☰ 按钮悬停区与宽度预算改用显示列宽（WZ-13/14）：
  悬停区原按 `" ☰ "` 字节长（5）算、实际渲染 4 列（☰ 双宽），悬停
  命中偏右一格；标签宽度预算未扣 ☰，标签多时 ☰ 与右状态区被挤出
  右缘。统一 `menu_button_display_cells()` 真源（悬停/预算/渲染三处
  共用），加单测钉住字节数与列数差异。
- 修复选区内的光标单元格在窗口失焦时丢失选中高亮（W11）：
  `compute_cell_fg_bg` 的配色 match 里，光标格+失焦落到普通色分支；
  新增「选中且非聚焦活动」分支优先返回选中色（Xvfb 双窗口截图：
  失焦窗口选区行与光标格高亮均保持）。
- `us_layout_shift` 补全 US 标点映射（`,`→`<`、`/`→`?` 等六个）：
  X11 把 SHIFT+标点解成 shifted 字符（如 `<`），默认键 permute 变体
  合成缺少映射导致 `CTRL|SHIFT+,`（OpenSettings）与 `CTRL|SHIFT+/`
  （ShowKeybinds）等 SHIFT+标点默认键物理不可达（上游 #1906 同族）。
  壁纸选择器键位因此从 `Leader Shift+/` 改为 `Leader i`（用户 Lua
  绑定无变体合成，shifted 标点形式同样不可达）。
- WZ-12 核验驳回不改代码：实测右键点击 (640,450) 菜单
  actual_bounds=(581,146,246×308)，翻转与 clamp 是窗口小于屏幕时
  的正确行为，padding 叠加 ≤11px 不构成贴底裁切（探测日志留档）。
- 右键上下文菜单接入正规 mouse binding 机制（WZ-05/WEZ-INT-03）：
  原先 `mouse_event_terminal` 在绑定匹配之前硬拦截右键 Press，绕过
  用户 `mouse_bindings` 与 `bypass_mouse_reporting_modifiers`。改为新增
  `KeyAssignment::ShowPaneContextMenu` + `InputMap` 默认注册
  `{Down streak1 Right, mods=NONE, mouse_reporting=false, region=Pane}`
  （`mouse_right_click_menu=false` 时不注册）；菜单在点击位置弹出
  （键盘触发时窗口居中）。语义：非抓取 pane 裸右键弹菜单；应用抓取
  鼠标时裸右键透传应用，Shift+右键（bypass 翻转）弹菜单——抓取态下
  菜单从「不可达」变为「Shift 可达」。Xvfb 四场景截图核对通过
  （含 SGR 鼠标上报透传实证）。
- 主菜单/设置/快捷键三个浮层补默认键位（WZ-16/WEZ-UX-01）：
  `Ctrl+Shift+M` / `Ctrl+Shift+,` / `Ctrl+Shift+/`（应用抓鼠标时
  键盘入口可达；fork dotfiles 因 `disable_default_key_bindings`
  另在 leader 层绑定 `Leader m` / `Leader s` / `Leader k`）。
  `mouse_region_roundtrip` 测试补 `MenuButton` 变体（WEZ-TEST-01）。
- 修复安装链的版本漂移与静默覆盖（WEZ-BUILD-01/02、WEZ-HYG-02）：
  `gx_install.sh` 曾「target/release 存在即复用」，实测把落后 HEAD 数天
  的混版二进制装进版本目录（`wezterm` 与 `wezterm-gui` 版本串都不同），
  回滚目标被同名覆盖。现在：默认总是 `make build BUILD_OPTS=--release`
  （cargo 增量，仅 `--reuse-build` 跳过）；安装前断言四二进制自报版本
  一致（`wezterm-gui --version` 的占位串已根治——`env_bootstrap::
  bootstrap()` 移到 clap parse 之前；`strip-ansi-escapes` 补
  `--version`）；`VERSION_DIR` 追加四二进制联合内容哈希，同 commit 的
  脏树/异 feature 重构建不再静默覆盖；`.gx-managed` 元数据（版本/时间/
  源 commit/内容哈希）正式生成；`wezterm-version/build.rs` 补
  `rerun-if-changed=.git/HEAD`（分支切换不再留下陈旧版本串）。安装末尾
  回收 `~/.local/opt/wezterm-gx` 旧版本目录（保留当前+最新 2 个）与
  `.bak-gx-*` 备份（保留最新 3 份）——真机已累积 1.1GB/7 份。
- `~/.zshrc` 的 cursor-mode 键位块归属改为 oh-my-zsh gx 层（2026-09-21
  拍板）：`dotfiles/install.sh` 默认不再追加该块（`--zshrc` 显式开关
  保留给无 gx 层的机器，`--no-zshrc` 兼容保留）；检测到
  `~/.oh-my-zsh/.gx-managed` 时备份并清理历史追加的
  `# >>> wezterm-gx >>>` 标记块。沙箱演练：默认不建/不改 `.zshrc`、
  gx 层在场时清理历史块、`--zshrc` 幂等追加各验证通过。
- 修复 gui-settings.json 首载按 env/XDG 而非生效 wezterm.lua 目录解析
  （WEZ-CFG-01）：`try_load` 在应用 sidecar 之后才 `set_var(
  WEZTERM_CONFIG_DIR)`，首载与重载解析到不同路径，`--config-file`
  隔离环境会读写用户真实 sidecar；CLI 早期 Deny 校验还会把 sidecar
  的失效键变成致命错误。`gui_settings::apply_to_lua` 新增显式 `dir`
  参数（`try_load` 传配置文件目录、无文件默认路径传 None），
  `store_key_in_dir` 公开化；CLI overrides 早期校验拆出
  `validate_cli_overrides`（不掺 sidecar）。新增 4 条单测（显式目录
  优先于 ambient、重复加载同路径等），隔离实测：`--config-file
  /tmp/iso/wezterm.lua` 只读 iso 目录的 sidecar。
- 修复设置浮层配色预览把整份配置重载挂在按键重复率上：`settings.rs` 的
  预览写每窗口 `config_overrides` 并调 `TermWindow::config_was_reloaded`，
  于是每经过一行就重跑一遍用户 Lua、重建全部字体、对每个 pane
  `set_config`，并经 `apply_dimensions` 沿 PTY 打出 SIGWINCH——1001 条配色
  里长按 ↓ 直接卡死，嵌套的 herdr/Claude Code 还会被逐层放大重绘。预览
  改走新增的窗口级临时调色板 `TermWindow::{preview_palette,
  set_preview_palette,pane_palette}`：按名先查 `config.color_schemes`
  （Lua 里定义的与 `color_scheme_dirs` 加载的），再回退内置
  `config::COLOR_SCHEMES`（进程内已解析），取到后叠加用户 `colors` 覆盖
  （查找与推导次序复刻 `Config::resolve_color_scheme` 与
  `resolved_palette`，同名时用户那套优先），只 bump
  quad/shape 失效代数、丢 fancy tab bar 缓存并 invalidate，不碰配置、
  字体与窗口尺寸；配置落地仍只在 Enter 确认时走一次
  `gui_settings::store_key` + `config::reload()`。
- 修复点浮层外/被另一浮层顶掉时配色预览永久残留：还原原本只挂在 Esc 上，
  其余两条关闭路径会把预览留成每窗口 override（优先级高于全局配置，
  `ReloadConfiguration` 也清不掉）。`modal.rs::Modal` 新增
  `on_dismissed`，`TermWindow::{cancel_modal,set_modal}`（改收 `&mut self`）
  在摘除/顶掉浮层时统一回调，`SettingsOverlay` 在其中丢弃预览调色板——
  Esc、点浮层外、被另一浮层顶掉三条路径行为一致。设置浮层不再写
  `config_overrides`（`upsert_override`/`restore_overrides` 一并删除）。
- 设置浮层「外观」分区不再在每个输入事件里重建 1001 条配色列表：
  `compute` / `move_selection` / `mouse_event` 各要一次可见行列表，原先
  每次都重新排序全部方案名并跑一遍模糊匹配。改为按「分区 + 过滤文本」
  缓存 `Rc<Vec<Item>>`；同一行上的重复预览按方案名短路。
- 设置浮层的字号步进、开关与枚举切换不再即时写每窗口 `config_overrides`：
  原先「先 `upsert_override` 触发一次全量重载，再 `persist_and_reload`
  触发第二次」，一次确认要重跑两遍用户 Lua 与字体重建，而留下的 override
  优先级高于全局配置、`ReloadConfiguration` 也清不掉。现在统一只走一次
  `gui_settings::store_key` + `config::reload()`。随之「当前值」的真源改为
  刚落地的全局配置（`settings.rs::current_config`）而不是窗口的
  `ConfigHandle`——后者靠 `Window::notify` → SPAWN_QUEUE 异步回推，而 X11
  主循环先把排队的 X 事件一次排干才轮到 SPAWN_QUEUE，长按 Enter 时堆积的
  重复按键会连续读到同一份陈旧值（字号只动一格、开关连点两下不回弹）；
  下一个值与行标签都由纯函数 `settings.rs::pending_write` / `row_label`
  从该配置推出，确认后立刻重算浮层，勾选标记与「字号: 12.5」不再等一拍。
- 设置浮层切换分区时一并丢掉「外观」分区的配色预览：预览行在新分区已经
  不可见，留着会出现「窗口是预览色、界面上却没有任何一行对应它」的脱节，
  且在新分区按 Enter 会先闪回原配色再应用。
- 修复 tab 拖拽重排完全失效：`mouseevent.rs` 的 Release(Left) 分支与
  `TermWindow::finish_tab_drag` 各 `take()` 了一次 `tab_drag`，调用方先取空
  之后 `finish_tab_drag` 必定提前返回，「按住左键拖拽重排 tab」是死路径。
  `take()` 收敛到 `finish_tab_drag` 一处；插入位计算抽成纯函数
  `mouseevent.rs::drop_index`（顺带修好指针拖出窗口左侧时 `x as usize`
  回绕、被当成「拖到最右」的边界），配 6 条单测（最右/最左/原位/越界/
  负坐标/相邻换位）。Xvfb 前后对照截图：修复前拖到最右顺序不变，修复后
  A|B|C → B|C|A → 拖回 A|B|C。
- 修复右键上下文菜单中文标签被拦腰截断：`context_menu.rs` 用
  `chars().count()` 估算宽度，CJK 一格占两列因而少算一半，box model 随后
  按 bounds 把标签截断。改用 `termwiz::cell::unicode_column_width`（分隔行
  同理），`+ 16.` 与 `* 1.2` 魔数换成与 `Element` 构造共用的
  padding/margin/border 常量推导（新增纯函数 `content_width_cells`
  与 `menu_box_size` 及 4 条单测）。
- 修复设置浮层「外观」分区过滤框打不进小写 `j`/`k`：`settings.rs` 的
  导航 arm 排在通配过滤 arm 之前，1001 条配色里搜不到 `jellybeans`/
  `kanagawa`，而过滤是该列表唯一可用入口。按键路由抽成纯函数
  `settings.rs::classify_key`，`j`/`k` 在有过滤框的分区让位给过滤输入，
  ↑↓ 与 Ctrl+p/n 在任何分区都仍是导航；Backspace 与字符输入一致地重置
  选中行与滚动位置（7 条单测）。
- 修复设置浮层选中行滚出可视区：滚动窗口用终端字体度量、渲染却用命令
  面板字体度量。`SettingsOverlay::max_rows_on_screen` 的结果在 `compute()`
  里缓存进 `visible_rows`，`move_selection` 直接复用，两侧行预算严格一致
  （Xvfb 前后对照：刻意拉开两种字号后，修复前连按 20 次 ↓ 选中行不可见，
  修复后选中行留在可视区末行）。
- 修复点击浮层内边距/边框/外边距一圈以及 chrome 行右侧空白被当作
  「点击浮层外」误关：这一圈不属于任何已登记行，`resolve_ui_item` 命中
  不到就走了点外关闭。四个浮层（右键菜单/设置/快捷键/命令面板）的外层
  容器统一打 `UIItemType::Modal(MODAL_CHROME_ROW)`，chrome 行补
  `min_width(Percent(1.))` 铺满宽度；命中顺序真源抽成
  `mouseevent.rs::hit_ui_item` 并加子行优先于外层容器的单测。
- 修复 DECSET 2026 同步输出 hold 无超时：guest 在 `?2026h` 之后崩溃/挂死
  会让 pane 永久不刷新且动作队列无界增长。`mux/src/lib.rs::parse_buffered_data`
  的 hold 改为带到期时间的状态机（`SyncOutputHold`），到期强制 flush 并
  `log::warn!` 一次；新增配置 `mux_synchronized_output_timeout_ms`（默认
  150，`0` = 不超时保留旧语义，文档见 `docs/config/lua/config/`）。同时收到
  `?2026h` 时仅当块前确有未刷新动作才先 flush（否则只开启 hold），一帧
  `?2026h…?2026l` 只呈现一次，消除 herdr 切标签时的中间态闪烁。
- 修复 `--config 'a=b;c=d;…'` 仅第一对生效：`set_config_overrides`
  现按**顶层分号**展开为多对覆盖（花括号/圆括号/方括号嵌套与引号
  字符串内的分号保持字面，如 `keys={{a=1};{b=2}}` 不受影响），后续
  段必须是 `name=expr` 形式否则报错；新增 5 条拆分单测。
- 修复浮层 chrome 行（标题/分区/页脚/过滤输入行）点击被当作
  「点击浮层外」误关：chrome 行统一打 `UIItemType::Modal(MODAL_CHROME_ROW)`
  哨兵进 hit map，各 Modal（命令面板/设置/快捷键页）mouse_event
  对哨兵与越界行号加防护（Xvfb 实测：点击设置标题浮层保持打开，
  点击浮层外仍正常关闭）。
- 修复 ssh e2e 在非 root 环境全挂：`wezterm-ssh/tests/sshd.rs` 的
  sshd 配置 `UsePAM yes` 在非 root sshd 下 PAM account 阶段拒绝当前
  用户（`Access denied … by PAM account configuration`）、连接在
  publickey 认证中途被断开；改为 `UsePAM no`（测试仅走 publickey，
  不需要 PAM），本机 50/50 全过。
- 设置浮层顶部行被窗口边缘截断：原 bounds 高度按固定 8 行预算
  传入 box model，列表较长时顶部溢出裁剪；改为与命令面板一致的全
  终端高度 bounds（Xvfb 截图验证标题/分区/页脚完整）。
- `scripts/ui_smoke.sh` 在 ffmpeg 未编入 png 编码器的发行版上直接
  失败：增加编码器探测，缺失时回退 mjpeg 输出 `before.jpg`
  （result.json 的 screenshots 字段随实际文件名登记）。
- 复审更正与遗留：早前记录的「`--config keys` + `wezterm-gui start`
  不生效」为**误判**（截图 CDN 同名缓存 + OCR 未反色的双重假阴性；
  多 `--config` 旗标与分号形式实测均生效）。WebGPU 前端经 lavapipe
  软件渲染验证：release 构建下适配器枚举与渲染循环正常（debug 构建
  的 wgpu 校验层与旧 Vulkan loader 组合会崩，非 wezterm 代码问题）；
  xwd 截图为黑帧属 lavapipe 的 X11 present 不写回像素，视觉效果
  仍需真实硬件确认。macOS 侧交叉编译被 C 依赖与 Apple SDK 阻断，
  本轮对 `#[cfg(target_os="macos")]` 代码零改动（仅迁移平台无关的
  键帽排序逻辑），macOS 实测仍 PENDING。
- 修复 `dotfiles/install.sh` 配置步骤的启动竞态：原先先 `rm -rf`
  `~/.config/wezterm` 再整树 `cp -a`（19MB 快照需秒级），空窗期内启动
  wezterm 会报 `wezterm.lua: No such file or directory`。改为暂存目录
  整树拷贝 + 连续 rename 原子换入（备份同步改为 rename），空窗缩至
  微秒级；EXIT trap 清理异常残留的暂存目录。实测安装期间并发 40 次
  配置加载零失败。
- 修复 `make test` 的 escape-parser no_std 轮编译失败（上游
  8d668a78c 移除 macro_use 时遗漏测试模块的 `alloc::format` 导入）。
- 框架 `make generated-check` 恒报漂移：键表比对改为经钉版 stylua
  （`ci/stylua.toml`，与 docs 构建同约定）格式化后比较，缺工具时显式
  跳过；`make generated-write` 由 `scripts/generated_write.sh` 包装
  上游脚本并补格式化；stylua 2.5.2 入 `setup_env.sh` 钉版；shell
  补全随 clap 生成器演进重建。`ui_smoke.sh` 修正 `--config` 全局参数
  位置并加 `-n` 隔离用户配置（本机配置在裸 Xvfb 渲染全黑）。
- 恢复 Linux 壁纸快捷键：`Alt+.` / `Alt+,` 切换下一张 / 上一张，
  `Alt+/` 随机、`Ctrl+Alt+/` 选择、`Alt+b` 切换纯色专注模式；
  避免壁纸控制跟随通用 `Ctrl+Shift` 修饰键变更。同步本机配置、
  `dotfiles/wezterm-config/` 与 Oh My Zsh 的 `gx/wezterm/` 快照。
- 壁纸与标签直达键迁出裸 Alt 层（GX-10）：裸 `Alt+.` / `Alt+b` 是
  readline 标准键（末参数插入、退词），GUI 截获后永不下发 PTY；`Alt+w`
  还曾无确认销毁 pane。壁纸五键（随机/上一张/下一张/选择器/专注模式）
  改挂 leader（`Ctrl+Shift+Space` 前缀），分屏 `Alt+\` 系并入统一的
  `Ctrl+Shift(+Alt)+\`，标签直达 `Alt+1..9` 改 `Leader 1..9`，`Alt+w`
  保留但 `confirm=true`。壁纸键本是 `action_callback`，不出现在快捷键
  速查浮层（改前改后一致）；浮层的内建命令键位显示不受影响，无需另改。
  仅改 `dotfiles/wezterm-config/config/bindings.lua`；Oh My Zsh 侧
  `gx/wezterm/config/bindings.lua` 镜像由 ohmyzsh 车道同步。

## 0.1.0(TBD)

### Added

- 初始化 AI 协作开发框架：根 `AGENTS.md` 启动协议 + `docs/AGENT_RULES/`
  领域规则与 `routes.toml` 路由闭集 + `scripts/resolve_agent_rules.py`
  （多路径并集、目录展开、review 任务、只读 `--check`）。
- 命令层：Makefile 追加框架目标（`make help / framework-check /
  framework-ready / ai-doctor / ci-check / setup / evidence / graph / kb /
  ui-smoke` 等），`docs/dev-framework.json` 绑定真实命令；
  `scripts/setup_env.sh` 将 cargo-nextest 与 graphifyy 钉版安装到仓库内
  `.local/tools/`（幂等，`--check` 只读诊断）。
- 文档：`docs/{README,ARCHITECTURE,DEVELOPMENT,MAKE_COMMANDS,TESTING,
  AI_TOOLS,RELEASE}.md` 中文手册；`docs/mkdocs-base.yml` 追加 `exclude_docs`
  使框架文档不进公共文档站。
- 测试与证据：`scripts/generated_check.sh`（派生文件只读校验）、
  `scripts/ui_smoke.sh`（Xvfb 隔离显示 + xwd 截图 + ffmpeg 转 PNG，证据写
  ignored 的 `.ui-evidence/<分支>/<任务>/<批次>/`）；框架脚本自带 unittest
  （`make framework-test`）。
- 图谱与知识库：`scripts/graphify.sh`（钉版 graphifyy，代码-only AST 索引，
  排除生成数据表；指纹新鲜度 `make graph-check`）；`scripts/build_agent_kb.py`
  （受控语料切块 + source_hashes，`make kb / kb-check`）。
- AI 工具面：ZCode / Claude Code / Codex 共享配置（`.zcode/config.json`、
  `.claude/settings.json`、`.codex/config.toml`）与共享 PreToolUse 安全门；
  Kimi 经根 `AGENTS.md` 工作。
- 代码注释规范：`docs/AGENT_RULES/code-comments.md`（上游英文注释保持、
  fork 新增标记、Lua API rustdoc 约定、`模块路径::符号` 引用），并对核心
  模块头补少量中文导读（仅模块头，不动正文，控制上游同步冲突面）。

### Fixed

- `.codex/hooks/pre_tool_use_policy.py` 适配器语言混装修正为真 Python
  （此前为 bash 内容冒充 `.py`，直调引擎探针全绿、按注册入口探针才暴露）；
  注册入口探针、适配器语言完整性（python3→ast.parse、bash→`bash -n`）与
  codex/zcode hooks 配置形状锁（事件数组表、命令嵌套、timeout 秒制、
  `config_file` 注册键）已锁进 `scripts/test_ai_tool_hooks.py`。
- 采纳 skill 更新（2026-09-16）的并行会话纪律：写入仓库前复查并行信号、
  发现并行推进转只读验收不双写、用 ctime/清单快照判定进度；探针输入中的
  危险命令字面量拆分构造（development.md 不变量）。
