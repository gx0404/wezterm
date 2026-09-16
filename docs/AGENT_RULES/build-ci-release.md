# build-ci-release：构建、CI、依赖与生成物

## 范围

`ci/`、`.github/`、`.cirrus.yml`、`nix/`、根 `Cargo.toml`/`Cargo.lock`、
`deny.toml`、`cooldown.toml`、`get-deps`、`.cargo/`、`.rustfmt.toml`、
`mkdocs_macros.py`、`wezterm-version/`。

## 符号真源

- 命令入口：根 `Makefile`（上游段：test/check/build/fmt/docs/servedocs；
  框架段见 development 域）。CI 矩阵由 `ci/generate-workflows.py` 生成
  （gen_*.yml 不要手改——改生成器再重新生成）。
- 版本：`wezterm-version/build.rs`——存在 `../.tag`（CI 注入）则用之，否则
  `git show --format=%cd-%h` 生成 `WEZTERM_CI_TAG`（形如 20260915-135123-
  2658f629c）；fork 的流程版本另见根 CHANGELOG.md（development 域）。
- 依赖：全部经根 `[workspace.dependencies]`（约 240 项）；许可检查
  `deny.toml`；自动升级走 `cooldown.toml` + cargo-cooldown（仅上游仓库启用）。
- 子模块：`deps/harfbuzz/harfbuzz`、`deps/freetype/{freetype2,libpng,zlib}`
  ——只能 `git submodule update` 同步，不手改内部。
- NixOS VM：`nix/flake.nix#testing-on-gnome|plasma`（人工 GUI 冒烟用；
  nix 构建会移除 wezterm-ssh/tests，因沙箱内 sshd 密钥交换失败）。

## 生成物清单（改了生成器必须重建并审 diff）

| 产物 | 生成器 | 检查 |
|---|---|---|
| `assets/shell-completion/{bash,zsh,fish,…}` | `ci/update-derived-files.sh`（用 target/debug/wezterm） | `make generated-check` |
| `docs/examples/default-*-key-table.markdown` | 同上（show-keys --lua） | 同上 |
| `docs/cli/**` 各子命令 help | 同上 | 同上 |
| `docs/config/lua/**`、`docs/SUMMARY.md`、`../mkdocs.yml` | `ci/generate-docs.py` + rustdoc 注释 | `make generated-check` |
| `config/src/scheme_data.rs` | `sync-color-schemes`（联网） | 触发式重建 |
| `wezterm-char-props/src/{emoji_variation,nerdfonts_data}.rs` | `wezterm-char-props/codegen`（`cargo run`） | 触发式重建 |
| `wezterm-gui/src/unicode_names.rs` | 上游 codegen | 不在 fork 重建 |

`make generated-check`（scripts/generated_check.sh）只读比对；有意更新用
`make generated-write`（= ci/update-derived-files.sh，需先构建 debug 二进制）。

## 不变量

- gen_*.yml 是生成物：CI 行为改动只能改 `ci/generate-workflows.py` 然后
  重新生成全部 gen workflow，并审 diff。
- ssh e2e（wezterm-ssh/tests）在 CI 由工作流装 openssh-server 提供；本地跑
  需要 `/usr/sbin/sshd` 存在（`make test-integration` 会真实起 sshd 子进程）。
- fmt 用 nightly（`.rustfmt.toml`）；lint 门是 `cargo +nightly fmt --check`。
- 发布链（tag.sh/create-release.sh/deploy.sh/appimage.sh/windows-installer.iss
  及分发渠道脚本）只在上游仓库运行；本 fork 改这些文件仅为了上游同步后可
  复用，不在本地执行发布动作（hooks 会拦）。

## 禁止项

- 不手改 Cargo.lock 里钉版的子模块内容；不为绕过检查降级 deny.toml 规则。
- 不在 fork 里启用 cargo-cooldown/发布类 workflow 的凭据。
- 新依赖三步走：workspace.dependencies 登记 → cargo check 确认 feature 面
  → `cargo deny check` 通过；并在交付说明里给出理由。

## 验证

- 定向：`make check`；CI 形态本地复现 `make ci-check`。
- 改 workflow 生成器后：`python3 ci/generate-workflows.py` 重生成并
  `git diff` 审阅（确认只改了预期矩阵）。
- 改构建脚本后：`make build` 在改动平台可用（跨平台改动无环境时记 PENDING，
  借上游 CI 结果佐证并注明）。
