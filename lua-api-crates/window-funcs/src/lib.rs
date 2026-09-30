use config::lua::get_or_create_sub_module;
use config::lua::mlua::{self, Lua};
use luahelper::impl_lua_conversion_dynamic;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;
use wezterm_dynamic::{FromDynamic, ToDynamic};
use window::{Appearance, Connection, ConnectionOps};

fn get_conn() -> mlua::Result<Rc<Connection>> {
    Connection::get().ok_or_else(|| {
        mlua::Error::external("cannot get window Connection: not running on the gui thread?")
    })
}

// fork: the config is also evaluated off the GUI thread (the config file
// watcher reloads on its own thread) and windows without overrides use
// that result as is. There is no Connection there, so these report what
// the GUI thread last saw instead of assuming Light / failing.
static LAST_KNOWN_APPEARANCE: Mutex<Option<Appearance>> = Mutex::new(None);
static LAST_KNOWN_SCREENS: Mutex<Option<Screens>> = Mutex::new(None);

/// fork: record the appearance reported by the windowing environment
/// (set by the GUI when it changes, and on every GUI-thread query).
pub fn set_last_known_appearance(appearance: Appearance) {
    if let Ok(mut last) = LAST_KNOWN_APPEARANCE.lock() {
        *last = Some(appearance);
    }
}

fn last_known_appearance() -> Option<Appearance> {
    LAST_KNOWN_APPEARANCE.lock().ok().and_then(|last| *last)
}

fn remember_screens(screens: &Screens) {
    if let Ok(mut last) = LAST_KNOWN_SCREENS.lock() {
        *last = Some(screens.clone());
    }
}

fn last_known_screens() -> Option<Screens> {
    LAST_KNOWN_SCREENS.lock().ok().and_then(|last| last.clone())
}

#[derive(Debug, Clone, FromDynamic, ToDynamic)]
pub struct ScreenInfo {
    pub name: String,
    pub x: isize,
    pub y: isize,
    pub width: isize,
    pub height: isize,
    pub scale: f64,
    pub max_fps: Option<usize>,
    pub effective_dpi: Option<f64>,
}
impl_lua_conversion_dynamic!(ScreenInfo);

#[derive(Debug, Clone, FromDynamic, ToDynamic)]
pub struct Screens {
    pub main: ScreenInfo,
    pub active: ScreenInfo,
    pub by_name: HashMap<String, ScreenInfo>,
    pub origin_x: isize,
    pub origin_y: isize,
    pub virtual_width: isize,
    pub virtual_height: isize,
}
impl_lua_conversion_dynamic!(Screens);

impl From<window::screen::ScreenInfo> for ScreenInfo {
    fn from(info: window::screen::ScreenInfo) -> Self {
        Self {
            name: info.name,
            x: info.rect.min_x(),
            y: info.rect.min_y(),
            width: info.rect.width(),
            height: info.rect.height(),
            scale: info.scale,
            max_fps: info.max_fps,
            effective_dpi: info.effective_dpi,
        }
    }
}

impl From<window::screen::Screens> for Screens {
    fn from(screens: window::screen::Screens) -> Self {
        let origin_x = screens.virtual_rect.min_x();
        let origin_y = screens.virtual_rect.min_y();
        let virtual_width = screens.virtual_rect.width();
        let virtual_height = screens.virtual_rect.height();
        Self {
            main: screens.main.into(),
            active: screens.active.into(),
            by_name: screens
                .by_name
                .into_iter()
                .map(|(k, info)| (k, info.into()))
                .collect(),
            origin_x,
            origin_y,
            virtual_width,
            virtual_height,
        }
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let window_mod = get_or_create_sub_module(lua, "gui")?;

    window_mod.set(
        "screens",
        lua.create_function(|_, _: ()| {
            let conn = match get_conn() {
                Ok(conn) => conn,
                Err(err) => return last_known_screens().ok_or(err),
            };
            let screens: Screens = conn
                .screens()
                .map_err(|err| mlua::Error::external(format!("{err:#}")))?
                .into();
            remember_screens(&screens);
            Ok(screens)
        })?,
    )?;

    window_mod.set(
        "get_appearance",
        lua.create_function(|_, _: ()| {
            Ok(match Connection::get() {
                Some(conn) => {
                    let appearance = conn.get_appearance();
                    set_last_known_appearance(appearance);
                    appearance.to_string()
                }
                None => {
                    // fork: off the GUI thread report what the GUI last
                    // saw; if the gui hasn't started yet, assume light
                    last_known_appearance()
                        .unwrap_or(Appearance::Light)
                        .to_string()
                }
            })
        })?,
    )?;

    Ok(())
}

// fork: test threads have no Connection, like the config file watcher
#[cfg(test)]
mod tests {
    use super::*;

    fn eval<T: for<'lua> mlua::FromLua<'lua>>(lua: &Lua, expr: &str) -> mlua::Result<T> {
        let code = format!("return package.loaded.wezterm.gui.{expr}");
        lua.load(code.as_str()).eval()
    }

    #[test]
    fn appearance_off_the_gui_thread_is_the_last_one_the_gui_saw() {
        let lua = Lua::new();
        register(&lua).unwrap();
        assert_eq!(eval::<String>(&lua, "get_appearance()").unwrap(), "Light");
        set_last_known_appearance(Appearance::Dark);
        assert_eq!(eval::<String>(&lua, "get_appearance()").unwrap(), "Dark");
    }

    #[test]
    fn screens_off_the_gui_thread_are_the_last_ones_the_gui_saw() {
        let lua = Lua::new();
        register(&lua).unwrap();
        assert!(eval::<String>(&lua, "screens().main.name").is_err());
        let info = ScreenInfo {
            name: "main".to_string(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
            max_fps: None,
            effective_dpi: None,
        };
        remember_screens(&Screens {
            main: info.clone(),
            active: info.clone(),
            by_name: HashMap::from([("main".to_string(), info)]),
            origin_x: 0,
            origin_y: 0,
            virtual_width: 1920,
            virtual_height: 1080,
        });
        assert_eq!(eval::<String>(&lua, "screens().main.name").unwrap(), "main");
    }
}
