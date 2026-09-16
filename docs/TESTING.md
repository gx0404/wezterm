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
| UI 冒烟 | `make ui-smoke` | target/debug/wezterm-gui、Xvfb、xwd、ffmpeg | 窗口可起、标记文本渲染 / 不能证交互细节 |
| 实机平台 | NixOS VM（上游流程） | nix | GNOME/KDE 下真实桌面行为 / 人工流程 |
| 框架自身 | `make framework-test` | 无 | resolver/hooks/KB 契约 / 不测产品代码 |

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
3. **读回图片**：执行者必须实际查看 before.png，核对标记文本
   `WEZTERM-UI-SMOKE-OK-…` 与基本布局（有字、有色、无花屏）。
4. 判定：核对通过→ result.json 改 `status: PASS, images_reviewed: true`；
   失败→ `status: FAIL` 并保留截图，修复后**另开批次**复测。
5. 手动配色截图（上游流程）：`ci/make-color-screen-shots.sh`（xwininfo
   选窗 + ImageMagick），产物进 `docs/colorschemes/`，属上游文档链。

## CI（上游）

gen_* 工作流（由 `ci/generate-workflows.py` 生成）在 PR/push 时构建矩阵
（centos/debian/fedora/macos/ubuntu/windows）并跑 `cargo nextest run
--all --no-fail-fast`；fmt.yml（nightly rustfmt --check）、termwiz.yml、
wezterm_ssh.yml（双后端矩阵）独立触发。fork 不改这些工作流；本地等效门
是 `make ci-check`。
