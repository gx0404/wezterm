# wezterm fork 开发文档索引（AI 协作框架）

本目录下的这组文档服务 fork 的开发与 AI 协作，**不进公共文档站**
（`docs/mkdocs-base.yml` 的 `exclude_docs` 已排除）。产品用户文档见
`docs/` 其余部分与 wezterm.org。

| 文档 | 职责 |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | 架构：crate 地图、数据流主线、状态所有者、生成物 |
| [DEVELOPMENT.md](DEVELOPMENT.md) | 开发流程：从验收点到交付的闭环 |
| [MAKE_COMMANDS.md](MAKE_COMMANDS.md) | 全部 make 目标的语义、前置与副作用 |
| [TESTING.md](TESTING.md) | 测试分层、前置、补跑与证据 |
| [AI_TOOLS.md](AI_TOOLS.md) | AI 工具面：规则加载差异、hooks、验证记录 |
| [RELEASE.md](RELEASE.md) | 版本与发布：两套版本体系的边界 |
| `AGENT_RULES/` | 领域规则闭集（`routes.toml` 路由，resolver 解析） |

规则如何加载、领域如何路由，见根 `AGENTS.md` 与 `docs/AGENT_RULES/README.md`。
框架配置真源：`docs/dev-framework.json`。
