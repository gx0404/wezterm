# product-docs：产品文档站

## 范围

`docs/`（除 AGENT_RULES 与框架手册外）、`docs-internal/`。上游 mkdocs
（material 主题）+ mdbook 双形态文档站，发布到 wezterm.org（上游 GH Pages）。

## 符号真源

- 站点配置：`docs/mkdocs-base.yml`（strict: true；fork 追加了 exclude_docs
  段排除框架文档）；生成的 `mkdocs.yml`（gitignored）INHERIT 它。
- 导航生成：`ci/generate-docs.py` 扫描 docs/ 与 lua-api-crates 生成索引页
  与 nav；`docs/SUMMARY.md`（mdbook）同源生成。
- Lua API 参考：`docs/config/lua/**`（542 个文件）由 Rust doc 注释经文档
  链路生成——**不是手写文档**。
- changelog：`docs/changelog.md`（上游产品变更，`### Continuous/Nightly`
  未发布段 + `#### Changed/Added/Fixed`；版本名=日期-时间-hash）。
- 宏：`mkdocs_macros.py` 提供 `since` 宏（版本标注，配合
  `docs/releases.json`）。
- 截图：`docs/screenshots/`（静态资产）；配色页由
  `ci/make-color-screen-shots.sh` 手动流程生成。

## 不变量

- **strict 构建**：mkdocs strict=true，警告即失败；新增页面必须进 nav
  （由 generate-docs.py 管）或列入 exclude_docs。
- **gelatyx 扫描**：docs/ 全部 md 的 lua 代码块被 `ci/build-docs.sh` 用
  stylua --check 校验；框架文档（AGENT_RULES 等）因此不写 lua 块。
- **上游文档不手改语义**：本 fork 对 docs/ 的改动限于随上游同步；fork 特有
  说明写在自己的手册（docs/{ARCHITECTURE,DEVELOPMENT,…}.md，已 exclude），
  避免同步冲突。错误修正应走上游 PR（由人类操作）。
- **exclude_docs 维护**：新增框架侧 docs 顶层文件必须同步加进
  mkdocs-base.yml 的 exclude_docs 列表，否则污染公共站。

## 禁止项

- 不手改 `docs/config/lua/**` 与索引页（走 make generated-write 链）。
- 不把 AI 会话链接/内部流程说明写进产品文档。

## 验证

- `make generated-check`（索引/TOC 派生一致）。
- 文档站构建需 docker/podman（ci/build-docs.sh），本地无容器环境时记
  PENDING，上游 pages CI 会最终把关。
