use crate::{Child, ChildKiller, ExitStatus};
use anyhow::Context as _;
use std::io::{Error as IoError, Result as IoResult};
use std::os::windows::io::AsRawHandle;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use windows_sys::Win32::Foundation::STILL_ACTIVE;
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, TerminateProcess, WaitForSingleObject, INFINITE,
};

pub mod conpty;
mod procthreadattr;
mod pseudocon;

use filedescriptor::OwnedHandle;

#[derive(Debug)]
pub struct WinChild {
    proc: Mutex<OwnedHandle>,
    /// fork: set once `Future::poll` has started the exit waiter thread.
    waiter: Option<Arc<ExitWaiter>>,
}

/// fork: state shared between a polled `WinChild` and its exit waiter
/// thread.
#[derive(Debug)]
struct ExitWaiter {
    /// The waker of the most recent poll; taken by the thread on exit.
    waker: Mutex<Option<Waker>>,
    /// Set by the thread once the process handle has been signaled.
    exited: AtomicBool,
}

impl WinChild {
    pub(crate) fn new(proc: OwnedHandle) -> Self {
        Self {
            proc: Mutex::new(proc),
            waiter: None,
        }
    }

    fn is_complete(&mut self) -> IoResult<Option<ExitStatus>> {
        let mut status: u32 = 0;
        let proc = self.proc.lock().unwrap().try_clone().unwrap();
        let res = unsafe { GetExitCodeProcess(proc.as_raw_handle(), &mut status) };
        if res != 0 {
            if status == STILL_ACTIVE as u32 {
                Ok(None)
            } else {
                Ok(Some(ExitStatus::with_exit_code(status)))
            }
        } else {
            Ok(None)
        }
    }

    fn do_kill(&mut self) -> IoResult<()> {
        let proc = self.proc.lock().unwrap().try_clone().unwrap();
        let res = unsafe { TerminateProcess(proc.as_raw_handle(), 1) };
        let err = IoError::last_os_error();
        if res == 0 {
            Err(err)
        } else {
            Ok(())
        }
    }
}

impl ChildKiller for WinChild {
    fn kill(&mut self) -> IoResult<()> {
        self.do_kill().ok();
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        let proc = self.proc.lock().unwrap().try_clone().unwrap();
        Box::new(WinChildKiller { proc })
    }
}

#[derive(Debug)]
pub struct WinChildKiller {
    proc: OwnedHandle,
}

impl ChildKiller for WinChildKiller {
    fn kill(&mut self) -> IoResult<()> {
        let res = unsafe { TerminateProcess(self.proc.as_raw_handle(), 1) };
        let err = IoError::last_os_error();
        if res == 0 {
            Err(err)
        } else {
            Ok(())
        }
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        let proc = self.proc.try_clone().unwrap();
        Box::new(WinChildKiller { proc })
    }
}

impl Child for WinChild {
    fn try_wait(&mut self) -> IoResult<Option<ExitStatus>> {
        self.is_complete()
    }

    fn wait(&mut self) -> IoResult<ExitStatus> {
        if let Ok(Some(status)) = self.try_wait() {
            return Ok(status);
        }
        let proc = self.proc.lock().unwrap().try_clone().unwrap();
        unsafe {
            WaitForSingleObject(proc.as_raw_handle(), INFINITE);
        }
        let mut status: u32 = 0;
        let res = unsafe { GetExitCodeProcess(proc.as_raw_handle(), &mut status) };
        if res != 0 {
            Ok(ExitStatus::with_exit_code(status))
        } else {
            Err(IoError::last_os_error())
        }
    }

    fn process_id(&self) -> Option<u32> {
        let res = unsafe { GetProcessId(self.proc.lock().unwrap().as_raw_handle()) };
        if res == 0 {
            None
        } else {
            Some(res)
        }
    }

    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        let proc = self.proc.lock().unwrap();
        Some(proc.as_raw_handle())
    }
}

impl std::future::Future for WinChild {
    type Output = anyhow::Result<ExitStatus>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<anyhow::Result<ExitStatus>> {
        // fork: publish the current waker before checking for completion,
        // so that an exit racing with this poll still wakes this task.
        if let Some(waiter) = &self.waiter {
            *waiter.waker.lock().unwrap() = Some(cx.waker().clone());
        }
        match self.is_complete() {
            Ok(Some(status)) => Poll::Ready(Ok(status)),
            Err(err) => Poll::Ready(Err(err).context("Failed to retrieve process exit status")),
            Ok(None) => {
                match &self.waiter {
                    // fork: the wait is over, yet the exit code reads as
                    // STILL_ACTIVE: the child really exited with 259.
                    Some(waiter) if waiter.exited.load(Ordering::Acquire) => {
                        return Poll::Ready(Ok(ExitStatus::with_exit_code(STILL_ACTIVE as u32)));
                    }
                    Some(_) => {}
                    None => {
                        // fork: the waiter thread owns its duplicate of the
                        // process handle until the wait is over.  It used
                        // to get only the raw value of a duplicate that was
                        // closed as soon as poll returned, so it waited on
                        // a closed (or recycled) handle.  It is started
                        // once; later polls only refresh the waker.
                        let proc = self.proc.lock().unwrap().try_clone()?;
                        let waiter = Arc::new(ExitWaiter {
                            waker: Mutex::new(Some(cx.waker().clone())),
                            exited: AtomicBool::new(false),
                        });
                        let thread_waiter = Arc::clone(&waiter);
                        std::thread::spawn(move || {
                            unsafe {
                                WaitForSingleObject(proc.as_raw_handle() as _, INFINITE);
                            }
                            drop(proc);
                            thread_waiter.exited.store(true, Ordering::Release);
                            if let Some(waker) = thread_waiter.waker.lock().unwrap().take() {
                                waker.wake();
                            }
                        });
                        self.waiter = Some(waiter);
                    }
                }
                Poll::Pending
            }
        }
    }
}

// fork(PTY-10): WinChild 作为 Future 时，等待线程必须持有自己的进程句柄直到
// 等待结束，且只起一次；用阻塞在 stdin 上的 cmd.exe 控制退出时机。
#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Wake, Waker};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct CountingWaker {
        wakes: AtomicUsize,
    }

    impl Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.wakes.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn future_waits_on_a_live_handle_and_spawns_one_waiter() {
        let mut child = Command::new("cmd.exe")
            .raw_arg("/D /Q /K")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cmd.exe");
        let mut win = WinChild::new(OwnedHandle::dup(&child).expect("dup process handle"));

        let counter = Arc::new(CountingWaker::default());
        let waker = Waker::from(Arc::clone(&counter));
        let mut cx = Context::from_waker(&waker);

        assert!(Pin::new(&mut win).poll(&mut cx).is_pending());
        let first = Arc::clone(win.waiter.as_ref().expect("waiter started"));
        assert!(Pin::new(&mut win).poll(&mut cx).is_pending());
        let second = win.waiter.as_ref().expect("waiter kept");
        assert!(Arc::ptr_eq(&first, second), "re-polling must not respawn");

        // 进程仍在运行：等待线程不得提前唤醒。旧实现在 poll 返回时就关闭了
        // 交给线程的句柄，线程等的是已关闭或被复用的句柄：要么立即失败而
        // 提前唤醒，要么等错对象而永不唤醒（下方 "no wake after exit"）。
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(counter.wakes.load(Ordering::SeqCst), 0, "spurious wake");

        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(b"exit 3\r\n")
            .expect("write exit");
        let deadline = Instant::now() + Duration::from_secs(10);
        while counter.wakes.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "no wake after exit");
            std::thread::sleep(Duration::from_millis(10));
        }

        match Pin::new(&mut win).poll(&mut cx) {
            Poll::Ready(Ok(status)) => assert_eq!(status.exit_code(), 3),
            other => panic!("expected exit status, got {:?}", other),
        }
        child.wait().ok();
    }
}
