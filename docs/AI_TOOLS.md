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
- **适配器语言纪律**：`python3` 登记调用的脚本必须是真 Python（shell
  冒充 `.py` 只有按注册入口探针才拦得住——2026-09 落地事故沉淀）；
  `bash` 调用的 `.sh` 过 `bash -n`。两类检查都锁在
  `scripts/test_ai_tool_hooks.py` 的 RegisteredAdapterIntegrity。
- 改模式必须跑：`python3 -m unittest discover -s scripts -p
  test_ai_tool_hooks.py`（含允许/拒绝探针、注册入口探针与配置形状锁）。

## 权限面（Claude settings.json 要点）

- 项目默认模式为 `acceptEdits`；allow 放行仓内 Read/Edit/Write、Glob/Grep、
  整个 Bash 工具以及 WebFetch/WebSearch/Agent/Skill，普通开发操作不再逐次询问。
- 不设项目级 `permissions.ask`；原先 cargo build/run、make build/test/setup 等
  确认规则已移除。Bash 整体授权覆盖管道、环境变量和 shell 展开，不再逐条补前缀。
- deny（读 .env/.pem/.git）与现有 PreToolUse 安全门原样保留；危险模式仍按
  `.claude/hooks/dangerous_patterns.conf` 拒绝或询问。Bash 放行不是文件系统沙箱。
- 仅修改本仓 `.claude/settings.json`，不改用户全局权限。Claude Code 自 v2.1.257
  起忽略项目/本地配置中的 `defaultMode: bypassPermissions`，因此这里使用
  项目范围支持的 `acceptEdits` + 工具 allow，而不是写入无效的 bypass 设置。
- 运行中通常会热重载权限规则；已经显示的旧确认框可取消后重试，必要时从仓根
  重启并恢复会话。用户/受管设置中的 ask/deny 和客户端内置安全检查仍可能生效。
  配置语义见 [官方设置说明](https://code.claude.com/docs/en/settings) 与
  [权限规则](https://code.claude.com/docs/en/permissions)。

## 逐客户端验证记录

| 项 | ZCode | Claude | Codex | Kimi |
|---|---|---|---|---|
| 配置语法解析 | PASS（JSON） | PASS（JSON + Claude Code 2.1.288 doctor） | PASS（TOML） | N/A（无目录） |
| 项目权限加载与 Bash 实测 | N/A | PASS（2026-10-03，CLI 2.1.288：会话模式为 acceptEdits，含 PATH、管道与 `${PIPESTATUS[0]}` 的 `cargo build --help` 执行成功，permission_denials 为空；未传权限覆盖参数，未实际构建） | N/A | N/A |
| hooks 探针（无副作用 JSON） | PASS（离线探针：`python3 -m unittest discover -s scripts -p test_ai_tool_hooks.py`；含**注册入口探针**（按配置原样解释器+适配器调用）与 codex/zcode 配置形状锁） | PASS（同左，block_dangerous.sh 直接探针） | PASS（--protocol codex 探针 + python3 注册入口探针） | N/A |
| 新会话规则加载 | PENDING（首次真实会话时补验） | PENDING | PENDING | PENDING |
| MCP | N/A（本仓无项目级 MCP） | N/A | N/A | N/A |

hooks 探针可离线复验：
`echo '{"tool_name":"Bash","tool_input":{"command":"git push --force origin x"}}' | bash .claude/hooks/block_dangerous.sh`
应输出 deny 决策 JSON；
`...{"command":"git status"}...` 应无输出（放行）。
各工具真实会话内的加载与拦截在首次使用时补验并更新本表。
