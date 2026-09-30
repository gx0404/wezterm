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
GX 编译前将提交的 UTC 时间与 8 位 SHA 写入 ignored `.tag`，沿用上游版本格式，
保证 Windows/Linux 以及缓存重建得到同一产品版本。

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
`--stage-dir <新目录>` 只构建、核验并输出载荷（Windows：`app/`、`fonts/`；deb：
`root/`）与 `stage-manifest.json`（deb 另含 `deb_depends`），不调用 Inno Setup /
dpkg-deb，可与 `--container` 组合。`gx0404/gx_shell` 只保存组件 manifest 和集成
打包代码，不再保存 WezTerm 源码；从本仓 `gx` 分支选择完整 SHA，在仓外独立
checkout 并递归初始化子模块，再调用此入口。脏检查只统计组件工作树，容器构建
挂载该 Git 工作树。stage 构建仍需要 Git checkout；源码归档只保证指纹生成/校验
无需 Git，不将缺少子模块和提交身份的归档伪装成可发布构建。

集成 manifest 至少应锁定 `repository`（`https://github.com/gx0404/wezterm.git`）、
`branch`（`gx`，仅供维护导航）、`revision`（完整 40 位 SHA，实际 checkout 真源）、
`submodules`（递归）、`package_version`（本仓 CHANGELOG）、`license`（`LICENSE.md`）
以及指纹登记表路径 `scripts/gx-config-releases.json`。`stage-manifest.json` 的
`source_commit` 必须等于组件 `revision`，而非集成仓提交；`package_version` 也
不可用 GX Shell 的版本替代。集成侧分别记录两仓身份，并校验 `source_dirty=false`、
`platform`、`architecture`、`product_version`、`resource_version`、`binaries`；
deb 另外消费 `deb_depends` 与 `linux_compatibility`。原有 stage 布局保持不变。
Windows 需要 MSVC、Perl、Rust >= 1.89、Inno Setup >= 7.1（长插件路径支持）；可用
`winget install --id JRSoftware.InnoSetup.7 --exact --version 7.1.0 --scope user`
安装编译器，非标准位置通过 `ISCC` 指定。Linux 原生构建基线固定 Ubuntu 20.04
amd64，先安装 Rust，再执行根 `get-deps`；打包还需 pkg-config、binutils。
打包入口兼容 20.04 自带的 Python 3.8，用户安装和运行不依赖 Python。
较新的 Linux/WSL 使用 `--container`，自动准备 Ubuntu 20.04 Docker 构建环境；
`--container --check` 只检查 Docker，不下载镜像或安装依赖。Cargo 缓存与构建产物
使用独立 `wezterm-gx-focal-*` Docker volumes，最终包仍写入宿主的 `dist/`。
也可指定 `--cache-dir <dir>` 将 registry/git/target 缓存放在宿主目录；CI 使用
`.local/gx-deb-cache` 并通过 Actions cache 跨 runner 复用。
构建退出时会将显式缓存目录归还宿主用户，确保含私有权限文件的 crate 也能归档。
容器和两平台 CI 共用 `scripts/gx_package.py::RUST_VERSION` 钉定的 Rust 版本，
升级只改此处。
Windows CI 从 [Inno Setup 官方 Release](https://github.com/jrsoftware/issrc/releases/tag/is-7_1_0)
下载 7.1.0 并核对固定 SHA-256，避免第三方软件源缺失版本导致构建中断。
`ISCC` 显式路径优先于 PATH；预检拒绝旧编译器，避免长 Git 插件元数据路径在
安装时触发 MAX_PATH 限制。

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
`.wezterm.lua` 和个人配置保留。首次使用的普通用户（配置目录不存在，或只有设置页写的
`gui-settings.json`，该文件保留）由原生启动器初始化；
插件备份在 `%APPDATA%\wezterm-gx\backups` 或 `$XDG_DATA_HOME/wezterm-gx/backups`
（未设 XDG 时为 `~/.local/share/wezterm-gx/backups`），不污染插件扫描目录。
首次接管四个打包插件时保留备份和 state/；卸载不删除用户数据。
已有配置按文件迁移（`wezterm-gx/config-version` 记下载荷版本、已发布指纹表摘要与配置目录；
指纹表或根因文件变化时重跑）：与任一已发布版本相同的文件（文本 CRLF 视同 LF）先备份到
上述 `backups/<时间戳>/wezterm-config/` 再换成载荷版；新增文件补装（入口 `wezterm.lua`
不是 GX 发布的版本时只补新代码 require 的模块），删掉的已发布文件只在新代码 require 时
补回；改过的文件、只读文件与符号链接保留，原因和载荷路径写进
`wezterm-gx/config-migration.log`；不再随包的文件原样留下；`gui-settings.json` 永不触碰。
各发布版都没变过的文件（如 `wezterm.lua`）改了也不挡升级；改了本次也更新的文件时，它
require 的已更新模块及依赖者留在旧版，日志写明根因文件，该文件换回载荷版后下次启动补做。
相互 require 的一组文件全有或全无，失败整组回滚，最多尝试 3 次并复用同一备份目录。
已发布文件指纹表 `scripts/gx-launcher/released.rs` 从仓内不可变登记表
`scripts/gx-config-releases.json` 生成；`python scripts/gx_config_fingerprints.py --check`
只读校验，在没有 Git 的源码归档、浅克隆和任意父目录中都必须通过。登记表记录各版
repository/tag/commit、config tree 与每个文件的 blob/长度/FNV-1a，旧条目及顺序
不可修改（顺序决定 bitmask）。已冻结 WezTerm GX 0.3.0、GX Shell 0.1.0/0.2.0，
76 条合并指纹与迁出前逐项相同；启动器代码不变。当前 `u8` 最多容纳 8 个版本，
超过时拒绝登记，须先设计启动器位宽迁移。

发布后，在本仓执行以下显式登记（参数取真实发布信息，不使用工作区文件）：

```console
python scripts/gx_config_fingerprints.py --record --source-repo <release-checkout> --name "GX Shell X.Y.Z (tag gx-shell-vX.Y.Z)" --repository https://github.com/gx0404/gx_shell --tag gx-shell-vX.Y.Z --commit <full-commit-sha> --prefix <config-tree-path>
python scripts/gx_config_fingerprints.py --check --verify-git --source-repo <history-checkout>
```

`--record` 要求 tag 指向给定完整提交，并只追加登记。`--verify-git` 严格复算每个
已登记发布：指定 checkout 必须包含相应历史对象，否则失败，不把缺失当作通过。
拆仓后发布的新 GX Shell 若仅锁定组件，指纹应从该 manifest 钉定的 WezTerm
发布 checkout 登记（`--repository`/`--tag`/`--commit` 指向组件，`--name` 仍标注
GX Shell 版本，`--prefix dotfiles/wezterm-config`），不能再引用集成仓的源码路径。
该组件 SHA 应先获得对应不可变 tag，再执行登记。

集成仓的发布门应显式运行 `--check --gx-shell-changelog <integration-CHANGELOG>`：
最大标题以外的带日期版本必须已登记。组件默认检查不隐式访问这个文件，未来漏登
仍由集成侧门和发布流程负责拦截，而不是靠邻接目录猜测。
升级前应保存会话并关闭旧版窗口及后台 mux；Windows 中旧进程可能锁住插件目录，
启动器会保留原插件并给出重试提示，不强制结束用户 shell。启动器预建 resurrect
会话目录；直接加载插件时，缺失目录的异步创建会避开 Lua require 的 C 调用边界。

GitHub 操作流程：

1. 本 fork 的默认分支设为 `gx`，`main` 专门同步上游。
   `gx-release` 必须存在于默认分支，目标构建分支也须含同版打包脚本；
   GitHub 的手动入口要求的是默认分支，不限定分支名为 `main`。
2. Actions → gx-release → Run workflow，Branch 选择 `gx`，
   ref 保持默认即可，**version 留空自动取目标提交的 CHANGELOG 版本**。
   若手填 X.Y.Z，必须与 CHANGELOG 一致；错误会同时显示输入值、期望值和修正方式。
   publish 默认为 true；关掉仍完整构建和验证，但不创建标签或 Release。
3. prepare 固定 SHA/版本/工具链；发布模式先只读检查正式版本和标签冲突，再开始
   Windows 构建及安装冒烟；Linux 在 20.04 容器构建一次，
   同一个 deb 分别在干净 20.04、24.04 容器安装、升级、GUI 冒烟及卸载。
4. 独立 verify 任务在所有安装测试通过后核对双平台 SHA、产品版本、资源指纹、
   完整二进制清单与校验和；**仅构建模式也必须通过该检查**。
5. publish=true 时创建/复用同 SHA 标签，上传完整草稿，逐文件核对服务端大小和
   SHA-256 后再公开；只有该任务拥有 contents: write 权限。
   已发布版本、指向其他提交的标签、不同资源、缺失或损坏文件均拒绝；失败草稿
   仅在已上传文件大小和摘要完全相同时可继续。普通 push/tag 不触发此发版流程。

同一仓库的 GX 运行串行排队，避免自动版本与手填版本同时发布同一标签。任务 Summary
展示固定 SHA、版本、工具链和运行模式；关掉 publish 后，从成功运行底部 Artifacts
下载 `gx-windows-installer` 和 `gx-linux-deb`（保留 14 天）。版本已经发布时，可关掉
publish 做回归构建；要正式再发一个版本，先在 CHANGELOG 增加新的 SemVer 并提交。
不要重跑旧失败任务期待它使用新代码；修复推送后重新 Run workflow。

`gx_smoke_windows.ps1` 只允许运行在可丢弃 Actions runner；Linux 生命周期测试
默认也限制在 Actions，本地可在测试系统显式传 `--allow-system-install`，且拒绝
替换已有 wezterm-gx 包。用户初始化逻辑另有无需安装权限的原生单元测试。
`linux-verify` 使用 20.04/24.04 矩阵，以普通测试账户启动 GUI；任一系统失败均
阻止发布。截图分别上传到 `evidence-linux-20.04` 与 `evidence-linux-24.04`。
Windows 安装/升级/卸载日志和字体加载记录上传到 `evidence-windows`。
构建成功后，即使 Windows 安装测试失败也保留安装器供排障；发布仍要求所有验证通过。
两平台测试均强制走插件升级分支，逐文件比较配置、壁纸设置、会话和自定义插件的 SHA-256，
确认升级与卸载保留用户数据。自定义插件夹具含真实 Git 元数据与离线 remote；升级后
再次加载配置，并检查 GUI 日志，避免插件加载失败但程序仍启动造成冒烟误报通过。
该测试不等同于人工观察 Windows 冷启动闪窗。
本地以 root 驱动测试时必须通过
`GX_SMOKE_USER` 指定非 root 账户，安装动作与用户初始化分开执行。

## 工作流目录与上游同步

GX 分支的 `.github/workflows/` 保留两个独立入口：

- `gx-ci.yml`：`gx` 的 push、目标为该分支的 PR 自动触发，
  也可手动运行。检查格式、框架规则及 Windows/Linux 构建与测试，不生成安装包
  或发布 Release；同一 PR/分支的新运行取消旧 CI，不影响发布流程。
- `gx-release.yml`：仅手动触发，构建并验证双平台安装包，按 `publish` 选项发布。

35 个上游流程及旧版 `gx-windows-build` 仍原样保存在
`.github/workflows-archive/`，GitHub 不从归档目录加载工作流。停用操作本身
不会隐藏侧栏条目；移出定义后，旧运行记录仍可能让对应的历史流程保留在列表中。
旧 Windows 构建记录与产物保留。

`main` 的上游工作流与提交历史保持原样。以后先在 `main` 同步上游，再正常
合并到 GX 分支；Git 可通过文件重命名识别已归档流程，遇到冲突时保留归档
路径并合入上游内容。合并后检查 `.github/workflows/`，将新引入的非 GX
工作流也原样移入归档目录，逐文件核对内容并审阅 diff。

上游生成器仍输出到原来的 `.github/workflows/`；若有意运行生成器，完成后
按同样方式归档生成结果。归档只调整入口位置，不手改生成文件正文。

## 上游发布边界

上游发布链：`ci/tag-name.sh` → `ci/tag.sh` → gen_*_tag 工作流构建 →
`ci/create-release.sh`（gh release --prerelease）→ `ci/deploy.sh` 按平台
打包（macOS 签名公证/deb/AppImage/Inno Setup）→ 分发渠道（flathub/
winget/homebrew/copr）。这些只在上游仓库由维护者运行；本 fork 的 hooks
对 `gh release`/`cargo publish`/推 upstream 直接 deny。

GX 使用上述独立打包/发布入口；上游发布链与分发渠道仍由上游维护者管理。
