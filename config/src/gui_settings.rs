//! fork 新增：GUI 设置页的持久化文件 `gui-settings.json`。
//!
//! 位置：与实际生效的 wezterm.lua 同目录（该目录为 HOME 时退回 XDG
//! 配置目录，避免在 HOME 根部落文件）。
//! 加载优先级：wezterm.lua < gui-settings.json < `--config` CLI 覆盖 <
//! 每窗口 `set_config_overrides`（运行时预览）。
//! 该文件由 GUI 设置页独占写入（临时文件 + rename 原子换入），
//! 绝不反写用户的 Lua 配置。设置页应用 = 写文件 + 显式 `config::reload()`，
//! 不依赖目录 watcher 的触发。

use crate::i18n::UiLanguage;
use crate::{json_to_dynamic, xdg_config_home, HOME_DIR};
use mlua::Lua;
use std::path::{Path, PathBuf};
use wezterm_dynamic::Value;

pub const GUI_SETTINGS_FILE: &str = "gui-settings.json";

/// Where the GUI settings file lives: next to the effective wezterm.lua
/// when that directory is known and isn't $HOME, otherwise the XDG
/// wezterm config dir.
pub fn settings_path() -> PathBuf {
    match std::env::var_os("WEZTERM_CONFIG_DIR").map(PathBuf::from) {
        Some(dir) if dir != *HOME_DIR => dir.join(GUI_SETTINGS_FILE),
        _ => xdg_config_home().join(GUI_SETTINGS_FILE),
    }
}

fn settings_file_in_dir(dir: Option<&Path>) -> PathBuf {
    match dir {
        Some(dir) if dir != HOME_DIR.as_path() => dir.join(GUI_SETTINGS_FILE),
        _ => settings_path(),
    }
}

fn parse_file(path: &Path) -> Option<serde_json::Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            log::warn!("gui-settings: cannot read {}: {err}", path.display());
            return None;
        }
    };
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(err) => {
            // A corrupt GUI-owned file must not take the whole config down;
            // fall back to defaults and let the next settings write repair it.
            log::warn!("gui-settings: ignoring malformed {}: {err}", path.display());
            None
        }
    }
}

/// Load the settings sidecar next to `dir` (the config file's directory)
/// as a dynamic Value; `Value::Null` when absent or unreadable.
pub fn load_value_in_dir(dir: Option<&Path>) -> Value {
    parse_file(&settings_file_in_dir(dir))
        .map(|json| json_to_dynamic(&json))
        .unwrap_or(Value::Null)
}

/// Apply the settings sidecar onto a Lua config value. Unlike
/// `Config::apply_overrides_obj_to`, an invalid key logs a warning and is
/// skipped instead of failing the whole config load: this file is owned
/// by the GUI, so a stale key (e.g. after a downgrade) must not break
/// the user's session.
pub fn apply_to_lua<'l>(
    lua: &'l Lua,
    mut config: mlua::Value<'l>,
) -> anyhow::Result<mlua::Value<'l>> {
    let sidecar = load_value_in_dir(None);
    let obj = match &sidecar {
        Value::Object(obj) => obj,
        _ => return Ok(config),
    };
    if obj.is_empty() {
        return Ok(config);
    }

    let setter: mlua::Function = lua
        .load(
            r#"
                    -- pcall so that an invalid key/value pair is reported
                    -- back as a value instead of consuming the config table
                    return function(config, key, value)
                        local ok, err = pcall(function()
                            config[key] = value
                        end)
                        if ok then
                            return config, nil
                        end
                        return config, tostring(err)
                    end
                    "#,
        )
        .eval()?;

    for (key, value) in obj {
        let lkey = match luahelper::dynamic_to_lua_value(lua, key.clone()) {
            Ok(v) => v,
            Err(err) => {
                log::warn!("gui-settings: skip key {key:?}: {err:#}");
                continue;
            }
        };
        let lvalue = match luahelper::dynamic_to_lua_value(lua, value.clone()) {
            Ok(v) => v,
            Err(err) => {
                log::warn!("gui-settings: skip value for {key:?}: {err:#}");
                continue;
            }
        };
        let result: anyhow::Result<(mlua::Value, Option<String>)> = setter
            .call((config.clone(), lkey, lvalue))
            .map_err(|e| anyhow::Error::msg(e.to_string()));
        match result {
            Ok((next, err)) => {
                if let Some(err) = err {
                    log::warn!("gui-settings: ignoring invalid key {key:?}: {err}");
                }
                config = next;
            }
            Err(err) => log::warn!("gui-settings: failed to apply key {key:?}: {err:#}"),
        }
    }
    Ok(config)
}

fn dynamic_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::I64(i) => (*i).into(),
        Value::U64(u) => (*u).into(),
        Value::F64(f) => serde_json::Number::from_f64(f.into_inner())
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Array(a) => serde_json::Value::Array(a.iter().map(dynamic_to_json).collect()),
        Value::Object(o) => serde_json::Value::Object(
            o.iter()
                .map(|(k, v)| {
                    let key = match k {
                        Value::String(s) => s.clone(),
                        other => format!("{other:?}"),
                    };
                    (key, dynamic_to_json(v))
                })
                .collect(),
        ),
    }
}

/// Upsert a single settings key, preserving other keys, then atomically
/// replace the file (write to a sibling temp file + rename).
pub fn store_key(key: &str, value: &Value) -> anyhow::Result<()> {
    let path = settings_path();
    let mut root = parse_file(&path).unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        log::warn!(
            "gui-settings: {} is not a JSON object; rewriting it",
            path.display()
        );
        root = serde_json::json!({});
    }
    root.as_object_mut()
        .expect("root is an object")
        .insert(key.to_string(), dynamic_to_json(value));

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&root)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Cheap language probe for early CLI startup: read the sidecar's
/// top-level `language` without executing any Lua.
pub fn peek_language() -> Option<UiLanguage> {
    let root = parse_file(&settings_path())?;
    root.get("language")?.as_str().and_then(UiLanguage::parse)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "wezterm-gui-settings-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn store_and_load_roundtrip() {
        let dir = temp_dir();
        store_key_in_dir(&dir, "language", &Value::String("en".into())).unwrap();
        store_key_in_dir(
            &dir,
            "font_size",
            &Value::F64(ordered_float::OrderedFloat(13.5)),
        )
        .unwrap();

        let loaded = load_value_in_dir(Some(&dir));
        let obj = match &loaded {
            Value::Object(obj) => obj,
            other => panic!("expected object, got {other:?}"),
        };
        assert_eq!(
            obj.get_by_str("language"),
            Some(&Value::String("en".into()))
        );
        assert!(matches!(obj.get_by_str("font_size"), Some(Value::F64(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_file_is_ignored() {
        let dir = temp_dir();
        std::fs::write(dir.join(GUI_SETTINGS_FILE), "{not json").unwrap();
        assert!(matches!(load_value_in_dir(Some(&dir)), Value::Null));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_null() {
        let dir = temp_dir();
        assert!(matches!(load_value_in_dir(Some(&dir)), Value::Null));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The real store_key targets the ambient settings_path(); redirect it
    // via WEZTERM_CONFIG_DIR for the roundtrip test.
    fn store_key_in_dir(dir: &Path, key: &str, value: &Value) -> anyhow::Result<()> {
        // nextest runs each test in its own process, so env mutation is safe
        std::env::set_var("WEZTERM_CONFIG_DIR", dir);
        let result = store_key(key, value);
        std::env::remove_var("WEZTERM_CONFIG_DIR");
        result
    }
}
