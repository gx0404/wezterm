---
description: AI 工具面与框架自身规则入口
paths:
  - ".claude/**"
  - ".codex/**"
  - ".zcode/**"
  - ".agents/**"
  - "docs/AGENT_RULES/**"
  - "docs/dev-framework.json"
  - "scripts/**"
  - "Makefile"
---

本机薄适配：运行 `python3 scripts/resolve_agent_rules.py <paths...>` 并阅读输出；规则正文只在 `docs/AGENT_RULES/development.md`。危险模式改动必须重跑 `python3 -m unittest discover -s scripts -p test_ai_tool_hooks.py`。
