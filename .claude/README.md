# .claude（团队共享，fork 维护）

Claude Code 的项目配置面。规则架构：根 `CLAUDE.md`（`@AGENTS.md` 薄入口）+
`docs/AGENT_RULES/*.md` 领域规则（由 `scripts/resolve_agent_rules.py` 按路径解析）。

- `settings.json`：权限 allow/ask/deny 与 PreToolUse 安全门（团队共享，无个人状态）。
- `hooks/dangerous_patterns.conf`：危险模式唯一真源（SHELL/FILE 两段）。
- `hooks/block_dangerous.sh` → `hooks/pre_tool_use_gate.py`：Claude 协议适配。
- `rules/*.md`：带 `paths:` 的薄提醒，正文一律在 docs/AGENT_RULES/。
- `agents/code-reviewer.md`：只读评审 subagent（无 Edit/Write 工具）。

个人状态（`settings.local.json`、会话、日志）不入库；ZCode 的 `.zcode/config.json`
与 Codex 的 `.codex/hooks/` 复用同一策略真源。工具差异表见 `docs/AI_TOOLS.md`。
