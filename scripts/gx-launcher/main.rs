//! GX package entry points. Compiled with rustc (Rust >= 1.89), no Cargo deps.
#![cfg_attr(all(windows, not(gx_cli), not(test)), windows_subsystem = "windows")]

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
#[path = "os/windows.rs"]
mod os;
#[cfg(unix)]
#[path = "os/unix.rs"]
mod os;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

#[derive(Debug)]
struct Layout {
    resources: PathBuf,
    home: PathBuf,
    config: PathBuf,
    plugins: PathBuf,
    managed: PathBuf,
}

fn ensure_state_dirs(plugin: &Path) -> io::Result<()> {
    if plugin.join("plugin/resurrect/state_manager.lua").is_file() {
        for kind in ["workspace", "window", "tab"] {
            fs::create_dir_all(plugin.join("state").join(kind))?;
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, dest: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(source)?;
    if meta.file_type().is_symlink() {
        return Err(invalid(format!(
            "Refusing to copy a symlink: {}",
            source.display()
        )));
    }
    if meta.is_dir() {
        fs::create_dir_all(dest)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dest.join(entry.file_name()))?;
        }
    } else if meta.is_file() {
        fs::copy(source, dest)?;
        // Packaged files may be read-only. The user's copy must be editable.
        let mut permissions = fs::metadata(dest)?.permissions();
        os::make_writable(&mut permissions);
        fs::set_permissions(dest, permissions)?;
    } else {
        return Err(invalid(format!("Not a regular file: {}", source.display())));
    }
    Ok(())
}

struct Stage(PathBuf);
impl Stage {
    fn new(parent: &Path, name: &str) -> io::Result<Self> {
        fs::create_dir_all(parent)?;
        let path = parent.join(name);
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        // Only the unique directory created by Stage::new is ever removed.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn initialize(layout: &Layout) -> io::Result<()> {
    let version = fs::read_to_string(layout.resources.join("resource-version"))?;
    let version = version.trim();
    if version.len() != 64 || !version.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid("Invalid GX resource-version"));
    }
    let snapshot = layout.resources.join("dotfiles");
    if !snapshot.join("wezterm-config/wezterm.lua").is_file() {
        return Err(invalid("GX configuration payload is incomplete"));
    }
    fs::create_dir_all(&layout.managed)?;
    // OS locks are released even if the process crashes; no stale lock cleanup.
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(layout.managed.join("initialize.lock"))?;
    lock.lock()?;
    let stamp = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos(),
        std::process::id()
    );

    if !layout.config.exists() && !layout.home.join(".wezterm.lua").exists() {
        let parent = layout
            .config
            .parent()
            .ok_or_else(|| invalid("Invalid config directory"))?;
        let stage = Stage::new(parent, &format!(".wezterm-gx-{stamp}"))?;
        copy_tree(&snapshot.join("wezterm-config"), &stage.0.join("config"))?;
        // Stage on the destination filesystem, including custom XDG mounts.
        fs::rename(stage.0.join("config"), &layout.config)?;
    }

    let previous = fs::read_to_string(layout.managed.join("resource-version")).unwrap_or_default();
    let mut plugins = fs::read_dir(snapshot.join("plugins"))?.collect::<Result<Vec<_>, _>>()?;
    plugins.sort_by_key(|p| p.file_name());
    if plugins.is_empty() {
        return Err(invalid("GX plugin payload is empty"));
    }
    fs::create_dir_all(&layout.plugins)?;
    for plugin in plugins {
        let name = plugin.file_name();
        let dest = layout.plugins.join(&name);
        if previous.trim() == version
            && dest.join(".git").is_dir()
            && dest.join("plugin/init.lua").is_file()
        {
            ensure_state_dirs(&dest)?;
            continue;
        }
        if fs::symlink_metadata(&dest).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(invalid(format!(
                "Refusing to replace a plugin symlink: {}",
                dest.display()
            )));
        }
        let stage = Stage::new(&layout.managed, &format!("stage-{stamp}"))?;
        let candidate = stage.0.join("plugin");
        copy_tree(&plugin.path(), &candidate)?;
        if !candidate.join("gitdir/HEAD").is_file() || !candidate.join("plugin/init.lua").is_file()
        {
            return Err(invalid(format!(
                "Incomplete plugin: {}",
                plugin.path().display()
            )));
        }
        fs::rename(candidate.join("gitdir"), candidate.join(".git"))?;
        if dest.join("state").exists() {
            copy_tree(&dest.join("state"), &candidate.join("state"))?;
        }
        ensure_state_dirs(&candidate)?;
        let backup = layout.managed.join("backups").join(&stamp).join(&name);
        let had_previous = dest.exists();
        if had_previous {
            fs::create_dir_all(backup.parent().unwrap())?;
            fs::rename(&dest, &backup).map_err(|error| invalid(format!(
                "Cannot back up plugin {} to {}: {error}. Close other WezTerm windows and background mux sessions, then retry. Your existing plugin and sessions are unchanged.", dest.display(), backup.display()
            )))?;
        }
        if let Err(error) = fs::rename(&candidate, &dest) {
            if had_previous {
                fs::rename(&backup, &dest).map_err(|restore| {
                    invalid(format!(
                        "Upgrade failed: {error}; restore failed: {restore}; backup: {}",
                        backup.display()
                    ))
                })?;
            }
            return Err(error);
        }
    }
    let marker = layout.managed.join(format!("version-{stamp}"));
    fs::write(&marker, format!("{version}\n"))?;
    fs::rename(marker, layout.managed.join("resource-version"))?;
    Ok(())
}

fn run() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let bin = exe
        .parent()
        .ok_or_else(|| invalid("Missing executable directory"))?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let init_only = args.len() == 1 && args[0] == "--gx-initialize-only";
    let informational = args.len() == 1
        && ["--version", "-V", "--help", "-h"]
            .iter()
            .any(|a| args[0] == *a);
    if !informational {
        initialize(&os::layout(bin)?)?;
    }
    if init_only {
        return Ok(());
    }
    let real = if cfg!(gx_cli) {
        "wezterm"
    } else {
        "wezterm-gui"
    };
    let mut command = Command::new(bin.join(format!("{real}{}", std::env::consts::EXE_SUFFIX)));
    command.args(args);
    os::launch(command)
}

fn main() {
    if let Err(error) = run() {
        os::report_error(&format!("WezTerm GX could not start:\n{error}"));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: Stage,
        layout: Layout,
    }
    impl Fixture {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = Stage::new(
                &std::env::temp_dir(),
                &format!("gx-test-{}-{stamp}", std::process::id()),
            )
            .unwrap();
            let layout = Layout {
                resources: root.0.join("resources"),
                home: root.0.join("用户 home"),
                config: root.0.join("用户 home/.config/wezterm"),
                plugins: root.0.join("data/wezterm/plugins"),
                managed: root.0.join("data/wezterm-gx"),
            };
            let cfg = layout.resources.join("dotfiles/wezterm-config");
            fs::create_dir_all(&cfg).unwrap();
            fs::write(cfg.join("wezterm.lua"), "return {}").unwrap();
            let plugin = layout.resources.join("dotfiles/plugins/example");
            fs::create_dir_all(plugin.join("plugin")).unwrap();
            fs::create_dir_all(plugin.join("gitdir")).unwrap();
            fs::write(plugin.join("plugin/init.lua"), "new code").unwrap();
            fs::write(plugin.join("gitdir/HEAD"), "ref: refs/heads/main").unwrap();
            fs::write(layout.resources.join("resource-version"), "a".repeat(64)).unwrap();
            Self { root, layout }
        }
    }

    #[test]
    fn fresh_install_and_repeat_preserve_edits() {
        let f = Fixture::new();
        initialize(&f.layout).unwrap();
        let config = f.layout.config.join("wezterm.lua");
        fs::write(&config, "personal config").unwrap();
        initialize(&f.layout).unwrap();
        assert_eq!(fs::read_to_string(config).unwrap(), "personal config");
        assert!(f.layout.plugins.join("example/.git/HEAD").is_file());
        assert!(!f.layout.managed.join("backups").exists());
        assert_eq!(fs::read_dir(&f.layout.plugins).unwrap().count(), 1);
    }

    #[test]
    fn creates_and_repairs_resurrect_state_before_lua_require() {
        let f = Fixture::new();
        let module = f
            .layout
            .resources
            .join("dotfiles/plugins/example/plugin/resurrect");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("state_manager.lua"), "return {}").unwrap();
        initialize(&f.layout).unwrap();
        let state = f.layout.plugins.join("example/state");
        for kind in ["workspace", "window", "tab"] {
            assert!(state.join(kind).is_dir());
        }
        fs::remove_dir(state.join("window")).unwrap();
        initialize(&f.layout).unwrap();
        assert!(state.join("window").is_dir());
    }

    #[test]
    fn upgrade_keeps_sessions_other_plugins_and_backup() {
        let f = Fixture::new();
        let plugin = f.layout.plugins.join("example");
        fs::create_dir_all(plugin.join("state/workspace")).unwrap();
        fs::write(plugin.join("state/workspace/工作.json"), "session").unwrap();
        fs::write(plugin.join("old-code"), "old").unwrap();
        fs::create_dir_all(f.layout.plugins.join("other-plugin")).unwrap();
        initialize(&f.layout).unwrap();
        assert_eq!(
            fs::read_to_string(plugin.join("state/workspace/工作.json")).unwrap(),
            "session"
        );
        assert!(f.layout.plugins.join("other-plugin").is_dir());
        let backup = fs::read_dir(f.layout.managed.join("backups"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::read_to_string(backup.join("example/old-code")).unwrap(),
            "old"
        );
        assert!(!plugin.join("old-code").exists());
    }

    #[test]
    fn incomplete_payload_does_not_move_existing_plugin() {
        let f = Fixture::new();
        let plugin = f.layout.plugins.join("example");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("old-code"), "keep").unwrap();
        fs::remove_file(
            f.layout
                .resources
                .join("dotfiles/plugins/example/gitdir/HEAD"),
        )
        .unwrap();
        assert!(initialize(&f.layout).is_err());
        assert_eq!(fs::read_to_string(plugin.join("old-code")).unwrap(), "keep");
        assert!(!f.layout.managed.join("resource-version").exists());
    }

    #[test]
    fn respects_legacy_config_and_serializes_first_start() {
        let f = Fixture::new();
        fs::create_dir_all(&f.layout.home).unwrap();
        fs::write(f.layout.home.join(".wezterm.lua"), "personal").unwrap();
        std::thread::scope(|scope| {
            let a = scope.spawn(|| initialize(&f.layout));
            let b = scope.spawn(|| initialize(&f.layout));
            a.join().unwrap().unwrap();
            b.join().unwrap().unwrap();
        });
        assert!(!f.layout.config.exists());
        assert!(!f.layout.managed.join("backups").exists());
        assert!(f.root.0.exists());
    }

    #[cfg(windows)]
    #[test]
    fn locked_plugin_reports_retry_without_losing_sessions() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        let plugin = f.layout.plugins.join("example");
        fs::create_dir_all(plugin.join("state")).unwrap();
        fs::write(plugin.join("state/session.json"), "keep").unwrap();
        let _locked = File::options()
            .read(true)
            .share_mode(3)
            .open(plugin.join("state/session.json"))
            .unwrap();
        let error = initialize(&f.layout).unwrap_err().to_string();
        assert!(error.contains("Close other WezTerm"));
        assert_eq!(
            fs::read_to_string(plugin.join("state/session.json")).unwrap(),
            "keep"
        );
        assert!(!f.layout.managed.join("resource-version").exists());
    }
}
