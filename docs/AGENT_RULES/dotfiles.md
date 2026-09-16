# dotfiles：本机 wezterm 环境快照与跨机安装链

## 范围

`dotfiles/**`：`wezterm-config/`（配置快照）、`plugins/`（插件快照）、
`fonts/`（精选字重）、`templates/`、`assets/`（安装模板与图标）、
`install.sh` / `install.ps1`（安装器）、`README.md`、`PROVENANCE.md`。

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
  不改写用户已有的 `~/.wezterm.lua`（只警告优先级冲突）。
- bundle 二进制必须来自本分支构建：默认 docker `ubuntu:20.04` 容器保证
  glibc ≤ 2.31 兼容（目标机 20.04/24.04 通吃）；本机构建回退
  （`GX_USE_LOCAL=1`）必须在产物文件名与 manifest.env 标注 glibc 要求。

## 禁止项

- 不收录运行态：`~/.cache/wezterm/`、`check_update`、resurrect 已保存会话。
- 不把上游 `assets/` 资源复制进 dotfiles/（图标例外，来源在 PROVENANCE
  登记）。
- `install.ps1` 只做用户级安装（不写 Program Files / HKLM）。

## 验证

- 语法：`bash -n dotfiles/install.sh scripts/gx_install.sh`；
  `python3 -m py_compile scripts/gx_bundle.py`。
- 沙箱安装：`HOME=$(mktemp -d) dotfiles/install.sh --check` 干跑 → 全量安装
  → 沙箱 HOME 下 `wezterm --version` / `wezterm ls-fonts` 冒烟（配置可解析、
  字体/插件就位）。
- GUI 可见行为（壁纸/状态栏/tab 标题）：Xvfb 截图留证
  `.ui-evidence/`（读回核对，遵守 development.md 证据纪律）。
- `make gx-sync` 干跑：除 PROVENANCE 登记的收录期有意改动外应无差异。
- 容器构建产物：`objdump -T` 抽查 GLIBC 符号最大版本 ≤ 2.31。
