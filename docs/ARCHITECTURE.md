# 架构（fork 维护）

> 事实以当前 checkout 源码为准；本文给导航与主线。符号引用格式
> `模块路径::符号`。

## 定位

WezTerm：GPU 加速、Lua 配置、内建多路复用（mux）的跨平台终端模拟器。
本 fork（gx0404/wezterm）与上游 wezterm/wezterm 保持同步，叠加 AI 协作
开发框架与少量定制。

## Workspace 布局（68 包）

- **终端仿真核心**：`term/`（wezterm-term，模型与状态机应用层）、
  `wezterm-escape-parser/`（转义序列→语义 Action，no_std）、`vtparse/`
  （DEC 状态机，no_std）、`wezterm-cell/`（Cell/CellAttributes）、
  `wezterm-surface/`（Line/Surface/Change，no_std）、`termwiz/`
  （可独立发布的终端基础库，re-export 上述 crate）。
- **GUI**：`wezterm-gui/`（TermWindow、渲染管线、overlay/modal、输入）、
  `window/`（平台窗口抽象：X11/Wayland 枚举 + macos + windows）、
  `wezterm-font/`（字体加载/shaping/fallback）、`wezterm-input-types/`。
- **多路复用**：`mux/`（全局单例 Mux、Domain/Pane/Tab）、
  `wezterm-client/`（RPC 客户端与 GUI socket 发现）、
  `wezterm-mux-server{,-impl}/`（服务端与 TLS pki）、`codec/`（帧协议）、
  `wezterm-uds/`。
- **配置**：`config/`（+`derive/`）、`luahelper/`、`lua-api-crates/`
  （15 个 Lua API 面 crate）、`wezterm-dynamic/`（动态值层）、
  `env-bootstrap/`、`sync-color-schemes/`（配色数据生成器）。
- **进程/IO**：`pty/`（portable-pty，含串口）、`procinfo/`、
  `filedescriptor/`、`umask/`。
- **SSH**：`wezterm-ssh/`（libssh-rs/ssh2 双后端）。
- **CLI**：`wezterm/`（主 CLI + 21 个 cli 子命令）、
  `wezterm-gui-subcommands/`（GUI/CLI 共享 clap 定义）。
- **支撑**：`promise`（异步骨架，smol 系）、`bidi`（UAX#9）、`bintree`
  （pane 布局树）、`lfucache`、`rangeset`、`ratelim`、`base91`、
  `color-types`、`wezterm-blob-leases`（图片租约）等。
- **构建辅助**：`deps/`（cairo 补丁版 cairo-sys-rs、freetype/harfbuzz/
  fontconfig 静态链接辅助 + 4 个上游子模块）、`wezterm-version/`
  （版本 build.rs）。

## 数据流主线

1. **输出（PTY→屏幕）**：子进程字节 → `mux` 读线程
   （`read_from_pane_pty` → socketpair → `parse_buffered_data`，含
   DECSET 2026 同步输出与合并延迟）→ `Pane::perform_actions` →
   `term::Terminal::advance_bytes`（escape 解析 →
   `terminalstate::Performer::perform` → `TerminalState`/`Screen`）→
   `MuxNotification::PaneOutput` → GUI 失效 → `TermWindow::paint_impl`
   （CellCluster 聚类 → 字体 shaping（bidi run 方向）→ GlyphCache →
   纹理 atlas → quad）。
2. **输入（键盘→PTY）**：平台事件（xkbcommon/Win32/Cocoa）→
   `window::WindowEvent::KeyEvent` → `TermWindow` keyevent（leader →
   `InputMap` → `KeyAssignment` 或编码写入 pane.writer）→ pty/ssh。
3. **配置**：wezterm.lua → `config::lua::make_lua_context`（15 个
   lua-api-crates 注册；严格 config_builder 即时校验）→
   `wezterm_dynamic::Value` → `Config::from_dynamic` → `ConfigHandle`
   （generation；fs-watch 200ms 去抖重载，失败保留旧配置）。
4. **进程拓扑**：GUI 总是内嵌 mux + `gui-sock-{pid}` listener；
   `wezterm start` 先试让既有实例代生（discovery + SpawnV2，校验
   exe/config 一致）；远端域经 `codec` 帧协议（leb128 len/serial/ident +
   bincode，可 zstd 压缩）复用同一 Domain/Pane 抽象。

## 状态所有者（改状态前先对表）

| 状态 | 所有者 |
|---|---|
| 终端模型（屏幕/光标/margins/palette/图像缓存） | `term/src/terminalstate/mod.rs::TerminalState` |
| scrollback 物理行 | `term/src/screen.rs::Screen` |
| pane/tab/window 拓扑 | `mux/src/lib.rs::MUX`（单例）+ `tab.rs::Tab`（bintree） |
| GUI 窗口/视口/选区缓存 | `wezterm-gui/src/termwindow/mod.rs::TermWindow` |
| Window↔mux window 映射 | `wezterm-gui/src/frontend.rs::GuiFrontEnd::known_windows` |
| 字体解析与缓存 | `wezterm-font/src/lib.rs::FontConfigInner` |
| 配置与重载 | `config/src/lib.rs::ConfigInner`（Lua 仅主线程） |

## 关键不变量（摘要）

- seqno：每次 `advance_bytes` 递增；`SequenceNo` 只在单个 Surface 内有意义。
- 行号四类型（Phys/Visible/ScrollbackOrVisible/Stable）防混用。
- 事件只在平台事件循环线程派发；跨线程用 `Window::notify`。
- Lua Send 但 !Sync：主线程专用，重载经 `LuaPipe` 回运。
- 异步骨架唯一：promise + smol；不引入 tokio。
- 平台代码只进 `os/`、`locator/`、`rasterizer/`、`win/` 既定位置。

## 生成物（不手改）

| 产物 | 生成器 |
|---|---|
| `wezterm-gui/src/unicode_names.rs` | 上游 codegen |
| `wezterm-char-props/src/{emoji_variation,nerdfonts_data}.rs` | `wezterm-char-props/codegen` |
| `config/src/scheme_data.rs` | `sync-color-schemes`（联网） |
| `assets/shell-completion/*`、`docs/examples/*key-table*`、`docs/cli/*` help | `ci/update-derived-files.sh`（用构建出的 wezterm） |
| `docs/**/index.md`、`docs/SUMMARY.md`、`mkdocs.yml` | `ci/generate-docs.py` |
| `docs/config/lua/**` | Rust doc 注释链 |
| `graphify-out/*`、`kb/chunks.json` | `make graph` / `make kb`（fork） |

## 图谱与知识库

- 代码图谱：`make graph`（graphifyy 钉版，代码-only AST，排除表
  `.graphifyignore`）；查询 `scripts/graphify.sh query "..."`。
- 知识库：`kb/chunks.json`（`make kb` 重建，语料=规则+手册+CHANGELOG+
  crate 地图）。
