# mux-domain：多路复用与客户端/服务端协议

## 范围

`mux/`、`wezterm-client/`、`wezterm-mux-server/`、`wezterm-mux-server-impl/`、
`codec/`、`wezterm-uds/`。GUI 内嵌 mux、独立 mux server、远程客户端共用
同一套 Domain/Pane 抽象。

## 符号真源

- 全局单例：`mux/src/lib.rs::MUX`（`Mux::get/set_mux/shutdown`）。状态所有者
  `Mux::tabs/panes/windows`（RwLock）、domains、subscribers、clients。
- 通知：`mux/src/lib.rs::MuxNotification`（PaneOutput/PaneAdded/…/Empty），
  经 `notify_from_any_thread` 发布；GUI 前端与 CLI 都靠它驱动。
- 输出泵：`read_from_pane_pty`（阻塞读线程）→ socketpair →
  `parse_buffered_data`（escape 解析，含 DECSET 2026 同步输出 hold/flush 与
  coalesce 延迟）→ `send_actions_to_mux` → `Pane::perform_actions`。
- 域与 pane：`mux/src/domain.rs::Domain` trait（LocalDomain、
  `ssh.rs::RemoteSshDomain`）；`pane.rs::Pane` trait +
  `localpane.rs::LocalPane`；`tab.rs::Tab`（`TabInner` 内 bintree pane 树 +
  zoomed）；`termwiztermtab.rs::TermWizTerminal`（overlay 用的内存终端，
  `allocate()` 工厂）。
- 客户端：`wezterm-client/src/client.rs::Client`（unix/TLS/ssh 三种构造；
  RPC 经 `send_pdu`）；`domain.rs::ClientDomain`/`pane/clientpane.rs::ClientPane`
  把远端 mux 镜像成本地 Pane；GUI socket 发现 `discovery.rs`。
- 服务端：`wezterm-mux-server-impl/src/{local,dispatch,sessionhandler,pki}.rs`；
  daemonize 会 re-exec 自身（fork 破坏 smol reactor，见 main.rs 注释）。
- 帧协议：`codec/src/lib.rs::Pdu`（宏生成）与
  `encode_raw_as_vec/decode_raw_async`——leb128 长度（含压缩标记）+ serial +
  ident + bincode payload；serial 单调用于丢弃过期响应。
- ssh agent 转发：`mux/src/ssh_agent.rs::AgentProxy`。

## 不变量

- **生命周期**：pty EOF 后按 `exit_behavior`（Hold/CloseOnCleanExit/Close）
  在**主线程异步**执行清理；`prune_dead_windows` 在 `Activity::count()>0`
  （有启动中任务）时跳过；mux 变空发 `MuxNotification::Empty` 驱动 GUI 退出。
  改退出路径必须保留"启动中不清理"的保护。
- **通知线程边界**：`MuxWindowBuilder` 的 Drop 发 `WindowCreated` 时区分
  主线程直发与 spawn_into_main_thread（Wayland 时序敏感）；不要从任意线程
  直接触碰窗口对象。
- **GUI 内嵌 listener**：GUI 总是起 `gui-sock-{pid}` 的 LocalListener；
  `wezterm start` 先 `discovery::resolve_gui_sock_path` 尝试让既有实例代生
  （SpawnV2，校验 exe/config 一致性）。改动 spawn 协议要同审两端版本兼容
  （`verify_version_compat`）。
- **PDU 兼容**：`codec::Pdu` 是跨进程契约；新增/变更字段考虑旧客户端共存，
  依赖 `GetCodecVersion/Ping` 通道；不要复用 serial。
- **TLS 证书**：`pki.rs::Pki::init` 用 rcgen 每次生成 CA/服务端证书；客户端
  证书经 `GetTlsCreds` 申请。不要把证书/私钥写入仓库或日志。
- **限流**：pane 推流经 `ratelim::RateLimiter`（config 可调）；新推流路径
  必须挂限流，防止远端客户端被淹没。

## 禁止项

- 不在 mux 层引用 GUI/窗口类型（保持 headless 可用：mux-server、CLI 依赖它）。
- 不绕过 `Pane` trait 直接操作 LocalPane 内部 term（跨 crate 用 TerminalView
  等只读接口）。
- 不引入 tokio/多运行时——异步骨架是 `promise` + smol（见 support-crates）。

## 验证

- 定向：`cargo nextest run -p mux -p codec`（tab/pane 树与帧编码单测）。
- 协议变更：`-p wezterm-client` + 手动起 mux-server 连一轮（`wezterm cli list`）。
- TLS 域：`wezterm cli tlscreds` 流程人工核对；证书生成逻辑改动的证据留在
  `.ui-evidence/` 或交付说明（不截图命令行凭据）。
