# .zcode（团队共享，fork 维护）

ZCode 项目配置面。规则入口是仓库根 `AGENTS.md`（ZCode 不经 CLAUDE.md 间接读取，
根文件必须自足）；领域规则经 `python3 scripts/resolve_agent_rules.py <paths...>` 解析。

- `config.json`：仅挂 hooks；`enabled: true` 与毫秒单位 `timeoutMs` 缺一不可
  （缺失或用秒单位 `timeout` 时 hook 静默不执行）。
- 安全门复用 `.claude/hooks/block_dangerous.sh`（策略真源
  `.claude/hooks/dangerous_patterns.conf`，ZCode 与 Claude 输入协议兼容，
  已用无副作用 JSON 探针实测允许/拒绝）。

plans/auth/provider/logs 等个人状态不入库。工具差异与逐项验证记录见 `docs/AI_TOOLS.md`。
