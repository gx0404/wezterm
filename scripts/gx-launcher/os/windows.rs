use super::{invalid, Layout};
use std::fs::Permissions;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

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

pub fn launch(mut command: Command) -> io::Result<()> {
    if cfg!(gx_cli) {
        let status = command.status()?;
        std::process::exit(status.code().unwrap_or(1));
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
