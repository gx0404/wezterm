# Changelog（gx0404/wezterm fork）

本文件记录 fork 层面已实现的可观察变更（AI 协作框架、开发流程、定制功能）。
上游 WezTerm 的产品级变更见 `docs/changelog.md`（随上游同步，不在本文件重复维护，
避免同步上游时产生冲突）。

版本真源：本文件 `## X.Y.Z(日期|TBD)` 标题中的最大 SemVer（`scripts/version.py`，
`make version` 只读查询）。WezTerm 产品自身的版本号由 `wezterm-version/build.rs`
按 git 提交时间与哈希生成，两套体系互不干扰（见 docs/RELEASE.md）。

## 0.2.0(TBD)

### Added

- 收录本机 wezterm 用户环境快照为仓库真源 `dotfiles/`：配置工作区
  （含壁纸与生效中的事件脚本，`config/launch.lua`、`config/domains.lua`
  的 Windows 死路径改动态拼接）、4 个插件按 wezterm 插件加载器转义目录名
  快照（pin 见 `dotfiles/PROVENANCE.md`）、精选字体（JetBrainsMono Nerd
  Font 6 字重 + Noto Sans CJK Regular/Bold）、desktop entry / wrapper /
  zshrc 模板。
- 跨机一键安装链：`dotfiles/install.sh`（Linux 用户级、幂等、`--check`
  干跑）与 `dotfiles/install.ps1`（Windows 用户级）；`make gx-bundle`
  经 docker ubuntu:20.04 容器构建 glibc≤2.31 兼容二进制并组装自包含离线
  包（Ubuntu 20.04/24.04 通吃）；`make gx-install` 提供在线源码构建路径；
  `make gx-sync` 把本机配置改动收回仓库（默认只读对比）。
- Windows 分支构建 workflow `.github/workflows/gx-windows-build.yml`
  （workflow_dispatch / gx-v* tag 触发，产出四件套 zip 供 gx-bundle 组装）。
- 领域规则 `docs/AGENT_RULES/dotfiles.md` + 路由；`dist/`、`target-gx-*/`
  入 `.gitignore` fork 段。

- UI 交互增强（源自对 herdr 与 tmux 的交互调研）：鼠标绑定新增
  `region` 维度（可按 TabBar/Tab/NewTabButton/LeftStatus/RightStatus/
  Split/ScrollThumb 等区域定向绑定；未指定 region 的绑定语义不变）；
  copy mode 新增 mark 标记（`m` 设标记、`'` 交换式跳转）、段落移动
  （`{`/`}`）、`MoveToLine` 行跳转、无选区时复制光标处词、
  `PipeSelection` 管道赋值与 `copy_mode_mark_bg/fg` 配色；tmux 域
  pane 的 copy mode 搜索默认可用；bell 治理三件套：
  `bell_notification_handling` 聚焦抑制、`bell_requests_attention`
  失焦 WM 提醒（X11 urgency / macOS dock bounce）与 Lua
  `window:request_attention()`、`bell_cooldown_ms` 每 pane 节流。
- 命令面板鼠标交互：悬停行即选中、左键点击直接执行（与 Enter 共用
  激活路径并记录 frecency）；`Modal::mouse_event` 通道接通，行矩形经
  hit map（`UIItemType::Modal`）路由，region 绑定新增 `Modal` 区域。
- `make gx-upgrade`：一条命令完成本机替换——docker ubuntu:20.04 容器
  构建 release 四件套（无 sudo，get-deps 唯一真源）→ `dotfiles/install.sh`
  用户级部署（配置/插件/字体随快照，全带时间戳备份）→ 版本验证；
  消除手工两步安装尾巴。

### Fixed

- 修复 `make test` 的 escape-parser no_std 轮编译失败（上游
  8d668a78c 移除 macro_use 时遗漏测试模块的 `alloc::format` 导入）。
- 框架 `make generated-check` 恒报漂移：键表比对改为经钉版 stylua
  （`ci/stylua.toml`，与 docs 构建同约定）格式化后比较，缺工具时显式
  跳过；`make generated-write` 由 `scripts/generated_write.sh` 包装
  上游脚本并补格式化；stylua 2.5.2 入 `setup_env.sh` 钉版；shell
  补全随 clap 生成器演进重建。`ui_smoke.sh` 修正 `--config` 全局参数
  位置并加 `-n` 隔离用户配置（本机配置在裸 Xvfb 渲染全黑）。
- 恢复 Linux 壁纸快捷键：`Alt+.` / `Alt+,` 切换下一张 / 上一张，
  `Alt+/` 随机、`Ctrl+Alt+/` 选择、`Alt+b` 切换纯色专注模式；
  避免壁纸控制跟随通用 `Ctrl+Shift` 修饰键变更。同步本机配置、
  `dotfiles/wezterm-config/` 与 Oh My Zsh 的 `gx/wezterm/` 快照。

## 0.1.0(TBD)

### Added

- 初始化 AI 协作开发框架：根 `AGENTS.md` 启动协议 + `docs/AGENT_RULES/`
  领域规则与 `routes.toml` 路由闭集 + `scripts/resolve_agent_rules.py`
  （多路径并集、目录展开、review 任务、只读 `--check`）。
- 命令层：Makefile 追加框架目标（`make help / framework-check /
  framework-ready / ai-doctor / ci-check / setup / evidence / graph / kb /
  ui-smoke` 等），`docs/dev-framework.json` 绑定真实命令；
  `scripts/setup_env.sh` 将 cargo-nextest 与 graphifyy 钉版安装到仓库内
  `.local/tools/`（幂等，`--check` 只读诊断）。
- 文档：`docs/{README,ARCHITECTURE,DEVELOPMENT,MAKE_COMMANDS,TESTING,
  AI_TOOLS,RELEASE}.md` 中文手册；`docs/mkdocs-base.yml` 追加 `exclude_docs`
  使框架文档不进公共文档站。
- 测试与证据：`scripts/generated_check.sh`（派生文件只读校验）、
  `scripts/ui_smoke.sh`（Xvfb 隔离显示 + xwd 截图 + ffmpeg 转 PNG，证据写
  ignored 的 `.ui-evidence/<分支>/<任务>/<批次>/`）；框架脚本自带 unittest
  （`make framework-test`）。
- 图谱与知识库：`scripts/graphify.sh`（钉版 graphifyy，代码-only AST 索引，
  排除生成数据表；指纹新鲜度 `make graph-check`）；`scripts/build_agent_kb.py`
  （受控语料切块 + source_hashes，`make kb / kb-check`）。
- AI 工具面：ZCode / Claude Code / Codex 共享配置（`.zcode/config.json`、
  `.claude/settings.json`、`.codex/config.toml`）与共享 PreToolUse 安全门；
  Kimi 经根 `AGENTS.md` 工作。
- 代码注释规范：`docs/AGENT_RULES/code-comments.md`（上游英文注释保持、
  fork 新增标记、Lua API rustdoc 约定、`模块路径::符号` 引用），并对核心
  模块头补少量中文导读（仅模块头，不动正文，控制上游同步冲突面）。

### Fixed

- `.codex/hooks/pre_tool_use_policy.py` 适配器语言混装修正为真 Python
  （此前为 bash 内容冒充 `.py`，直调引擎探针全绿、按注册入口探针才暴露）；
  注册入口探针、适配器语言完整性（python3→ast.parse、bash→`bash -n`）与
  codex/zcode hooks 配置形状锁（事件数组表、命令嵌套、timeout 秒制、
  `config_file` 注册键）已锁进 `scripts/test_ai_tool_hooks.py`。
- 采纳 skill 更新（2026-09-16）的并行会话纪律：写入仓库前复查并行信号、
  发现并行推进转只读验收不双写、用 ctime/清单快照判定进度；探针输入中的
  危险命令字面量拆分构造（development.md 不变量）。
