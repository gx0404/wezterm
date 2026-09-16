# .agents

跨工具共享说明：用户级 skills（graphify、update-ai-settings 等）安装在
`~/.agents/skills/`，不进仓库。本仓库的规则真源是根 `AGENTS.md` +
`docs/AGENT_RULES/`（经 `scripts/resolve_agent_rules.py` 解析），任何工具的
适配层都只引用 resolver，不复制清单。工具差异与验证记录见 `docs/AI_TOOLS.md`。
