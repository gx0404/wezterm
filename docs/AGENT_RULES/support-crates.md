# support-crates：支撑库

## 范围

`promise/`、`filedescriptor` 之外的小型支撑库：`bidi/`、`bintree/`、
`rangeset/`、`lfucache/`、`frecency/`、`tabout/`、`base91/`、`ratelim/`、
`async_ossl/`、`color-types/`、`strip-ansi-escapes/`、`wezterm-open-url/`、
`wezterm-blob-leases/`。（filedescriptor/procinfo/umask 在 process-io 域。）

## 各自一句话

- `promise`：单线程 promise/future + `spawn/spawn_into_main_thread/
  ScopedExecutor/block_on`——**全仓异步骨架**（基于 smel/async-io，不是
  tokio）。
- `bidi`：UAX#9 双向文本（`BidiContext::resolve_paragraph`），
  `ParagraphDirectionHint` 供 term/字体/GUI 消费；tests/conformance.rs 跑
  Unicode 官方 BidiTest/BidiCharacterTest。
- `bintree`：Zipper 二叉树——mux Tab 的 pane 布局数据结构。
- `rangeset`：相邻整数合并的区间集合——字体 coverage 判断。
- `lfucache`：LFU 缓存（容量可从 ConfigHandle 读、命中率上报）；GUI 形状/
  行/四边形缓存都用它。
- `frecency`：频率+新近打分——选择器历史排序。
- `tabout`：文本表格对齐；`base91`：basE91 编解码（Lua/插件数据）。
- `ratelim`：令牌桶限流（mux 推流）。
- `async_ossl`：openssl 流的 async 适配（TLS mux 域）。
- `color-types`（crate wezterm-color-types）：SrgbaPixel/SrgbaTuple/
  LinearRgba 颜色数学（sRGB↔linear）。
- `strip-ansi-escapes`：ANSI 过滤（bin + lib）。
- `wezterm-open-url`：跨平台打开 URL/文件。
- `wezterm-blob-leases`：大数据 blob 租约（imgcat/Sixel 图片存 mux 侧，
  lease id 传 GUI 解码）。

## 不变量

- **异步骨架唯一**：全仓只用 promise+smol；新代码不得引入 tokio/async-std。
- **no_std 面**：bidi 是 no_std；保持 alloc-only。
- **纯库纪律**：这些 crate 不依赖 term/mux/GUI（依赖方向只能向下/向旁）；
  它们多数可独立发布，公开 API 变更按破坏性变更对待。
- **语义稳定**：bidi/conformance、rangeset、base91 属于"跨实现 oracle"性质
  ——行为变更必须带对应用例更新与理由，不许顺手改。

## 禁止项

- 不为局部需求 fork/复制这些库的语义到调用方。
- 不在支撑库里加 config 具体字段的直接读取（除了 lfucache 既有的容量
  约定）。

## 验证

- 定向：`cargo nextest run -p wezterm-bidi -p lfucache -p rangeset -p base91`。
- bidi 行为变更：conformance 数据不得裁剪来"修"失败。
