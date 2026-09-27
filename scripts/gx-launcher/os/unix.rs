use super::{invalid, Layout};
use std::fs::Permissions;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn layout(bin: &Path) -> io::Result<Layout> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| invalid("HOME is not set"))?;
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("wezterm");
    let resources = bin.join("../../share/wezterm-gx");
    Ok(Layout {
        resources,
        home,
        config,
        plugins: data.join("wezterm/plugins"),
        managed: data.join("wezterm-gx"),
    })
}

pub fn make_writable(permissions: &mut Permissions) {
    permissions.set_mode(permissions.mode() | 0o600);
}

pub fn launch(mut command: Command) -> io::Result<()> {
    Err(command.exec())
}

pub fn report_error(message: &str) {
    eprintln!("{message}");
}
