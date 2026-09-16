# process-io：PTY、进程与文件描述符

## 范围

`pty/`（portable-pty）、`procinfo/`、`filedescriptor/`、`umask/`。

## 符号真源

- PTY 抽象：`pty/src/lib.rs`——`PtySize`、`MasterPty`（resize/take_writer/
  try_clone_reader）、`Child`/`ChildKiller`、`SlavePty`、`PtySystem`
  （`local_pty_system()`）；unix openpty+fork、Windows ConPTY/WinPty 在
  `src/win/`；**串口实现同一 MasterPty 抽象**（`src/serial.rs`，LocalDomain
  的 serial 域基础）。
- 进程描述：`pty/src/cmdbuilder.rs::CommandBuilder`（argv/env/cwd/登录
  shell 组装）——mux `Domain::spawn`/`fixup_command` 与 GUI start 的契约。
- 进程信息：`procinfo/src/lib.rs::LocalProcessInfo`（进程树/cwd/exe，pane
  foreground process 的来源）。
- FD 抽象：`filedescriptor/src/lib.rs::FileDescriptor/OwnedHandle/Pipe` +
  可移植 poll（macOS 实际是 select 包装）。
- umask：`umask/` 的 `UmaskSaver`（spawn/daemonize 前保存恢复）。

## 不变量

- **抽象优先**：上游消费方（mux、wezterm-ssh 经 re-export）只依赖 trait；
  新平台能力先扩 trait 再实现，不允许 cfg 泄漏到调用方。
- **ConPTY 兼容**：Windows 实现依赖系统 conpty 可用性探测；改动需在
  WinPty 回退路径上同样验证（无环境记 PENDING）。
- **CommandBuilder 语义**：env 继承与登录 shell 修饰的规则影响所有 spawn
  路径（含 ssh 域）；改动要有针对性测试并列出受影响域。

## 禁止项

- 不在 PTY 层做策略（exit_behavior、进程组回收归属 mux）。
- 不引入阻塞式同步等待进 GUI 主循环（子进程等待都在 mux 线程）。

## 验证

- 定向：`cargo nextest run -p portable-pty -p procinfo -p filedescriptor`。
- 串口/Windows 无真实设备环境：相关行为记 PENDING，不伪造通过。
