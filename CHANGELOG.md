# Changelog（gx0404/wezterm fork）

本文件记录 fork 层面已实现的可观察变更（AI 协作框架、开发流程、定制功能）。
上游 WezTerm 的产品级变更见 `docs/changelog.md`（随上游同步，不在本文件重复维护，
避免同步上游时产生冲突）。

版本真源：本文件 `## X.Y.Z(日期|TBD)` 标题中的最大 SemVer（`scripts/version.py`，
`make version` 只读查询）。WezTerm 产品自身的版本号由 `wezterm-version/build.rs`
按 git 提交时间与哈希生成，两套体系互不干扰（见 docs/RELEASE.md）。

## 0.1.0(TBD)

### Added

- 初始化 AI 协作开发框架：根 `AGENTS.md` 启动协议 + `docs/AGENT_RULES/`
  领域规则与 `routes.toml` 路由闭集 + `scripts/resolve_agent_rules.py`
  （多路径并集、目录展开、review 任务、只读 `--check`）。
- 命令层：Makefile 追加框架目标（`make help / framework-check /
  framework-ready / ai-doctor / ci-check / setup / evidence / graph / kb /
  ui-smoke` 等），`docs/dev-framework.json` 绑定真实命令；
  `scripts/setup_env.sh` 将 cargo-nextest 与 graphifyy 钉版安装到仓库内
  `.local/tools/`（幂等，`--check` 只读诊断）。
- 文档：`docs/{README,ARCHITECTURE,DEVELOPMENT,MAKE_COMMANDS,TESTING,
  AI_TOOLS,RELEASE}.md` 中文手册；`docs/mkdocs-base.yml` 追加 `exclude_docs`
  使框架文档不进公共文档站。
- 测试与证据：`scripts/generated_check.sh`（派生文件只读校验）、
  `scripts/ui_smoke.sh`（Xvfb 隔离显示 + xwd 截图 + ffmpeg 转 PNG，证据写
  ignored 的 `.ui-evidence/<分支>/<任务>/<批次>/`）；框架脚本自带 unittest
  （`make framework-test`）。
- 图谱与知识库：`scripts/graphify.sh`（钉版 graphifyy，代码-only AST 索引，
  排除生成数据表；指纹新鲜度 `make graph-check`）；`scripts/build_agent_kb.py`
  （受控语料切块 + source_hashes，`make kb / kb-check`）。
- AI 工具面：ZCode / Claude Code / Codex 共享配置（`.zcode/config.json`、
  `.claude/settings.json`、`.codex/config.toml`）与共享 PreToolUse 安全门；
  Kimi 经根 `AGENTS.md` 工作。
- 代码注释规范：`docs/AGENT_RULES/code-comments.md`（上游英文注释保持、
  fork 新增标记、Lua API rustdoc 约定、`模块路径::符号` 引用），并对核心
  模块头补少量中文导读（仅模块头，不动正文，控制上游同步冲突面）。
