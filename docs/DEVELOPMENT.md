# 开发流程（闭环）

从任务到交付的标准路径。每步的"必须"由领域规则与完成门把关。

## 1. 明确验收点

先写下"做完长什么样"：行为、命令、截图、测试。没有可验证验收点的任务
先和人对齐。

## 2. 加载规则

```bash
python3 scripts/resolve_agent_rules.py <本轮触及路径...>
# 审核任务：--task review；写代码任务会额外交付 code-comments 规则
```

scope 扩大后用完整集合重跑。读输出文档再动手。

## 3. 实现与文档

- 先读代码再改（`docs/ARCHITECTURE.md` 的状态所有者表）。
- 新依赖走 `[workspace.dependencies]` + `deny.toml`。
- 注释遵守 code-comments（上游文件英文、fork 标记、Lua API rustdoc）。
- 行为变更同步 CHANGELOG.md（fork 段）。

## 4. 针对性检查

```bash
cargo nextest run -p <受影响 crate>   # 最小测试面
make check                             # 类型面
make lint                              # nightly rustfmt --check
```

## 5. 适用集成与截图

- GUI 可见行为：`make evidence TASK=<任务>` 分配证据目录 →
  `make ui-smoke`（或带 --out 指向该目录）→ **读回 before.png** 核对 →
  更新 result.json（status=PASS/FAIL、images_reviewed=true）。
- 触到生成物：`make generated-check`；有意更新 `make generated-write`
  后审 diff。
- 触到规则/路由/图谱语料：`make framework-check` / `make graph` / `make kb`。

## 6. 修复复测

失败证据保留在同一批次目录；修复另开批次复测，不覆盖历史。

## 7. 生成物与版本

- 图谱/KB 漂移：`make graph-check / kb-check` 红了就重建并审 diff。
- fork 版本：CHANGELOG.md 的 `## X.Y.Z(TBD)`；`make version` 查询。

## 8. 复审

- `python3 scripts/resolve_agent_rules.py --task review <paths...>`
  按输出规程执行（或用 .claude/agents/code-reviewer、
  .codex/agents/wezterm-reviewer 只读评审员）。

## 9. 交付

按根 AGENTS.md 完成门自检后，交付说明包含：改动清单、真源、实际通过的
检查、未运行项（PENDING）与补验命令、证据路径。commit 用
`type(scope): 中文描述`，只精确暂存相关文件。
