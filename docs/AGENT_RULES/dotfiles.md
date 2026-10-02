# dotfiles：本机 wezterm 环境快照与跨机安装链

## 范围

`dotfiles/**`：`wezterm-config/`（配置快照）、`plugins/`（插件快照）、
`fonts/`（精选字重）、`templates/`、`assets/`（安装模板与图标）、
`install.sh` / `install.ps1`（安装器）、`README.md`、`PROVENANCE.md`。
另含把快照装进用户环境的 `scripts/gx-launcher/**`、`scripts/gx_package.py`、
`scripts/gx_config_fingerprints.py`（同时受 development 域约束）。

## 符号真源

- 快照溯源：`dotfiles/PROVENANCE.md`（来源仓库与 commit、插件 pin 表、
  收录期相对本机的有意改动、不收录项）。
- 安装器：`dotfiles/install.sh`（Linux 用户级、幂等、`--check` 干跑）；
  `dotfiles/install.ps1`（Windows 用户级）。目标路径依据产品源码：
  配置查找序 `config/src/config.rs::load_with_overrides`（两平台都支持
  `$HOME/.config/wezterm/wezterm.lua`，`~/.wezterm.lua` 更优先）、
  插件目录 `lua-api-crates/plugin/src/lib.rs::RepoSpec::plugins_dir` 与
  转义算法 `compute_repo_dir`（`/`→`sZs`、`:`→`sCs`、`.`→`sDs`）、
  Windows 数据目录 = Roaming AppData（`config/src/config.rs::compute_data_dir`）。
- 打包/同步：`scripts/gx_bundle.py`（`bundle` / `sync` 子命令）；
  源码路径安装 `scripts/gx_install.sh`；命令入口见根 Makefile 追加段。
- 原生安装包：`scripts/gx_package.py`（Windows EXE / Ubuntu 20.04、24.04 通用 amd64 deb）；
  `scripts/gx-launcher/` 是独立 std-only Rust 启动器（Rust >= 1.89），由 rustc
  直接编译，不改变产品 Cargo workspace。首次启动初始化配置（配置目录不存在，或只有
  设置页写的 `gui-settings.json` 时）；已有配置逐文件迁移（`main.rs::migrate_config`），
  由 `main.rs::upgrade_config` 决定何时运行：wezterm-gx 数据目录的 `config-version`
  记下载荷版本、指纹表摘要、配置目录，以及挡住其他文件或写入失败的文件的指纹；任何一项
  变了才重跑，失败的连续重试 3 次启动、备份写进同一个时间戳目录。结果记在同目录
  `config-migration.log`，日志与 `config-version` 写失败不影响启动。已发布文件指纹表
  `scripts/gx-launcher/released.rs` 由 `scripts/gx_config_fingerprints.py` 从仓内
  `scripts/gx-config-releases.json` 生成；登记数据只从显式发布提交提取，不从工作区
  配置推断。插件升级备份在独立 wezterm-gx 数据目录，保留 state/，还原 gitdir → .git。
- apt 构建依赖唯一真源：根 `get-deps`（docker 构建镜像与 gx-install 都
  调用它，不在 dotfiles/ 另立清单）。

## 不变量

- 本机 `~/.config/wezterm` 保持独立目录（不 symlink 进仓库）；仓库快照是
  移植真源，本机改动经 `make gx-sync` 回收（默认只读对比，
  `GX_SYNC_WRITE=1` 写回）。
- 插件目录名必须与 `compute_repo_dir` 的转义结果一致，否则 wezterm 认为
  未安装而重新 clone；升级插件 = 整目录重新快照 + 更新 PROVENANCE.md 的
  pin 表。
- sync 永不自动删除仓库侧文件（REMOVED 只报告）；`.git` 与 resurrect
  `state/`（会话数据）一律不入库。
- 敏感信息（凭据、token、非回环 IP、hostname）禁入 `dotfiles/`；新增文件
  前人工过一遍。
- install.sh 幂等且不 sudo：覆盖已有目标前先备份 `.bak-gx-<ts>`；
  不改写用户已有的 `~/.wezterm.lua`（只警告优先级冲突）；整目录换入配置时保留原
  `gui-settings.json`（install.ps1 同）。
- bundle 二进制必须来自本分支构建：默认 docker `ubuntu:20.04` 容器保证
  glibc ≤ 2.31 兼容（目标机 20.04/24.04 通吃）；本机构建回退
  （`GX_USE_LOCAL=1`）必须在产物文件名与 manifest.env 标注 glibc 要求。
- 原生 deb 同样固定在 Ubuntu 20.04 构建（新版 Linux 用 `--container`），沿用
  vendored-openssl feature；全部程序与启动器检查 GLIBC ≤ 2.31、无动态 OpenSSL。
  运行依赖经 dpkg-shlibdeps 生成并补齐 dlopen 图形库/字体缓存/zsh/CA 证书。
  同一个包必须在干净 20.04 与 24.04 环境通过安装生命周期及 GUI 验证后才发布。
- 安装器不以管理员/root 身份初始化其他用户 HOME；用户数据只由普通用户
  启动器写入。卸载保留配置、插件会话与备份；用户/系统模式字体按对应范围安装。
- Windows 启动器启动子进程前清除继承的「忽略 Ctrl+C」标志（`os/windows.rs::restore_ctrl_c`）；
  CLI 启动器自己吞下 Ctrl+C/Ctrl+Break、等子进程结束并返回其退出码（`os/windows.rs::wait`），
  事件由共享控制台交给子进程处理。
- 配置迁移逐文件决定：与任一已发布版本相同（文本 CRLF 视同 LF，含 NUL 字节的二进制
  逐字节比较）且可写 → 备份到 `backups/<时间戳>/wezterm-config/` 后换成载荷版（Unix 保留
  原权限位）；从没发布过的新文件 → 入口 `wezterm.lua` 是 GX 已发布/当前版本时补装；
  发布过却不见了的文件视为用户删掉（如壁纸浮层删除的壁纸），只有要运行的代码 require
  它才补回；改过的、只读的、符号链接及符号链接目录内的文件 → 保留并记日志；不再随包的
  文件原样留下。`gui-settings.json` 是用户数据：载荷不得包含（`gx_package.py` 打包跳过，
  `.gitignore` 与 `gx_bundle.py sync` 忽略），迁移永不触碰。
- 迁移一致性：文件与它字面 `require` 的配置模块一起移动。新版文件等它 require 的模块都
  到新版才写；留在旧版本、且能从 `wezterm.lua` require 到的文件（用户改过的、只读的、被
  挡住的已发布副本；`tests/` 等手动运行的不算）把它 require 的、本次会变的已发布模块也
  留在原版本，并逐级连带引用这些模块的新文件。例外：发布过且每个版本都与载荷一致的文件
  （如至今未改过的 `wezterm.lua`、`colors/custom.lua`），用户改它不挡任何文件——新模块
  本来就与它的原样配套。所以真正会挡住升级的是改过「本次也更新了」的文件（如
  `config/bindings.lua` 会连带 `config/plugins.lua`、`utils/backdrops.lua`、
  `config/appearance.lua`）；日志写成 `skipped X: held back because <根文件> is modified
  locally`，根文件改动（例如手动合并）后下次启动重跑。一起移动的一组文件全有或全无：
  先全部写临时文件并校验、再备份、再逐个 rename，中途失败把已换的文件放回。非字面量的
  动态 `require` 不参与判断，受管模块仍应保持接口向后兼容。
- 发布后用 `gx_config_fingerprints.py --record` 从显式 `--source-repo` 的发布
  tag/完整提交追加到 `scripts/gx-config-releases.json`，同时生成 `released.rs`。
  登记表冻结 repository/tag/commit/config tree/blob 与归一化指纹；旧条目及顺序
  不可改写（顺序决定 Rust bitmask），最多 8 个版本，扩容须同步调整启动器类型。
  `--check` 在浅克隆/源码归档中仅校验登记表，不读取父目录或旧 gx_shell 对象；
  `--verify-git --source-repo <历史checkout>` 才严格复算所有发布对象，缺失即失败。
  GX Shell 集成端显式传 `--gx-shell-changelog <path>` 保留已发布版本覆盖门。
  漏登会导致未修改的配置无法升级；历史 WezTerm GX 0.3.0 / GX Shell 0.1.0、0.2.0
  的 76 条合并指纹和 bitmask 由单测金标保护，不因拆仓改变升级语义。

## 禁止项

- 不收录运行态：`~/.cache/wezterm/`、`check_update`、resurrect 已保存会话。
- 不把上游 `assets/` 资源复制进 dotfiles/（图标例外，来源在 PROVENANCE
  登记）。
- `install.ps1` 只做用户级安装（不写 Program Files / HKLM）。

## 验证

- 语法：`bash -n dotfiles/install.sh scripts/gx_install.sh`；
  `python3 -m py_compile scripts/gx_bundle.py`。
- 纯 Lua 单测：`dotfiles/wezterm-config/tests/pure_fn_test.lua` 经 wezterm mlua
  运行（`wezterm --config-file <该文件> show-keys`，末行 `PURE_FN_TEST: ALL PASS`）；
  `scripts/tests/gx_shells.lua`/`gx_resurrect.lua` 用项目钉版 lua54
  （`make setup` 装到 `.local/tools/lua/bin/`，wezterm 模块与文件系统自带 stub）。
- 启动器与指纹表：`rustc --edition=2021 --test scripts/gx-launcher/main.rs -o
  .local/gx-tests/launcher-tests && .local/gx-tests/launcher-tests`；
  `python3 -m unittest discover -s scripts -p 'test_gx_*.py'` 与
  `python3 scripts/gx_config_fingerprints.py --check`（不需要历史对象）；需要来源审计时
  再显式加 `--verify-git --source-repo <历史checkout>`，缺少任一发布对象即失败。
- 沙箱安装：`HOME=$(mktemp -d) dotfiles/install.sh --check` 干跑 → 全量安装
  → 沙箱 HOME 下 `wezterm --version` / `wezterm ls-fonts` 冒烟（配置可解析、
  字体/插件就位）。
- GUI 可见行为（壁纸/状态栏/tab 标题）：Xvfb 截图留证
  `.ui-evidence/`（读回核对，遵守 development.md 证据纪律）。
- `make gx-sync` 干跑：除 PROVENANCE 登记的收录期有意改动外应无差异。
- 容器构建产物：`objdump -T` 抽查 GLIBC 符号最大版本 ≤ 2.31。
