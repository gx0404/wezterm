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
  把事件回调挂到 wezterm-gui 的 `dispatch_window_event`。
- 平台选择：`window/src/os/x_and_wayland.rs`——Linux 上 `Connection`/
  `Window` 是 `X11(..)|Wayland(..)` 枚举，`create_new()` 先 Wayland（config
  `enable_wayland` + feature）失败回落 X11；其余平台 `os/{macos,windows}`。
- 键盘编码：`os/x11/keyboard.rs::XKeymap`（xkbcommon+compose；X keycode 有
  +8 偏移）、`os/xkeysyms.rs::keysym_to_keycode`、各平台 keycodes.rs；
  输入类型真源在 `wezterm-input-types`（window 直接 re-export）。
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
