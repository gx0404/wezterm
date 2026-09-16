# code-comments：代码注释与文档注释规范（--task code）

## 适用

所有 Rust/shell/python 源码的注释与 rustdoc。本仓是与上游每日同步的
fork，注释策略的第一目标是**不放大 rebase 冲突面**，第二目标才是可读性。

## 上游文件（上游已存在的文件）

- 保持英文注释；不批量翻译、不重排既有注释、不做"顺手"注释美化——这些
  会让下次同步上游时产生无意义冲突。
- 必须在上游文件里补充说明时：写在紧邻代码处，用英文；fork 特有的语义
  用行注释加 `fork:` 前缀标记，便于同步时快速识别，例如：

```rust
// fork: we route OSC 52 to the pane's own window rather than the
// most recently focused one.
```

- **例外：模块头中文导读**。经用户授权，对少数核心上游文件允许在模块头
  （文件最顶部、原有头注释之后）加一段 `// fork(zh):` 开头的中文导读，
  说明模块职责、状态所有者与边界；每文件一段、控制在 10 行内，不进正文。
  当前登记的导读文件见本文件末尾清单。

- 提交给上游的修复，注释按上游惯例（英文、解释 why 而非 what）。

## fork 新增文件

- 新文件（本框架引入或 fork 功能新增）模块头可用中文导读：说明模块职责、
  状态所有者、与相邻模块的边界；正文注释中英不限，但标识符、日志、错误
  消息保持英文（与代码库一致）。
- 导读注释控制在模块头部，不逐行注释翻译代码。

## rustdoc 与 Lua API 文档（关键生成链）

- `docs/config/lua/**` 由 Rust doc 注释生成：Lua API 面上的公开项（config
  字段、wezterm.* 函数、KeyAssignment 等）的 doc comment 就是产品文档。
  修改这些注释等于修改文档，必须：
  1. 写清语义、取值、默认值与 `{{since('<version>')}}` 标注；
  2. 跑 `make generated-check` 确认派生文档同步（或有意重建后审 diff）。
- 例子代码用 mdbook 风格 fenced block；**docs 内 lua 块必须 stylua 格式**
  （gelatyx --check 门）。

## 生成文件标记

- 生成数据表（`wezterm-gui/src/unicode_names.rs`、
  `wezterm-char-props/src/{emoji_variation,nerdfonts_data}.rs`、
  `config/src/scheme_data.rs` 等）头部自带生成器说明；不要手改正文，也不
  要往里加手写注释（下次重建即丢）。

## 引用约定

- 长期文档与规则引用代码用 `模块路径::符号`（如 `term/src/screen.rs::Screen`），
  不钉行号；审核发现可用当时行号，但注明符号名。
- 注释解释"为什么"与约束（不变量、线程边界、兼容性陷阱），不复述代码
  字面行为；能由类型/测试表达的约束优先落在类型与测试里。

## 验证

- `make lint`（rustfmt 会检查部分注释排版）；Lua API 注释改动跑
  `make generated-check`。
- 新增 fork 导读注释的文件清单在交付说明列出（控制总量，避免噪音）。

## 中文导读登记（fork(zh)，保持小而准）

- `term/src/terminal.rs`：Terminal 组合入口
- `term/src/terminalstate/mod.rs`：终端状态所有者
- `wezterm-escape-parser/src/parser/mod.rs`：解析分层
- `mux/src/lib.rs`：mux 全局单例与输出泵
- `wezterm-gui/src/termwindow/mod.rs`：GUI 窗口状态所有者
- `config/src/lib.rs`：配置加载与重载线程模型
- `codec/src/lib.rs`：客户端/服务端帧协议
- `window/src/lib.rs`：平台窗口抽象
