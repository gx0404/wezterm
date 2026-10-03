# gui-rendering：GUI 前端

## 范围

`wezterm-gui/`：TermWindow 状态、渲染管线、overlay/modal、输入处理、
Lua gui 命名空间。平台窗口与事件抽象在 platform-window 域（`window/`）。

## 符号真源

- 入口：`wezterm-gui/src/main.rs::run_terminal_gui`（建 mux、尝试让既有
  GUI 实例代生、`GuiFrontEnd`、`run_forever`）；子命令定义在
  `wezterm-gui-subcommands`（cli-main 域共享）。
- 窗口状态所有者：`wezterm-gui/src/termwindow/mod.rs::TermWindow`——窗口
  句柄、`ConfigHandle` 与 `config_overrides`、字体 `Rc<FontConfiguration>`、
  `RenderState`（GL 或 WebGPU 二选一）、形状/行缓存（LfuCache）、
  `tab_state`/`pane_state`（每 pane 视口/选区/overlay 缓存）、`modal`、
  `preview_palette`（fork：窗口级易失预览调色板，只经
  `TermWindow::set_preview_palette` 设/清并在那里统一丢色相关缓存；设上
  之后 `palette()`/`pane_palette()` 是渲染取色的唯一入口，不走配置重载。
  fork：连续变更经 100ms 节流合并——palette 赋值与双代数失效在
  `apply_preview_palette` 同一同步步完成，pending 未冲刷前保持旧值，
  不出现错色帧）。
  跨线程通知统一走 `TermWindowNotif`（`Window::notify` → 主线程
  `dispatch_notif`）。
- 渲染：`termwindow/render/paint.rs::paint_impl`（'pass 循环处理
  `OutOfTextureSpace`：重建/增长 atlas → 图像按 AllowImage 链降级）；
  `renderstate.rs::RenderState/TripleLayerQuadAllocator`；webgpu 后端在
  `termwindow/webgpu.rs::WebGpuState`。字形缓存 `glyphcache.rs::GlyphCache`。
- overlay/modal：`termwindow/overlay/`（selector/launcher/copy/quickselect/
  confirm/prompt/debug）与 `termwindow/modal.rs::Modal` trait（fork：
  `Modal::on_dismissed` 是三条关闭路径——Esc、点浮层外、被 `set_modal`
  顶掉——的统一回调，易失状态只在这里还原）；overlay pane
  用 `mux::termwiztermtab::allocate` 造内存终端，跑在独立线程，结束经
  `schedule_cancel_overlay` 回主线程。
- 输入：`termwindow/keyevent.rs`（leader 键 → `inputmap.rs::InputMap` 命中
  → `perform_key_assignment` 大分派；未命中经 dead-key/compose 编码写入
  pane）；鼠标在 `mouseevent.rs`（选区、超链接 `UIItem::hit_test`）。
  粘贴漏斗 `termwindow/clipboard.rs::paste_from_clipboard`（fork：配置
  `clipboard_image_paste` 非 none 时先取剪贴板图片，spawn 线程转码后经
  OSC 1337 `perform_actions` 内联应用或写临时文件粘贴路径，失败回落文本）。
- 前端映射：`frontend.rs::GuiFrontEnd`——`known_windows` 是 GUI Window ↔
  mux window 的映射所有者；订阅 `MuxNotification` 驱动重绘与退出。

## 不变量

- **pane 树归属 mux**：TermWindow 只缓存视口/选区/overlay；布局事实（Tab 的
  bintree、zoom）以 `mux::Tab` 为准，GUI 不得自建第二份布局真源。
- **事件只在平台事件循环线程派发**（`window/src/lib.rs::WindowEventSender`
  契约）；其它线程一律 `Window::notify` 投递 `TermWindowNotif::Apply`。
- **Lua 事件队列**：`EventState::{None,InProgress,InProgressWithQueued}` 每
  类窗口事件最多 1 执行 + 1 挂起；不要绕过该状态直接 emit。
- **前端二选一**：`config.front_end` 决定 glium（OpenGL）或 WebGPU；
  `RenderContext/RenderFrame` 是双后端抽象，改动需两后端同审（无对应环境
  时另一侧记 PENDING，不得声称已验证）。
- **缓存失效代数**：`quad_generation/shape_generation` 递增驱动 LfuCache
  失效；config reload 走 `config_was_reloaded` 刷新字体与形状缓存，新增
  配置相关缓存必须挂进这条链（fork 已挂入：`title_config_cache`）。
- **标题刷新合并**（fork）：`termwindow/title_update.rs`——标题类 Alert 先按
  `window_contains_pane` 过滤，`update_title` 只置 pending，约 40ms 合并窗口
  后执行一次 `update_title_impl`；焦点变化/切 tab/非 live `apply_dimensions`
  直接刷新。format-* 回调共用一份 `TitleFormatArgs`（tabs/panes/config 表，
  handler 视为只读），config 表缓存在 `TermWindow.title_config_cache`。
- **live resize**（fork）：`termwindow/live_resize.rs`——live 期间只 resize
  活动 tab，其余标脏，100ms 静止或切到该 tab 时补齐；布局真源仍是 mux::Tab。
- 纯色背景层（`BackgroundSource::Color`）直接画四边形不占 atlas，
  `ATLAS_SIZE` 初值 1024。文本浮层共享行样式在 `overlay/style.rs`
  （`overlay_selected_*`、`copy_mode_status_*`），滚动条 thumb 在
  `render/scroll_thumb.rs`（`scroll_bar_thumb_width`，命中区仍整宽）。
- **Element 浮层统一样式**（fork）：命令面板/设置/快捷键/壁纸/右键菜单五个
  Modal 一律经 `termwindow/overlay_style.rs::OverlayStyle` 构造外框（圆角
  `overlay_corner_radius` + 描边 `overlay_border_color`）、行（选中 =
  `command_palette_selection_*` 底色 + 左侧 `command_palette_accent_color`
  强调条）、chrome 行与分隔线；可见行数用 `row_px()`，`context_menu.rs::
  menu_box_size` 用 `chrome_px()` 同步内边距，不得各自硬编码尺寸/颜色。
  圆角由 `box_model.rs` 画「底色扇形 + 边框色弧线」，半径用 `Pixels/Points`
  才是正圆（`render/corners.rs::rounded_corners`）。fancy tab bar 标签圆角读
  `window_frame.tab_corner_radius`，关闭按钮悬停色读 `close_button_hover_bg`。
- GL 上下文丢失走 `termwindow/gpu_recovery.rs`：`do_paint` 只记录并
  `notify(RebuildRenderState)`，主线程两段式重建（释放 shape 缓存/RenderState
  → `enable_opengl()` → `RenderState::new` → 代数递增），指数退避最多 8 次。
- 设置浮层写入 `gui-settings.json` 的键必须登记进 `gui_settings::GUI_OWNED_KEYS`
  （现有：wallpaper、color_scheme、default_shell、window_material 等）。
- 统计记录器（fork）：`stats::Stats::init` 只在 `config::common_init` 之后、
  `periodic_stat_logging > 0` 时安装；未安装时 `metrics::*!` 宏是 no-op，热路径不加
  锁。新增统计点不要假设 recorder 一定存在；从 0 改非 0 需重启 GUI。
- **纹理耗尽降级链**：`AllowImage::Yes→Scale(2/4/8)→No`；新图像路径要接进
  该链而不是自行吞错。
- **wezterm.gui 只在 GUI 进程注册**（window-funcs crate 由 main.rs 单独
  add_context_setup_func）；config-lua 域的注册中心不含它。

## 禁止项

- 不在渲染循环里做布局/状态变更（渲染只读 pane 视口）。
- 不新增 `#[cfg(target_os)]` 于本域通用代码——平台差异下沉到 `window/os/`。
- 不直接写 `TerminalState`；GUI 侧选区模型在 `src/selection.rs`，经
  `SelectionCoordinate` 体系操作。

## 验证

- 定向：`cargo nextest run -p wezterm-gui`（少量单测）；类型改动跑
  `make check`。
- 任何可见行为变更：`make ui-smoke`（Xvfb + xwd 截图）并读回图片核对；
  证据目录 `make evidence TASK=<任务名>` 分配，结果登记 images_reviewed。
- 字体/整形相关联动 font-shaping 域验证；窗口行为在 NixOS VM
  （`nix#testing-on-gnome/plasma`）人工核对（见 docs/TESTING.md）。
