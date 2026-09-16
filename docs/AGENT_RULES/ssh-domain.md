# ssh-domain：SSH 会话

## 范围

`wezterm-ssh/`：SSH 客户端协议实现（认证、配置解析、sftp、agent 转发）、
双后端封装、e2e 测试设施。mux 侧的 RemoteSshDomain/ClientDomain 归
mux-domain 域。

## 符号真源

- 会话入口：`wezterm-ssh/src/session.rs::Session`（Clone 的轻句柄，Drop 发
  `SessionDropped`）+ `connect(ConfigMap) -> (Session, Receiver<SessionEvent>)`
  ——内部线程跑 `sessioninner.rs::SessionInner::run()`（poll 循环管理
  channels/files/dirs 与 keepalive）。
- 事件驱动认证：`SessionEvent::{Banner,HostVerify,Authenticate,
  HostVerificationFailed,Error,Authenticated}`；交互式逻辑在 `auth.rs`。
- 配置解析：`config.rs` 是 openssh 兼容解析器——支持 ProxyCommand（token
  展开 %h %n %p %r），**不支持 ProxyJump**（%j 展开为空，见代码注释）；
  选项兼容性以 tests 里的解析用例为准。
- 双后端：Cargo features `libssh-rs` 与 `ssh2`（默认都开，可单独关）；
  运行时由 config `wezterm_ssh_backend` 选择；统一封装层
  `sessionwrap/channelwrap/filewrap/dirwrap/sftpwrap.rs`。
- e2e 设施：`tests/sshd.rs::Sshd::spawn`（TempDir 生成密钥/config，起
  `/usr/sbin/sshd -D -p <port>`，端口探测重试）+ rstest fixture
  `sshd()/session()`；e2e 在 `tests/e2e/{sftp,agent_forward}.rs`。

## 不变量

- **feature 矩阵**：改动必须分别在 libssh-rs、ssh2 单开下编译
  （上游 CI wezterm_ssh.yml 就是这个矩阵）；封装层不得把某后端私有类型
  泄漏出 crate。
- **非阻塞线程模型**：SessionInner 用 filedescriptor::poll 驱动，不引入
  tokio；唤醒靠 socketpair 管道。改 I/O 路径保持该模型。
- **凭据纪律**：密钥/口令只经 ConfigMap 与认证回调流转；不写日志、不落盘、
  不进测试快照。测试密钥全部由 Sshd fixture 临时生成。
- **e2e 前置**：本机需 `/usr/sbin/sshd` 可执行（CI 由工作流安装）；
  `make test-integration` = `cargo nextest run -p wezterm-ssh`。

## 禁止项

- 不在 SSH 层做 UI/连接向导（connui 在 mux-domain）。
- 不悄悄放宽主机密钥校验（HostVerify 必须经事件决策）。

## 验证

- 定向：`cargo nextest run -p wezterm-ssh`（含 e2e，自动起 sshd）。
- feature 矩阵：`cargo check -p wezterm-ssh --no-default-features
  --features libssh-rs` 与 `--features ssh2` 各一轮。
- 改 openssh 配置解析：补 config.rs 解析测试用例（对照 openssh 行为）。
