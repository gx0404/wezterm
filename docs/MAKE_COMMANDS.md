# Make 命令手册

命令真源：根 `Makefile`；框架命令经 `scripts/dev_framework.py` 调度
（配置 `docs/dev-framework.json`）。工具解析序：`.local/tools` > PATH
（安装 `make setup`）。诊断用 `make ai-doctor`（只读，不安装）。

## 上游目标（语义随上游）

| 目标 | 作用 | 前置/副作用 |
|---|---|---|
| `make all` / `make build` | 依次构建 wezterm、wezterm-gui、wezterm-mux-server、strip-ansi-escapes（release） | 需系统图形依赖（`./get-deps`）；耗时 |
| `make check` | cargo check + 4 个指定包（escape-parser/cell/surface/ssh） | 只读 |
| `make test` | cargo nextest 全量 + escape-parser no_std 轮 | 需 nextest（make setup）；ssh e2e 需本机 sshd |
| `make fmt` | cargo +nightly fmt（**会改文件**） | 需 nightly 工具链 |
| `make docs` / `make servedocs` | 构建文档站（docker/podman + gelatyx + 网络） | 上游流程，本地无容器时不可用 |

## 框架目标（fork 维护）

| 目标 | 作用 | 前置/副作用 |
|---|---|---|
| `make help` | 目标总览 | 无 |
| `make setup` | 钉版安装 nextest + venv(graphifyy/tomli) 到 `.local/tools/` | 联网下载；幂等 |
| `make ai-doctor` | 只读诊断命令入口 | 无 |
| `make framework-check` | resolver --check（规则闭集/体积/排序守门） | 无 |
| `make framework-ready` | 配置完整门（拒绝 pending 命令） | 无 |
| `make framework-test` | 框架脚本 unittest | 无 |
| `make ci-check` | resolver --check → version --check → lint → typecheck → test | 编译+全量测试，耗时 |
| `make version` / `version-check` / `version-write` | fork 版本（CHANGELOG 最大 SemVer） | write 才写盘 |
| `make evidence TASK=x` | 分配 `.ui-evidence/<分支>/<任务>/<批次>/` | 建目录+初始 result.json |
| `make lint` | cargo +nightly fmt --check | 需 nightly |
| `make typecheck` | = make check | 只读 |
| `make test-integration` | cargo nextest run -p wezterm-ssh | 需 /usr/sbin/sshd |
| `make test-heavy` | nextest --all --no-fail-fast（CI 形态） | 同 make test |
| `make generated-check` | 派生文件只读比对（补全/键表/docs 索引） | 二进制链需 target/debug/wezterm，缺则跳过该段并注明 |
| `make generated-write` | = scripts/generated_write.sh（上游 update-derived-files.sh + 键表 stylua 格式化） | **改文件**；需 target/debug/wezterm、钉版 stylua |
| `make ui-smoke` | Xvfb 隔离显示截图冒烟 | 需 target/debug/wezterm-gui、Xvfb、xwd、ffmpeg |
| `make graph` / `make graph-check` | 重建/校验代码图谱 | graph 需 venv（make setup） |
| `make kb` / `make kb-check` | 重建/校验知识库 | kb 写盘，kb-check 只读 |
| `make gx-bundle` | docker ubuntu:20.04 容器构建 release 四件套并组装离线安装包 `dist/*.tar.xz`（有 Windows 包时顺带产出 zip） | 需 docker（或 `GX_USE_LOCAL=1` 本机构建，产物标注 glibc）；联网装依赖，耗时 |
| `make gx-install` | 源码路径安装：rust 检查 → `./get-deps`（需 sudo，交互确认）→ release 构建 → `dotfiles/install.sh` 部署 | 改 `$HOME` 下用户文件（先备份）；联网 |
| `make gx-sync` | 对比本机 `~/.config/wezterm`、插件目录与 `dotfiles/` 快照差异 | 只读；`GX_SYNC_WRITE=1` 写回仓库 |
| `make dev` | cargo run -p wezterm-gui（交互起 GUI） | 需显示；编译耗时 |

## 环境变量

- `WEZTERM_TOOLCHAIN_ROOT`：重定向 `.local/tools`（CI/离线复用）。
- `WEZTERM_GRAPHIFY_CLI` / `WEZTERM_GRAPHIFY_ALLOW_ANY_VERSION=1`：
  图谱 CLI 覆盖/版本放行（升级比对时用）。
- `TASK=`：`make evidence TASK=<任务名>`（缺省 ui-smoke）。
- `GX_USE_LOCAL=1`：`make gx-bundle` 回退本机构建（产物只兼容本机 glibc）。
- `GX_WINDOWS_ZIP=<path>`：指定 Windows 构建包（缺省时尝试 `gh` 拉取
  gx-windows-build workflow 产物或复用 `dist/` 现成包）。
- `GX_SYNC_WRITE=1`：`make gx-sync` 把本机改动写回 `dotfiles/`。

## 已知边界

- `make docs` 依赖 docker/podman 与 GitHub API（release 信息注入），fork
  日常不做文档站构建；上游 pages CI 把关。
- Windows/macOS 平台门在本机无环境时以 PENDING 记录，由上游 CI 矩阵
  佐证（gen_* 工作流）。
