pub mod connection;
pub mod event;
mod extra_constants;
mod keycodes;
mod wgl;
pub mod window;

pub use self::window::*;
pub use connection::*;
pub use event::*;

// fork: hybrid-graphics drivers (NVIDIA Optimus, AMD PowerXpress /
// switchable graphics) look these DWORDs up in the *executable's* export
// table at process start; a non-zero value asks the driver to run the
// process on the discrete GPU instead of the integrated one. Machines with a
// single GPU ignore them. The statics live in this rlib, so
// wezterm-gui/build.rs adds the export directives for the final binary, and
// Connection::create_new reads them via hybrid_gpu_hints() so the object
// that defines them is always part of the link.
#[allow(non_upper_case_globals)]
#[no_mangle]
#[used]
pub static NvOptimusEnablement: u32 = 1;

#[allow(non_upper_case_globals)]
#[no_mangle]
#[used]
pub static AmdPowerXpressRequestHighPerformance: u32 = 1;

/// fork: returns `(NvOptimusEnablement, AmdPowerXpressRequestHighPerformance)`.
/// The volatile reads stop the optimizer from constant-folding the statics,
/// which keeps a real reference to their symbols in the final link.
pub fn hybrid_gpu_hints() -> (u32, u32) {
    unsafe {
        (
            std::ptr::read_volatile(&NvOptimusEnablement),
            std::ptr::read_volatile(&AmdPowerXpressRequestHighPerformance),
        )
    }
}

/// Convert a rust string to a windows wide string
pub fn wide_string(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Returns true if we are running in an RDP session.
/// See <https://docs.microsoft.com/en-us/windows/win32/termserv/detecting-the-terminal-services-environment>
pub fn is_running_in_rdp_session() -> bool {
    use winapi::shared::minwindef::DWORD;
    use winapi::um::processthreadsapi::{GetCurrentProcessId, ProcessIdToSessionId};
    use winapi::um::winuser::{GetSystemMetrics, SM_REMOTESESSION};
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    if unsafe { GetSystemMetrics(SM_REMOTESESSION) } != 0 {
        return true;
    }

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let terminal_server =
        match hklm.open_subkey("SYSTEM\\CurrentControlSet\\Control\\Terminal Server\\") {
            Ok(k) => k,
            Err(_) => return false,
        };

    let glass_session_id: DWORD = match terminal_server.get_value("GlassSessionId") {
        Ok(sess) => sess,
        Err(_) => return false,
    };

    unsafe {
        let mut current_session = 0;
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut current_session) != 0 {
            // If we're not the glass session then we're a remote session
            current_session != glass_session_id
        } else {
            false
        }
    }
}
