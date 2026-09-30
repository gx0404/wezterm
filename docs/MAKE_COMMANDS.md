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
| `make setup` | 钉版安装 nextest、stylua + venv(graphifyy/tomli) 到 `.local/tools/`（Windows 装 Windows 版、跳过 venv） | 联网下载；幂等 |
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
| `make package` | 按宿主平台调用 GX 原生打包入口 | 与下面两项相同；不上传、不安装 |
| `make gx-package-windows` | 编译并生成完整 Windows x64 安装 EXE、清单和 SHA-256 | Windows、MSVC、Rust ≥ 1.89、Inno Setup ≥ 7.1；Windows 可直接运行 `python scripts/gx_package.py windows` |
| `make gx-package-deb` | 编译并生成兼容 Ubuntu 20.04/24.04 的同一个 amd64 deb、清单和 SHA-256 | 原生构建需 Ubuntu 20.04、Rust ≥ 1.89、`get-deps`、pkg-config；新版 Linux/WSL 用 `python3 scripts/gx_package.py deb --container`；不修改用户配置 |
| `make gx-install` | 源码路径安装：rust 检查 → `./get-deps`（需 sudo，交互确认）→ release 构建 → `dotfiles/install.sh` 部署 | 改 `$HOME` 下用户文件（先备份）；联网 |
| `make gx-upgrade` | **一条命令本机替换**：docker 容器构建 release 四件套（无 sudo；`GX_USE_LOCAL=1` 本机构建）→ `dotfiles/install.sh` 用户级部署（配置/插件/字体随快照，全带备份）→ 版本验证 | 改 `$HOME` 下用户文件（先备份）；需 docker、联网；已开窗口仍跑旧二进制需重启 |
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
- `ISCC`：指定 Windows Inno Setup 编译器的完整路径。

原生打包入口支持 `--check` 只读预检、`--bin-dir <release目录>` 复用匹配当前
提交的四个程序、`--output-dir <目录>` 指定产物位置，以及与 CHANGELOG 一致的
`--version X.Y.Z`。手动 GitHub 构建与发布见 [RELEASE.md](RELEASE.md)。

## Windows 本地构建（gnu 工具链）

适用范围：rustup gnu 主机（`x86_64-pc-windows-gnu`）、钉版 Rust 1.96.1（同
`scripts/gx_package.py`）+ nightly（fmt）+ MSYS2（`C:\msys64`）。gx-ci 与
`make gx-package-windows` 仍用 MSVC，本节只管本机开发期编译与测试。

### 工具与 PATH

在 MSYS2 终端安装（perl 仅在未启用系统 OpenSSL、需要编 vendored OpenSSL 时
才用到）：

```bash
pacman -S --needed mingw-w64-x86_64-{gcc,lld,sccache,openssl,python,lua54} make
```

gcc 连带 binutils 的 dlltool，并提供仓库 `.cargo/config.toml` 指定的
`x86_64-w64-mingw32-gcc` 链接驱动；python 提供框架脚本用的 `python3`
（Windows 自带的 `python3` 只是商店占位符）；lua54 提供跑
`scripts/tests/*.lua` 的 `lua5.4`。

每个会话先设 PATH（不改系统 PATH）：

| Shell | 命令 |
|---|---|
| Git Bash | `export PATH="/c/msys64/mingw64/bin:$PATH:/c/msys64/usr/bin"` |
| PowerShell | `$env:Path = "C:\msys64\mingw64\bin;$env:Path;C:\msys64\usr\bin"` |

mingw64 在前，提供 gcc/dlltool/ld.lld/python3 与测试二进制运行所需的 MinGW
运行时 DLL；usr/bin 放末尾只为提供 make，不遮蔽 Git 自带工具。

### 机器级 cargo 配置

`~/.cargo/config.toml` 追加：

```toml
[build]
rustc-wrapper = "C:/msys64/mingw64/bin/sccache.exe"

[target.x86_64-pc-windows-gnu]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]

[profile.dev]
debug = "line-tables-only"

[env]
X86_64_PC_WINDOWS_GNU_OPENSSL_NO_VENDOR = "1"
X86_64_PC_WINDOWS_GNU_OPENSSL_DIR = "C:/msys64/mingw64"
X86_64_PC_WINDOWS_GNU_OPENSSL_STATIC = "1"
WINAPI_NO_BUNDLED_LIBRARIES = "1"
```

| 配置 | 原因 |
|---|---|
| `[env]` 三个 OpenSSL 变量 | openssl-src 在 Windows 宿主上不转发 cargo jobserver（`openssl_src::Build::try_build` 仅在 `!cfg!(windows)` 时把 `CARGO_MAKEFLAGS` 交给 make），vendored OpenSSL 的 make 永远单线程：基线冷构建中 openssl-sys 构建脚本独占 1404.7 s，只用 1 核。openssl-sys 优先读 `<TARGET>_OPENSSL_*` 前缀变量，所以只影响 gnu 目标（MSVC 打包仍走 vendored）；改用 MSYS2 预编译 OpenSSL（3.6.4，vendored 为 3.6.3）静态链接 |
| `WINAPI_NO_BUNDLED_LIBRARIES` | winapi 0.3 自带的 `libwinapi_*.a` 实为 GNU ld 链接脚本（`INPUT(...)`），lld 报 unknown file type；设置后 winapi（以及 ntapi）改用 MinGW 自带的标准导入库。ntapi 的构建脚本读这个变量却没声明 rerun-if-env-changed：在已有 target 目录里切换它时，要删该目录下的 `debug/build/ntapi-*` 与 `debug/.fingerprint/ntapi-*`，否则报找不到 `-lwinapi_ntdll` |
| `link-arg=-fuse-ld=lld` | lld 多线程链接替代 GNU ld（经仓库配置的 `x86_64-w64-mingw32-gcc` 驱动调用 mingw64 下的 `ld.lld`） |
| `debug = "line-tables-only"` | 保留 backtrace 文件行号，明显缩短编译与链接；需要调试器看局部变量时临时设 `CARGO_PROFILE_DEV_DEBUG=full`（切换会触发重编） |
| `rustc-wrapper`（sccache） | cargo clean、新 target 目录、多 worktree 时复用依赖与 C 代码的编译结果；走增量编译的工作区 crate 不进缓存 |

放在机器级而非仓库：make、直接 cargo、IDE 看到同一套 rustflags/profile，避免
指纹不一致互相触发全量重编（临时设 `RUSTFLAGS` 会整体覆盖上面的 rustflags，
同样触发重编）；仓库 `.cargo/config.toml` 是上游文件，按 fork 治理不改。代价是
它对本机该用户的所有 cargo 项目生效（sccache 与 `[profile.dev]` 对 msvc 目标
同样生效），其他项目首次构建会整体重编。

sccache 缓存上限（默认 10 GiB）写在 `%APPDATA%\Mozilla\sccache\config\config`，
设为 40 GiB；改后执行 `sccache --stop-server` 生效，`sccache --show-stats`
看命中：

```toml
[cache.disk]
size = 42949672960
```

并行度：cargo 默认 jobs 等于逻辑核数、nextest 默认同数测试线程，无需另设；
提速来自消除串行段（单线程的 vendored OpenSSL make、GNU ld 链接）。

### Git Bash 下的 make 与框架命令

- Git 与 MSYS2 两套 msys-2.0.dll 构建不同，Git Bash 启动 MSYS2 make 时子进程
  只拿到 PATH/SYSTEMROOT 等 5 个变量。Makefile fork 段只补缺失的
  TMP/TEMP/USERPROFILE/LOCALAPPDATA：缺 TMP/TEMP 时 dlltool 报无法在
  `C:\WINDOWS\` 建临时文件，缺 USERPROFILE 时 Python `Path.home()` 崩溃，缺
  LOCALAPPDATA 时找不到用户级 Inno Setup。
- 在 Git Bash 里 export 的变量（`RUSTFLAGS`、`CARGO_*`、`K9_UPDATE`、
  `GX_SYNC_WRITE` 等）过不了这道边界，要写成 make 命令行变量（如
  `make test K9_UPDATE=1`）或写进 cargo 配置。ProgramFiles 与 HOME 未恢复：经
  make 跑 framework-test 时 Windows 资源编译器用例会被跳过，HOME 是 MSYS2 的
  `/home/<用户>`。从 PowerShell/cmd 启动 make 不受影响。
- 框架命令的 `.sh` 入口（setup、generated-check、graph 等）经 PATH 中的
  Git Bash 或 MSYS2 bash 执行，跳过 System32 与 WindowsApps 下的 WSL 启动器；
  找不到 bash 时 `make ai-doctor` 报 MISSING，`make <目标>` 报错说明原因。
- `make setup` 安装钉版 nextest/stylua 的 Windows 版（sha256 校验）；框架 venv
  跳过（python3 ≥ 3.11 自带 tomllib，resolver 可直接运行；`make graph` 需自备
  graphify）。下载优先用系统自带 `curl.exe`，走 Windows 证书库（杀软或代理解密
  HTTPS 时，MSYS2 curl 的自带 CA 包会报自签名证书）；解压需要 `unzip`（Git for
  Windows 自带）。

### 实测对比

i9-13900H（20 线程），2026-09-30 同一提交：

| 场景 | 优化前 | 优化后 |
|---|---|---|
| 冷构建全部测试二进制（`cargo test --workspace --no-run`，空 target） | 1613 s | 410 s（3.93 倍） |
| 核心 crate 增量（touch `term/src/terminal.rs` 后重跑上一行，中位数） | 31 s | 28 s |
| wezterm-gui 链接增量（touch `wezterm-gui/src/main.rs` 后 `cargo build -p wezterm-gui`，中位数） | 11 s | 6 s |
| sccache 热缓存冷构建（删除 target 后在同一路径重跑冷构建） | — | 248 s（命中率 97.2%） |
| 冷构建平均活跃编译单元 | 3.71 | 16.57 |
| 冷构建串行时段（活跃单元 ≤1） | 1271 s | 12.7 s |
| 冷构建中 openssl-sys 构建脚本 | 1404.7 s | 6.1 s |
| 冷构建整机 CPU 采样（含杀软等后台噪声） | 42.1% | 61.1% |

口径：`date +%s` 计墙钟，增量两项各跑 3 次取中位数；平均活跃编译单元取自
`cargo --timings`，是构建自身的并发度，比整机 CPU% 更能反映利用率。sccache 的
缓存键含 target 目录绝对路径，换目录时命中率低得多（主 `target/` 首次重编实测
约四成）。

剩余瓶颈：关键路径是 freetype → harfbuzz 构建脚本（各约 100 s）→ wezterm-font
→ window → wezterm-gui；libgit2-sys、libssh-rs-sys、cairo-sys-rs 构建脚本各
150–210 s，不在关键路径上但与 Rust 编译抢核。新生成的 exe 首次运行明显偏慢：
全新 target 上 `make test` 共 841 s，其中约 527 s 花在 nextest 首次枚举 72 个新
测试二进制上（再次运行时枚举加执行约 10 s），`wezterm.exe --version` 首次 24 s、
再次不到 1 s；构建初期构建脚本 exe 的启动同样偏慢。这与杀软扫描新可执行文件的
表现一致（未证实），给 target 目录加杀软排除可能还有收益（用户自行决定）。核心
crate 增量以工作区 crate 的增量编译为主，sccache 不覆盖，收益有限。

### 与 CI 的差异及未采纳项

本机 gnu + 系统 OpenSSL，CI 为 msvc + vendored；CI 仍按原矩阵把关。本机复现
CI 形态见 [TESTING.md](TESTING.md) 的「Windows 本地验收前置」。

未采纳：nightly 的 `-Zthreads` 并行前端与 cranelift 后端（偏离钉版稳定
工具链）；切 MSVC 目标（需管理员安装 VS Build Tools）；永久 PATH、电源模式
「最佳性能」、杀软排除项（系统级设置，本机未采用，可按需自行设置）。

## 已知边界

- `make docs` 依赖 docker/podman 与 GitHub API（release 信息注入），fork
  日常不做文档站构建；上游 pages CI 把关。
- Windows 本机开发期构建与测试见上节（gnu 工具链），MSVC 形态由 gx-ci
  把关；macOS 平台门在本机无环境时以 PENDING 记录，由上游 CI 矩阵佐证
  （gen_* 工作流）。
