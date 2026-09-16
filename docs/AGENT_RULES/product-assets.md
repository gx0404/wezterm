# product-assets：静态资源

## 范围

`assets/` 除 shell-integration（归 terminal-model）外的全部：`fonts/`、
`icon/`、`flatpak/`、`macos/`、`open-wezterm-here/`、`shell-completion/`、
`windows/`、`wezterm.desktop`、`wezterm.appdata.xml`、`wezterm-nautilus.py`。

## 符号真源

- shell 补全：`assets/shell-completion/{bash,zsh,fish,elvish,powershell,
  fig…}` 由 `ci/update-derived-files.sh` 从 `wezterm shell-completion`
  生成——不手改（hooks 亦拦直接写 docs/changelog 之外的重点生成物，补全
  走 `make generated-check` 把关）。
- 打包引用：`ci/appimage.sh`、`ci/windows-installer.iss`、flatpak/与
  macos/ 资源被对应发布脚本引用（上游执行）；`.desktop`/appdata 的图标
  路径与 `assets/icon/` 联动。
- vendored 字体：`assets/fonts/SymbolsNerdFontMono-Regular.ttf` 等被
  wezterm-font 的 vendor feature 与 docs 构建（favicon/字体复制）引用。

## 不变量

- 二进制资源的替换必须说明来源与许可（字体/图标各有 LICENSE 归属）。
- 引用路径改动要与打包脚本/安装器同步（一处改漏=发布产物缺文件）。
- `.desktop` 的 `Exec`/`TryExec` 与实际二进制名强耦合。

## 禁止项

- 不手改生成型资产（shell-completion）；不提交无关格式化 diff。

## 验证

- `make generated-check`（补全一致性）。
- 引用改动：`grep` 全仓引用点 + 对应平台打包脚本人工核对（无环境记
  PENDING）。
