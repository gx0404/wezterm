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
  Windows 上 `.sh` 入口经 PATH 中的 Git Bash/MSYS2 bash 执行（跳过 System32、
  WindowsApps 下的 WSL 启动器），找不到时 doctor 报 MISSING、run 报错。
- 版本：`scripts/version.py`——根 CHANGELOG.md 的最大 SemVer 为真源，
  `--check` 校验镜像、`--write` 原子写入（本仓 version_targets 为空，见
  docs/RELEASE.md 的两套版本体系说明）。
- 工具钉版：`scripts/setup_env.sh` → `.local/tools/`（nextest/stylua 预编译包
  sha256 钉版 + venv{graphifyy, tomli}；Windows 下载对应预编译包，venv 使用
  Scripts/ 布局，补 `Scripts/python3.exe` 与 `bin/graphify` shim）。lua 5.4
  同为钉版（Windows 用 LuaBinaries 预编译包、下载失败回退官方源码 mingw 编译；
  Linux/mac 官方源码 posix 编译），供 `scripts/tests/*.lua` 纯 Lua 单测使用。
  Windows 另装 NASM 3.02、Strawberry Perl 5.42.3.1 portable，并从该 sha256
  校验包提取 GNU Make 4.4.1 与 libintl/libiconv DLL 到 `make/bin/`；许可保留在
  `perl/licenses/`。Makefile 前置项目工具 PATH，按 make 宿主选择路径分隔符，
  框架 Python 优先使用仓内 venv。venv 只用 Python>=3.10 的真实解释器，跳过
  WindowsApps 别名；仓库改名/移动后 venv 绝对路径失效，删除该 venv 后重跑。
  Windows 首次在 Git Bash 执行 `bash scripts/setup_env.sh`，无需预装 make；
  已安装后可用 `make setup`。安装结束与 `--check` 都执行全环境健康门，严格
  校验钉版工具的可运行性和版本；Windows 还调用 MSVC wrapper 预检，缺项非零退出。
  `WEZTERM_TOOLCHAIN_ROOT` 可覆盖工具根，跨 Git Bash/cmd 使用 Windows 绝对路径
  （如 `E:/checkout/.local/tools`），不改变构建产物目录。
- Windows MSVC 入口：`scripts/gx_msvc_env.cmd --check` / `make check|build|test`，
  在 cmd/PowerShell 中经 wrapper 执行；通过 vswhere 选择已安装且含 x64 C++ 工具
  的 Visual Studio，不限定年份。只在子进程环境选择 Rust 1.96.1 MSVC、前置项目
  工具并追加 C/C++ `/utf-8`，不改全局工具链/PATH；预检包含 Windows SDK、CMake、
  Rust 与 nightly rustfmt。直接使用 Git `usr/bin/sh.exe`，避免启动器抢先加载
  Git Perl；wrapper 清除继承的 make flags，Makefile 在 Windows 不向 NMake 导出
  `MAKEFLAGS/MFLAGS/GNUMAKEFLAGS`。
- hooks：`.claude/hooks/dangerous_patterns.conf` 是危险模式唯一真源，
  `pre_tool_use_gate.py` 消费（claude/codex 协议适配；ZCode 复用 claude 形）。

## 不变量

- **构建产物仓内封闭**：编译、构建的一切产物与缓存只落仓内既定位置——
  `target/`（Makefile 与 MSVC wrapper 统一固定 `CARGO_TARGET_DIR`）、
  `.local/sccache`（sccache 编译缓存，`SCCACHE_DIR`）、`.local/tmp`（构建进程
  临时目录，固定 `TMP/TEMP/TMPDIR`，不沿用仓外值）、`deps/`（get-deps）、
  `.ui-evidence/`（截图证据）与打包 stage 目录；`.local/` 整目录 gitignore。
  禁止把 `CARGO_TARGET_DIR`/`OUT_DIR`/`SCCACHE_DIR`/`TMP|TEMP` 指到仓库外，
  也不得依赖指向仓外的外部环境变量默认值；不经 make 直接调用 cargo/脚本构建
  的会话同样受此约束。用户级共享层（rustup 工具链本体、`~/.cargo` 依赖源
  缓存）不是构建产物，不在此列。
- **工具安装仓内封闭**：agent 为本仓安装的任何工具（汇编器、Perl、格式化器、
  测试运行器、解释器等）只能经 `scripts/setup_env.sh` 钉版（URL + sha256）装进
  `.local/tools/`；禁止用 winget/choco/scoop/msiexec/`pip --user` 等装到系统级
  或用户级的仓外路径（hook 在 `dangerous_patterns.conf` 拦截）。VS Build Tools、
  Windows SDK、Inno Setup 这类无法仓内化的系统组件由人类安装，并在
  `docs/MAKE_COMMANDS.md`/`docs/TESTING.md`/`docs/RELEASE.md` 登记。仓库路径必须
  纯 ASCII：Windows 原生构建链（Strawberry Perl/nmake 等）对非 ASCII 路径有
  编码问题。
- **并行会话防双写**：写入仓库前复查并行信号（untracked/修改清单短间隔
  增长、出现非本轮新建的产物目录）；发现并行推进即转只读验收 + 逐项声明
  的外科修复，不双写。判定进度用 ctime 或文件清单快照，不信 mtime
  （保留 mtime 的拷贝会漏报）；「命令没有输出」先按产物存在性判断是否
  真的执行过。
- 规则/路由改动后必须 `make framework-check`（闭集 + 体积 + 排序）通过；
  危险模式改动必须 `python3 -m unittest discover -s scripts -p test_ai_tool_hooks.py`。
- lint 门不得吞退出码（禁止 `--exit-zero`、禁止 `| tail` 接验收命令）；
  诊断（ai-doctor）只报告缺失并给修复命令，不隐式安装。
- 生成物纪律：`graphify-out/graph.json`、`kb/chunks.json`、
  `graphify-out/source-fingerprint.json` 指纹只能经 `make graph / make kb`
  重建；有意变更才写盘并审 diff，默认
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
