# testing：测试数据与测试纪律

## 范围

`test-data/`（手动 fixtures）与全仓测试组织约定。各域的定向测试命令在
各自规则文档；本文件管数据与通用纪律。

## 符号真源

- `test-data/` 下的 .sh/.txt/.py 是**手动测试 fixtures**（对照真实终端
  行为的样本），不被自动测试引用；修改要说明来源场景。
- 自动测试分布：以 crate 内联 `#[cfg(test)]` 为主（term 的
  `term/src/test/`、escape-parser、surface、termwiz、config 等）；集成
  测试目录仅 `wezterm-ssh/tests`（sshd e2e）、`bidi/tests/conformance.rs`
  （Unicode 官方数据）、`wezterm-dynamic/tests`。
- 断言库：k9（`k9::snapshot!` 结构快照 + `assert_equal` 别名）；快照更新
  用 `K9_UPDATE=1 cargo test`（只更新受影响断言，更新后必须人工审 diff）。
- 运行器：cargo-nextest（`make test` 上游语义：全量 + escape-parser
  no_std 第二轮）。

## 不变量

- **新行为必须有失败路径覆盖**：修 bug 先写会失败的用例（或 snapshot 前
  后对照），再修；不许只加"恰好通过"的用例。
- **快照是契约**：更新 k9 快照属于行为变更，交付说明必须列出并给出理由；
  禁止批量刷新快照混进无关改动。
- **conformance/oracle 数据不得裁剪**：bidi 的 Unicode 测试数据、escape
  的 round-trip 用例是跨实现 oracle，失败只能修实现。
- **e2e 隔离**：sshd fixture 全部用 TempDir 临时密钥与高位端口，端口占用
  即失败重试，不杀不明进程；不依赖宿主 sshd 配置。
- **必要工具缺失不得静默 skip**：缺 sshd/显示/工具链时该层记 PENDING 并
  给补验命令，不许整族 skip 后报告绿。

## 禁止项

- 不把网络依赖引进单测（sync-color-schemes 的联网同步是显式生成器，不是
  测试）。
- 不用 `--no-fail-fast` 掩盖局部失败（它是 CI 收集完整信号用的，本地定位
  时逐个看）。

## 验证

- 定向：`cargo nextest run -p <受影响 crate>`；数据文件改动跑引用它的
  手动场景并在交付说明记录。
- 全量：`make test`；CI 形态：`make test-heavy`。
