# 版本与发布

## 两套版本体系（互不干扰）

1. **产品版本（上游体系）**：`wezterm-version/build.rs` 在编译期生成——
   CI 打 tag 时读 `../.tag`，否则 `git show --format=%cd-%h` 得到形如
   `20260915-135123-2658f629c` 的 `WEZTERM_CI_TAG`；各 crate Cargo.toml
   的 version 只是占位。**不要**给 crate 手写语义版本。
2. **fork 流程版本（本框架）**：根 `CHANGELOG.md` 的
   `## X.Y.Z(日期|TBD)` 标题，最大数值 SemVer 为真源
   （`scripts/version.py`，`make version` 只读）。当前无镜像文件
   （`docs/dev-framework.json` 的 version_targets 为空），发布打包需求
   出现时再登记。

## fork changelog 纪律

- 记录 fork 层面已实现的可观察变更（框架、流程、定制功能）：行为、影响、
  验证。
- 上游产品变更在 `docs/changelog.md`，随上游同步产生，**不手写**
  （hooks 拦截；见 build-ci-release.md）。

## GX 安装包与手动发布

原生安装包版本来自根 CHANGELOG 最大 SemVer，正式标签为 `gx-vX.Y.Z`。
四个程序仍使用原有日期/hash 版本；构建清单同时记录包版本、产品版本、完整
提交 SHA、资源指纹、二进制校验和与 dirty 状态。dirty 包可供本地验证，不能发布。

本地命令（仓库根目录）：

```console
python scripts/gx_package.py windows --check
python scripts/gx_package.py windows
python3 scripts/gx_package.py deb --check
python3 scripts/gx_package.py deb
# 在较新的 Linux / WSL 上使用 Docker 构建相同的通用 deb：
python3 scripts/gx_package.py deb --container
```

`--bin-dir <dir>` 复用已构建的四个程序，仍检查其架构、版本与 HEAD；未指定时
执行 `cargo build --locked --release`。`--output-dir <dir>` 更改输出目录；
`make gx-package-windows` / `make gx-package-deb` / `make package` 是相应封装。
Windows 需要 MSVC、Perl、Rust >= 1.89、Inno Setup 6.7.3；可用
`winget install --id JRSoftware.InnoSetup --exact --version 6.7.3 --scope user`
安装编译器，非标准位置通过 `ISCC` 指定。Linux 原生构建基线固定 Ubuntu 20.04
amd64，先安装 Rust，再执行根 `get-deps`；打包还需 pkg-config、binutils。
打包入口兼容 20.04 自带的 Python 3.8，用户安装和运行不依赖 Python。
较新的 Linux/WSL 使用 `--container`，自动准备 Ubuntu 20.04 Docker 构建环境；
`--container --check` 只检查 Docker，不下载镜像或安装依赖。Cargo 缓存与构建产物
使用独立 `wezterm-gx-focal-*` Docker volumes，最终包仍写入宿主的 `dist/`。

同一个 deb 支持 Ubuntu 20.04 和 24.04，无须用户选择发行版或手动更换库。
构建沿用现有 `wezterm-ssh/vendored-openssl` feature 静态链接 OpenSSL，证书仍来自
系统 `ca-certificates`；其余依赖由 20.04 的 `dpkg-shlibdeps` 生成，apt 自动解析。
六个 ELF（含两个启动器）必须满足 GLIBC 符号版本 ≤ 2.31 且不动态依赖 libssl /
libcrypto，否则拒绝打包与发布。`--bin-dir` 也受同样检查；在 24.04 上直接编译的
旧二进制不能通过此门。清单的 `linux_compatibility` 记录逐程序 ABI 审计。

产物在 `dist/`：`WezTerm-GX-X.Y.Z-Setup-x64.exe`、
`wezterm-gx_X.Y.Z_amd64.deb`，各自附带 `.manifest.json`、`.sha256`。
Windows 首版未配置代码签名。deb 用 `sudo apt install ./wezterm-gx_X.Y.Z_amd64.deb`
安装，apt 负责运行依赖。`wezterm-gx` 是 Linux CLI，`wezterm-gx-gui` 是桌面入口；
Windows 快捷方式调用 `wezterm-gx.exe`，命令行入口为 `wezterm-gx-cli.exe`。

Windows 默认用户安装到 `%LOCALAPPDATA%\Programs\WezTerm GX`，可在向导选择
所有用户（Program Files）。无版本子目录；同安装范围原位升级，切换范围先
卸载旧范围再安装。旧用户级 `WezTerm (gx)` 快捷方式仅在目标属于旧 GX 目录时
更新；系统安装不冒充其他用户改写其快捷方式。旧版本目录保留供人工回退。

配置继续位于用户的 `.config/wezterm`（尊重 XDG_CONFIG_HOME），既有
`.wezterm.lua` 和个人配置保留。首次使用的普通用户由原生启动器初始化；
插件备份在 `%APPDATA%\wezterm-gx\backups` 或 `$XDG_DATA_HOME/wezterm-gx/backups`
（未设 XDG 时为 `~/.local/share/wezterm-gx/backups`），不污染插件扫描目录。
首次接管四个打包插件时保留备份和 state/；卸载不删除用户数据。
升级前应保存会话并关闭旧版窗口及后台 mux；Windows 中旧进程可能锁住插件目录，
启动器会保留原插件并给出重试提示，不强制结束用户 shell。启动器预建 resurrect
会话目录；直接加载插件时，缺失目录的异步创建会避开 Lua require 的 C 调用边界。

GitHub 操作流程：

1. 将 `gx-release` 入口通过正常合并进入默认分支 `main`，目标构建分支也须含
   同版打包脚本；只有默认分支存在 workflow_dispatch 才能从 Actions 页面运行。
2. Actions → gx-release → Run workflow，填写 ref（默认 feature/gx_wezterm）
   和 X.Y.Z（必须匹配目标提交的 CHANGELOG）。publish 默认为 true；关掉只构建。
3. prepare 固定 SHA → Windows 构建及安装冒烟；Linux 在 20.04 容器构建一次，
   同一个 deb 分别在干净 20.04、24.04 容器安装、升级、GUI 冒烟及卸载。
4. publish=true 且两平台成功时核对清单，创建/复用同 SHA 标签，上传完整草稿再公开。
   已发布版本、指向其他提交的标签、不同资源、缺失或损坏文件均拒绝；失败草稿
   仅在已上传文件摘要完全相同时可继续。普通 push/tag 不触发此发版流程。

`gx_smoke_windows.ps1` 只允许运行在可丢弃 Actions runner；Linux 生命周期测试
默认也限制在 Actions，本地可在测试系统显式传 `--allow-system-install`，且拒绝
替换已有 wezterm-gx 包。用户初始化逻辑另有无需安装权限的原生单元测试。
`linux-verify` 使用 20.04/24.04 矩阵，以普通测试账户启动 GUI；任一系统失败均
阻止发布。截图分别上传到 `evidence-linux-20.04` 与 `evidence-linux-24.04`。
本地以 root 驱动测试时必须通过
`GX_SMOKE_USER` 指定非 root 账户，安装动作与用户初始化分开执行。

## 上游发布边界

上游发布链：`ci/tag-name.sh` → `ci/tag.sh` → gen_*_tag 工作流构建 →
`ci/create-release.sh`（gh release --prerelease）→ `ci/deploy.sh` 按平台
打包（macOS 签名公证/deb/AppImage/Inno Setup）→ 分发渠道（flathub/
winget/homebrew/copr）。这些只在上游仓库由维护者运行；本 fork 的 hooks
对 `gh release`/`cargo publish`/推 upstream 直接 deny。

GX 使用上述独立打包/发布入口；上游发布链与分发渠道仍由上游维护者管理。
