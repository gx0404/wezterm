# wezterm（gx0404 fork）

Wez's Terminal Emulator 的 fork：GPU 加速、Lua 配置、内建 mux 的跨平台终端模拟器。

本文件是跨工具启动协议（≤16 KiB）。领域规则正文在 `docs/AGENT_RULES/*.md`，
由路由器按路径解析；本文件的索引仅为导航，实际必读集合以路由器输出为准。

## 规则加载协议

任务开始时对本轮全部触及路径运行 resolver（目录自动展开为 Git 可见文件，多路径
取并集；scope 扩大后必须用完整集合重跑）：

```bash
python3 scripts/resolve_agent_rules.py <paths...>
python3 scripts/resolve_agent_rules.py --task review <paths...>  # 审核任务
python3 scripts/resolve_agent_rules.py --check                   # 闭集/体积守门
```

读完输出列出的每份文档再动手。未知路径/任务、路由缺失以退出码 2 失败是有意
设计。禁止在子目录新增 AGENTS.md；禁止把领域规则正文复制进工具私有目录。

## 语言与协作

- 与人类用其使用的语言交流；代码、标识符保持英文；commit message 的描述用
  中文（`type(scope): 中文描述`，type 前缀保持英文 conventional 格式）。
- 规则真源唯一：领域规则只维护在 `docs/AGENT_RULES/`；工具适配层与 reviewer
  只引用 resolver，不复制清单。
- 先读代码再下结论；长期引用写 `模块路径::符号`，不钉行号。
- 完成前对照「完成门」自检；不确定就报告，不猜测。

## fork 治理

- 本仓库是 `gx0404/wezterm` fork（upstream `wezterm/wezterm`，日常活跃）。
  同步上游用普通 merge/rebase 产生的新提交；**永不 force push、永不推送
  upstream**（hooks 与本节双保险）。
- 控制上游同步冲突面：除 `.gitignore`、`Makefile`、`docs/mkdocs-base.yml`
  三处带标记的追加段外，框架文件一律是上游不存在的新路径；上游文件的语义
  改动只在随上游同步时产生，不主动重排/重注释上游正文（见 code-comments.md）。
- 上游 issue/PR 由人类操作；agent 不代为提交。发布链（ci/tag.sh、
  ci/create-release.sh、ci/deploy.sh）只在上游仓库运行，本 fork 不复建。
- fork 层面的可观察变更记录在根 `CHANGELOG.md`；上游产品变更在
  `docs/changelog.md`（随上游同步产生，不手写）。

## 项目模型

- Cargo workspace 68 包、约 20 万行手写 Rust + 约 21 万行生成数据表
  （`wezterm-gui/src/unicode_names.rs`、`wezterm-char-props/src/emoji_variation.rs`、
  `nerdfonts_data.rs`、`config/src/scheme_data.rs` 等；再生成命令见
  build-ci-release.md 的生成物清单）。
- 数据流主线：PTY 字节 → `term::Terminal::advance_bytes` → escape 解析
  （`vtparse` 状态机 → `wezterm-escape-parser::Action`）→
  `term::terminalstate::TerminalState`/`Screen`（行存 `wezterm-surface::Line`，
  格子 `wezterm-cell::Cell`）→ `MuxNotification::PaneOutput` →
  `wezterm-gui` `TermWindow::paint_impl`（形状缓存→GlyphCache→纹理 atlas）。
- 状态所有者：终端模型在 `term/src/terminalstate/mod.rs::TerminalState`
  （seqno 每次 advance 递增；行号用四种不同类型防混用）；mux 全局单例
  `mux/src/lib.rs::MUX`（pane 树在 `Tab` 的 bintree，GUI 只缓存视口/选区）；
  GUI 窗口状态在 `wezterm-gui/src/termwindow/mod.rs::TermWindow`。
- 配置链：wezterm.lua → `config::lua::make_lua_context`（15 个 lua-api-crates
  经 env-bootstrap 注册）→ `wezterm_dynamic::Value` → `Config::from_dynamic` →
  `ConfigHandle`（generation 计数；Lua 仅主线程，重载经 `LuaPipe` 回主线程）。
- 平台抽象：`window` crate（Linux 上 Connection/Window 是 X11|Wayland 枚举）；
  平台代码只进 `window/os/`、`wezterm-font/locator|shaper`、`pty/src/win` 等
  既定位置。`wezterm-ssh` 构建期 feature 二选一（libssh-rs/ssh2）。
- 工具链：cargo + cargo-nextest（项目钉版装 `.local/tools/`，`make setup`）；
  fmt 需 nightly；测试断言用 k9（`K9_UPDATE=1` 更新快照）。

## 常用命令

| 命令 | 用途 |
|---|---|
| `make check` / `make build` | 快速类型检查 / 构建四个发布二进制 |
| `make test` | cargo nextest 全量（含 escape-parser no_std 轮） |
| `cargo nextest run -p <crate>` | 定向测试（日常迭代首选） |
| `make fmt` / `make lint` | nightly rustfmt 格式化 / --check |
| `make setup` / `make ai-doctor` | 安装项目钉版工具 / 只读诊断 |
| `make framework-check` / `make framework-ready` | 规则闭集守门 / 配置完整门 |
| `make ci-check` | resolver+版本+lint+typecheck+test 聚合门 |
| `make generated-check` | 派生文件（补全/键表/文档索引）只读校验 |
| `make ui-smoke` / `make evidence TASK=x` | Xvfb 截图冒烟 / 分配证据目录 |
| `make graph` / `make graph-check` / `make kb` / `make kb-check` | 图谱与知识库 |
| `make framework-test` | 框架脚本自身 unittest |

全表与前置/副作用见 `docs/MAKE_COMMANDS.md`。

## 跨域硬边界

各条详情见括号内领域文档：

- **行号类型纪律**：`PhysRowIndex/VisibleRowIndex/ScrollbackOrVisibleRowIndex/StableRowIndex`
  刻意不同宽不同号，禁止随手转换；scrollback 物理行只归 `Screen` 所有
  （terminal-model.md）。
- **escape 分层**：`vtparse` 只做状态机分类、`wezterm-escape-parser` 赋语义、
  `term` 应用到模型，三层不得越层引用；escape-parser 保持 no_std 可构建，
  `Action` 枚举有 size_of 断言（escape-parsing 归 terminal-model.md 统述）。
- **mux 单例与生命周期**：pane 输出只在 parse 线程→`perform_actions`→
  `notify_from_any_thread` 链路上推进；pty EOF 清理由 `exit_behavior` 决定且
  必须回主线程执行；`MuxNotification::Empty` 驱动 GUI 退出
  （mux-domain.md）。
- **Lua 线程与重载**：Lua Send 但 !Sync，只允许主线程引用；配置重载失败保留
  旧配置仅更新错误；`TerminalConfiguration::generation` 每次变更必须递增
  （config-lua.md）。
- **GUI 渲染不变量**：pane 树属于 mux，`TermWindow` 只存视口/选区/overlay 缓存；
  纹理耗尽按 `AllowImage` 降级链处理；每类窗口 Lua 事件最多 1 执行 + 1 挂起
  （gui-rendering.md）。
- **平台隔离**：跨平台核心不得出现 `#[cfg(target_os)]`，平台实现只进既定
  `os/`、`locator/`、`win/` 位置；Linux X11/Wayland 走枚举分发
  （platform-window.md）。
- **依赖纪律**：新依赖必须走根 `Cargo.toml` 的 `[workspace.dependencies]` 并
  过 `deny.toml` 许可检查；生成数据表只能经各自 codegen/同步工具重建，不手改
  （build-ci-release.md、code-comments.md）。
- **生成物纪律**：受控产物（shell 补全、键表 markdown、docs 索引、图谱、KB）
  默认只检查（`make generated-check / graph-check / kb-check`），有意变更才
  重建并审 diff（development.md）。

## 提交规范

conventional commit 格式：小写英文 type 前缀（可带 scope），冒号后的描述用
中文；无 emoji，无 AI co-author 行：

```text
fix(term): 修复 DECSTBC 边界行的清除范围
```

只精确暂存本轮相关文件（不 `git add -A`/`-f`）；不 push 除非被明确要求；
push 目标只允许 origin。提交前提出 commit message 并对齐。

## 领域规则索引

| 域 | 覆盖 |
|---|---|
| `build-ci-release` | ci/、.github/、nix/、workspace 依赖与版本、生成物清单 |
| `cli-main` | wezterm/ 主 CLI 与 clap 子命令定义 |
| `config-lua` | config/、lua-api-crates/、wezterm-dynamic、Lua 加载与热重载 |
| `development` | 框架自身：scripts/、AGENT_RULES、Makefile、工具面 |
| `dotfiles` | dotfiles/ 用户环境快照与跨机安装链（gx-bundle/install/sync） |
| `font-shaping` | wezterm-font/、wezterm-char-props/、deps/ 构建辅助 |
| `gui-rendering` | wezterm-gui/ 渲染、overlay、输入处理 |
| `mux-domain` | mux/、wezterm-client/、mux-server、codec 帧协议 |
| `platform-window` | window/、wezterm-input-types/ 平台抽象 |
| `process-io` | pty/、procinfo/、filedescriptor/、umask/ |
| `product-assets` | assets/ 资源（shell-integration 除外） |
| `product-docs` | docs/ 产品文档站（AGENT_RULES 除外） |
| `ssh-domain` | wezterm-ssh/ 与双后端 feature |
| `support-crates` | promise/bidi/lfucache 等支撑库 |
| `terminal-model` | term/、wezterm-cell/、wezterm-surface/、shell-integration |
| `testing` | test-data/ 手动 fixtures 与测试数据纪律 |
| `code-review` | 审核任务专用（`--task review`） |

## 完成门

1. `python3 scripts/resolve_agent_rules.py --check` 通过（scope 扩大后重跑）。
2. `make framework-ready` 通过；改动命令配置后 `make ai-doctor` 无缺项。
3. 格式与静态检查：`make lint`（或说明 nightly 缺失并给出补装命令）；涉及
   类型改动跑 `make check`。
4. 最小针对性测试：`cargo nextest run -p <受影响 crate>`；动了 ssh e2e 相关
   跑 `make test-integration`；全量 `make ci-check` 在合并前跑。
5. 触到生成物（补全/键表/文档索引/图谱/KB/scheme_data）必须重建并审 diff，
   或说明为何无影响。
6. GUI 可见行为变更：`make ui-smoke` 截图并**读回图片**核对，证据留在
   `.ui-evidence/`（ignored）。
7. `git diff --check` 干净；交付说明列出：改动、真源、实际通过的检查、
   未运行项（PENDING）与理由。
