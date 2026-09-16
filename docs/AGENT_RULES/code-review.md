# code-review：审核任务规程（--task review）

## 角色与边界

- reviewer 只读：不修改源码、不维护白名单/字段词表副本、不代跑会改状态
  的命令（setup/graph/kb/generated-write 等）。
- 开始前：读根 `AGENTS.md`；对涉及路径跑
  `python3 scripts/resolve_agent_rules.py --task review <paths...>` 并读完
  输出文档；再看 `git diff`（含暂存与未暂存）与相关调用方。

## 检查清单

按涉及域取舍（领域细则见对应规则文档）：

1. **正确性**：状态所有者是否被绕过（TerminalState/MUX 单例/TermWindow/
   FontConfigInner）；线程边界（Lua 主线程、事件循环线程、parse 线程）；
   seqno/generation 是否正确递增；行号类型是否混用。
2. **契约**：codec Pdu 与 cli 输出格式是兼容面——破坏性变更是否已声明；
   CellCluster/Texture2d/FontShaper/MasterPty 等 trait 是否被实现完整。
3. **平台**：是否引入了越界的 `#[cfg(...)]`；X11/Wayland 枚举路径两侧
   是否同审；无环境侧是否如实标注 PENDING。
4. **上游同步面**：对上游文件的修改是否最小化（追加式、带 fork 标记）；
   是否动了生成物（shell-completion/键表/docs 索引/scheme_data/emoji 表）
   而没有走再生成命令。
5. **依赖与安全**：新依赖是否经 workspace.dependencies + deny.toml；
   unsafe/FFI 边界理由；凭据是否可能进日志/快照。
6. **测试**：新行为是否有失败路径覆盖；快照更新是否被逐个说明；被跳过
   的层是否有 PENDING 说明而非静默绿。
7. **框架一致性**（涉 scripts/AGENT_RULES/Makefile 时）：resolver --check
   是否仍过；命令绑定是否 argv 化；hooks 模式改动是否跑了探针测试。

## 输出格式

- 严重：必须修复才能合入（正确性/安全/数据丢失/契约破坏）
- 中：应修复，可有理由地延后
- 轻：风格与改进建议

每条给 `文件:行`、原因、修复建议；最后结论二选一：可合入 / 需修复后
复审。发现的问题即使当场被作者修复，也要在报告中列出（含修复方式）。
