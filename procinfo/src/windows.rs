#![cfg(windows)]

use super::*;
use ntapi::ntpebteb::PEB;
use ntapi::ntpsapi::{
    NtQueryInformationProcess, ProcessBasicInformation, ProcessWow64Information,
    PROCESS_BASIC_INFORMATION,
};
use ntapi::ntrtl::RTL_USER_PROCESS_PARAMETERS;
use ntapi::ntwow64::RTL_USER_PROCESS_PARAMETERS32;
use std::ffi::OsString;
use std::mem::MaybeUninit;
use std::os::windows::ffi::OsStringExt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use winapi::shared::minwindef::{DWORD, FILETIME, LPVOID};
use winapi::shared::ntdef::{FALSE, NT_SUCCESS};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::memoryapi::ReadProcessMemory;
use winapi::um::processthreadsapi::{GetCurrentProcessId, GetProcessTimes, OpenProcess};
use winapi::um::shellapi::CommandLineToArgvW;
use winapi::um::tlhelp32::*;
use winapi::um::winbase::{LocalFree, QueryFullProcessImageNameW};
use winapi::um::winnt::{HANDLE, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

/// Manages a Toolhelp32 snapshot handle
struct Snapshot(HANDLE);

impl Snapshot {
    pub fn new() -> Option<Self> {
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        // fork: CreateToolhelp32Snapshot reports failure as
        // INVALID_HANDLE_VALUE, not NULL.
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Self(handle))
        }
    }

    pub fn iter(&self) -> ProcIter<'_> {
        ProcIter {
            snapshot: &self,
            first: true,
        }
    }

    pub fn entries() -> Vec<PROCESSENTRY32W> {
        match Self::new() {
            Some(snapshot) => snapshot.iter().collect(),
            None => vec![],
        }
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// fork: how long a process snapshot may be shared between callers.
/// Tab bar/title refreshes query every pane at (almost) the same time;
/// a single Toolhelp32 snapshot of every process on the system serves
/// them all instead of taking one snapshot per pane.
const SNAPSHOT_TTL: Duration = Duration::from_millis(500);

struct CachedSnapshot {
    entries: Arc<Vec<PROCESSENTRY32W>>,
    taken: Instant,
}

static SNAPSHOT_CACHE: Mutex<Option<CachedSnapshot>> = Mutex::new(None);

/// fork: return the process-wide shared snapshot, taking a new one when
/// it is older than `SNAPSHOT_TTL` or when `force` is set.  The lock is
/// held while snapshotting so that concurrent callers wait for that one
/// snapshot rather than each taking their own.  A failed (empty)
/// snapshot is not cached.
fn snapshot_entries(force: bool) -> Arc<Vec<PROCESSENTRY32W>> {
    let mut cache = SNAPSHOT_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !force {
        if let Some(cached) = cache.as_ref() {
            if cached.taken.elapsed() < SNAPSHOT_TTL {
                return Arc::clone(&cached.entries);
            }
        }
    }
    let entries = Arc::new(Snapshot::entries());
    if entries.is_empty() {
        cache.take();
    } else {
        cache.replace(CachedSnapshot {
            entries: Arc::clone(&entries),
            taken: Instant::now(),
        });
    }
    entries
}

/// fork: assemble the process tree rooted at `pid` from a snapshot.
/// A `ppid -> children` index makes this O(n) in the snapshot size,
/// rather than rescanning the whole snapshot for every tree node.
/// `visited` guards against cycles produced by pid reuse (a parent
/// that exited and whose pid was recycled by one of its descendants).
/// `read` fills in the details of a single process; its `children`
/// are replaced by the ones linked here.
fn build_tree(
    pid: u32,
    procs: &[PROCESSENTRY32W],
    read: &mut dyn FnMut(&PROCESSENTRY32W) -> LocalProcessInfo,
) -> Option<LocalProcessInfo> {
    let root = procs.iter().find(|info| info.th32ProcessID == pid)?;

    let mut kids: HashMap<u32, Vec<&PROCESSENTRY32W>> = HashMap::new();
    for info in procs {
        kids.entry(info.th32ParentProcessID).or_default().push(info);
    }

    fn build(
        info: &PROCESSENTRY32W,
        kids: &HashMap<u32, Vec<&PROCESSENTRY32W>>,
        visited: &mut HashSet<u32>,
        read: &mut dyn FnMut(&PROCESSENTRY32W) -> LocalProcessInfo,
    ) -> LocalProcessInfo {
        let mut children = HashMap::new();
        if let Some(list) = kids.get(&info.th32ProcessID) {
            for kid in list {
                if visited.insert(kid.th32ProcessID) {
                    children.insert(kid.th32ProcessID, build(kid, kids, visited, read));
                }
            }
        }
        let mut proc = read(info);
        proc.children = children;
        proc
    }

    let mut visited = HashSet::new();
    visited.insert(pid);
    Some(build(root, &kids, &mut visited, read))
}

struct ProcIter<'a> {
    snapshot: &'a Snapshot,
    first: bool,
}

impl<'a> Iterator for ProcIter<'a> {
    type Item = PROCESSENTRY32W;

    fn next(&mut self) -> Option<Self::Item> {
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as _;
        let res = if self.first {
            self.first = false;
            unsafe { Process32FirstW(self.snapshot.0, &mut entry) }
        } else {
            unsafe { Process32NextW(self.snapshot.0, &mut entry) }
        };
        if res == 0 {
            None
        } else {
            Some(entry)
        }
    }
}

fn wstr_to_path(slice: &[u16]) -> PathBuf {
    match slice.iter().position(|&c| c == 0) {
        Some(nul) => OsString::from_wide(&slice[..nul]),
        None => OsString::from_wide(slice),
    }
    .into()
}

fn wstr_to_string(slice: &[u16]) -> String {
    wstr_to_path(slice).to_string_lossy().into_owned()
}

/// fork: UNICODE_STRING lengths are u16 byte counts, so a command line
/// is at most 65534 bytes.  Upstream capped every read at MAX_PATH * 4
/// bytes (520 UTF-16 units) and, for any longer command line, gave up on
/// the whole parameter block, losing the cwd that new tabs and splits
/// inherit along with it.
const MAX_CMDLINE_BYTES: usize = 65534;
/// fork: extended-length paths are limited to 32767 UTF-16 units.
const MAX_CWD_BYTES: usize = 32767 * 2;
/// fork: buffer size, in UTF-16 units, for QueryFullProcessImageNameW,
/// which fails outright rather than truncating when it is too small.
const MAX_IMAGE_NAME_CHARS: usize = 32768;

/// fork: argv and cwd are read independently so that failing to read
/// one of them does not discard the other.
struct ProcParams {
    argv: Option<Vec<String>>,
    cwd: Option<PathBuf>,
    console: HANDLE,
}

/// A handle to an opened process
struct ProcHandle {
    pid: u32,
    proc: HANDLE,
}

impl ProcHandle {
    pub fn new(pid: u32) -> Option<Self> {
        if pid == unsafe { GetCurrentProcessId() } {
            // Avoid the potential for deadlock if we're examining ourselves
            log::trace!("ProcHandle::new({}): skip because it is my own pid", pid);
            return None;
        }
        let options = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ;
        log::trace!("ProcHandle::new({}): OpenProcess", pid);
        let handle = unsafe { OpenProcess(options, FALSE as _, pid) };
        log::trace!("ProcHandle::new({}): OpenProcess -> {:?}", pid, handle);
        if handle.is_null() {
            return None;
        }
        Some(Self { pid, proc: handle })
    }

    /// Returns the executable image for the process
    pub fn executable(&self) -> Option<PathBuf> {
        let mut buf = vec![0u16; MAX_IMAGE_NAME_CHARS];
        let mut len = buf.len() as DWORD;
        let res = unsafe { QueryFullProcessImageNameW(self.proc, 0, buf.as_mut_ptr(), &mut len) };
        if res == 0 {
            None
        } else {
            let len = (len as usize).min(buf.len());
            Some(wstr_to_path(&buf[..len]))
        }
    }

    /// Wrapper around NtQueryInformationProcess that fetches `what` as `T`
    fn query_proc<T>(&self, what: u32) -> Option<T> {
        let mut data = MaybeUninit::<T>::uninit();
        let res = unsafe {
            NtQueryInformationProcess(
                self.proc,
                what,
                data.as_mut_ptr() as _,
                std::mem::size_of::<T>() as _,
                std::ptr::null_mut(),
            )
        };
        if !NT_SUCCESS(res) {
            return None;
        }
        let data = unsafe { data.assume_init() };
        Some(data)
    }

    /// Read a `T` from the target process at the specified address
    fn read_struct<T>(&self, addr: LPVOID) -> Option<T> {
        let mut data = MaybeUninit::<T>::uninit();
        let res = unsafe {
            ReadProcessMemory(
                self.proc,
                addr as _,
                data.as_mut_ptr() as _,
                std::mem::size_of::<T>() as _,
                std::ptr::null_mut(),
            )
        };
        if res == 0 {
            return None;
        }
        let data = unsafe { data.assume_init() };
        Some(data)
    }

    /// If the process is a 32-bit process running on Win64, return the address
    /// of its process parameters.
    /// Otherwise, return None to indicate a native win64 process.
    fn get_peb32_addr(&self) -> Option<LPVOID> {
        let peb32_addr: LPVOID = self.query_proc(ProcessWow64Information)?;
        if peb32_addr.is_null() {
            None
        } else {
            Some(peb32_addr)
        }
    }

    /// Returns the cwd and args for the process
    pub fn get_params(&self) -> Option<ProcParams> {
        match self.get_peb32_addr() {
            Some(peb32) => self.get_params_32(peb32),
            None => self.get_params_64(),
        }
    }

    fn get_basic_info(&self) -> Option<PROCESS_BASIC_INFORMATION> {
        self.query_proc(ProcessBasicInformation)
    }

    fn get_peb(&self, info: &PROCESS_BASIC_INFORMATION) -> Option<PEB> {
        self.read_struct(info.PebBaseAddress as _)
    }

    fn get_proc_params(&self, peb: &PEB) -> Option<RTL_USER_PROCESS_PARAMETERS> {
        self.read_struct(peb.ProcessParameters as _)
    }

    /// Returns the cwd and args for a 64 bit process
    fn get_params_64(&self) -> Option<ProcParams> {
        let info = self.get_basic_info()?;
        let peb = self.get_peb(&info)?;
        let params = self.get_proc_params(&peb)?;

        let cmdline = self.read_process_wchar(
            params.CommandLine.Buffer as _,
            params.CommandLine.Length as _,
            MAX_CMDLINE_BYTES,
        );
        let cwd = self.read_process_wchar(
            params.CurrentDirectory.DosPath.Buffer as _,
            params.CurrentDirectory.DosPath.Length as _,
            MAX_CWD_BYTES,
        );

        Some(ProcParams {
            argv: cmdline.map(|cmdline| cmd_line_to_argv(&cmdline)),
            cwd: cwd.map(|cwd| wstr_to_path(&cwd)),
            console: params.ConsoleHandle,
        })
    }

    fn get_proc_params_32(&self, peb32: LPVOID) -> Option<RTL_USER_PROCESS_PARAMETERS32> {
        self.read_struct(peb32)
    }

    /// Returns the cwd and args for a 32 bit process
    fn get_params_32(&self, peb32: LPVOID) -> Option<ProcParams> {
        let params = self.get_proc_params_32(peb32)?;

        let cmdline = self.read_process_wchar(
            params.CommandLine.Buffer as _,
            params.CommandLine.Length as _,
            MAX_CMDLINE_BYTES,
        );
        let cwd = self.read_process_wchar(
            params.CurrentDirectory.DosPath.Buffer as _,
            params.CurrentDirectory.DosPath.Length as _,
            MAX_CWD_BYTES,
        );

        Some(ProcParams {
            argv: cmdline.map(|cmdline| cmd_line_to_argv(&cmdline)),
            cwd: cwd.map(|cwd| wstr_to_path(&cwd)),
            console: params.ConsoleHandle as _,
        })
    }

    /// Copies a sized WSTR from the address in the process
    fn read_process_wchar(
        &self,
        ptr: LPVOID,
        byte_size: usize,
        max_bytes: usize,
    ) -> Option<Vec<u16>> {
        if byte_size > max_bytes {
            // Defend against implausibly large paths, just in
            // case we're reading the wrong offset into a kernel struct
            return None;
        }

        let mut buf = vec![0u16; byte_size / 2];
        let mut bytes_read = 0;

        // fork: read whole UTF-16 units only; an odd (corrupt) byte_size
        // must not overrun `buf`.
        let res = unsafe {
            ReadProcessMemory(
                self.proc,
                ptr as _,
                buf.as_mut_ptr() as _,
                buf.len() * 2,
                &mut bytes_read,
            )
        };
        if res == 0 {
            return None;
        }

        // In the unlikely event that we have a short read,
        // truncate the buffer to fit.
        let wide_chars_read = bytes_read / 2;
        buf.resize(wide_chars_read, 0);

        // Ensure that it is NUL terminated
        match buf.iter().position(|&c| c == 0) {
            Some(n) => {
                // Truncate to include existing NUL but no later chars
                buf.resize(n + 1, 0);
            }
            None => {
                // Add a NUL
                buf.push(0);
            }
        }

        Some(buf)
    }

    /// Retrieves the start time of the process
    fn start_time(&self) -> Option<u64> {
        const fn empty() -> FILETIME {
            FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            }
        }

        let mut start = empty();
        let mut exit = empty();
        let mut kernel = empty();
        let mut user = empty();

        let res =
            unsafe { GetProcessTimes(self.proc, &mut start, &mut exit, &mut kernel, &mut user) };
        if res == 0 {
            return None;
        }

        Some((start.dwHighDateTime as u64) << 32 | start.dwLowDateTime as u64)
    }
}

/// Parse a command line string into an argv array
fn cmd_line_to_argv(buf: &[u16]) -> Vec<String> {
    let mut argc = 0;
    let argvp = unsafe { CommandLineToArgvW(buf.as_ptr(), &mut argc) };
    if argvp.is_null() {
        return vec![];
    }

    let argv = unsafe { std::slice::from_raw_parts(argvp, argc as usize) };
    let mut args = vec![];
    for &arg in argv {
        let len = unsafe { libc::wcslen(arg) };
        let arg = unsafe { std::slice::from_raw_parts(arg, len) };
        args.push(wstr_to_string(arg));
    }
    unsafe { LocalFree(argvp as _) };
    args
}

impl Drop for ProcHandle {
    fn drop(&mut self) {
        log::trace!("ProcHandle::drop(pid={} proc={:?})", self.pid, self.proc);
        unsafe { CloseHandle(self.proc) };
    }
}

impl LocalProcessInfo {
    pub fn current_working_dir(pid: u32) -> Option<PathBuf> {
        log::trace!("current_working_dir({})", pid);
        let proc = ProcHandle::new(pid)?;
        let params = proc.get_params()?;
        params.cwd
    }

    pub fn executable_path(pid: u32) -> Option<PathBuf> {
        log::trace!("executable_path({})", pid);
        let proc = ProcHandle::new(pid)?;
        proc.executable()
    }

    pub fn with_root_pid(pid: u32) -> Option<Self> {
        log::trace!("LocalProcessInfo::with_root_pid({}), getting snapshot", pid);
        // fork: share one snapshot between callers (see SNAPSHOT_TTL);
        // a root that is missing from the shared snapshot may simply have
        // been spawned after it was taken, so retry once with a fresh one.
        let mut procs = snapshot_entries(false);
        if !procs.iter().any(|info| info.th32ProcessID == pid) {
            procs = snapshot_entries(true);
        }
        log::trace!("Got snapshot");

        fn read_proc(info: &PROCESSENTRY32W) -> LocalProcessInfo {
            let mut executable = None;
            let mut start_time = 0;
            let mut cwd = PathBuf::new();
            let mut argv = vec![];
            let mut console = 0;

            if let Some(proc) = ProcHandle::new(info.th32ProcessID) {
                if let Some(exe) = proc.executable() {
                    executable.replace(exe);
                }
                if let Some(params) = proc.get_params() {
                    if let Some(dir) = params.cwd {
                        cwd = dir;
                    }
                    if let Some(args) = params.argv {
                        argv = args;
                    }
                    console = params.console as _;
                }
                if let Some(start) = proc.start_time() {
                    start_time = start;
                }
            }

            let executable = executable.unwrap_or_else(|| wstr_to_path(&info.szExeFile));
            let name = match executable.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => String::new(),
            };

            LocalProcessInfo {
                pid: info.th32ProcessID,
                ppid: info.th32ParentProcessID,
                name,
                executable,
                cwd,
                argv,
                start_time,
                status: LocalProcessStatus::Run,
                children: HashMap::new(),
                console,
            }
        }

        build_tree(pid, &procs, &mut read_proc)
    }
}

// fork(A6-1): 进程级共享快照缓存与 O(n) 建树的单测。建树用合成快照项与
// 假 reader 驱动；缓存与 cache-miss 回退用真实快照与子进程验证。
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    fn entry(pid: u32, ppid: u32) -> PROCESSENTRY32W {
        let mut e: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as _;
        e.th32ProcessID = pid;
        e.th32ParentProcessID = ppid;
        e
    }

    fn fake_info(info: &PROCESSENTRY32W) -> LocalProcessInfo {
        LocalProcessInfo {
            pid: info.th32ProcessID,
            ppid: info.th32ParentProcessID,
            name: String::new(),
            executable: PathBuf::new(),
            argv: vec![],
            cwd: PathBuf::new(),
            status: LocalProcessStatus::Run,
            start_time: 0,
            console: 0,
            children: HashMap::new(),
        }
    }

    fn child_pids(info: &LocalProcessInfo) -> Vec<u32> {
        let mut pids: Vec<u32> = info.children.keys().copied().collect();
        pids.sort_unstable();
        pids
    }

    #[test]
    fn build_tree_links_children_and_reads_each_entry_once() {
        // 1 → {2, 3}；2 → 4；4 → 6；6 又被记成 1 的父进程（pid 复用造成的环）；
        // 5 挂在无关父进程 99 下，不应进入子树。
        let procs = vec![
            entry(1, 6),
            entry(2, 1),
            entry(3, 1),
            entry(4, 2),
            entry(5, 99),
            entry(6, 4),
        ];
        let mut reads: HashMap<u32, usize> = HashMap::new();
        let tree = build_tree(1, &procs, &mut |info| {
            *reads.entry(info.th32ProcessID).or_insert(0) += 1;
            fake_info(info)
        })
        .expect("root present");

        assert_eq!(tree.pid, 1);
        assert_eq!(child_pids(&tree), vec![2, 3]);
        assert_eq!(child_pids(&tree.children[&2]), vec![4]);
        assert_eq!(child_pids(&tree.children[&2].children[&4]), vec![6]);
        // 环被 visited 截断：6 之下不再出现 1
        assert!(tree.children[&2].children[&4].children[&6]
            .children
            .is_empty());
        assert!(tree.children[&3].children.is_empty());

        let mut read_pids: Vec<u32> = reads.keys().copied().collect();
        read_pids.sort_unstable();
        assert_eq!(read_pids, vec![1, 2, 3, 4, 6]);
        assert!(reads.values().all(|&n| n == 1), "{reads:?}");
    }

    #[test]
    fn build_tree_returns_none_for_missing_root() {
        let procs = vec![entry(1, 0), entry(2, 1)];
        assert!(build_tree(42, &procs, &mut |info| fake_info(info)).is_none());
    }

    #[test]
    fn snapshot_cache_is_shared_within_ttl() {
        let first = snapshot_entries(false);
        assert!(!first.is_empty(), "real snapshot should list processes");
        let second = snapshot_entries(false);
        assert!(Arc::ptr_eq(&first, &second), "within TTL must reuse");
        let forced = snapshot_entries(true);
        assert!(!Arc::ptr_eq(&first, &forced), "force must re-snapshot");
    }

    /// 起一个阻塞在 stdin 上的 cmd.exe，便于在其存活期间读取它的进程信息。
    fn spawn_idle_cmd(extra: &str, cwd: &std::path::Path) -> std::process::Child {
        Command::new("cmd.exe")
            .raw_arg(format!("/D /Q /K rem {extra}"))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cmd.exe")
    }

    #[test]
    fn with_root_pid_resnapshots_when_root_missing_from_cache() {
        // 先填充缓存，再起子进程：缓存里没有它的 pid，必须回退重拍快照
        let _ = snapshot_entries(false);
        let cwd = std::env::temp_dir();
        let mut child = spawn_idle_cmd("cache-miss", &cwd);
        let info = LocalProcessInfo::with_root_pid(child.id());
        child.kill().ok();
        child.wait().ok();
        let info = info.expect("fresh child must be found");
        assert_eq!(info.pid, child.id());
    }

    #[test]
    fn long_command_line_keeps_argv_and_cwd() {
        // 命令行超过 520 个 UTF-16 字符时，旧实现整体放弃读取，连 cwd 一起丢
        let cwd = std::env::temp_dir();
        let long_arg = "x".repeat(2000);
        let mut child = spawn_idle_cmd(&long_arg, &cwd);
        let pid = child.id();
        let info = LocalProcessInfo::with_root_pid(pid);
        let cwd_only = LocalProcessInfo::current_working_dir(pid);
        child.kill().ok();
        child.wait().ok();

        let info = info.expect("child must be found");
        assert!(
            info.argv.iter().any(|arg| arg == &long_arg),
            "argv lost: {:?}",
            info.argv.iter().map(String::len).collect::<Vec<_>>()
        );
        let canon = |p: &std::path::Path| std::fs::canonicalize(p).expect("canonicalize");
        assert_eq!(canon(&info.cwd), canon(&cwd));
        assert_eq!(canon(&cwd_only.expect("cwd")), canon(&cwd));
    }

    #[test]
    fn executable_path_resolves() {
        let cwd = std::env::temp_dir();
        let mut child = spawn_idle_cmd("exe", &cwd);
        let exe = LocalProcessInfo::executable_path(child.id());
        child.kill().ok();
        child.wait().ok();
        let exe = exe.expect("executable path");
        assert!(
            exe.file_name()
                .map(|n| n.eq_ignore_ascii_case("cmd.exe"))
                .unwrap_or(false),
            "{exe:?}"
        );
    }
}
