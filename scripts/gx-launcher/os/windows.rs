use super::{invalid, Layout};
use std::fs::{Metadata, Permissions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[link(name = "kernel32")]
extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

pub fn layout(bin: &Path) -> io::Result<Layout> {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| invalid("USERPROFILE is not set"))?;
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| invalid("APPDATA is not set"))?;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("wezterm");
    Ok(Layout {
        resources: bin.join("resources"),
        home,
        config,
        plugins: appdata.join("wezterm/plugins"),
        managed: appdata.join("wezterm-gx"),
    })
}

pub fn make_writable(permissions: &mut Permissions) {
    permissions.set_readonly(false);
}

// No mode bits on Windows; the upgrade keeps read-only files instead of replacing them.
pub fn keep_mode(_original: &Metadata, _path: &Path) -> io::Result<()> {
    Ok(())
}

// Children inherit "ignore Ctrl+C" (SetConsoleCtrlHandler(NULL, TRUE) or a new process group)
// from whoever started us; restore normal Ctrl+C before the terminal starts shells.
pub fn restore_ctrl_c() {
    unsafe {
        SetConsoleCtrlHandler(None, 0);
    }
}

// Ctrl+C and Ctrl+Break reach the child through the shared console; the CLI launcher survives
// them so it can wait for the child and return its exit code.
pub fn wait(mut command: Command) -> io::Result<i32> {
    unsafe extern "system" fn leave_to_child(event: u32) -> i32 {
        i32::from(event == 0 || event == 1)
    }
    unsafe {
        SetConsoleCtrlHandler(Some(leave_to_child), 1);
    }
    Ok(command.status()?.code().unwrap_or(1))
}

pub fn launch(mut command: Command) -> io::Result<()> {
    restore_ctrl_c();
    if cfg!(gx_cli) {
        std::process::exit(wait(command)?);
    }
    command.spawn()?;
    Ok(())
}

pub fn report_error(message: &str) {
    eprintln!("{message}");
    if !cfg!(gx_cli) {
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(
                window: *mut std::ffi::c_void,
                text: *const u16,
                title: *const u16,
                flags: u32,
            ) -> i32;
        }
        let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "WezTerm GX".encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10);
        }
    }
}
