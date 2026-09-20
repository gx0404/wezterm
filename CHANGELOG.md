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
  （字号 0.5 步进增减与重置）。预览走每窗口 `config_overrides`
  （易失），应用写入 `gui-settings.json` 并 `config::reload()` 全局
  生效、跨重启持久化；Tab 切换分区、↑↓ 选择、Enter 应用、鼠标
  悬停/点击与命令面板共用 Modal 通道，配置重载（含语言切换）即时
  重绘浮层文案。
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

- 修复 tab 拖拽重排完全失效：`mouseevent.rs` 的 Release(Left) 分支与
  `TermWindow::finish_tab_drag` 各 `take()` 了一次 `tab_drag`，调用方先取空
  之后 `finish_tab_drag` 必定提前返回，「按住左键拖拽重排 tab」是死路径。
  `take()` 收敛到 `finish_tab_drag` 一处；插入位计算抽成纯函数
  `mouseevent.rs::drop_index`（顺带修好指针拖出窗口左侧时 `x as usize`
  回绕、被当成「拖到最右」的边界），配 7 条单测（最右/最左/原位/越界/
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
