# 测试与证据

分层语义：每层写明能证明什么、不能代证什么；不同层不能互证。

## 分层清单

| 层 | 命令 | 前置 | 能证明 / 不能代证 |
|---|---|---|---|
| 静态-格式 | `make lint` | nightly 工具链 | 排版一致 / 不能证行为 |
| 静态-类型 | `make typecheck`（=make check） | cargo | 编译期契约 / 不能证运行时 |
| 单元 | `cargo nextest run -p <crate>` | nextest（make setup） | 模块内行为+失败路径 / 不能证跨进程 |
| 全量 | `make test` | nextest；ssh e2e 需 /usr/sbin/sshd | 全 workspace（含 escape-parser no_std 轮） / 不能证 GUI 视觉 |
| 集成-ssh | `make test-integration` | 本机 sshd（fixture 自动起高位端口实例） | SSH 认证/sftp/agent 转发 e2e / 不能证其它域 |
| heavy（CI 形态） | `make test-heavy` | 同上 | 全量+失败不中断收集完整信号 / 不是更高覆盖 |
| 生成物 freshness | `make generated-check` | 二进制链需 target/debug/wezterm | 派生文件与生成器一致 / 不能证生成器正确 |
| 图谱/KB freshness | `make graph-check / kb-check` | 无 | 产物与源码指纹一致 / 不能证内容质量 |
| UI 冒烟 | `make ui-smoke`（Windows 分派到 `scripts/ui_smoke_windows.ps1`） | Linux：target/debug/wezterm-gui、Xvfb、xwd、ffmpeg；Windows：target/release/wezterm-gui.exe（或 `-Exe`）、pwsh/powershell、交互式桌面 | 窗口可起、标记文本渲染、浮层可打开 / 不能证交互细节 |
| 实机平台 | NixOS VM（上游流程） | nix | GNOME/KDE 下真实桌面行为 / 人工流程 |
| 框架自身 | `make framework-test` | 无 | resolver/hooks/KB 契约 / 不测产品代码 |

## Windows 本地验收前置

仓库路径必须纯 ASCII：非 ASCII 路径会让 Perl/nmake 把 OpenSSL 产物写进乱码
目录。先由用户提供 Git for Windows、Python>=3.10 的真实解释器、rustup 与
Rust 1.96.1 MSVC、nightly rustfmt，以及带 x64 C++ 工具和 Windows SDK 的 Visual
Studio。wrapper 通过 vswhere 发现已安装的 VS，不限定 2022；普通构建不需要 Inno
Setup，打包时才需人工安装 Inno Setup >=7.1。工具详情见
[MAKE_COMMANDS.md](MAKE_COMMANDS.md#首次安装与-windows-msvc-入口)。

首次在仓库根目录的 **Git Bash** 安装项目工具，不要求已有 make：

```bash
bash scripts/setup_env.sh
bash scripts/setup_env.sh --check
```

安装器钉版安装 nextest、StyLua、Lua、框架 venv、NASM 和 Strawberry Perl，并从
已校验的 Perl portable 包提取 GNU Make 4.4.1 及 libintl/libiconv DLL。venv 补
`Scripts/python3.exe`，无需修改用户 Python 别名。安装结束与 `--check` 都严格
检查工具版本、可运行性和 MSVC 编译前置；健康门通过不代表产品构建或测试通过。

随后在仓库根目录的 **cmd 或 PowerShell** 执行（下列命令两者通用）：

```bat
.\scripts\gx_msvc_env.cmd --check
.\scripts\gx_msvc_env.cmd make check
.\scripts\gx_msvc_env.cmd make build
.\scripts\gx_msvc_env.cmd make test
```

wrapper 只在子进程环境选择 `1.96.1-x86_64-pc-windows-msvc`，前置仓内工具及
Git for Windows，不改全局默认 Rust、PATH 或用户配置。Rust 版本与
`scripts/gx_package.py` 对齐，nextest 以 `scripts/setup_env.sh` 为准；升级时同步
wrapper。`--check` 还验证 CMake、Windows SDK 与 nightly rustfmt。构建统一写入
`target/`，sccache 与临时文件分别固定为 `.local/sccache`、`.local/tmp`，不再使用
`target-gx-msvc/`。工具根覆盖形式见命令手册。

wrapper 自动追加 C/C++ `/utf-8`，与 NASM 3.02 一起保护非 UTF-8 系统代码页场景，
但不能替代纯 ASCII 仓库路径。它直接使用 Git `usr/bin/sh.exe`，避免 Git Perl
遮蔽项目 Perl；Makefile 阻止 GNU Make 的 flags 传给 OpenSSL NMake。需要锁定依赖
或收集全部测试失败时，可以在同一入口显式运行：

```bat
.\scripts\gx_msvc_env.cmd make build "BUILD_OPTS=--release --locked"
.\scripts\gx_msvc_env.cmd cargo nextest run --locked --all --no-fail-fast --test-threads 2
.\scripts\gx_msvc_env.cmd cargo nextest run --locked -p wezterm-escape-parser
```

`BUILD_OPTS` 会整体覆盖默认 `--release`，只写 `--locked` 会退回 dev 构建。
Windows checkout 可能带 CRLF；Linux/WSL 验证应使用 Linux 的 LF checkout，不直接
复用 CRLF shell 脚本。

## 测试约定

- 断言库 k9：`k9::snapshot!` 快照 + `assert_equal` 别名；更新快照
  `K9_UPDATE=1 cargo test`，更新即行为变更，必须逐个审 diff 并在交付说明
  列出。
- 测试组织：以 crate 内联 `#[cfg(test)]` 为主（term 的脚手架
  `term/src/test/mod.rs::TestTerm`）；集成目录仅 wezterm-ssh/tests、
  bidi/tests、wezterm-dynamic/tests。
- 新行为先写会失败的用例再修；必要工具缺失时记 PENDING（附补验命令），
  不得整族 skip 后报绿。
- oracle 数据（bidi conformance、escape round-trip）不得裁剪"修绿"。

## 截图与证据闭环

1. `make evidence TASK=<任务>` 分配批次目录（`.ui-evidence/<分支URL编码>/
   <任务>/<UTC时间戳-uuid>/`，git check-ignore 验证过才写入）。
2. `make ui-smoke`（或 `scripts/ui_smoke.sh --out <批次目录>`）：隔离
   Xvfb 显示（占用即失败）→ 启动 wezterm-gui → 渲染标记文本 → xwd 抓屏
   → ffmpeg 转 `before.png` → 写初始 result.json（status=CAPTURED，
   images_reviewed=false）。
   Windows 分支（`os.name == 'nt'` 时 `make ui-smoke` / `python3
   scripts/dev_framework.py run ui-smoke [--out <批次目录>]` 自动转到
   `scripts/ui_smoke_windows.ps1`，用 pwsh、找不到再用 powershell）：真实桌面
   窗口，没有 Xvfb 隔离；`PrintWindow(PW_RENDERFULLCONTENT)` 抓图，黑图或底部白带再
   临时 `TOPMOST|NOACTIVATE` + `CopyFromScreen`，全程不 `SetForegroundWindow`。
   `--out` 必须是证据根下的仓库相对路径，缺省为 `.ui-evidence/smoke/<时间戳>-<pid>`。
   要选浮层/配置/可执行文件时直接调脚本：`pwsh -File scripts/ui_smoke_windows.ps1
   -Out <批次目录> [-Exe ..] [-ConfigFile ..|-NoConfig] [-Overlay
   none|palette|settings|keybinds|menu|wallpaper|context-menu|confirm|launcher|copy-mode] [-Label ..]`，
   输出 `<Label>[-<overlay>].png`，result.json 的 `captures`（抓图方式、键位表、
   抓图时是否前台）与 `skipped`（浮层在该配置下不可达，不算失败）逐项登记。
   键盘浮层只在前台窗口属于被测 wezterm 时发键，否则记 skipped、不向别的程序发键；
   键位表按配置推断（默认键 / GX 配置的 leader 层），本机 leader 被别的程序占作
   全局热键时用 `-LeaderKey F13..F20` 替换；输入法处于中文模式会吞掉 leader 层的
   字母键，脚本发键前把窗口输入法切到字母数字、抓图后还原（`-KeepIme` 关闭）。
3. **读回图片**：执行者必须实际查看 before.png，核对标记文本
   `WEZTERM-UI-SMOKE-OK-…` 与基本布局（有字、有色、无花屏）。
4. 判定：核对通过→ result.json 改 `status: PASS, images_reviewed: true`；
   失败→ `status: FAIL` 并保留截图，修复后**另开批次**复测。
5. 手动配色截图（上游流程）：`ci/make-color-screen-shots.sh`（xwininfo
   选窗 + ImageMagick），产物进 `docs/colorschemes/`，属上游文档链。
6. Windows 性能探针（不进 CI，真实桌面窗口，运行期间别动键鼠）：
   `pwsh -File scripts/perf_probe_windows.ps1 -Out <批次目录> -Scenario
   idle|cat|loop|scroll|spinner [-Seconds 60] [-MaxFps <n>|follow] [-Exe ..]
   [-ConfigFile ..|-NoConfig]`。以 `periodic_stat_logging=10` 启动，输出
   `metrics-<场景>-<fps>.txt`（stderr 原文）、`cpu-<场景>.csv`（wezterm-gui 与其
   子进程树的 CPU，按逻辑核数归一）、`gpu-engine-<场景>.csv`（GPU Engine 计数器）、
   `gpu-<场景>.txt`（nvidia-smi pmon 原文，WDDM 下常无数值）与
   `summary-<场景>.json`（绘制速率/时延、各缓存命中/未命中、atlas 分配失败率、
   CPU/GPU 均值）。同名输出已存在时拒绝覆盖；批次 result.json 的 `probes`
   登记各场景。性能对比必须同一机器、同一配置、同一 `-MaxFps` 才有意义。

## CI（GX）

`.github/workflows/gx-ci.yml` 在 `feature/gx_wezterm` 的 push、目标为该分支的
PR 时自动运行，也支持手动运行；不按文件类型过滤，避免漏掉 Lua、脚本或规则变更。

- 快速检查：resolver 闭集、框架配置、版本、冻结配置指纹、Python 框架测试与 nightly rustfmt。
  `gx_config_fingerprints.py --check` 不依赖发布 Git 对象；单测覆盖无 Git 归档、
  任意父目录、历史指纹金标及显式发布对象审计。
- Ubuntu 24.04 / Windows 2025：类型检查、四个产品二进制构建、nextest 全量测试，
  以及 escape-parser 独立 no_std 测试轮。Rust 与 nextest 版本分别读取现有打包
  脚本与环境安装脚本的钉版声明。
- Linux 必须安装并确认 SSH 服务端和客户端工具可用，避免缺少 sshd 导致测试
  静默跳过；Windows 源码中原有 ignored 的 SSH e2e 由 Linux 侧覆盖。
- Windows 先确认 SDK 可用，再运行 `scripts/test_windows_resources.py`，用真实
  RC 编译三个程序模板，验证中文/空格资源路径和非 UTF-8 默认代码页；SDK 缺失
  在 CI 中直接失败，不接受跳过。

自动 CI 只有只读仓库权限；同一 PR/分支的新提交取消旧检查，不取消 `gx-release`。
它不替代手动发布流程的安装/升级/卸载及 GUI 冒烟，也不证明其它发行版或 macOS
兼容性。本地聚合检查入口仍是 `make ci-check`，框架测试另跑 `make framework-test`。

## CI（上游归档）

由 `ci/generate-workflows.py` 生成的 gen_* 多平台矩阵，以及 fmt、termwiz、
wezterm_ssh 双后端检查等工作流，原样保存在 `.github/workflows-archive/`。
它们不在 GX 分支触发；上游生成器与工作流正文保持不变，`main` 用于同步上游。
