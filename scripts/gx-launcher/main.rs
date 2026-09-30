//! GX package entry points. Compiled with rustc (Rust >= 1.89), no Cargo deps.
#![cfg_attr(all(windows, not(gx_cli), not(test)), windows_subsystem = "windows")]

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
#[path = "os/windows.rs"]
mod os;
#[cfg(unix)]
#[path = "os/unix.rs"]
mod os;
mod released;

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

// Config files that belong to the user even if a payload ships one.
const USER_DATA: &[&str] = &["gui-settings.json", "gui-settings.json.tmp"];
// Starts in a row that retry a failed config upgrade before it waits for something to change.
const MAX_ATTEMPTS: u32 = 3;

// A released config file: path, fingerprint and the bitmask of releases that shipped it.
type Shipped<'a> = (&'a str, usize, u64, u8);

fn lf_text(bytes: &[u8]) -> Vec<u8> {
    let mut text = Vec::with_capacity(bytes.len());
    for (index, &byte) in bytes.iter().enumerate() {
        if byte != b'\r' || bytes.get(index + 1) != Some(&b'\n') {
            text.push(byte);
        }
    }
    text
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

// Files with NUL bytes are binary and compared exactly; text is compared with CRLF read as LF.
fn normalized(bytes: &[u8]) -> Cow<'_, [u8]> {
    if bytes.contains(&0) {
        Cow::Borrowed(bytes)
    } else {
        Cow::Owned(lf_text(bytes))
    }
}

// released.rs lists released config files as (path, length, FNV-1a 64) of the normalized bytes.
fn fingerprint(bytes: &[u8]) -> (usize, u64) {
    let bytes = normalized(bytes);
    (bytes.len(), fnv1a(&bytes))
}

// A launcher that learns about another release re-runs the upgrade for the same payload.
fn table_digest(released: &[Shipped]) -> u64 {
    let mut bytes = Vec::new();
    for &(path, len, hash, releases) in released {
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&(len as u64).to_le_bytes());
        bytes.extend_from_slice(&hash.to_le_bytes());
        bytes.push(releases);
    }
    fnv1a(&bytes)
}

fn under(base: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

// What a later start compares to notice that a file was edited, fixed or made writable.
fn probe(config: &Path, rel: &str) -> String {
    let path = under(config, rel);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() => match fs::read(&path) {
            Ok(bytes) => {
                let (len, hash) = fingerprint(&bytes);
                let mode = if meta.permissions().readonly() {
                    "ro"
                } else {
                    "rw"
                };
                format!("{len}:{hash:016x}:{mode}")
            }
            Err(_) => "unreadable".into(),
        },
        Ok(_) => "other".into(),
        Err(_) => "missing".into(),
    }
}

fn write_state(layout: &Layout, stamp: &str, name: &str, text: &str) -> io::Result<()> {
    let marker = layout.managed.join(format!("version-{stamp}"));
    fs::write(&marker, text)?;
    fs::rename(marker, layout.managed.join(name))
}

fn payload_files(dir: &Path, prefix: &str, files: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid(format!("Unsupported file name: {}", path.display())))?;
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            payload_files(&path, &rel, files)?;
        } else if !kind.is_file() {
            return Err(invalid(format!("Not a regular file: {}", path.display())));
        } else if !USER_DATA.contains(&rel.as_str()) {
            files.push(rel);
        }
    }
    Ok(())
}

// Config modules loaded with a literal `require('a.b')` or `require "a.b"`, resolved like the
// package.path set by wezterm.lua: a/b.lua, then a/b/init.lua. `pcall(require, ...)` is optional,
// and `require('wezterm')` is WezTerm's built-in module, not the entry file.
fn required_modules<'a>(source: &[u8], managed: &BTreeSet<&'a str>) -> Vec<&'a str> {
    let text = String::from_utf8_lossy(source);
    let mut modules = Vec::new();
    for (at, word) in text.match_indices("require") {
        if text[..at].ends_with(|c: char| c.is_alphanumeric() || "_.:".contains(c)) {
            continue;
        }
        let rest = text[at + word.len()..].trim_start();
        let rest = rest.strip_prefix('(').map_or(rest, str::trim_start);
        let Some(quote) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') else {
            continue;
        };
        let Some(name) = rest[1..]
            .split(quote)
            .next()
            .filter(|name| *name != "wezterm")
        else {
            continue;
        };
        let base = name.replace('.', "/");
        let module = [format!("{base}.lua"), format!("{base}/init.lua")]
            .iter()
            .find_map(|path| managed.get(path.as_str()).copied());
        if let Some(module) = module.filter(|module| !modules.contains(module)) {
            modules.push(module);
        }
    }
    modules
}

#[derive(PartialEq)]
enum State {
    Current,
    Released,
    Missing,
    Modified,
    Kept(String),
}

impl State {
    fn describe(&self) -> &str {
        match self {
            State::Modified => "modified locally",
            State::Kept(reason) => reason,
            _ => "not updated",
        }
    }
}

struct Managed<'a> {
    new: Vec<u8>,
    // The user's copy of a released file, for the backup and a rollback.
    old: Vec<u8>,
    state: State,
    // Every release shipped exactly this file, so edits to it cannot rely on this upgrade.
    unchanged: bool,
    // Some release shipped this path, so a missing copy was deleted by the user.
    shipped: bool,
    // Config modules required by the shipped file and by the user's copy.
    requires: Vec<&'a str>,
    pins: Vec<&'a str>,
}

// Returns the user's bytes whenever the file was read.
fn user_state(
    config: &Path,
    rel: &str,
    new: &[u8],
    released: &[Shipped],
) -> io::Result<(State, Vec<u8>)> {
    let mut path = config.to_path_buf();
    let mut parts = rel.split('/').peekable();
    while let Some(part) = parts.next() {
        path.push(part);
        let meta = match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok((State::Missing, Vec::new()))
            }
            meta => meta?,
        };
        let reason = match parts.peek() {
            Some(_) if !meta.is_dir() => "inside a symlink or a non-directory",
            None if !meta.is_file() => "a symlink or not a regular file",
            _ => continue,
        };
        return Ok((State::Kept(reason.into()), Vec::new()));
    }
    let old = fs::read(&path)?;
    if normalized(&old) == normalized(new) {
        return Ok((State::Current, Vec::new()));
    }
    let print = fingerprint(&old);
    let state = if !released
        .iter()
        .any(|&(file, len, hash, _)| file == rel && (len, hash) == print)
    {
        State::Modified
    } else if fs::symlink_metadata(&path)?.permissions().readonly() {
        State::Kept("read-only".into())
    } else {
        State::Released
    };
    Ok((state, old))
}

// Adds a file without replacing one that appeared since the scan; file systems without hard
// links fall back to a rename.
fn place_new(temp: &Path, target: &Path) -> io::Result<()> {
    match fs::hard_link(temp, target) {
        Ok(()) => {
            let _ = fs::remove_file(temp);
            Ok(())
        }
        Err(_) if fs::symlink_metadata(target).is_err() => fs::rename(temp, target),
        Err(_) => Err(invalid(format!(
            "{} appeared during the upgrade",
            target.display()
        ))),
    }
}

// Every new copy is written to a temp file and every released copy is checked and backed up
// before the first rename; a failed rename puts back the files the group already replaced.
fn write_group(
    config: &Path,
    backups: &Path,
    stamp: &str,
    group: &[&str],
    files: &BTreeMap<&str, Managed>,
) -> Result<(), String> {
    let mut staged = Vec::new();
    let result = stage_group(config, backups, stamp, group, files, &mut staged)
        .and_then(|()| swap_group(group, files, &staged, stamp));
    for (temp, _) in &staged {
        let _ = fs::remove_file(temp);
    }
    result
}

fn stage_group(
    config: &Path,
    backups: &Path,
    stamp: &str,
    group: &[&str],
    files: &BTreeMap<&str, Managed>,
    staged: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), String> {
    for (index, &rel) in group.iter().enumerate() {
        let file = &files[rel];
        let target = under(config, rel);
        let temp = target.with_file_name(format!(".wezterm-gx-{stamp}-{index}"));
        let ready = target
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| {
                staged.push((temp.clone(), target.clone()));
                fs::write(&temp, &file.new)
            })
            .and_then(|()| {
                if file.state != State::Released {
                    return match fs::symlink_metadata(&target) {
                        Ok(_) => Err(invalid("it appeared during the upgrade")),
                        Err(_) => Ok(()),
                    };
                }
                if fs::read(&target)? != file.old {
                    return Err(invalid("it changed during the upgrade"));
                }
                os::keep_mode(&fs::symlink_metadata(&target)?, &temp)
            });
        ready.map_err(|error| format!("{rel}: {error}"))?;
    }
    for &rel in group {
        let file = &files[rel];
        if file.state == State::Released {
            let backup = under(backups, rel);
            backup
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| fs::write(&backup, &file.old))
                .map_err(|error| {
                    format!("{rel}: cannot back up to {}: {error}", backup.display())
                })?;
        }
    }
    Ok(())
}

fn swap_group(
    group: &[&str],
    files: &BTreeMap<&str, Managed>,
    staged: &[(PathBuf, PathBuf)],
    stamp: &str,
) -> Result<(), String> {
    for (index, (&rel, (temp, target))) in group.iter().zip(staged).enumerate() {
        let swapped = match files[rel].state {
            State::Released => fs::rename(temp, target),
            _ => place_new(temp, target),
        };
        let Err(error) = swapped else {
            continue;
        };
        let mut message = format!("{rel}: {error}; the files written with it were put back");
        for (&done, (_, target)) in group[..index].iter().zip(staged).rev() {
            let file = &files[done];
            let undone = if file.state == State::Released {
                let undo = target.with_file_name(format!(".wezterm-gx-{stamp}-undo"));
                fs::write(&undo, &file.old)
                    .and_then(|()| os::keep_mode(&fs::symlink_metadata(target)?, &undo))
                    .and_then(|()| fs::rename(&undo, target))
                    .inspect_err(|_| {
                        let _ = fs::remove_file(&undo);
                    })
            } else {
                fs::remove_file(target)
            };
            if let Err(error) = undone {
                message.push_str(&format!("; cannot put back {done}: {error}"));
            }
        }
        return Err(message);
    }
    Ok(())
}

fn append_log(layout: &Layout, stamp: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let text: String = lines
        .iter()
        .map(|line| format!("{stamp} {line}\n"))
        .collect();
    let written = fs::create_dir_all(&layout.managed).and_then(|()| {
        File::options()
            .create(true)
            .append(true)
            .open(layout.managed.join("config-migration.log"))?
            .write_all(text.as_bytes())
    });
    if let Err(error) = written {
        eprintln!("Cannot write the config upgrade log: {error}");
    }
}

#[derive(Default)]
struct Upgrade {
    // Files that could not be read or written; the next starts retry them.
    failed: Vec<String>,
    // Kept files that held others back; editing one re-runs the upgrade.
    held: Vec<String>,
}

/// Upgrades an existing config file by file. A copy of a released file is backed up and
/// replaced, a new file added, anything else kept and logged. A released file the user deleted
/// comes back only when code that will run requires it; without a GX wezterm.lua nothing else is
/// added either. Files move together with the config modules they require: new code waits for
/// the modules it requires, a file that stays behind keeps the released modules it requires, and
/// edits to a file whose shipped copy never changed hold nothing back. Each set of files that
/// moves together is written all or nothing.
fn migrate_config(
    layout: &Layout,
    stamp: &str,
    backup_stamp: &str,
    released: &[Shipped],
) -> io::Result<Upgrade> {
    let config = &layout.config;
    let mut upgrade = Upgrade::default();
    match fs::symlink_metadata(config) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(upgrade),
        Ok(meta) if !meta.is_dir() => {
            let note = format!("kept {}: a symlink or not a directory", config.display());
            append_log(layout, stamp, &[note]);
            return Ok(upgrade);
        }
        meta => {
            meta?;
        }
    }
    let payload = under(&layout.resources, "dotfiles/wezterm-config");
    let mut rels = Vec::new();
    payload_files(&payload, "", &mut rels)?;
    let names: BTreeSet<&str> = rels.iter().map(String::as_str).collect();
    let every = released.iter().fold(0, |every, entry| every | entry.3);
    let mut files = BTreeMap::new();
    for &rel in &names {
        let new = fs::read(under(&payload, rel))?;
        let (state, mut old) = user_state(config, rel, &new, released).unwrap_or_else(|error| {
            upgrade.failed.push(rel.to_string());
            (State::Kept(format!("unreadable ({error})")), Vec::new())
        });
        let (requires, pins) = if rel.ends_with(".lua") {
            (
                required_modules(&new, &names),
                required_modules(&old, &names),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let print = fingerprint(&new);
        let unchanged = released.iter().any(|&(file, len, hash, releases)| {
            file == rel && (len, hash) == print && releases == every
        });
        let shipped = released.iter().any(|entry| entry.0 == rel);
        if state != State::Released {
            old = Vec::new();
        }
        files.insert(
            rel,
            Managed {
                new,
                old,
                state,
                unchanged,
                shipped,
                requires,
                pins,
            },
        );
    }

    let gx_entry = files
        .get("wezterm.lua")
        .is_some_and(|file| matches!(file.state, State::Current | State::Released));
    let mut update: BTreeSet<&str> = files
        .iter()
        .filter(|(_, file)| matches!(file.state, State::Released | State::Missing))
        .map(|(rel, _)| *rel)
        .collect();
    // Only modules the config can load from wezterm.lua pin others; tests/ is run by hand.
    let mut loaded = BTreeSet::new();
    let mut stack = vec!["wezterm.lua"];
    while let Some(rel) = stack.pop() {
        let Some(file) = files.get(rel) else {
            continue;
        };
        if loaded.insert(rel) {
            stack.extend(file.requires.iter().chain(&file.pins));
        }
    }
    // Why a file stays behind: the module it waits for, or the file that keeps it.
    let mut cause = BTreeMap::new();
    loop {
        let at_new = |rel: &str| update.contains(rel) || files[rel].state == State::Current;
        let edited = |rel: &str| files[rel].state == State::Modified && files[rel].unchanged;
        let mut blocked = BTreeMap::new();
        for &rel in &update {
            let waits = files[rel]
                .requires
                .iter()
                .find(|&&dep| !at_new(dep) && !edited(dep));
            if let Some(&dep) = waits {
                blocked.insert(rel, dep);
            }
        }
        for (&rel, file) in &files {
            if loaded.contains(rel) && !at_new(rel) && file.state != State::Missing && !edited(rel)
            {
                for &dep in &file.pins {
                    if files[dep].state == State::Released && update.contains(dep) {
                        blocked.entry(dep).or_insert(rel);
                    }
                }
            }
        }
        // New files go into a GX config; deleted released files and the rest only when needed.
        let wanted = |rel: &str| gx_entry && !files[rel].shipped;
        let mut needed = BTreeSet::new();
        let mut stack: Vec<&str> = files
            .iter()
            .filter(|(rel, file)| at_new(rel) && (file.state != State::Missing || wanted(rel)))
            .map(|(rel, _)| *rel)
            .collect();
        while let Some(rel) = stack.pop() {
            for &dep in &files[rel].requires {
                if files[dep].state == State::Missing && update.contains(dep) && needed.insert(dep)
                {
                    stack.push(dep);
                }
            }
        }
        let unwanted: Vec<&str> = update
            .iter()
            .copied()
            .filter(|rel| {
                files[rel].state == State::Missing && !wanted(rel) && !needed.contains(rel)
            })
            .collect();
        if blocked.is_empty() && unwanted.is_empty() {
            break;
        }
        update.retain(|rel| !blocked.contains_key(rel) && !unwanted.contains(rel));
        cause.extend(blocked);
    }

    let mut log = Vec::new();
    let mut held = BTreeSet::new();
    for (&rel, file) in &files {
        if matches!(file.state, State::Modified | State::Kept(_)) {
            log.push(format!(
                "kept {rel}: {}; shipped version: {}",
                file.state.describe(),
                under(&payload, rel).display()
            ));
            continue;
        }
        let Some(mut origin) = cause.get(rel).copied() else {
            continue;
        };
        let mut seen = BTreeSet::from([rel]);
        while let Some(&next) = cause.get(origin) {
            if !seen.insert(origin) {
                break;
            }
            origin = next;
        }
        held.insert(origin);
        log.push(format!(
            "skipped {rel}: held back because {origin} is {}",
            files[origin].state.describe()
        ));
    }
    upgrade.held = held.into_iter().map(String::from).collect();

    let backups = layout
        .managed
        .join("backups")
        .join(backup_stamp)
        .join("wezterm-config");
    for group in groups(&update, &files) {
        match write_group(config, &backups, stamp, &group, &files) {
            Ok(()) => log.extend(group.iter().map(|rel| match files[rel].state {
                State::Released => {
                    format!("replaced {rel}; backup: {}", under(&backups, rel).display())
                }
                _ => format!("added {rel}"),
            })),
            Err(error) => {
                for rel in group {
                    upgrade.failed.push(rel.to_string());
                    log.push(format!("failed {rel}: {error}"));
                }
            }
        }
    }
    append_log(layout, stamp, &log);
    Ok(upgrade)
}

// Files that must change together because a require links them in either version, each set
// ordered with modules first; a require cycle goes in path order.
fn groups<'a>(
    update: &BTreeSet<&'a str>,
    files: &BTreeMap<&'a str, Managed<'a>>,
) -> Vec<Vec<&'a str>> {
    let mut linked: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for &rel in update {
        for &dep in files[rel].requires.iter().chain(&files[rel].pins) {
            if dep != rel && update.contains(dep) {
                linked.entry(rel).or_default().push(dep);
                linked.entry(dep).or_default().push(rel);
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut groups = Vec::new();
    for &start in update {
        if !seen.insert(start) {
            continue;
        }
        let mut members = vec![start];
        let mut stack = vec![start];
        while let Some(rel) = stack.pop() {
            for &next in linked.get(rel).into_iter().flatten() {
                if seen.insert(next) {
                    members.push(next);
                    stack.push(next);
                }
            }
        }
        members.sort_unstable();
        let mut ordered = Vec::new();
        while !members.is_empty() {
            let ready: Vec<&str> = members
                .iter()
                .copied()
                .filter(|&rel| {
                    files[rel]
                        .requires
                        .iter()
                        .all(|&dep| dep == rel || !members.contains(&dep))
                })
                .collect();
            let batch = if ready.is_empty() {
                members.clone()
            } else {
                ready
            };
            members.retain(|rel| !batch.contains(rel));
            ordered.extend(batch);
        }
        groups.push(ordered);
    }
    groups
}

/// Runs the config upgrade once per payload, fingerprint table and config directory, again when
/// a file that held others back or failed to update changes, and retries failures a few starts
/// in a row, backing up into the same directory each time.
fn upgrade_config(
    layout: &Layout,
    version: &str,
    stamp: &str,
    seeded: bool,
    released: &[Shipped],
) -> io::Result<()> {
    let head = format!(
        "{version} {:016x} {}",
        table_digest(released),
        layout.config.display()
    );
    let recorded = fs::read_to_string(layout.managed.join("config-version")).unwrap_or_default();
    let mut lines = recorded.lines();
    let same = lines.next() == Some(head.as_str());
    let (mut attempts, mut backup_stamp, mut changed) = (0, stamp.to_string(), false);
    if same {
        for line in lines {
            match line
                .split_once(' ')
                .map(|(key, rest)| (key, rest.split_once(' ')))
            {
                Some(("retry", Some((count, earlier)))) => {
                    attempts = count.parse().unwrap_or(MAX_ATTEMPTS);
                    backup_stamp = earlier.to_string();
                }
                Some(("watch", Some((seen, rel)))) => {
                    changed |= probe(&layout.config, rel) != seen;
                }
                _ => {}
            }
        }
    }
    if same && !changed && !(1..MAX_ATTEMPTS).contains(&attempts) {
        return Ok(());
    }
    if attempts == 0 {
        backup_stamp = stamp.to_string();
    }
    let upgrade = if seeded {
        Upgrade::default()
    } else {
        migrate_config(layout, stamp, &backup_stamp, released).map_err(|error| {
            invalid(format!(
                "Cannot upgrade the configuration in {}: {error}",
                layout.config.display()
            ))
        })?
    };
    let mut text = format!("{head}\n");
    if !upgrade.failed.is_empty() {
        text.push_str(&format!("retry {} {backup_stamp}\n", attempts + 1));
    }
    for rel in upgrade.held.iter().chain(&upgrade.failed) {
        text.push_str(&format!("watch {} {rel}\n", probe(&layout.config, rel)));
    }
    if let Err(error) = write_state(layout, stamp, "config-version", &text) {
        eprintln!("Cannot record the config upgrade: {error}");
    }
    Ok(())
}

// The Settings page can create the config directory with only gui-settings.json in it.
fn seedable(config: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(config) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Ok(meta) if meta.is_dir() => {
            for entry in fs::read_dir(config)? {
                let name = entry?.file_name();
                if !USER_DATA.iter().any(|data| name == *data) {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        found => found.map(|_| false),
    }
}

fn seed(layout: &Layout, snapshot: &Path, stamp: &str) -> io::Result<()> {
    let parent = layout
        .config
        .parent()
        .ok_or_else(|| invalid("Invalid config directory"))?;
    let stage = Stage::new(parent, &format!(".wezterm-gx-{stamp}"))?;
    let staged = stage.0.join("config");
    copy_tree(&snapshot.join("wezterm-config"), &staged)?;
    if fs::symlink_metadata(&layout.config).is_err() {
        // Stage on the destination filesystem, including custom XDG mounts.
        return fs::rename(&staged, &layout.config);
    }
    for name in USER_DATA {
        let settings = layout.config.join(name);
        if settings.is_file() {
            fs::copy(&settings, staged.join(name))?;
        }
    }
    // The old directory holds only the settings copied above; keep it until the swap is done.
    let previous = stage.0.join("previous");
    fs::rename(&layout.config, &previous)?;
    fs::rename(&staged, &layout.config).inspect_err(|_| {
        let _ = fs::rename(&previous, &layout.config);
    })
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

    let seeded = !layout.home.join(".wezterm.lua").exists() && seedable(&layout.config)?;
    if seeded {
        seed(layout, &snapshot, &stamp)?;
    }
    upgrade_config(layout, version, &stamp, seeded, released::RELEASED)?;

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
    write_state(layout, &stamp, "resource-version", &format!("{version}\n"))
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

    const OLD_LAUNCH: &str = "local options = {}\nreturn options\n";
    const NEW_LAUNCH: &str = "local gx_shell = require('utils.gx-shell')\nreturn {}\n";
    const HELPER: &str = "return {}\n";
    const BACKUP: &str = "backups/1/wezterm-config/config/launch.lua";
    const LOG: &str = "config-migration.log";

    // A table of files shipped by release 1 of 2, so none counts as unchanged in every release.
    fn released(files: &[(&'static str, &str)]) -> Vec<Shipped<'static>> {
        files
            .iter()
            .map(|&(path, text)| {
                let (len, hash) = fingerprint(text.as_bytes());
                (path, len, hash, 0b01)
            })
            .chain([("utils/other-release.lua", 0, 0, 0b10)])
            .collect()
    }

    fn migrate(f: &Fixture, stamp: &str, list: &[Shipped]) -> Upgrade {
        migrate_config(&f.layout, stamp, stamp, list).unwrap()
    }

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
            fs::create_dir_all(cfg.join("config")).unwrap();
            fs::create_dir_all(cfg.join("utils")).unwrap();
            fs::write(cfg.join("config/launch.lua"), NEW_LAUNCH).unwrap();
            fs::write(cfg.join("utils/gx-shell.lua"), HELPER).unwrap();
            let plugin = layout.resources.join("dotfiles/plugins/example");
            fs::create_dir_all(plugin.join("plugin")).unwrap();
            fs::create_dir_all(plugin.join("gitdir")).unwrap();
            fs::write(plugin.join("plugin/init.lua"), "new code").unwrap();
            fs::write(plugin.join("gitdir/HEAD"), "ref: refs/heads/main").unwrap();
            fs::write(layout.resources.join("resource-version"), "a".repeat(64)).unwrap();
            Self { root, layout }
        }

        fn user_file(&self, rel: &str, contents: impl AsRef<[u8]>) -> PathBuf {
            let path = self.layout.config.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }

        fn payload_file(&self, rel: &str, contents: impl AsRef<[u8]>) {
            let path = self
                .layout
                .resources
                .join("dotfiles/wezterm-config")
                .join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }

        fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.layout.config.join(rel)).unwrap()
        }

        fn log(&self) -> String {
            fs::read_to_string(self.layout.managed.join(LOG)).unwrap_or_default()
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

    #[test]
    fn fingerprint_hashes_text_as_lf_and_binaries_exactly() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(lf_text(b"a\r\nb\rc\r\n"), b"a\nb\rc\n");
        assert_eq!(fingerprint(b"a\r\nb"), (3, fnv1a(b"a\nb")));
        assert_eq!(fingerprint(b"\0a\r\nb"), (5, fnv1a(b"\0a\r\nb")));
    }

    #[test]
    fn released_table_keeps_earlier_launch_upgrades() {
        for entry in [
            (1935, 0x3a57_0763_d22b_e724, 0b01),
            (2339, 0x7586_010e_9f4e_d8dc, 0b10),
        ] {
            let (len, hash, releases) = entry;
            assert!(released::RELEASED.contains(&("config/launch.lua", len, hash, releases)));
        }
        assert!(released::RELEASED.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(released::RELEASED
            .iter()
            .all(|(path, ..)| !USER_DATA.contains(path)));
    }

    #[test]
    fn required_modules_follow_the_config_package_path() {
        let managed = BTreeSet::from([
            "config/init.lua",
            "config/launch.lua",
            "utils/gx-shell.lua",
            "wezterm.lua",
            "x.lua",
        ]);
        let source = br#"local Config = require('config')
local gx_shell = require "utils.gx-shell"
local menu = require("config.launch").launch_menu
local plugin = wezterm.plugin.require('https://github.com/x')
local ok = pcall(require, 'x')
local required = required('x')
require('config')
require('wezterm')
"#;
        assert_eq!(
            required_modules(source, &managed),
            ["config/init.lua", "utils/gx-shell.lua", "config/launch.lua"]
        );
    }

    #[test]
    fn migrates_untouched_release_launch_with_helper_and_backup() {
        let f = Fixture::new();
        let launch = f.user_file("config/launch.lua", OLD_LAUNCH);
        let list = released(&[("config/launch.lua", OLD_LAUNCH)]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(fs::read_to_string(&launch).unwrap(), NEW_LAUNCH);
        assert_eq!(f.read("utils/gx-shell.lua"), HELPER);
        let backup = f.layout.managed.join(BACKUP);
        assert_eq!(fs::read_to_string(backup).unwrap(), OLD_LAUNCH);
        for dir in ["config", "utils"] {
            assert_eq!(fs::read_dir(f.layout.config.join(dir)).unwrap().count(), 1);
        }
        assert!(!f.layout.config.join("wezterm.lua").exists());
    }

    #[test]
    fn migrates_crlf_release_launch_beside_identical_helper() {
        let f = Fixture::new();
        let old = OLD_LAUNCH.replace('\n', "\r\n");
        let launch = f.user_file("config/launch.lua", &old);
        let helper = f.user_file("utils/gx-shell.lua", HELPER.replace('\n', "\r\n"));
        let list = released(&[("config/launch.lua", OLD_LAUNCH)]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(fs::read_to_string(&launch).unwrap(), NEW_LAUNCH);
        assert_eq!(
            fs::read_to_string(helper).unwrap(),
            HELPER.replace('\n', "\r\n")
        );
        let backup = f.layout.managed.join(BACKUP);
        assert_eq!(fs::read_to_string(backup).unwrap(), old);
    }

    #[test]
    fn customized_launch_is_kept_without_helper() {
        let f = Fixture::new();
        let custom = format!("{OLD_LAUNCH}-- mine\n");
        let launch = f.user_file("config/launch.lua", &custom);
        let list = released(&[("config/launch.lua", OLD_LAUNCH)]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(fs::read_to_string(&launch).unwrap(), custom);
        assert!(!f.layout.config.join("utils").exists());
        assert!(!f.layout.managed.join("backups").exists());
        assert!(f
            .log()
            .contains("1 kept config/launch.lua: modified locally"));
    }

    #[test]
    fn different_helper_blocks_launch_migration() {
        let f = Fixture::new();
        let launch = f.user_file("config/launch.lua", OLD_LAUNCH);
        let helper = f.user_file("utils/gx-shell.lua", "return 'mine'\n");
        let list = released(&[("config/launch.lua", OLD_LAUNCH)]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(fs::read_to_string(&launch).unwrap(), OLD_LAUNCH);
        assert_eq!(fs::read_to_string(helper).unwrap(), "return 'mine'\n");
        assert!(!f.layout.managed.join("backups").exists());
        let log = f.log();
        let why = "held back because utils/gx-shell.lua is modified locally";
        assert!(
            log.contains(&format!("1 skipped config/launch.lua: {why}")),
            "{log}"
        );
    }

    #[test]
    fn current_launch_is_left_alone() {
        let f = Fixture::new();
        let current = NEW_LAUNCH.replace('\n', "\r\n");
        let launch = f.user_file("config/launch.lua", &current);
        f.user_file("utils/gx-shell.lua", HELPER);
        let list = released(&[
            ("config/launch.lua", OLD_LAUNCH),
            ("config/launch.lua", NEW_LAUNCH),
        ]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(fs::read_to_string(&launch).unwrap(), current);
        assert!(!f.layout.managed.join("backups").exists());
        assert_eq!(f.log(), "");
    }

    #[test]
    fn fresh_seed_is_not_migrated() {
        let f = Fixture::new();
        initialize(&f.layout).unwrap();
        let list = released(&[
            ("config/launch.lua", OLD_LAUNCH),
            ("config/launch.lua", NEW_LAUNCH),
        ]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(f.read("config/launch.lua"), NEW_LAUNCH);
        assert!(!f.layout.managed.join("backups").exists());
        assert_eq!(f.log(), "");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_launch_is_left_alone() {
        let f = Fixture::new();
        let target = f.root.0.join("repo/launch.lua");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, OLD_LAUNCH).unwrap();
        let launch = f.layout.config.join("config/launch.lua");
        fs::create_dir_all(launch.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &launch).unwrap();
        let list = released(&[("config/launch.lua", OLD_LAUNCH)]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert!(fs::symlink_metadata(&launch)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), OLD_LAUNCH);
        assert!(!f.layout.config.join("utils").exists());
        assert!(!f.layout.managed.join("backups").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directories_are_left_alone() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let repo = f.root.0.join("repo");
        fs::create_dir_all(repo.join("utils")).unwrap();
        fs::write(repo.join("utils/gx-shell.lua"), "return 1\n").unwrap();
        f.user_file("wezterm.lua", "return {}");
        symlink(repo.join("utils"), f.layout.config.join("utils")).unwrap();
        let list = released(&[("utils/gx-shell.lua", "return 1\n")]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        let helper = fs::read_to_string(repo.join("utils/gx-shell.lua")).unwrap();
        assert_eq!(helper, "return 1\n");
        assert!(!f.layout.config.join("config/launch.lua").exists());
        assert!(f
            .log()
            .contains("1 kept utils/gx-shell.lua: inside a symlink"));

        let linked = Fixture::new();
        fs::create_dir_all(linked.layout.config.parent().unwrap()).unwrap();
        symlink(&repo, &linked.layout.config).unwrap();
        assert!(migrate(&linked, "1", &list).failed.is_empty());
        assert_eq!(fs::read_dir(&repo).unwrap().count(), 1);
    }

    #[test]
    fn repeated_migration_is_idempotent() {
        let f = Fixture::new();
        let launch = f.user_file("config/launch.lua", OLD_LAUNCH);
        let list = released(&[("config/launch.lua", OLD_LAUNCH)]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert!(migrate(&f, "2", &list).failed.is_empty());
        assert_eq!(fs::read_to_string(&launch).unwrap(), NEW_LAUNCH);
        let backups = fs::read_dir(f.layout.managed.join("backups")).unwrap();
        assert_eq!(backups.count(), 1);
    }

    #[test]
    fn released_gx_config_is_upgraded_file_by_file() {
        const OLD_ENTRY: &str = "return 1\n";
        const OLD_IMAGE: &[u8] = b"\x89PNG\r\n\x1a\n\0old";
        const NEW_IMAGE: &[u8] = b"\x89PNG\r\n\x1a\n\0new";
        let f = Fixture::new();
        let new_launch = "require('utils.gx-shell')\nrequire('utils.shells')\n";
        f.payload_file("config/launch.lua", new_launch);
        f.payload_file("utils/shells.lua", "return {}\n");
        f.payload_file("backdrops/a.png", NEW_IMAGE);
        f.payload_file("backdrops/b.png", NEW_IMAGE);
        f.payload_file("README.md", "new\n");
        f.payload_file("gui-settings.json", "{}");
        f.user_file("wezterm.lua", OLD_ENTRY.replace('\n', "\r\n"));
        f.user_file("config/launch.lua", OLD_LAUNCH);
        f.user_file("backdrops/a.png", OLD_IMAGE);
        let mangled = b"\x89PNG\n\x1a\n\0old";
        f.user_file("backdrops/b.png", mangled);
        f.user_file("README.md", "my notes\n");
        f.user_file("config/mine.lua", "return 'mine'\n");
        let settings = r#"{"default_shell":"pwsh"}"#;
        f.user_file("gui-settings.json", settings);
        let mut list = released(&[
            ("wezterm.lua", OLD_ENTRY),
            ("config/launch.lua", OLD_LAUNCH),
            ("README.md", "old\n"),
        ]);
        let (len, hash) = fingerprint(OLD_IMAGE);
        list.extend([
            ("backdrops/a.png", len, hash, 0b01),
            ("backdrops/b.png", len, hash, 0b01),
        ]);

        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(f.read("wezterm.lua"), "return {}");
        assert_eq!(f.read("config/launch.lua"), new_launch);
        assert_eq!(f.read("utils/gx-shell.lua"), HELPER);
        assert_eq!(f.read("utils/shells.lua"), "return {}\n");
        let image = |name: &str| fs::read(f.layout.config.join("backdrops").join(name)).unwrap();
        assert_eq!(image("a.png"), NEW_IMAGE);
        assert_eq!(image("b.png"), mangled);
        assert_eq!(f.read("README.md"), "my notes\n");
        assert_eq!(f.read("config/mine.lua"), "return 'mine'\n");
        assert_eq!(f.read("gui-settings.json"), settings);
        let backups = f.layout.managed.join("backups/1/wezterm-config");
        let saved = |rel: &str| fs::read(backups.join(rel)).unwrap();
        assert_eq!(
            saved("wezterm.lua"),
            OLD_ENTRY.replace('\n', "\r\n").as_bytes()
        );
        assert_eq!(saved("config/launch.lua"), OLD_LAUNCH.as_bytes());
        assert_eq!(saved("backdrops/a.png"), OLD_IMAGE);
        assert!(!backups.join("README.md").exists());
        let log = f.log();
        for line in [
            "1 replaced config/launch.lua; backup: ",
            "1 added utils/shells.lua",
            "1 kept README.md: modified locally",
            "1 kept backdrops/b.png: modified locally",
        ] {
            assert!(log.contains(line), "{log}");
        }
        assert!(!log.contains("gui-settings"));

        assert!(migrate(&f, "2", &list).failed.is_empty());
        assert!(!f.layout.managed.join("backups/2").exists());
    }

    #[test]
    fn new_code_waits_for_the_modules_it_requires() {
        const OLD_HELPER: &str = "return 'old'\n";
        const ENTRY: &str = "require('config.launch')\nrequire('events.new-tab-button')\n";
        let f = Fixture::new();
        let old_launch = "require('utils.gx-shell')\nreturn 1\n";
        let button = "local launch = require('config.launch')\n";
        let old_test = "require('config.fonts')\n";
        f.payload_file("wezterm.lua", ENTRY);
        f.payload_file("config/launch.lua", "require('utils.shells')\n");
        f.payload_file("utils/shells.lua", "return {}\n");
        f.payload_file("events/new-tab-button.lua", format!("{button}return 2\n"));
        f.payload_file("config/fonts.lua", "return 2\n");
        f.payload_file("tests/pure_fn_test.lua", format!("{old_test}{button}"));
        f.user_file("wezterm.lua", ENTRY);
        f.user_file("config/launch.lua", old_launch);
        f.user_file("utils/gx-shell.lua", OLD_HELPER);
        f.user_file("utils/shells.lua", "return 'mine'\n");
        f.user_file("events/new-tab-button.lua", button);
        f.user_file("config/fonts.lua", "return 1\n");
        f.user_file("tests/pure_fn_test.lua", old_test);
        let list = released(&[
            ("config/launch.lua", old_launch),
            ("utils/gx-shell.lua", OLD_HELPER),
            ("events/new-tab-button.lua", button),
            ("config/fonts.lua", "return 1\n"),
            ("tests/pure_fn_test.lua", old_test),
        ]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(f.read("config/launch.lua"), old_launch);
        assert_eq!(f.read("utils/gx-shell.lua"), OLD_HELPER);
        assert_eq!(f.read("utils/shells.lua"), "return 'mine'\n");
        assert_eq!(f.read("events/new-tab-button.lua"), button);
        assert_eq!(f.read("tests/pure_fn_test.lua"), old_test);
        assert_eq!(f.read("config/fonts.lua"), "return 2\n");
        let log = f.log();
        for rel in [
            "config/launch.lua",
            "events/new-tab-button.lua",
            "utils/gx-shell.lua",
            "tests/pure_fn_test.lua",
        ] {
            let line = format!("1 skipped {rel}: held back because utils/shells.lua is modified");
            assert!(log.contains(&line), "{log}");
        }
        assert!(log.contains("1 replaced config/fonts.lua"), "{log}");
    }

    #[test]
    fn modified_file_keeps_the_released_modules_it_requires() {
        const ENTRY: &str = "require('config.appearance')\nrequire('config.bindings')\n";
        let f = Fixture::new();
        let old_plugins = "return { resurrect = {} }\n";
        let old_bindings = "local plugins = require('config.plugins')\nreturn {}\n";
        let old_appearance = "local plugins = require('config.plugins')\nreturn 1\n";
        f.payload_file(
            "config/plugins.lua",
            "return { resurrect = function() end }\n",
        );
        f.payload_file(
            "config/bindings.lua",
            format!("{old_bindings}-- new keys\n"),
        );
        f.payload_file(
            "config/appearance.lua",
            "local plugins = require('config.plugins')\nreturn 2\n",
        );
        f.payload_file("config/fonts.lua", "return 2\n");
        let old_status = "require('config.appearance')\nreturn 1\n";
        f.payload_file(
            "events/status.lua",
            "require('config.appearance')\nreturn 2\n",
        );
        f.payload_file("wezterm.lua", ENTRY);
        f.user_file("wezterm.lua", ENTRY);
        f.user_file("config/plugins.lua", old_plugins);
        let custom = format!("{old_bindings}-- my keys\n");
        f.user_file("config/bindings.lua", &custom);
        f.user_file("config/appearance.lua", old_appearance);
        f.user_file("config/fonts.lua", "return 1\n");
        f.user_file("events/status.lua", old_status);
        let list = released(&[
            ("config/plugins.lua", old_plugins),
            ("config/bindings.lua", old_bindings),
            ("config/appearance.lua", old_appearance),
            ("config/fonts.lua", "return 1\n"),
            ("events/status.lua", old_status),
        ]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(f.read("config/plugins.lua"), old_plugins);
        assert_eq!(f.read("config/bindings.lua"), custom);
        assert_eq!(f.read("config/appearance.lua"), old_appearance);
        assert_eq!(f.read("events/status.lua"), old_status);
        assert_eq!(f.read("config/fonts.lua"), "return 2\n");
        let log = f.log();
        let why = "held back because config/bindings.lua is modified locally";
        for line in [
            "1 kept config/bindings.lua: modified locally".to_string(),
            format!("1 skipped config/plugins.lua: {why}"),
            format!("1 skipped config/appearance.lua: {why}"),
            format!("1 skipped events/status.lua: {why}"),
            "1 replaced config/fonts.lua".to_string(),
        ] {
            assert!(log.contains(&line), "{log}");
        }
    }

    #[test]
    fn edited_entry_only_gets_files_that_updated_code_requires() {
        let f = Fixture::new();
        f.payload_file("README.md", "readme\n");
        f.payload_file("utils/math.lua", "return 2\n");
        f.user_file("wezterm.lua", "-- my own config\n");
        f.user_file("config/launch.lua", OLD_LAUNCH);
        f.user_file("utils/math.lua", "return 1\n");
        let list = released(&[
            ("config/launch.lua", OLD_LAUNCH),
            ("utils/math.lua", "return 1\n"),
        ]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(f.read("wezterm.lua"), "-- my own config\n");
        assert_eq!(f.read("config/launch.lua"), NEW_LAUNCH);
        assert_eq!(f.read("utils/gx-shell.lua"), HELPER);
        assert_eq!(f.read("utils/math.lua"), "return 2\n");
        assert!(!f.layout.config.join("README.md").exists());
    }

    #[test]
    fn foreign_config_is_left_alone() {
        let f = Fixture::new();
        f.user_file("wezterm.lua", "return { font_size = 20 }\n");
        assert!(migrate(&f, "1", released::RELEASED).failed.is_empty());
        assert_eq!(fs::read_dir(&f.layout.config).unwrap().count(), 1);
        assert!(!f.layout.managed.join("backups").exists());
    }

    #[test]
    fn config_upgrade_runs_once_per_payload_version_and_config() {
        let mut f = Fixture::new();
        initialize(&f.layout).unwrap();
        let helper = f.layout.config.join("utils/gx-shell.lua");
        fs::remove_file(&helper).unwrap();
        initialize(&f.layout).unwrap();
        assert!(!helper.exists());
        let version = "b".repeat(64);
        fs::write(f.layout.resources.join("resource-version"), &version).unwrap();
        initialize(&f.layout).unwrap();
        initialize(&f.layout).unwrap();
        assert_eq!(fs::read_to_string(&helper).unwrap(), HELPER);
        assert_eq!(f.log().matches("added utils/gx-shell.lua").count(), 1);
        let recorded = fs::read_to_string(f.layout.managed.join("config-version")).unwrap();
        assert!(recorded.starts_with(&format!("{version} ")));

        f.layout.config = f.root.0.join("xdg/wezterm");
        f.user_file("wezterm.lua", "return {}");
        f.user_file("config/launch.lua", NEW_LAUNCH);
        initialize(&f.layout).unwrap();
        assert_eq!(f.read("utils/gx-shell.lua"), HELPER);
        assert_eq!(f.log().matches("added utils/gx-shell.lua").count(), 2);
    }

    fn upgrade(f: &Fixture, stamp: &str, list: &[Shipped]) {
        upgrade_config(&f.layout, &"a".repeat(64), stamp, false, list).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_group_is_put_back_and_retried_into_the_same_backup() {
        use std::os::windows::fs::OpenOptionsExt;
        const OLD_HELPER: &str = "return 'old'\n";
        let f = Fixture::new();
        f.user_file("wezterm.lua", "return {}");
        let launch = f.user_file("config/launch.lua", OLD_LAUNCH);
        let helper = f.user_file("utils/gx-shell.lua", OLD_HELPER);
        let list = released(&[
            ("config/launch.lua", OLD_LAUNCH),
            ("utils/gx-shell.lua", OLD_HELPER),
        ]);
        {
            // Readable, but it cannot be replaced while this handle is open.
            let _locked = File::options()
                .read(true)
                .share_mode(1)
                .open(&launch)
                .unwrap();
            for stamp in ["1", "2", "3", "4"] {
                upgrade(&f, stamp, &list);
            }
        }
        assert_eq!(fs::read_to_string(&launch).unwrap(), OLD_LAUNCH);
        assert_eq!(fs::read_to_string(&helper).unwrap(), OLD_HELPER);
        let log = f.log();
        let failures = log.matches("failed config/launch.lua: config/launch.lua: ");
        assert_eq!(failures.count(), MAX_ATTEMPTS as usize, "{log}");
        assert!(
            log.contains("1 failed utils/gx-shell.lua: config/launch.lua: "),
            "{log}"
        );
        assert_eq!(
            fs::read_dir(f.layout.managed.join("backups"))
                .unwrap()
                .count(),
            1
        );
        for dir in ["config", "utils"] {
            assert_eq!(fs::read_dir(f.layout.config.join(dir)).unwrap().count(), 1);
        }
        upgrade(&f, "5", &list);
        assert_eq!(fs::read_to_string(&launch).unwrap(), OLD_LAUNCH);
        upgrade_config(&f.layout, &"b".repeat(64), "6", false, &list).unwrap();
        assert_eq!(fs::read_to_string(&launch).unwrap(), NEW_LAUNCH);
        assert_eq!(fs::read_to_string(&helper).unwrap(), HELPER);
    }

    #[test]
    fn deleted_released_files_stay_deleted_unless_code_needs_them() {
        const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\n\0image";
        let f = Fixture::new();
        f.payload_file("backdrops/old.png", IMAGE);
        f.payload_file("backdrops/new.png", IMAGE);
        f.user_file("wezterm.lua", "return {}");
        f.user_file("config/launch.lua", OLD_LAUNCH);
        let list = released(&[
            ("config/launch.lua", OLD_LAUNCH),
            ("backdrops/old.png", "a wallpaper the user deleted"),
            ("utils/gx-shell.lua", "return 'old'\n"),
        ]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert!(!f.layout.config.join("backdrops/old.png").exists());
        assert_eq!(
            fs::read(f.layout.config.join("backdrops/new.png")).unwrap(),
            IMAGE
        );
        assert_eq!(f.read("config/launch.lua"), NEW_LAUNCH);
        assert_eq!(f.read("utils/gx-shell.lua"), HELPER);
    }

    #[test]
    fn read_only_file_is_kept_until_it_is_writable() {
        const ENTRY: &str = "require('config.appearance')\n";
        let f = Fixture::new();
        let old_fonts = "return 1\n";
        let old_appearance = "require('config.fonts')\nreturn 1\n";
        let new_appearance = "require('config.fonts')\nreturn 2\n";
        f.payload_file("wezterm.lua", ENTRY);
        f.payload_file("config/fonts.lua", "return 2\n");
        f.payload_file("config/appearance.lua", new_appearance);
        f.user_file("wezterm.lua", ENTRY);
        f.user_file("config/launch.lua", NEW_LAUNCH);
        f.user_file("utils/gx-shell.lua", HELPER);
        let fonts = f.user_file("config/fonts.lua", old_fonts);
        f.user_file("config/appearance.lua", old_appearance);
        let mut permissions = fs::metadata(&fonts).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&fonts, permissions.clone()).unwrap();
        let list = released(&[
            ("config/fonts.lua", old_fonts),
            ("config/appearance.lua", old_appearance),
        ]);
        upgrade(&f, "1", &list);
        assert_eq!(f.read("config/fonts.lua"), old_fonts);
        assert_eq!(f.read("config/appearance.lua"), old_appearance);
        let log = f.log();
        let why = "held back because config/fonts.lua is read-only";
        assert!(log.contains("1 kept config/fonts.lua: read-only"), "{log}");
        assert!(
            log.contains(&format!("1 skipped config/appearance.lua: {why}")),
            "{log}"
        );
        upgrade(&f, "2", &list);
        assert_eq!(f.log(), log);
        permissions.set_readonly(false);
        fs::set_permissions(&fonts, permissions).unwrap();
        upgrade(&f, "3", &list);
        assert_eq!(f.read("config/fonts.lua"), "return 2\n");
        assert_eq!(f.read("config/appearance.lua"), new_appearance);
    }

    #[test]
    fn editing_a_file_that_held_others_back_reruns_the_upgrade() {
        const ENTRY: &str = "require('config.bindings')\n";
        let f = Fixture::new();
        let old_plugins = "return { resurrect = {} }\n";
        let new_plugins = "return { resurrect = function() end }\n";
        let old_bindings = "require('config.plugins')\nreturn 1\n";
        let new_bindings = "require('config.plugins')\nreturn 2\n";
        f.payload_file("wezterm.lua", ENTRY);
        f.payload_file("config/plugins.lua", new_plugins);
        f.payload_file("config/bindings.lua", new_bindings);
        f.user_file("wezterm.lua", ENTRY);
        f.user_file("config/launch.lua", NEW_LAUNCH);
        f.user_file("utils/gx-shell.lua", HELPER);
        f.user_file("config/plugins.lua", old_plugins);
        f.user_file("config/bindings.lua", format!("{old_bindings}-- mine\n"));
        let list = released(&[
            ("config/plugins.lua", old_plugins),
            ("config/bindings.lua", old_bindings),
        ]);
        upgrade(&f, "1", &list);
        assert_eq!(f.read("config/plugins.lua"), old_plugins);
        let marker = fs::read_to_string(f.layout.managed.join("config-version")).unwrap();
        assert!(marker.contains(" config/bindings.lua\n"), "{marker}");
        let log = f.log();
        upgrade(&f, "2", &list);
        assert_eq!(f.log(), log);
        // Merged by hand: the user took the shipped bindings.lua.
        f.user_file("config/bindings.lua", new_bindings);
        upgrade(&f, "3", &list);
        assert_eq!(f.read("config/plugins.lua"), new_plugins);
    }

    #[test]
    fn a_new_fingerprint_table_reruns_the_upgrade() {
        let f = Fixture::new();
        f.user_file("config/launch.lua", OLD_LAUNCH);
        let unknown = released(&[]);
        upgrade(&f, "1", &unknown);
        upgrade(&f, "2", &unknown);
        assert_eq!(f.read("config/launch.lua"), OLD_LAUNCH);
        assert_eq!(f.log().matches("kept config/launch.lua").count(), 1);
        upgrade(&f, "3", &released(&[("config/launch.lua", OLD_LAUNCH)]));
        assert_eq!(f.read("config/launch.lua"), NEW_LAUNCH);
    }

    #[test]
    fn edits_to_files_shipped_unchanged_hold_nothing_back() {
        const ENTRY: &str = "require('config.launch')\nrequire('config.appearance')\n";
        let f = Fixture::new();
        let colors = "return { red = 1 }\n";
        let old_appearance = "require('colors.custom')\nreturn 1\n";
        let new_appearance = "require('colors.custom')\nreturn 2\n";
        f.payload_file("wezterm.lua", ENTRY);
        f.payload_file("colors/custom.lua", colors);
        f.payload_file("config/appearance.lua", new_appearance);
        f.user_file("wezterm.lua", format!("{ENTRY}-- date_format = '%H:%M'\n"));
        f.user_file("colors/custom.lua", "return { red = 2 }\n");
        f.user_file("config/appearance.lua", old_appearance);
        f.user_file("config/launch.lua", OLD_LAUNCH);
        let mut list = released(&[
            ("config/appearance.lua", old_appearance),
            ("config/launch.lua", OLD_LAUNCH),
        ]);
        for (path, text) in [("wezterm.lua", ENTRY), ("colors/custom.lua", colors)] {
            let (len, hash) = fingerprint(text.as_bytes());
            list.push((path, len, hash, 0b11));
        }
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(f.read("config/launch.lua"), NEW_LAUNCH);
        assert_eq!(f.read("utils/gx-shell.lua"), HELPER);
        assert_eq!(f.read("config/appearance.lua"), new_appearance);
        assert_eq!(f.read("colors/custom.lua"), "return { red = 2 }\n");
        assert!(!f.log().contains("skipped"), "{}", f.log());
    }

    #[test]
    fn seeds_a_config_directory_that_only_holds_gui_settings() {
        let f = Fixture::new();
        let settings = f.user_file("gui-settings.json", r#"{"font_size":12}"#);
        initialize(&f.layout).unwrap();
        assert_eq!(f.read("config/launch.lua"), NEW_LAUNCH);
        assert_eq!(fs::read_to_string(settings).unwrap(), r#"{"font_size":12}"#);
        let parent = f.layout.config.parent().unwrap();
        assert_eq!(fs::read_dir(parent).unwrap().count(), 1);
    }

    #[test]
    fn a_file_that_appears_during_the_upgrade_is_not_replaced() {
        let f = Fixture::new();
        let temp = f.user_file(".wezterm-gx-1-0", "shipped");
        let target = f.user_file("config/fonts.lua", "user");
        assert!(place_new(&temp, &target).is_err());
        assert_eq!(f.read("config/fonts.lua"), "user");
        place_new(&temp, &f.layout.config.join("config/general.lua")).unwrap();
        assert_eq!(f.read("config/general.lua"), "shipped");
        assert!(!temp.exists());
    }

    #[cfg(unix)]
    #[test]
    fn replacement_keeps_the_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let launch = f.user_file("config/launch.lua", OLD_LAUNCH);
        fs::set_permissions(&launch, fs::Permissions::from_mode(0o640)).unwrap();
        let list = released(&[("config/launch.lua", OLD_LAUNCH)]);
        assert!(migrate(&f, "1", &list).failed.is_empty());
        assert_eq!(f.read("config/launch.lua"), NEW_LAUNCH);
        let mode = fs::metadata(&launch).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o640);
    }

    // Re-runs this test binary as launcher and child inside a console of their own.
    #[cfg(windows)]
    #[test]
    fn cli_launcher_outlives_ctrl_c_and_returns_the_child_exit_code() {
        use std::os::windows::process::CommandExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;
        #[link(name = "kernel32")]
        extern "system" {
            fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32;
            fn SetConsoleCtrlHandler(
                handler: Option<unsafe extern "system" fn(u32) -> i32>,
                add: i32,
            ) -> i32;
        }
        static CTRL_C: AtomicBool = AtomicBool::new(false);
        unsafe extern "system" fn record(event: u32) -> i32 {
            CTRL_C.store(event == 0, Ordering::SeqCst);
            1
        }
        let rerun = |role: &str| {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "tests::cli_launcher_outlives_ctrl_c_and_returns_the_child_exit_code",
                    "--exact",
                ])
                .env("GX_TEST_ROLE", role);
            command
        };
        match std::env::var("GX_TEST_ROLE").as_deref() {
            Ok("child") => {
                unsafe {
                    SetConsoleCtrlHandler(Some(record), 1);
                    GenerateConsoleCtrlEvent(0, 0);
                }
                for _ in 0..100 {
                    if CTRL_C.load(Ordering::SeqCst) {
                        // Leave time for Ctrl+C to end a launcher that does not survive it.
                        std::thread::sleep(Duration::from_millis(500));
                        std::process::exit(42);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                std::process::exit(3);
            }
            Ok("launcher") => {
                // As if started by a parent that ignores Ctrl+C.
                unsafe {
                    SetConsoleCtrlHandler(None, 1);
                }
                os::restore_ctrl_c();
                std::process::exit(os::wait(rerun("child")).unwrap());
            }
            _ => {
                // CREATE_NO_WINDOW: a hidden console, so the test runner never sees the Ctrl+C.
                let status = rerun("launcher")
                    .creation_flags(0x0800_0000)
                    .status()
                    .unwrap();
                assert_eq!(status.code(), Some(42));
            }
        }
    }
}
