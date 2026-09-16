# .codex（团队共享，fork 维护）

Codex 项目配置面。规则入口是仓库根 `AGENTS.md`（Codex 原生读取）。

- `config.toml`：审批策略、只读评审 agent 注册、环境变量清洗（排除凭据类）与
  PreToolUse 安全门。hooks 事件必须用数组表 `[[hooks.PreToolUse]]`，timeout 按秒。
- `hooks/pre_tool_use_policy.py`：Codex 协议适配器，策略真源复用
  `.claude/hooks/dangerous_patterns.conf`（Codex 的 apply_patch 无 file_path 字段，
  FILE 类模式靠 SHELL 重定向规则兜底，是已知边界）。
- `agents/wezterm-reviewer.toml`：read-only + approval never 的只读评审员。

用户级模型/provider/认证留在 `~/.codex/config.toml`。工具差异表见 `docs/AI_TOOLS.md`。
