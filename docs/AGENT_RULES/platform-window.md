# platform-window：平台窗口与事件抽象

## 范围

`window/`（X11/Wayland/Windows/macOS 窗口与事件循环、位图 atlas）、
`wezterm-input-types/`（KeyCode/KeyEvent/Modifiers/MouseEvent 定义）、
`wezterm-toast-notification/`（系统通知）。

## 符号真源

- 连接层：`window/src/connection.rs::ConnectionOps`（thread-local `CONN`、
  消息循环、DPI：macOS 72 其余 96、外观、屏幕枚举）。
- 窗口层：`window/src/lib.rs::WindowOps` trait + `WindowEvent` 枚举 +
  `WindowState` bitflags（can_resize/can_paint 门）；`WindowEventSender`
  把事件回调挂到 wezterm-gui 的 `dispatch_window_event`。剪贴板文本走
  `get_clipboard/set_clipboard`；fork 增 `get_clipboard_image`（默认实现
  解析 None，Windows 实现按 注册PNG → CF_DIBV5 → CF_DIB 取原始字节、
  后台线程读取，window 层不引 `image` 依赖、只还字节）。
- 平台选择：`window/src/os/x_and_wayland.rs`——Linux 上 `Connection`/
  `Window` 是 `X11(..)|Wayland(..)` 枚举，`create_new()` 先 Wayland（config
  `enable_wayland` + feature）失败回落 X11；其余平台 `os/{macos,windows}`。
- 键盘编码：`os/x11/keyboard.rs::XKeymap`（xkbcommon+compose；X keycode 有
  +8 偏移）、`os/xkeysyms.rs::keysym_to_keycode`、各平台 keycodes.rs；
  输入类型真源在 `wezterm-input-types`（window 直接 re-export）。
- vsync 关闭（fork）：Windows WGL 上下文创建后调 `wglSwapIntervalEXT(0)`
  （`os/windows/wgl.rs`），与 `egl.rs::SwapInterval(0)`、macOS 一致——帧率
  节流统一交给 GUI 的 max_fps 机制，不要在平台层重新启用 vsync（paint 会
  在 wndProc 内同步等 vblank 卡住消息循环）。
- 帧节流（fork，Windows）：`wm_paint` 节流期间 `ValidateRect` 避免
  `WM_PAINT` 空转；节流定时器是 `Connection.frame_timer`
  （`CreateWaitableTimerExW` HIGH_RESOLUTION，并入 `wait_message` 等待集，
  deadline 自本帧开始按 `effective_frame_interval(config.max_fps,
  max_fps_follows_display, monitor_refresh_hz)` 计算，创建失败回退 async-io
  定时器）；`Connection::create_new` 调 `timeBeginPeriod(1)`。
  `WindowInner.monitor_refresh_hz` 在创建、`WM_DISPLAYCHANGE`、换显示器时
  刷新。`default_dpi` 取光标所在显示器（尊重 `dpi_by_screen`/`dpi`），
  `WM_DPICHANGED` 采纳系统建议矩形。WGL `PixelFormatProfile::{Lean,Legacy}`
  先请求无 MSAA/深度/模板再回退，探测结果 thread_local 缓存、探测窗口销毁。
- 混合显卡（fork）：`os/windows/mod.rs` 导出 `NvOptimusEnablement` /
  `AmdPowerXpressRequestHighPerformance` 两个 `#[no_mangle] #[used]` static，
  `wezterm-gui/build.rs` 按 target env 用 `.def`（gnu）或 `/EXPORT`（msvc）
  把它们放进 exe 导出表（`objdump -p` 可见）；`Connection::create_new` 经
  `hybrid_gpu_hints()` 引用一次防止被链接器丢弃。
- Windows 外观与集成（fork）：`apply_theme` 在 `win32_frame_follow_colors`
  开启且 Win11 时设 DWM 35/36/34 颜色与 33 圆角偏好（按
  `window_frame.active_titlebar_bg` 亮度判深浅）；`request_attention` 用
  `FlashWindowEx`，`WindowOps::set_progress(ProgressState)` 默认空实现、
  Windows 用 `ITaskbarList3`；IME 位置重放 `WindowInner.last_ime_rect`；
  剪贴板读写都在后台线程退避重试；滚轮行数来自 `SPI_GETWHEELSCROLLLINES`
  并在 `WM_SETTINGCHANGE` 失效。
- 系统背景材质支持（fork）：`ConnectionOps::system_backdrop_support()` 是不带 self 的
  关联函数（配置在 Connection 创建前、以及 watcher 线程上求值），默认全 false；
  Windows 实现 `os/windows/window.rs::backdrop_support_for` 逐条对应 `apply_theme`
  的分支（`IS_WIN11_22H2` 走 DWMWA_SYSTEMBACKDROP_TYPE 三种都支持；否则亚克力要求
  build ≥ 17134 的 ACCENT_POLICY，`!IS_WIN10` 用 DWMWA_MICA_EFFECT 支持云母）。改
  `apply_theme` 的版本分支必须同步改它与 `backdrop_support_tests`。
- 鼠标侧键（fork）：Windows 的 `WM_XBUTTONDOWN/UP/DBLCLK` 经
  `os/windows/window.rs::mouse_button_event_kind` 映射为 `MousePress::X1/X2`，处理后
  按 MSDN 返回 TRUE；按住状态只取 wparam 低字（`mouse_buttons_from_wparam`），高字
  是发生变化的 X 键或滚轮增量；`WM_NCXBUTTON*` 不处理。其它平台目前不产生侧键
  按下事件（macOS 只置 `MouseButtons::X1/X2` 位；Wayland 的穷举 match 已补分支）。
- IME 预编辑（fork）：`DeadKeyStatus::Composing { text, cursor }` 的 `cursor` 是
  `text` 内的终端列偏移（不是字节或 UTF-16 单元），平台不提供时为 None。Windows 由
  `ImmContext::composing_status` 读 `GCS_CURSORPOS`，经 `utf16_offset_to_columns`
  换算（列宽规则与 GUI 一致，只为此在 Windows 目标依赖 `wezterm-cell` 的
  `unicode_column_width`，不引入 Cell/Grid 模型）；x11、wayland、macOS 目前为 None。
- 纹理契约：`window/src/bitmaps/atlas.rs::Atlas`（`allocate()` 失败给
  `OutOfTextureSpace`）+ `bitmaps/mod.rs::Texture2d` trait——GUI 的 atlas
  降级链依赖该契约。
- promise 调度：`window/src/spawn.rs::SPAWN_QUEUE` + 
  `connection.rs::register_promise_schedulers`（future 排队到主循环）。

## 不变量

- **事件线程**：事件只能在平台事件循环线程派发（WindowEventSender 契约）；
  其它线程用 `WindowOps::notify` 投递任意 `Any + Send + Sync` 通知。
- **枚举分发**：Linux 双后端的行为差异封在枚举 match 里；不得让上层直接
  触碰 x11/wayland 私有类型。
- **键盘映射一致性**：keycode→KeyCode 的表是输入兼容面；修键映射必须带
  平台与布局说明（回归风险高，先查上游 issue）。
- **DPI/外观**：`default_dpi` 与 `get_appearance` 是缩放与深浅色的依据，
  改动会全局影响渲染尺寸计算。

## 禁止项

- 通用代码不得出现 `#[cfg(target_os)]`/`#[cfg(windows)]`——平台实现只进
  `os/` 子目录（wezterm-gui 同样约束，见 gui-rendering）。
- 不在本 crate 引入终端模型概念（Cell/Grid）；它只服务窗口/输入/位图。

## 验证

- 定向：`cargo nextest run -p window`（用例少）；`make check`。
- 平台行为：本机 X11 冒烟 `make ui-smoke`；Wayland/macOS/Windows 无环境时
  记 PENDING 并注明由上游 CI（gen 工作流矩阵）佐证。
