# AGENT_RULES 索引与维护约定

本目录是领域规则的唯一正文存放处；`routes.toml` 是唯一机器真源。
使用方式见根 `AGENTS.md` 的规则加载协议（resolver 按路径解析，`--check`
守门闭集与体积）。

## 维护约定

- 新增领域规则：建 `<id>.md`（≤16 KiB，含范围/符号真源/不变量/禁止项/验证
  五要素）并在 `routes.toml` 登记 `[[rules]]`，然后跑
  `python3 scripts/resolve_agent_rules.py --check` 直到闭集通过。
- 移动/新增源码文件不需要改本目录，除非引入了新的顶层目录或新领域；此时
  必须让全仓文件仍恰好命中一个领域或 root_only。
- `root_only` 只登记真正没有领域约束的仓库元文件；禁止用宽模式把源码路径
  挂进 root_only 规避规则加载。
- 领域规则只写 `模块路径::符号` 与不变量，不抄字面值、不钉行号；现场手册
  （docs/MAKE_COMMANDS.md 等）允许数字但须注明真源符号。
- 正文不要写 lua 代码块（docs/ 全目录被 gelatyx --check 扫描，lua 块必须
  stylua 格式）；shell/toml 块不受影响。
- 本目录文档不进公共文档站（docs/mkdocs-base.yml 的 exclude_docs 已排除）。

## 领域清单一览

人工预览用；实际匹配以 `routes.toml` 与 resolver 输出为准：

| id | 覆盖 | 主题 |
|---|---|---|
| build-ci-release | ci/ .github/ nix/ Cargo.* deny.toml wezterm-version | 构建、CI、依赖、版本、生成物清单 |
| cli-main | wezterm/ wezterm-gui-subcommands/ | CLI 入口与子命令 |
| code-review | （task=review） | 只读审核规程 |
| config-lua | config/ lua-api-crates/ wezterm-dynamic/ env-bootstrap/ 等 | Lua 配置链 |
| development | scripts/ AGENT_RULES Makefile 工具面 | 框架自身 |
| font-shaping | wezterm-font/ wezterm-char-props/ deps/ | 字体与 shaping |
| gui-rendering | wezterm-gui/ | GUI 前端 |
| mux-domain | mux/ wezterm-client/ mux-server codec/ | 多路复用与协议 |
| platform-window | window/ wezterm-input-types/ | 平台窗口抽象 |
| process-io | pty/ procinfo/ filedescriptor/ umask/ | 进程与 IO |
| product-assets | assets/（shell-integration 除外） | 静态资源 |
| product-docs | docs/（AGENT_RULES 除外） | 产品文档站 |
| ssh-domain | wezterm-ssh/ | SSH 会话 |
| support-crates | promise bidi lfucache 等 | 支撑库 |
| terminal-model | term/ wezterm-cell/ wezterm-surface/ vtparse/ escape-parser | 终端模型 |
| testing | test-data/ | 测试数据 |
