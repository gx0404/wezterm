---
description: 发布与上游同步安全入口
paths:
  - "ci/**"
  - ".github/**"
  - "nix/**"
  - "docs/changelog.md"
  - "wezterm-version/**"
---

本机薄适配：运行 `python3 scripts/resolve_agent_rules.py <paths...>` 并阅读输出（会命中 build-ci-release）。fork 永不推送 upstream；发布链只在上游仓库运行。
