# AI 工具面

共享策略（shared）：无凭据配置入库，个人状态本机保留。规则真源唯一：
根 `AGENTS.md` + `docs/AGENT_RULES/`（resolver 解析）。

## 工具加载差异

| 工具 | 入口 | 领域规则获得方式 | 项目配置 |
|---|---|---|---|
| ZCode | 根 AGENTS.md（不经 CLAUDE.md） | resolver（显式跑） | `.zcode/config.json`（hooks） |
| Claude Code | `CLAUDE.md` 首行 `@AGENTS.md` | resolver + `.claude/rules/*.md` 薄提醒（带 paths frontmatter） | `.claude/settings.json`（权限+hooks） |
| Codex | 根 AGENTS.md（原生读取） | resolver（显式跑） | `.codex/config.toml`（审批+env 清洗+hooks+reviewer） |
| Kimi | 根 AGENTS.md | resolver（显式跑） | 无目录需求；`.kimi-code/` 本机 |

## hooks（PreToolUse 安全门）

- 策略真源：`.claude/hooks/dangerous_patterns.conf`（SHELL/FILE 两段，
  deny/ask 两级）。
- 适配：`.claude/hooks/block_dangerous.sh` → `pre_tool_use_gate.py`
  （claude 协议）；`.codex/hooks/pre_tool_use_policy.py`（codex 协议，
  permissionDecision 字段）；`.zcode/config.json` 直接复用 claude 形
  （`hooks.enabled: true` + 毫秒 `timeoutMs`，缺失即静默不执行——这是
  已知坑，配置里两字段都必须在）。
- Codex 注意：hooks 必须写成数组表 `[[hooks.PreToolUse]]` + 嵌套
  `[[hooks.PreToolUse.hooks]]`，timeout 按秒；apply_patch 无 file_path
  字段，FILE 类模式靠 SHELL 重定向规则兜底（已知边界）。
- 改模式必须跑：`python3 -m unittest discover -s scripts -p
  test_ai_tool_hooks.py`（含允许/拒绝探针）。

## 权限面（Claude settings.json 要点）

- allow：只读 git、resolver、框架检查类 make、cargo check/nextest/fmt --check。
- ask：push/commit、setup/graph/kb（写产物）、ci-check/build/test（耗时长、
  会起 sshd）、cargo run/build、rm。
- deny：读 .env/.pem/.git。

## 逐客户端验证记录

| 项 | ZCode | Claude | Codex | Kimi |
|---|---|---|---|---|
| 配置语法解析 | PASS（JSON） | PASS（JSON） | PASS（TOML） | N/A（无目录） |
| hooks 探针（无副作用 JSON） | PASS（离线探针：`python3 -m unittest discover -s scripts -p test_ai_tool_hooks.py`；ZCode 复用 claude 协议形状） | PASS（同左，block_dangerous.sh 直接探针） | PASS（--protocol codex 探针，permissionDecision 形状） | N/A |
| 新会话规则加载 | PENDING（首次真实会话时补验） | PENDING | PENDING | PENDING |
| MCP | N/A（本仓无项目级 MCP） | N/A | N/A | N/A |

hooks 探针可离线复验：
`echo '{"tool_name":"Bash","tool_input":{"command":"git push --force origin x"}}' | bash .claude/hooks/block_dangerous.sh`
应输出 deny 决策 JSON；
`...{"command":"git status"}...` 应无输出（放行）。
各工具真实会话内的加载与拦截在首次使用时补验并更新本表。
