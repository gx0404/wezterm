# cli-main：主 CLI 与子命令

## 范围

`wezterm/`（主 CLI 二进制：21 个 cli 子命令、asciicast 录制回放、tls 凭据）、
`wezterm-gui-subcommands/`（GUI/CLI 共用的 clap 定义）。

## 符号真源

- 入口：`wezterm/src/main.rs::main → run`——`designate_this_as_the_main_thread`
  → `env_bootstrap::bootstrap()` → UmaskSaver → clap `Opt`；结束 `Mux::shutdown()`。
- GUI 类子命令（start/ssh/serial/connect/ls-fonts/show-keys）经
  `delegate_to_gui` **exec 同目录 wezterm-gui 二进制并透传参数**（Windows 加
  `--attach-parent-console`）。
- cli 子命令：`wezterm/src/cli/mod.rs::CliSubCommand`（list、list-clients、
  proxy、tlscreds、move-pane-to-new-tab、split-pane、send-text、get-text、
  activate-pane-direction、get-pane-direction、kill-pane、activate-pane、
  adjust-pane-size、activate-tab、set-tab-title、set-window-title、
  rename-workspace、zoom-pane），每命令一个同名文件，`cmd.run(client).await`。
- 录制回放：`wezterm/src/asciicast.rs`（Record/Play + 平台 TTY 包装）。
- 子命令契约：`wezterm-gui-subcommands` 是 GUI 与 CLI 子命令一致性的单一
  事实源（`DEFAULT_WINDOW_CLASS`、`name_equals_value` 解析等）。

## 不变量

- `--config name=value` 覆盖经 `config::common_init` 的 overrides 生效；
  cli 输出格式 `table|json` 的字段是脚本契约，改名/删列是破坏性变更。
- `delegate_to_gui` 假设 wezterm-gui 与 wezterm 同目录同版本；不要引入
  跨版本混用的查找逻辑。
- cli 子命令都依赖 `wezterm-client::Client`（连 GUI socket 或 mux server）；
  连不上时的错误信息要可操作（提示 `wezterm start` 或 unix domain 路径）。

## 禁止项

- 不在 CLI 进程里链接 GUI 渲染栈（保持 headless 可用、启动快）。
- 不复制子命令定义到 wezterm/（改 wezterm-gui-subcommands 共享源）。

## 验证

- 定向：`cargo nextest run -p wezterm`。
- cli 行为：起 `make dev`（GUI）后 `wezterm cli list/split-pane/get-text`
  冒烟；输出格式变更在交付说明中列出新旧对照。
