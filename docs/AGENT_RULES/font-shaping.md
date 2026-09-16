# font-shaping：字体加载与整形

## 范围

`wezterm-font/`、`wezterm-char-props/`（含 codegen/）、`deps/`（cairo、
fontconfig、freetype、harfbuzz 的构建辅助 crate 与上游子模块）。

## 符号真源

- 状态所有者：`wezterm-font/src/lib.rs::FontConfiguration`（壳）+
  私有 `FontConfigInner`（TextStyle→`Rc<LoadedFont>` 缓存、字体库、locator、
  专用实体字体）；`LoadedFont`（fallback 链 + `Box<dyn FontShaper>` +
  惰性光栅化器）。
- shaping 接口：`wezterm-font/src/shaper/mod.rs::FontShaper` trait
  （`shape/metrics/metrics_for_idx`），harfbuzz 实现 `shaper/harfbuzz.rs`。
- 风格匹配：`FontConfiguration::match_style`（config 的 font_rules 把
  CellAttributes 映射到 TextStyle）。
- 平台门控：`locator/`（unix fontconfig / macos core_text / windows gdi，
  `new_locator()` 分发）、`rasterizer/`（freetype/colr）、mac 依赖在
  Cargo.toml target 段；features `vendor-jetbrains/roboto/noto-emoji/
  nerd-fonts-symbols`。
- 属性表：`wezterm-char-props/src/{emoji_variation,nerdfonts_data,widechar_width,
  emoji_presentation}.rs`——emoji_variation/nerdfonts_data 是 codegen 产物
  （`wezterm-char-props/codegen`：`cargo run > ../src/emoji_variation.rs`）。
- deps/：`deps/cairo` 是 cairo-sys-rs 补丁版（[patch.crates-io] 指向）；
  其余为静态链接系统图形库的 build 辅助；子模块内容不手改。

## 不变量

- **异步 fallback 链**：shape 发现缺字形 → `schedule_fallback_resolve` 投
  专用线程（locator+font_dirs+built_in 三路、按 coverage 排序去重）→ 结果
  回 `pending_fallback` → 下次 shape 重建 shaper 并返回
  `Err(ClearShapeCache{})`，GUI 收到后清形状缓存重排。不得在渲染线程同步
  做字体搜索。
- **缓存失效**：`FontConfiguration::change_scaling/config_changed` 负责清
  缓存；新增缓存字段必须接进这两个入口。
- **字形聚类契约**：`wezterm-surface::CellCluster`（byte_to_cell_idx 映射）
  是 shaping 的输入；bidi run 方向来自 `wezterm-bidi`（support-crates）。
- **生成表**：emoji/宽度/nerdfonts 数据只能经 codegen 重建；手改会在下次
  重建时丢失并污染 diff。
- vendored 字体 feature 与 `assets/fonts` 资源（product-assets 域）联动：
  改 vendor 开关要同步核对打包脚本引用。

## 禁止项

- 不在通用 shaping 代码里出现平台 cfg——平台只进 locator/rasterizer 既定
  位置。
- 不新增同步阻塞的字体网络/磁盘全盘扫描路径。
- 不手改 deps/ 子模块与生成数据表。

## 验证

- 定向：`cargo nextest run -p wezterm-font -p wezterm-char-props`。
- 视觉回归：`make ui-smoke` 截图读回；`wezterm-gui ls-fonts` 输出冒烟。
- codegen 改动：在 codegen 目录重跑生成 → `git diff` 审数据表变化。
