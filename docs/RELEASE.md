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

## 发布（本 fork 不执行）

上游发布链：`ci/tag-name.sh` → `ci/tag.sh` → gen_*_tag 工作流构建 →
`ci/create-release.sh`（gh release --prerelease）→ `ci/deploy.sh` 按平台
打包（macOS 签名公证/deb/AppImage/Inno Setup）→ 分发渠道（flathub/
winget/homebrew/copr）。这些只在上游仓库由维护者运行；本 fork 的 hooks
对 `gh release`/`cargo publish`/推 upstream 直接 deny。

若 fork 需要自用构建：`make build` 产出 release 二进制即可，不追加上游
分发链。
