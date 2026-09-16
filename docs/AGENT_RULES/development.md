# development：AI 协作框架自身

## 范围

`scripts/`、`docs/AGENT_RULES/**`、`docs/dev-framework.json`、
`docs/{README,ARCHITECTURE,DEVELOPMENT,MAKE_COMMANDS,TESTING,AI_TOOLS,RELEASE}.md`、
`Makefile`、`.gitignore`、`CHANGELOG.md`、`CONTRIBUTING.md`、工具面目录
（`.claude/ .codex/ .zcode/ .agents/`）、图谱与 KB 产物（`graphify-out/`、`kb/`）。

## 符号真源

- resolver：`scripts/resolve_agent_rules.py`（多路径并集、目录展开、
  `--task`、只读 `--check`；退出码 2 = 路由/闭集违规）。
- 命令调度：`scripts/dev_framework.py` 读 `docs/dev-framework.json`
  （schema 1；evidence_root 必须是隐藏且 ignored 的独立目录；ci 只含检查类
  目标且 lint/test 不可 N/A；configured 命令必须 argv 数组 + 仓内 cwd）。
- 版本：`scripts/version.py`——根 CHANGELOG.md 的最大 SemVer 为真源，
  `--check` 校验镜像、`--write` 原子写入（本仓 version_targets 为空，见
  docs/RELEASE.md 的两套版本体系说明）。
- 工具钉版：`scripts/setup_env.sh` → `.local/tools/`（nextest 预编译包
  sha256 钉版 + venv{graphifyy, tomli}）；Makefile 已把其 bin 前置 PATH。
- hooks：`.claude/hooks/dangerous_patterns.conf` 是危险模式唯一真源，
  `pre_tool_use_gate.py` 消费（claude/codex 协议适配；ZCode 复用 claude 形）。

## 不变量

- **并行会话防双写**：写入仓库前复查并行信号（untracked/修改清单短间隔
  增长、出现非本轮新建的产物目录）；发现并行推进即转只读验收 + 逐项声明
  的外科修复，不双写。判定进度用 ctime 或文件清单快照，不信 mtime
  （保留 mtime 的拷贝会漏报）；「命令没有输出」先按产物存在性判断是否
  真的执行过。
- 规则/路由改动后必须 `make framework-check`（闭集 + 体积 + 排序）通过；
  危险模式改动必须 `python3 -m unittest discover -s scripts -p test_ai_tool_hooks.py`。
- lint 门不得吞退出码（禁止 `--exit-zero`、禁止 `| tail` 接验收命令）；
  诊断（ai-doctor）只报告缺失并给修复命令，不隐式安装。
- 生成物纪律：`graphify-out/graph.json`、`kb/chunks.json`、`.graphify_*`
  指纹只能经 `make graph / make kb` 重建；有意变更才写盘并审 diff，默认
  `make graph-check / kb-check` 只读校验。
- 证据纪律：UI 证据只写 `.ui-evidence/`（ignored）；`make evidence TASK=x`
  分配的目录里 result.json 如实登记 status 与 images_reviewed，没读图不写
  true。
- 脚本兼容 Python 3.10（tomli 回退）；不引入仓库外 Python 依赖（venv 由
  setup_env.sh 管理）。
- hooks 探针纪律：探针必须**按工具配置的原样注册方式**再走一遍入口
  （含适配器与解释器），只直调引擎会漏掉入口层错误（如 shell 冒充 .py
  适配器）；`python3` 登记调用的脚本必须是真 Python（unittest 的
  RegisteredAdapterIntegrity 拦截）；探针输入里的危险命令字面量拆分构造，
  防宿主会话自身安全门拦截探针文本。
- 上游同步安全：本域对上游文件的修改仅限 `.gitignore`/`Makefile`/
  `docs/mkdocs-base.yml` 三处带标记追加段；其它上游文件一律不改语义。

## 禁止项

- 不把领域规则正文复制进工具目录或 AGENTS.md（唯一真源 docs/AGENT_RULES/）。
- 不绕过 resolver 直接猜规则文档；不用 `**` 万能路由。
- 不提交 `.local/`、`.ui-evidence/`、`.graphify-memory/` 等运行态。
- 不在 hooks 里做静默源码修改（PostToolUse 自动格式化之类）。

## 验证

- `make framework-check && make framework-ready && make ai-doctor`。
- 框架脚本自身：`make framework-test`（unittest：resolver 拒绝边界、
  hooks 探针、KB 契约）。
- 改命令绑定：`make ci-check` 实跑；改工具面：按 docs/AI_TOOLS.md 的
  逐客户端验证清单执行并登记结果。
