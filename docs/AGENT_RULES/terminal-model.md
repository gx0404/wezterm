# terminal-model：终端仿真核心

## 范围

`term/`（wezterm-term）、`wezterm-cell/`、`wezterm-surface/`、
`wezterm-escape-parser/`、`vtparse/`、`assets/shell-integration/`。
数据流：PTY 字节 → escape 解析 → 终端模型 → 供 mux/GUI 消费的行与格子。

## 符号真源

- 入口：`term/src/terminal.rs::Terminal`（`state: TerminalState` + escape
  `Parser` 的组合，Deref 到状态）；`Terminal::advance_bytes` 是字节进入模型的
  唯一常规入口，`perform_actions` 用于重放。
- 状态所有者：`term/src/terminalstate/mod.rs::TerminalState`（屏幕、光标、
  margins、鼠标/键盘编码、palette、image_cache、title、bidi 标记等约 60 字段）。
  动作应用只经 `terminalstate/performer.rs::Performer::perform`（VTActor 桥）。
- 屏幕与行：`term/src/screen.rs::Screen`（scrollback 物理所有者，
  `ScreenOrAlt` 主/备屏）；行存 `wezterm-surface/src/lib.rs::Line`（Vec 与
  ClusteredLine 两种存储，`compress_for_scrollback`/`coerce_vec_storage`）；
  格子 `wezterm-cell/src/lib.rs::Cell`/`CellAttributes`。
- 解析分层：`vtparse/src/lib.rs::VTParser`（DEC 状态机，只分类不赋义）→
  `wezterm-escape-parser/src/parser/mod.rs::Parser`（组装语义 `Action`，可
  编解码回转义序列）→ `term` 的 Performer 应用。`OneBased` 包装 CSI 参数。
- 语义区：`term/src/lib.rs::SemanticZone` 与
  `TerminalState::get_semantic_zones`（SemanticType 为 Output/Input/Prompt）。
- shell-integration：`assets/shell-integration/` 的脚本注入 OSC 133 等序列，
  是 SemanticZone 的上游约定；改脚本要同时核对 escape-parser 的 OSC 分派。

## 不变量

- **seqno**：`Terminal::advance_bytes` 每次调用先 `increment_seqno()`；
  `wezterm-surface::SequenceNo` 只在单个 Surface 实例内有意义。消费方靠
  seqno 判断行脏与否，禁止跳号或复用。
- **行号类型系统**：`PhysRowIndex=usize`（0=scrollback 顶）、
  `VisibleRowIndex=i64`、`ScrollbackOrVisibleRowIndex=i32`、
  `StableRowIndex=isize`（逻辑行号，purge 后仍稳定）。刻意不同宽不同号，
  让编译器拦截混用；换算只经 `Screen` 上的 `phys_row/stable_row_to_phys/
  visible_row_to_stable_row` 家族。
- **写侧线程模型**：`TerminalState.writer` 是 `ThreadedWriter`——把响应写
  侧挪到独立线程，避免大粘贴进 vim 时与输出互锁；不得改回同步写。
- **no_std 面**：`vtparse`、`wezterm-escape-parser`、`wezterm-surface`、
  `wezterm-cell`、`bidi` 保持 `no_std` 可构建（escape-parser 无 default
  feature；`make test` 第二轮就是 no_std 检查）。给这些 crate 加依赖必须
  核对 feature 矩阵（std/alloc/use_serde/use_image/tmux_cc/kitty-shm）。
- **枚举尺寸**：`wezterm-escape-parser/src/lib.rs::Action` 及其变体有
  `size_of` 断言测试（64 位目标）；新增变体注意别把尺寸改爆。
- **Surface resize**：`wezterm-surface::Surface::resize` 使变更流失效
  （seqno+1 清空 changes，下次全量重绘）；`DiffState` 会合并相邻文本、抑制
  冗余光标/属性变更，别在渲染层重复做这类合并。
- **vtparse 上限**：OSC 缓冲有 `MAX_OSC` 上限；加长 OSC 语义时同步评估两端。

## 禁止项

- 不在 `term` 之外的地方直接改 `TerminalState`（GUI/CLI 一律经公开 API 或
  `Change` 流）。
- 不把平台/窗口概念引进本域（输入用 `wezterm_input_types` 的类型即可）。
- 不手改生成数据表（`config/src/scheme_data.rs` 等，见 build-ci-release）；
  本域无生成表，但 emoji/宽度表在 `wezterm-char-props`（font-shaping 域）。
- 选区模型在 GUI 侧（`wezterm-gui/src/selection.rs`）；不要往 term 里加选区。

## 验证

- 定向：`cargo nextest run -p wezterm-term`（TestTerm 脚手架在
  `term/src/test/mod.rs`，k9 快照用 `K9_UPDATE=1` 更新）；
  escape 解析：`cargo nextest run -p wezterm-escape-parser`（含 no_std 轮与
  round-trip 编解码测试）；surface：`-p wezterm-surface`。
- 新增转义/OSC 行为：同时补 performer 侧单测与（若影响 shell-integration）
  人工核对脚本注入序列。
- 影响渲染路径（CellCluster/Line 压缩）时联动 `make ui-smoke` 截图读回。
