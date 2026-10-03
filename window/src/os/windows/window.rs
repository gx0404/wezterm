use super::*;
use crate::connection::ConnectionOps;
use crate::parameters::{self, Parameters};
use crate::{
    Appearance, Clipboard, ClipboardImage, ClipboardImageFormat, CursorIcon, DeadKeyStatus,
    Dimensions, Handled, KeyCode, KeyEvent, Modifiers, MouseButtons, MouseEvent, MouseEventKind,
    MousePress, Point, ProgressState, RawKeyEvent, Rect, RequestedWindowGeometry, ResolvedGeometry,
    ScreenPoint, ScreenRect, ULength, WindowDecorations, WindowEvent, WindowEventSender, WindowOps,
    WindowState,
};
use anyhow::{bail, Context};
use async_trait::async_trait;
use config::{ConfigHandle, ImePreeditRendering, SrgbaTuple, SystemBackdrop};
use lazy_static::lazy_static;
use promise::{Future, Promise};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, Win32WindowHandle, WindowHandle, WindowsDisplayHandle,
};
use shared_library::shared_library;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::TryInto;
use std::ffi::OsString;
use std::io::Error as IoError;
use std::num::NonZeroIsize;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Instant;
use wezterm_color_types::LinearRgba;
use wezterm_font::FontConfiguration;
use wezterm_input_types::KeyboardLedStatus;
use winapi::shared::minwindef::*;
use winapi::shared::ntdef::*;
use winapi::shared::windef::*;
use winapi::shared::winerror::S_OK;
use winapi::um::imm::*;
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::shellapi::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use winapi::um::shellscalingapi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use winapi::um::sysinfoapi::{GetTickCount, GetVersionExW};
use winapi::um::uxtheme::{
    CloseThemeData, GetThemeFont, GetThemeSysFont, OpenThemeData, SetWindowTheme,
};
use winapi::um::wingdi::{
    AlphaBlend, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    LOGFONTW, MAKEPOINTS,
};
use winapi::um::winnt::OSVERSIONINFOW;
use winapi::um::winuser::*;
use windows::UI::Color as WUIColor;
use windows::UI::ViewManagement::{UIColorType, UISettings};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const GCS_RESULTSTR: DWORD = 0x800;
const GCS_COMPSTR: DWORD = 0x8;
const ISC_SHOWUICOMPOSITIONWINDOW: DWORD = 0x80000000;

#[allow(non_snake_case)]
#[repr(C)]
pub struct CANDIDATEFORM {
    dwIndex: DWORD,
    dwStyle: DWORD,
    ptCurrentPos: POINT,
    rcArea: RECT,
}
pub type LPCANDIDATEFORM = *mut CANDIDATEFORM;

extern "system" {
    pub fn ImmGetCompositionStringW(himc: HIMC, index: DWORD, buf: LPVOID, buflen: DWORD) -> LONG;
    pub fn ImmSetCandidateWindow(himc: HIMC, lpCandidate: LPCANDIDATEFORM) -> BOOL;
}

lazy_static! {
    static ref IS_WIN10: bool = {
        let osver = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as _,
            ..Default::default()
        };

        if unsafe { GetVersionExW(&osver as *const _ as _) } == winapi::shared::minwindef::TRUE {
            osver.dwBuildNumber < 22000
        } else {
            true
        }
    };
    static ref IS_WIN11_22H2: bool = {
        let osver = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as _,
            ..Default::default()
        };

        if unsafe { GetVersionExW(&osver as *const _ as _) } == winapi::shared::minwindef::TRUE {
            osver.dwBuildNumber >= 22621
        } else {
            true
        }
    };
    static ref TITLE_FONT: Mutex<TitleFontCache> = Mutex::new(TitleFontCache {
        font: None,
        stale: true,
    });
}

/// fork: the system caption font, resolved lazily. Resolving it copies and
/// parses the whole font file, and WM_SETTINGCHANGE (which every top-level
/// window receives, often in bursts) used to do that each time even though
/// nothing consumed the result. Settings changes now only mark it stale and
/// get_os_parameters re-resolves it on demand.
struct TitleFontCache {
    font: Option<parameters::FontAndSize>,
    stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub(crate) struct HWindow(HWND);
unsafe impl Send for HWindow {}
unsafe impl Sync for HWindow {}

/// fork: 帧定时器到期扫描时单个窗口的状态（见 WindowInner::frame_timer_tick）
pub(crate) enum FrameTick {
    /// 未处于高精度定时器节流中
    Idle,
    /// 仍在节流，deadline 未到
    Pending(Instant),
    /// 节流已结束；invalidate 为真表示期间有被吞掉的 WM_PAINT 需补发
    Expired { invalidate: bool },
}

pub(crate) struct WindowInner {
    /// Non-owning reference to the window handle
    hwnd: HWindow,
    events: WindowEventSender,
    gl_state: Option<Rc<glium::backend::Context>>,
    /// Fraction of mouse scroll
    hscroll_remainder: i32,
    vscroll_remainder: i32,

    last_size: Option<Dimensions>,
    in_size_move: bool,
    dead_pending: Option<(Modifiers, u32)>,
    saved_placement: Option<WINDOWPLACEMENT>,
    track_mouse_leave: bool,
    window_drag_position: Option<ScreenPoint>,
    maximize_button_position: Option<ScreenRect>,

    keyboard_info: KeyboardLayoutInfo,
    appearance: Appearance,

    config: ConfigHandle,
    paint_throttled: bool,
    invalidated: bool,
    /// fork: 窗口当前所在显示器及其刷新率，供 max_fps_follows_display
    /// 决定帧间隔；在创建后、WM_DISPLAYCHANGE 与跨显示器移动时刷新
    monitor: HMONITOR,
    monitor_refresh_hz: Option<u32>,
    /// fork: 最近一次 wm_paint 的开始时刻；节流 deadline 从这里起算，
    /// paint 本身的耗时不再叠加到帧间隔上
    frame_start: Instant,
    /// 高精度帧定时器路径下本窗口的节流到期时刻；async_io 回退路径为 None
    next_paint_deadline: Option<Instant>,
    /// 回退路径的代数计数，让过期的 async_io 回调不会提前结束新一轮节流
    throttle_generation: u64,
    /// fork: 首帧 present 之前不执行 ShowWindow，避免 DWM 合成未初始化的
    /// 表面产生白帧；挂起的 show 命令存在 pending_show，兜底定时器由
    /// show_fallback_armed 保证只装一次
    first_frame_presented: bool,
    pending_show: Option<ShowWindowCommand>,
    show_fallback_armed: bool,
    /// fork: the last text cursor rect the GUI reported for IME placement.
    /// The GUI only resends the rect when it changes, so it is replayed
    /// after a resize, on focus and when a composition starts; otherwise
    /// the candidate window could stay at (0,0) or at another window's spot
    /// (all windows of the thread share the default input context).
    last_ime_rect: Option<Rect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct Window(HWindow);

fn wuicolor_to_linearrgba(color: WUIColor) -> LinearRgba {
    LinearRgba::with_srgba(color.R, color.G, color.B, 255)
}

fn rect_width(r: &RECT) -> i32 {
    r.right - r.left
}

fn rect_height(r: &RECT) -> i32 {
    r.bottom - r.top
}

fn adjust_client_to_window_dimensions(
    style: u32,
    width: usize,
    height: usize,
    dpi: u32,
) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width as _,
        bottom: height as _,
    };
    unsafe { AdjustWindowRectExForDpi(&mut rect, style, 0, 0, dpi) };

    (rect_width(&rect), rect_height(&rect))
}

fn rc_to_pointer(arc: &Rc<RefCell<WindowInner>>) -> *const RefCell<WindowInner> {
    let cloned = Rc::clone(arc);
    Rc::into_raw(cloned)
}

fn rc_from_pointer(lparam: LPVOID) -> Rc<RefCell<WindowInner>> {
    // Turn it into an Rc
    let arc = unsafe { Rc::from_raw(std::mem::transmute(lparam)) };
    // Add a ref for the caller
    let cloned = Rc::clone(&arc);

    // We must not drop this ref though; turn it back into a raw pointer!
    let _ = Rc::into_raw(arc);

    cloned
}

fn rc_from_hwnd(hwnd: HWND) -> Option<Rc<RefCell<WindowInner>>> {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as LPVOID };
    if raw.is_null() {
        None
    } else {
        Some(rc_from_pointer(raw))
    }
}

fn take_rc_from_pointer(lparam: LPVOID) -> Rc<RefCell<WindowInner>> {
    unsafe { Rc::from_raw(std::mem::transmute(lparam)) }
}

fn callback_behavior() -> glium::debug::DebugCallbackBehavior {
    if cfg!(debug_assertions) && false
    /* https://github.com/glium/glium/issues/1885 */
    {
        glium::debug::DebugCallbackBehavior::DebugMessageOnError
    } else {
        glium::debug::DebugCallbackBehavior::Ignore
    }
}

impl HasDisplayHandle for WindowInner {
    fn display_handle(&self) -> Result<DisplayHandle, HandleError> {
        unsafe {
            Ok(DisplayHandle::borrow_raw(RawDisplayHandle::Windows(
                WindowsDisplayHandle::new(),
            )))
        }
    }
}

impl HasWindowHandle for WindowInner {
    fn window_handle(&self) -> Result<WindowHandle, HandleError> {
        let mut handle =
            Win32WindowHandle::new(NonZeroIsize::new(self.hwnd.0 as _).expect("non-zero"));
        handle.hinstance = NonZeroIsize::new(unsafe { GetModuleHandleW(null()) } as _);
        unsafe { Ok(WindowHandle::borrow_raw(RawWindowHandle::Win32(handle))) }
    }
}

impl WindowInner {
    fn enable_opengl(&mut self) -> anyhow::Result<Rc<glium::backend::Context>> {
        let conn = Connection::get().unwrap();

        // fork: the GUI calls this again to recover from a lost context
        // (gpu_recovery.rs) after dropping its own references. Release ours
        // first: the EGL surface and the WGL pixel format are bound to this
        // HWND, and ANGLE only restores a lost device once every old
        // context is gone.
        self.gl_state.take();

        let gl_state = if self.config.prefer_egl {
            match conn.gl_connection.borrow().as_ref() {
                None => crate::egl::GlState::create(None, self.hwnd.0),
                Some(glconn) => {
                    crate::egl::GlState::create_with_existing_connection(glconn, self.hwnd.0)
                }
            }
        } else {
            Err(anyhow::anyhow!("Config says to avoid EGL"))
        }
        .and_then(|egl| unsafe {
            log::trace!("Initialized EGL!");
            conn.gl_connection
                .borrow_mut()
                .replace(Rc::clone(egl.get_connection()));
            let backend = Rc::new(egl);
            Ok(glium::backend::Context::new(
                backend,
                true,
                callback_behavior(),
            )?)
        })
        .or_else(|err| {
            log::trace!("EGL init failed {:?}, fall back to WGL", err);
            super::wgl::GlState::create(self.hwnd.0).and_then(|state| unsafe {
                Ok(glium::backend::Context::new(
                    Rc::new(state),
                    true,
                    callback_behavior(),
                )?)
            })
        })?;

        self.gl_state.replace(gl_state.clone());

        Ok(gl_state)
    }

    fn get_effective_dpi(&self) -> usize {
        let actual_dpi = unsafe { GetDpiForWindow(self.hwnd.0) } as f64;

        if self.config.dpi_by_screen.is_empty() {
            return self.config.dpi.unwrap_or(actual_dpi) as usize;
        }

        unsafe {
            let mut mi: MONITORINFOEXW = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            let mon = MonitorFromWindow(self.hwnd.0, MONITOR_DEFAULTTONEAREST);
            GetMonitorInfoW(mon, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO);

            if let Ok(info) = crate::os::windows::connection::ScreenInfoHelper::new() {
                let name = info.monitor_name(&mi);
                if let Some(dpi) = self.config.dpi_by_screen.get(&name).copied() {
                    return dpi as usize;
                }
            }

            actual_dpi as usize
        }
    }

    /// fork: 记录窗口所在显示器与刷新率。force 为 false 时仅在显示器句柄
    /// 变化（跨显示器移动）后重新读取；WM_DISPLAYCHANGE 等模式变化须 force
    fn refresh_monitor_info(&mut self, force: bool) {
        let mon = unsafe { MonitorFromWindow(self.hwnd.0, MONITOR_DEFAULTTONEAREST) };
        if !force && mon == self.monitor {
            return;
        }
        self.monitor = mon;
        self.monitor_refresh_hz = super::connection::monitor_refresh_rate(mon);
        log::trace!(
            "window {:?} now on monitor {:?} refresh={:?}Hz",
            self.hwnd,
            mon,
            self.monitor_refresh_hz
        );
    }

    /// 本窗口当前生效的最小帧间隔
    fn frame_interval(&self) -> std::time::Duration {
        effective_frame_interval(
            self.config.max_fps,
            self.config.max_fps_follows_display,
            self.monitor_refresh_hz,
        )
    }

    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd.0
    }

    /// fork: 把本帧的节流 deadline（frame_start + 帧间隔）交给 Connection 的
    /// 高精度帧定时器；没有高精度定时器（Win10 < 1803）时回退 async_io
    /// 定时器，语义相同只是精度受系统节拍限制
    fn schedule_paint_throttle(&mut self) {
        let deadline = self.frame_start + self.frame_interval();
        let conn = Connection::get().expect("Connection::init has not been called");
        if conn.request_frame_deadline(deadline) {
            self.next_paint_deadline = Some(deadline);
            return;
        }

        self.next_paint_deadline = None;
        self.throttle_generation = self.throttle_generation.wrapping_add(1);
        let generation = self.throttle_generation;
        let window_id = self.hwnd;
        promise::spawn::spawn(async move {
            async_io::Timer::at(deadline).await;
            Connection::with_window_inner(window_id, move |inner| {
                if inner.throttle_generation != generation {
                    // 期间已重新调度（配置重载/新一帧），这次回调作废
                    return Ok(());
                }
                inner.paint_throttled = false;
                if inner.invalidated {
                    unsafe {
                        InvalidateRect(inner.hwnd.0, null(), 0);
                    }
                }
                Ok(())
            });
        })
        .detach();
    }

    /// fork: 帧定时器到期时由 Connection 在主线程调用。`cutoff` 之前到期的
    /// 窗口结束节流并报告是否需要补发重绘；未到期的交回 deadline 供重设
    pub(crate) fn frame_timer_tick(&mut self, cutoff: Instant) -> FrameTick {
        match self.next_paint_deadline {
            None => FrameTick::Idle,
            Some(deadline) if deadline > cutoff => FrameTick::Pending(deadline),
            Some(_) => {
                self.next_paint_deadline = None;
                self.paint_throttled = false;
                FrameTick::Expired {
                    invalidate: self.invalidated,
                }
            }
        }
    }

    /// Check if we need to generate a resize callback.
    /// Calls resize if needed.
    /// Returns true if we did.
    fn check_and_call_resize_if_needed(&mut self) -> bool {
        // 窗口移动/尺寸变化都经此处，顺带检测是否换了显示器
        self.refresh_monitor_info(false);

        /*
        if self.gl_state.is_none() {
            // Don't cache state or generate resize callbacks until
            // we've set up opengl, otherwise we can miss propagating
            // some state during the initial window setup that results
            // in the window dimensions being out of sync with the dpi
            // when eg: the system display settings are set to 200%
            // scale factor.
            return false;
        }
        */

        let mut rect = RECT {
            left: 0,
            bottom: 0,
            right: 0,
            top: 0,
        };
        unsafe {
            GetClientRect(self.hwnd.0, &mut rect);
        }
        let pixel_width = rect_width(&rect) as usize;
        let pixel_height = rect_height(&rect) as usize;

        let current_dims = Dimensions {
            pixel_width,
            pixel_height,
            dpi: self.get_effective_dpi(),
        };

        let same = self
            .last_size
            .as_ref()
            .map(|&dims| dims == current_dims)
            .unwrap_or(false);
        self.last_size.replace(current_dims);

        if !same {
            // fork: replay the last known rect instead of resetting to (0,0);
            // the GUI does not resend an unchanged rect after the resize
            self.set_ime_window_position(self.last_ime_rect.unwrap_or_default());

            self.events.dispatch(WindowEvent::Resized {
                dimensions: current_dims,
                window_state: get_window_state(self.hwnd.0),
                live_resizing: self.in_size_move,
            });
        }

        !same
    }

    fn apply_decoration(&mut self) {
        let hwnd = self.hwnd.0;
        schedule_apply_decoration(hwnd, self.config.window_decorations);
    }
}

fn schedule_apply_decoration(hwnd: HWND, decorations: WindowDecorations) {
    promise::spawn::spawn(async move {
        apply_decoration_immediate(hwnd, decorations);
    })
    .detach();
}

fn apply_decoration_immediate(hwnd: HWND, decorations: WindowDecorations) {
    match rc_from_hwnd(hwnd) {
        Some(inner) => {
            if inner.borrow().saved_placement.is_some() {
                // We are full screen; ignore it for now
                return;
            }
        }
        None => return,
    };

    unsafe {
        let orig_style = GetWindowLongW(hwnd, GWL_STYLE);
        let style = decorations_to_style(decorations);
        let new_style = (orig_style & !(WS_OVERLAPPEDWINDOW as i32)) | style as i32;
        SetWindowLongW(hwnd, GWL_STYLE, new_style);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE
                | SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOOWNERZORDER
                | SWP_FRAMECHANGED,
        );
        apply_theme(hwnd);
    }
}

fn decorations_to_style(decorations: WindowDecorations) -> u32 {
    if decorations == WindowDecorations::RESIZE {
        WS_OVERLAPPEDWINDOW
    } else if decorations == WindowDecorations::TITLE {
        WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX
    } else if decorations == WindowDecorations::NONE {
        WS_POPUP
    } else if decorations == WindowDecorations::TITLE | WindowDecorations::RESIZE {
        WS_OVERLAPPEDWINDOW
    } else {
        WS_OVERLAPPEDWINDOW
    }
}

/// fork: 光标所在的显示器；取不到光标位置时回退主显示器。新窗口按
/// CW_USEDEFAULT 放置时通常落在启动它的那块屏，用光标位置近似
pub(crate) fn cursor_monitor() -> HMONITOR {
    let mut pt = POINT { x: 0, y: 0 };
    let mon = unsafe {
        if GetCursorPos(&mut pt) != 0 {
            MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)
        } else {
            null_mut()
        }
    };
    if mon.is_null() {
        let primary = unsafe { MonitorFromWindow(null_mut(), MONITOR_DEFAULTTOPRIMARY) };
        assert!(!primary.is_null(), "MonitorFromWindow() returned NULL");
        primary
    } else {
        mon
    }
}

/// fork: 显示器的有效 DPI（MDT_EFFECTIVE_DPI）；mon 为空或查询失败为 None
pub(crate) fn monitor_dpi(mon: HMONITOR) -> Option<u32> {
    if mon.is_null() {
        return None;
    }
    let mut dpi_x = 0;
    let mut dpi_y = 0;
    let hr = unsafe { GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    if hr == S_OK && dpi_x != 0 {
        Some(dpi_x)
    } else {
        None
    }
}

impl Window {
    fn create_window(
        config: ConfigHandle,
        class_name: &str,
        name: &str,
        geometry: ResolvedGeometry,
        lparam: *const RefCell<WindowInner>,
    ) -> anyhow::Result<HWND> {
        let class_name = wide_string(class_name);
        let h_inst = unsafe { GetModuleHandleW(null()) };
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: h_inst,
            // FIXME: this resource is specific to the wezterm build and this should
            // really be made generic for other sorts of windows.
            // The ID is defined in assets/windows/resource.rc
            hIcon: unsafe { LoadIconW(h_inst, MAKEINTRESOURCEW(0x101)) },
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
        };

        if unsafe { RegisterClassW(&class) } == 0 {
            let err = IoError::last_os_error();
            match err.raw_os_error() {
                Some(code)
                    if code == winapi::shared::winerror::ERROR_CLASS_ALREADY_EXISTS as i32 => {}
                _ => return Err(err.into()),
            }
        }

        let decorations = config.window_decorations;
        let style = decorations_to_style(decorations);
        // fork: 与 ConnectionOps::default_dpi 同口径取光标所在显示器，客户区
        // 尺寸与非客户区边框按同一 DPI 换算
        let frame_dpi = monitor_dpi(cursor_monitor()).unwrap_or(USER_DEFAULT_SCREEN_DPI as u32);
        let (width, height) =
            adjust_client_to_window_dimensions(style, geometry.width, geometry.height, frame_dpi);

        let (x, y) = match (geometry.x, geometry.y) {
            (Some(x), Some(y)) => (x, y),
            _ => {
                if (style & WS_POPUP) == 0 {
                    (CW_USEDEFAULT, CW_USEDEFAULT)
                } else {
                    // WS_POPUP windows need to specify the initial position.
                    // We pick the middle of the primary monitor

                    unsafe {
                        let mut mi: MONITORINFO = std::mem::zeroed();
                        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                        GetMonitorInfoW(
                            MonitorFromWindow(std::ptr::null_mut(), MONITOR_DEFAULTTOPRIMARY),
                            &mut mi,
                        );

                        let mon_width = mi.rcMonitor.right - mi.rcMonitor.left;
                        let mon_height = mi.rcMonitor.bottom - mi.rcMonitor.top;

                        (
                            mi.rcMonitor.left + (mon_width - width) / 2,
                            mi.rcMonitor.top + (mon_height - height) / 2,
                        )
                    }
                }
            }
        };

        let name = wide_string(name);
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                name.as_ptr(),
                style,
                x,
                y,
                width,
                height,
                null_mut(),
                null_mut(),
                null_mut(),
                std::mem::transmute(lparam),
            )
        };

        if hwnd.is_null() {
            let err = IoError::last_os_error();
            bail!("CreateWindowExW: {}", err);
        }

        // We have to re-apply the styles otherwise they don't
        // completely stick
        schedule_apply_decoration(hwnd, decorations);

        Ok(hwnd)
    }

    pub async fn new_window<F>(
        class_name: &str,
        name: &str,
        geometry: RequestedWindowGeometry,
        config: Option<&ConfigHandle>,
        _font_config: Rc<FontConfiguration>,
        event_handler: F,
    ) -> anyhow::Result<Window>
    where
        F: 'static + FnMut(WindowEvent, &Window),
    {
        let events = WindowEventSender::new(event_handler);

        let config = match config {
            Some(c) => c.clone(),
            None => config::configuration(),
        };
        let appearance = get_appearance();

        let inner = Rc::new(RefCell::new(WindowInner {
            hwnd: HWindow(null_mut()),
            appearance,
            events,
            gl_state: None,
            vscroll_remainder: 0,
            hscroll_remainder: 0,
            keyboard_info: KeyboardLayoutInfo::new(),
            last_size: None,
            in_size_move: false,
            dead_pending: None,
            saved_placement: None,
            track_mouse_leave: false,
            window_drag_position: None,
            maximize_button_position: None,
            config: config.clone(),
            paint_throttled: false,
            invalidated: true,
            monitor: null_mut(),
            monitor_refresh_hz: None,
            frame_start: Instant::now(),
            next_paint_deadline: None,
            throttle_generation: 0,
            first_frame_presented: false,
            pending_show: None,
            show_fallback_armed: false,
            last_ime_rect: None,
        }));

        // Careful: `raw` owns a ref to inner, but there is no Drop impl
        let raw = rc_to_pointer(&inner);

        let conn = Connection::get().expect("Connection::init was not called");

        let geometry = conn.resolve_geometry(geometry);

        let hwnd = match Self::create_window(config, class_name, name, geometry, raw) {
            Ok(hwnd) => HWindow(hwnd),
            Err(err) => {
                // Ensure that we drop the extra ref to raw before we return
                drop(unsafe { Rc::from_raw(raw) });
                return Err(err);
            }
        };
        let window_handle = Window(hwnd);
        {
            let mut inner = inner.borrow_mut();
            inner.events.assign_window(window_handle.clone());
            inner.refresh_monitor_info(true);
        }

        apply_theme(hwnd.0);
        enable_blur_behind(hwnd.0);

        // Make window capable of accepting drag and drop
        unsafe {
            DragAcceptFiles(hwnd.0, winapi::shared::minwindef::TRUE);
        }

        conn.windows
            .borrow_mut()
            .insert(hwnd.clone(), Rc::clone(&inner));

        Ok(window_handle)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ShowWindowCommand {
    Normal,
    Minimize,
    Maximize,
}

fn schedule_show_window(hwnd: HWindow, show: ShowWindowCommand) {
    // ShowWindow can call to the window proc and may attempt
    // to lock inner, so we avoid locking it ourselves here
    log::trace!("scheduling ShowWindowCommand {show:?}");
    promise::spawn::spawn(async move {
        unsafe {
            log::trace!("applying ShowWindowCommand {show:?}");
            ShowWindow(
                hwnd.0,
                match show {
                    ShowWindowCommand::Normal => SW_NORMAL,
                    ShowWindowCommand::Minimize => SW_MINIMIZE,
                    ShowWindowCommand::Maximize => SW_MAXIMIZE,
                },
            );
        }
    })
    .detach();
}

impl WindowInner {
    fn close(&mut self) {
        let hwnd = self.hwnd;
        promise::spawn::spawn(async move {
            unsafe {
                DestroyWindow(hwnd.0);
            }
        })
        .detach();
    }

    /// fork: 首帧 present 之前把 show 请求挂起，等 notify_first_frame_presented
    /// 或兜底定时器再真正 ShowWindow。隐藏窗口收不到 WM_PAINT，所以主动派发
    /// NeedRepaint 驱动 GPU 尽快出帧；schedule_show_window 自身是排队执行，
    /// 在此处持锁调用是安全的
    fn request_show(&mut self, show: ShowWindowCommand) {
        if self.first_frame_presented {
            schedule_show_window(self.hwnd, show);
            return;
        }
        log::trace!("deferring {show:?} until first frame is presented");
        self.pending_show.replace(show);
        self.arm_first_frame_fallback();
        self.events.dispatch(WindowEvent::NeedRepaint);
    }

    fn first_frame_presented(&mut self) {
        self.first_frame_presented = true;
        if let Some(show) = self.pending_show.take() {
            log::trace!("first frame presented; applying deferred {show:?}");
            schedule_show_window(self.hwnd, show);
        }
    }

    /// 首帧永远失败（如 GPU 初始化失败）时也不能让用户面对无窗口进程，
    /// 超时后照常把窗口显示出来。fork: 兜底从 1.5s 缩到 300ms，让 present
    /// 失败路径尽快 show；此时首帧背景由 fill_unpresented_background
    /// 填终端背景色兜底，白帧风险可控
    fn arm_first_frame_fallback(&mut self) {
        if self.show_fallback_armed {
            return;
        }
        self.show_fallback_armed = true;
        let hwnd = self.hwnd;
        promise::spawn::spawn(async move {
            async_io::Timer::after(std::time::Duration::from_millis(300)).await;
            Connection::with_window_inner(hwnd, |inner| {
                if !inner.first_frame_presented {
                    log::warn!("no frame presented within 300ms of show(); showing window anyway");
                    inner.first_frame_presented();
                }
                Ok(())
            });
        })
        .detach();
    }

    fn set_cursor(&mut self, cursor: Option<CursorIcon>) {
        apply_mouse_cursor(cursor);
    }

    fn set_window_position(&self, coords: ScreenPoint) {
        let hwnd = self.hwnd.0;
        log::trace!("set_window_position wants {coords:?}");
        promise::spawn::spawn(async move {
            log::trace!("set_window_position apply {coords:?}");
            let mut rect = RECT {
                left: 0,
                bottom: 0,
                right: 0,
                top: 0,
            };
            unsafe {
                GetWindowRect(hwnd, &mut rect);

                let origin = client_to_screen(hwnd, Point::new(0, 0));
                let delta_x = origin.x as i32 - rect.left;
                let delta_y = origin.y as i32 - rect.top;

                MoveWindow(
                    hwnd,
                    coords.x as i32 - delta_x,
                    coords.y as i32 - delta_y,
                    rect_width(&rect),
                    rect_height(&rect),
                    1,
                );
            }
        })
        .detach();
    }

    fn set_title(&mut self, title: &str) {
        let title = wide_string(title);
        unsafe {
            SetWindowTextW(self.hwnd.0, title.as_ptr());
        }
    }

    fn set_text_cursor_position(&mut self, cursor: Rect) {
        self.last_ime_rect = Some(cursor);
        self.set_ime_window_position(cursor);
    }

    /// fork: re-apply the last rect reported by the GUI (see last_ime_rect);
    /// a no-op until the GUI has reported one
    fn replay_ime_window_position(&mut self) {
        if let Some(rect) = self.last_ime_rect {
            self.set_ime_window_position(rect);
        }
    }

    fn set_ime_window_position(&mut self, cursor: Rect) {
        let imc = ImmContext::get(self.hwnd.0);
        match self.config.ime_preedit_rendering {
            ImePreeditRendering::Builtin => {
                // fork: several IMEs (TSF based ones such as Microsoft Pinyin
                // among them) anchor their candidate list to the composition
                // window even while it is hidden, so position both
                imc.set_composition_window_position(cursor);
                imc.set_candidate_window_position(cursor);
            }
            ImePreeditRendering::System => imc.set_composition_window_position(cursor),
        }
    }

    fn config_did_change(&mut self, config: &ConfigHandle) {
        self.config = config.clone();
        self.apply_decoration();
        // fork: max_fps / max_fps_follows_display 重载后立刻生效：正处于
        // 节流窗口时按新间隔从本帧开始重算 deadline
        if self.paint_throttled {
            self.schedule_paint_throttle();
        }
    }

    fn toggle_fullscreen(&mut self) {
        unsafe {
            let hwnd = self.hwnd.0;
            let style = GetWindowLongW(hwnd, GWL_STYLE);
            let config = self.config.clone();
            if let Some(placement) = self.saved_placement.take() {
                promise::spawn::spawn(async move {
                    let style = decorations_to_style(config.window_decorations);
                    SetWindowLongW(hwnd, GWL_STYLE, style as i32);
                    SetWindowPlacement(hwnd, &placement);
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE
                            | SWP_NOSIZE
                            | SWP_NOZORDER
                            | SWP_NOOWNERZORDER
                            | SWP_FRAMECHANGED,
                    );
                })
                .detach();
            } else {
                let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
                GetWindowPlacement(hwnd, &mut placement);

                self.saved_placement.replace(placement);
                promise::spawn::spawn(async move {
                    let mut mi: MONITORINFO = std::mem::zeroed();
                    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                    GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY), &mut mi);
                    SetWindowLongW(hwnd, GWL_STYLE, style & !(WS_OVERLAPPEDWINDOW as i32));
                    SetWindowPos(
                        hwnd,
                        HWND_TOP,
                        mi.rcMonitor.left,
                        mi.rcMonitor.top,
                        mi.rcMonitor.right - mi.rcMonitor.left,
                        mi.rcMonitor.bottom - mi.rcMonitor.top,
                        SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
                    );
                })
                .detach();
            }
        }
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<DisplayHandle, HandleError> {
        unsafe {
            Ok(DisplayHandle::borrow_raw(RawDisplayHandle::Windows(
                WindowsDisplayHandle::new(),
            )))
        }
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle, HandleError> {
        let conn = Connection::get().expect("raw_window_handle only callable on main thread");
        let handle = conn.get_window(self.0).expect("window handle invalid!?");

        let inner = handle.borrow();
        let handle = inner.window_handle()?;
        unsafe { Ok(WindowHandle::borrow_raw(handle.as_raw())) }
    }
}

#[async_trait(?Send)]
impl WindowOps for Window {
    async fn enable_opengl(&self) -> anyhow::Result<Rc<glium::backend::Context>> {
        let window = self.0;
        promise::spawn::spawn(async move {
            if let Some(handle) = Connection::get().unwrap().get_window(window) {
                let mut inner = handle.borrow_mut();
                inner.enable_opengl()
            } else {
                anyhow::bail!("invalid window");
            }
        })
        .await
    }

    fn notify<T: Any + Send + Sync>(&self, t: T)
    where
        Self: Sized,
    {
        Connection::with_window_inner(self.0, move |inner| {
            inner
                .events
                .dispatch(WindowEvent::Notification(Box::new(t)));
            Ok(())
        });
    }

    fn close(&self) {
        Connection::with_window_inner(self.0, |inner| {
            inner.close();
            Ok(())
        });
    }

    fn show(&self) {
        Connection::with_window_inner(self.0, |inner| {
            inner.request_show(ShowWindowCommand::Normal);
            Ok(())
        });
    }

    fn hide(&self) {
        Connection::with_window_inner(self.0, |inner| {
            // fork: 显式 hide 优先于挂起的 show，避免首帧后又把窗口弹回来
            inner.pending_show.take();
            Ok(())
        });
        schedule_show_window(self.0, ShowWindowCommand::Minimize);
    }

    fn focus(&self) {
        let window = self.0;
        let handle = window.0;
        promise::spawn::spawn(async move {
            // In some situation, calling SetForegroundWindow could not bring up the window,
            // This is a little hack which can "steal" the foreground window permission
            // We only call this function in the window creation, so it should be fine.
            // See : https://stackoverflow.com/questions/10740346/setforegroundwindow-only-working-while-visual-studio-is-open
            unsafe {
                let alt_sc = MapVirtualKeyW(VK_MENU as u32, MAPVK_VK_TO_VSC);

                let mut inputs: [INPUT; 2] = [
                    INPUT {
                        type_: INPUT_KEYBOARD,
                        u: Default::default(),
                    },
                    INPUT {
                        type_: INPUT_KEYBOARD,
                        u: Default::default(),
                    },
                ];
                *inputs[0].u.ki_mut() = KEYBDINPUT {
                    wVk: VK_LMENU as u16,
                    wScan: alt_sc as u16,
                    dwFlags: KEYEVENTF_EXTENDEDKEY,
                    dwExtraInfo: 0,
                    time: 0,
                };
                *inputs[1].u.ki_mut() = KEYBDINPUT {
                    wVk: VK_LMENU as u16,
                    wScan: alt_sc as u16,
                    dwFlags: KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP,
                    dwExtraInfo: 0,
                    time: 0,
                };

                // Simulate a key press and release
                SendInput(
                    inputs.len() as u32,
                    inputs.as_mut_ptr(),
                    std::mem::size_of::<INPUT>() as i32,
                );

                SetForegroundWindow(handle);
            }
        })
        .detach();
    }

    fn maximize(&self) {
        Connection::with_window_inner(self.0, |inner| {
            inner.request_show(ShowWindowCommand::Maximize);
            Ok(())
        });
    }

    fn restore(&self) {
        Connection::with_window_inner(self.0, |inner| {
            inner.request_show(ShowWindowCommand::Normal);
            Ok(())
        });
    }

    fn notify_first_frame_presented(&self) {
        Connection::with_window_inner(self.0, |inner| {
            inner.first_frame_presented();
            Ok(())
        });
    }

    fn set_cursor(&self, cursor: Option<CursorIcon>) {
        Connection::with_window_inner(self.0, move |inner| {
            inner.set_cursor(cursor);
            Ok(())
        });
    }

    fn invalidate(&self) {
        let hwnd = self.0 .0;
        log::trace!("WindowOps::invalidate calling InvalidateRect");
        unsafe {
            InvalidateRect(hwnd, null(), 0);
        }
    }

    fn request_attention(&self) {
        let hwnd = self.0;
        // fork: flash the taskbar button (not the caption) until the window
        // is brought to the foreground; run from the message loop like the
        // other window operations
        promise::spawn::spawn(async move {
            let mut info = FLASHWINFO {
                cbSize: std::mem::size_of::<FLASHWINFO>() as UINT,
                hwnd: hwnd.0,
                dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
                uCount: 0,
                dwTimeout: 0,
            };
            unsafe {
                FlashWindowEx(&mut info);
            }
        })
        .detach();
    }

    fn set_progress(&self, progress: ProgressState) {
        let hwnd = self.0;
        // fork: the COM calls may pump messages, so keep them out of the
        // caller's stack (the GUI calls this while handling a notification)
        promise::spawn::spawn(async move {
            set_taskbar_progress(hwnd.0, progress);
        })
        .detach();
    }

    fn set_title(&self, title: &str) {
        let title = title.to_owned();
        Connection::with_window_inner(self.0, move |inner| {
            inner.set_title(&title);
            Ok(())
        });
    }

    fn toggle_fullscreen(&self) {
        Connection::with_window_inner(self.0, move |inner| {
            inner.toggle_fullscreen();
            Ok(())
        });
    }

    fn config_did_change(&self, config: &ConfigHandle) {
        let config = config.clone();
        Connection::with_window_inner(self.0, move |inner| {
            inner.config_did_change(&config);
            Ok(())
        });
    }

    fn set_text_cursor_position(&self, cursor: Rect) {
        Connection::with_window_inner(self.0, move |inner| {
            inner.set_text_cursor_position(cursor);
            Ok(())
        });
    }

    fn set_inner_size(&self, width: usize, height: usize) {
        Connection::with_window_inner(self.0, move |inner| {
            let hwnd = inner.hwnd;
            let decorations = inner.config.window_decorations;
            promise::spawn::spawn(async move {
                log::trace!("set_inner_size called with {width}x{height}");
                let frame_dpi = unsafe { GetDpiForWindow(hwnd.0) };
                let (width, height) = adjust_client_to_window_dimensions(
                    decorations_to_style(decorations),
                    width,
                    height,
                    frame_dpi,
                );
                let window_state = get_window_state(hwnd.0);
                if window_state.can_resize() {
                    log::trace!("set_inner_size now calling SetWindowPos with {width}x{height}");
                    unsafe {
                        SetWindowPos(
                            hwnd.0,
                            hwnd.0,
                            0,
                            0,
                            width,
                            height,
                            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
                        );
                        wm_paint(hwnd.0, 0, 0, 0);
                        if let Some(inner) = rc_from_hwnd(hwnd.0) {
                            let mut inner = inner.borrow_mut();
                            inner.events.dispatch(WindowEvent::SetInnerSizeCompleted);
                        }
                    }
                } else {
                    log::trace!(
                        "ignoring set_inner_size({width}, {height}) call \
                                because window_state is {window_state:?}"
                    );
                }
            })
            .detach();
            Ok(())
        });
    }

    fn set_maximize_button_position(&self, coords: ScreenRect) {
        Connection::with_window_inner(self.0, move |inner| {
            inner.maximize_button_position = Some(coords);
            Ok(())
        });
    }

    fn set_window_position(&self, coords: ScreenPoint) {
        Connection::with_window_inner(self.0, move |inner| {
            inner.set_window_position(coords);
            Ok(())
        });
    }

    fn get_clipboard(&self, _clipboard: Clipboard) -> Future<String> {
        let mut promise = Promise::new();
        let future = promise.get_future().unwrap();
        // fork: read on the clipboard thread, which retries while another
        // process holds the clipboard open
        run_on_clipboard_thread(move || {
            promise.result(read_clipboard_text());
        });
        future
    }

    fn set_clipboard(&self, _clipboard: Clipboard, text: String) {
        // fork: write on the clipboard thread with backoff instead of giving
        // up silently after ten Sleep(0) attempts
        run_on_clipboard_thread(move || write_clipboard_text(&text));
    }

    fn get_clipboard_image(&self, _clipboard: Clipboard) -> Future<Option<ClipboardImage>> {
        let mut promise = Promise::new();
        let future = promise.get_future().unwrap();
        // fork: 图片载荷可达数 MB，与文本读写一样排进剪贴板线程，读完再 resolve
        run_on_clipboard_thread(move || {
            promise.ok(read_clipboard_image());
        });
        future
    }

    fn set_window_drag_position(&self, coords: ScreenPoint) {
        Connection::with_window_inner(self.0, move |inner| {
            inner.window_drag_position = Some(coords);

            Ok(())
        });
    }

    fn get_os_parameters(
        &self,
        config: &ConfigHandle,
        window_state: WindowState,
    ) -> anyhow::Result<Option<Parameters>> {
        let hwnd = self.0 .0;
        anyhow::ensure!(!hwnd.is_null(), "HWND is null");

        let has_focus = unsafe { GetFocus() } == hwnd;
        let is_full_screen = window_state.contains(WindowState::FULL_SCREEN);

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let use_accent = hkcu
            .open_subkey("SOFTWARE\\Microsoft\\Windows\\DWM")?
            .get_value::<u32, _>("ColorPrevalence")?;
        let settings = UISettings::new()?;
        let top_border_color = if has_focus {
            if use_accent == 1 {
                wuicolor_to_linearrgba(settings.GetColorValue(UIColorType::Accent)?)
            } else {
                if *IS_WIN10 {
                    LinearRgba(0.01, 0.01, 0.01, 0.67)
                } else {
                    LinearRgba(0.026, 0.026, 0.026, 0.5)
                }
            }
        } else {
            if *IS_WIN10 {
                LinearRgba(0.024, 0.024, 0.024, 0.5)
            } else {
                LinearRgba(0.028, 0.028, 0.028, 0.5)
            }
        };

        const BASE_BORDER: ULength = ULength::new(0);
        let is_resize = config.window_decorations == WindowDecorations::RESIZE;

        let title_font = {
            let mut cache = TITLE_FONT.lock().expect("locking title_font");
            if cache.stale {
                cache.stale = false;
                if let Some(font) = unsafe { load_title_font(hwnd) } {
                    cache.font = Some(font);
                }
            }
            cache.font.clone()
        };

        Ok(Some(Parameters {
            title_bar: parameters::TitleBar {
                padding_left: ULength::new(0),
                padding_right: ULength::new(0),
                height: None,
                font_and_size: title_font,
            },
            border_dimensions: Some(parameters::Border {
                top: if is_resize && !*IS_WIN10 && !is_full_screen {
                    BASE_BORDER + ULength::new(1)
                } else {
                    BASE_BORDER
                },
                left: BASE_BORDER,
                bottom: if is_resize && *IS_WIN10 && !is_full_screen {
                    BASE_BORDER + ULength::new(2)
                } else {
                    BASE_BORDER
                },
                right: BASE_BORDER,
                color: top_border_color,
            }),
        }))
    }
}

/// fork: state of the GUI thread's ITaskbarList3 (taskbar progress)
#[derive(Clone, Copy)]
enum TaskbarList {
    Untried,
    Unavailable,
    Ready(*mut winapi::um::shobjidl_core::ITaskbarList3),
}

thread_local! {
    static TASKBAR_LIST: std::cell::Cell<TaskbarList> = std::cell::Cell::new(TaskbarList::Untried);
}

/// fork: the GUI thread's ITaskbarList3, created on first use. It is never
/// released: it lives as long as the GUI thread, and tearing it down during
/// process exit gains nothing. A failed creation is not retried.
unsafe fn taskbar_list() -> Option<*mut winapi::um::shobjidl_core::ITaskbarList3> {
    use winapi::shared::winerror::{FAILED, RPC_E_CHANGED_MODE};
    use winapi::shared::wtypesbase::CLSCTX_INPROC_SERVER;
    use winapi::um::combaseapi::{CoCreateInstance, CoInitializeEx};
    use winapi::um::objbase::COINIT_APARTMENTTHREADED;
    use winapi::um::shobjidl_core::{CLSID_TaskbarList, ITaskbarList3};
    use winapi::Interface;

    match TASKBAR_LIST.with(|cell| cell.get()) {
        TaskbarList::Ready(list) => return Some(list),
        TaskbarList::Unavailable => return None,
        TaskbarList::Untried => {}
    }
    TASKBAR_LIST.with(|cell| cell.set(TaskbarList::Unavailable));

    // S_FALSE (already initialized) is fine, and RPC_E_CHANGED_MODE means
    // the thread already joined the MTA, where the object still works
    let hr = CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED);
    if FAILED(hr) && hr != RPC_E_CHANGED_MODE {
        log::warn!("CoInitializeEx failed ({:#x}); no taskbar progress", hr);
        return None;
    }

    let mut list: *mut ITaskbarList3 = null_mut();
    let hr = CoCreateInstance(
        &CLSID_TaskbarList,
        null_mut(),
        CLSCTX_INPROC_SERVER,
        &ITaskbarList3::uuidof(),
        &mut list as *mut *mut ITaskbarList3 as *mut LPVOID,
    );
    if FAILED(hr) || list.is_null() {
        log::warn!(
            "creating ITaskbarList3 failed ({:#x}); no taskbar progress",
            hr
        );
        return None;
    }
    let hr = (*list).HrInit();
    if FAILED(hr) {
        log::warn!(
            "ITaskbarList3::HrInit failed ({:#x}); no taskbar progress",
            hr
        );
        (*list).Release();
        return None;
    }

    TASKBAR_LIST.with(|cell| cell.set(TaskbarList::Ready(list)));
    Some(list)
}

/// fork: ITaskbarList3 state flag plus, for the states that show a bar, its
/// value out of 100
fn taskbar_progress(progress: ProgressState) -> (winapi::um::shobjidl_core::TBPFLAG, Option<u64>) {
    use winapi::um::shobjidl_core::{
        TBPF_ERROR, TBPF_INDETERMINATE, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED,
    };

    // An error or paused report without a percentage (OSC 9;4;2 and
    // 9;4;4 commonly omit it) would leave the colored bar empty and
    // invisible, so show it full instead
    fn full_if_zero(pct: u8) -> u64 {
        match pct.min(100) {
            0 => 100,
            pct => pct as u64,
        }
    }

    match progress {
        ProgressState::None => (TBPF_NOPROGRESS, None),
        ProgressState::Indeterminate => (TBPF_INDETERMINATE, None),
        ProgressState::Normal(pct) => (TBPF_NORMAL, Some(pct.min(100) as u64)),
        ProgressState::Error(pct) => (TBPF_ERROR, Some(full_if_zero(pct))),
        ProgressState::Paused(pct) => (TBPF_PAUSED, Some(full_if_zero(pct))),
    }
}

fn set_taskbar_progress(hwnd: HWND, progress: ProgressState) {
    use winapi::shared::winerror::FAILED;

    let (flag, value) = taskbar_progress(progress);
    unsafe {
        let Some(list) = taskbar_list() else {
            return;
        };
        // SetProgressValue turns NOPROGRESS/INDETERMINATE into NORMAL, so set
        // the value first and the state last
        if let Some(value) = value {
            (*list).SetProgressValue(hwnd, value, 100);
        }
        let hr = (*list).SetProgressState(hwnd, flag);
        if FAILED(hr) {
            log::debug!("ITaskbarList3::SetProgressState failed: {:#x}", hr);
        }
    }
}

#[cfg(test)]
mod taskbar_progress_tests {
    use super::taskbar_progress;
    use crate::ProgressState;
    use winapi::um::shobjidl_core::{
        TBPF_ERROR, TBPF_INDETERMINATE, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED,
    };

    #[test]
    fn states_map_to_taskbar_flags() {
        assert_eq!(
            taskbar_progress(ProgressState::None),
            (TBPF_NOPROGRESS, None)
        );
        assert_eq!(
            taskbar_progress(ProgressState::Indeterminate),
            (TBPF_INDETERMINATE, None)
        );
        assert_eq!(
            taskbar_progress(ProgressState::Normal(42)),
            (TBPF_NORMAL, Some(42))
        );
        assert_eq!(
            taskbar_progress(ProgressState::Error(30)),
            (TBPF_ERROR, Some(30))
        );
        assert_eq!(
            taskbar_progress(ProgressState::Paused(70)),
            (TBPF_PAUSED, Some(70))
        );
    }

    #[test]
    fn percentages_are_clamped() {
        assert_eq!(
            taskbar_progress(ProgressState::Normal(250)),
            (TBPF_NORMAL, Some(100))
        );
        assert_eq!(
            taskbar_progress(ProgressState::Normal(0)),
            (TBPF_NORMAL, Some(0))
        );
    }

    #[test]
    fn error_and_paused_without_percentage_show_a_full_bar() {
        assert_eq!(
            taskbar_progress(ProgressState::Error(0)),
            (TBPF_ERROR, Some(100))
        );
        assert_eq!(
            taskbar_progress(ProgressState::Paused(0)),
            (TBPF_PAUSED, Some(100))
        );
    }
}

type ClipboardJob = Box<dyn FnOnce() + Send>;

lazy_static! {
    /// fork: every clipboard access runs on this one background thread, in
    /// the order it was requested. Opening the clipboard can block while
    /// another process holds it, which must not stall the GUI thread, and a
    /// single FIFO keeps "copy, then paste" from being reordered by thread
    /// scheduling.
    static ref CLIPBOARD_THREAD: Mutex<std::sync::mpsc::Sender<ClipboardJob>> = {
        let (tx, rx) = std::sync::mpsc::channel::<ClipboardJob>();
        std::thread::Builder::new()
            .name("clipboard".to_string())
            .spawn(move || {
                for job in rx {
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
                        log::error!("clipboard job panicked");
                    }
                }
            })
            .expect("failed to spawn clipboard thread");
        Mutex::new(tx)
    };
}

/// fork: queue a job on the clipboard thread
fn run_on_clipboard_thread(job: impl FnOnce() + Send + 'static) {
    let job: ClipboardJob = Box::new(job);
    let sent = match CLIPBOARD_THREAD.lock() {
        Ok(tx) => tx.send(job).is_ok(),
        Err(_) => false,
    };
    if !sent {
        log::error!("clipboard thread is gone; clipboard request dropped");
    }
}

/// fork: number of OpenClipboard attempts before giving up. Clipboard
/// managers, rdpclip and password managers keep the clipboard open for a
/// few milliseconds after every change, longer than the Sleep(0) retries in
/// clipboard-win last.
const CLIPBOARD_OPEN_ATTEMPTS: u32 = 16;

/// fork: wait before retrying after failed attempt `attempt` (0-based):
/// 10ms, growing by 1ms per attempt up to 20ms
fn clipboard_retry_delay(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis(10 + u64::from(attempt.min(10)))
}

/// fork: open the clipboard, backing off while another process holds it.
/// The returned guard closes it again. Blocks; clipboard thread only.
fn open_clipboard_with_retry() -> Option<clipboard_win::Clipboard> {
    for attempt in 0..CLIPBOARD_OPEN_ATTEMPTS {
        match clipboard_win::Clipboard::new() {
            Ok(clipboard) => return Some(clipboard),
            Err(err) if attempt + 1 == CLIPBOARD_OPEN_ATTEMPTS => {
                log::warn!(
                    "unable to open clipboard after {} attempts: {}",
                    CLIPBOARD_OPEN_ATTEMPTS,
                    err
                );
            }
            Err(_) => std::thread::sleep(clipboard_retry_delay(attempt)),
        }
    }
    None
}

#[cfg(test)]
mod clipboard_retry_tests {
    use super::{clipboard_retry_delay, CLIPBOARD_OPEN_ATTEMPTS};
    use std::time::Duration;

    #[test]
    fn delay_backs_off_from_10_to_20_ms() {
        assert_eq!(clipboard_retry_delay(0), Duration::from_millis(10));
        assert_eq!(clipboard_retry_delay(5), Duration::from_millis(15));
        assert_eq!(clipboard_retry_delay(10), Duration::from_millis(20));
        assert_eq!(clipboard_retry_delay(u32::MAX), Duration::from_millis(20));
    }

    #[test]
    fn total_wait_stays_bounded() {
        // the last failed attempt does not sleep
        let total: Duration = (0..CLIPBOARD_OPEN_ATTEMPTS - 1)
            .map(clipboard_retry_delay)
            .sum();
        assert!(
            (10..=20).contains(&CLIPBOARD_OPEN_ATTEMPTS),
            "{CLIPBOARD_OPEN_ATTEMPTS}"
        );
        assert!(total >= Duration::from_millis(150), "{total:?}");
        assert!(total <= Duration::from_millis(400), "{total:?}");
    }
}

fn read_clipboard_text() -> anyhow::Result<String> {
    let _clipboard = open_clipboard_with_retry()
        .ok_or_else(|| anyhow::anyhow!("clipboard is held by another process"))
        .context("Error getting clipboard")?;
    let text: String = clipboard_win::get(clipboard_win::formats::Unicode)
        .map_err(|err| anyhow::anyhow!("{}", err))
        .context("Error getting clipboard")?;
    Ok(text.replace("\r\n", "\n"))
}

fn write_clipboard_text(text: &str) {
    let Some(_clipboard) = open_clipboard_with_retry() else {
        log::warn!(
            "clipboard write of {} bytes dropped: clipboard is busy",
            text.len()
        );
        return;
    };
    if let Err(err) = clipboard_win::raw::set_string(text) {
        log::warn!("failed to set clipboard text: {}", err);
    }
}

// fork: 按 注册PNG → CF_DIBV5 → CF_DIB 优先级探测剪贴板图片并取原始字节。
// 探测走 GetPriorityClipboardFormat（无需打开剪贴板）；读取需独占打开，
// 剪贴板被其他进程短暂占用时按 open_clipboard_with_retry 退避重试。
// 返回 None 表示无可用图片格式
fn read_clipboard_image() -> Option<ClipboardImage> {
    let mut candidates: Vec<(u32, ClipboardImageFormat)> = Vec::new();
    if let Some(png) = clipboard_win::raw::register_format("PNG") {
        candidates.push((png.get(), ClipboardImageFormat::Png));
    }
    candidates.push((
        clipboard_win::formats::CF_DIBV5,
        ClipboardImageFormat::DibV5,
    ));
    candidates.push((clipboard_win::formats::CF_DIB, ClipboardImageFormat::Dib));

    let ids: Vec<u32> = candidates.iter().map(|(id, _)| *id).collect();
    let format = clipboard_win::raw::which_format_avail(&ids)?;
    let (format, kind) = candidates
        .iter()
        .find(|(id, _)| *id == format.get())
        .copied()?;

    let Some(clipboard) = open_clipboard_with_retry() else {
        log::warn!("unable to open clipboard to read image");
        return None;
    };

    let mut data = Vec::new();
    let result = clipboard_win::raw::get_vec(format, &mut data);
    drop(clipboard);

    match result {
        Ok(size) if size > 0 => Some(ClipboardImage { data, format: kind }),
        Ok(_) => None,
        Err(err) => {
            log::warn!("error reading clipboard image format {}: {}", format, err);
            None
        }
    }
}

unsafe fn get_title_log_font(hwnd: HWND, hdc: HDC) -> Option<LOGFONTW> {
    let mut log_font = LOGFONTW::default();
    let theme = OpenThemeData(hwnd, wide_string("HEADER").as_ptr());
    if !theme.is_null() {
        let res = GetThemeFont(
            theme,
            hdc,
            extra_constants::HP_HEADERITEM,
            extra_constants::HIS_NORMAL,
            extra_constants::TMT_CAPTIONFONT,
            &mut log_font,
        );
        if res == S_OK {
            CloseThemeData(theme);
            return Some(log_font);
        }
    }

    let res = GetThemeSysFont(theme, extra_constants::TMT_CAPTIONFONT, &mut log_font);
    if !theme.is_null() {
        CloseThemeData(theme);
    }

    if res == S_OK {
        Some(log_font)
    } else {
        None
    }
}

unsafe fn load_title_font(hwnd: HWND) -> Option<parameters::FontAndSize> {
    let hdc = GetDC(hwnd);
    if hdc.is_null() {
        return None;
    }

    let font = get_title_log_font(hwnd, hdc)
        .and_then(|lf| wezterm_font::locator::gdi::parse_log_font(&lf, hdc).ok());

    ReleaseDC(hwnd, hdc);
    font
}

/// fork: see TitleFontCache
fn mark_title_font_stale() {
    TITLE_FONT.lock().expect("locking title_font").stale = true;
}

/// Set up bidirectional pointers:
/// hwnd.USERDATA -> WindowInner
/// WindowInner.hwnd -> hwnd
unsafe fn wm_nccreate(hwnd: HWND, _msg: UINT, _wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let create: &CREATESTRUCTW = &*(lparam as *const CREATESTRUCTW);
    let inner = rc_from_pointer(create.lpCreateParams);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as _);
    inner.borrow_mut().hwnd = HWindow(hwnd);

    None
}

/// Called when the window is being destroyed.
/// Goal is to release the WindowInner reference that was stashed
/// in the window by wm_nccreate.
unsafe fn wm_ncdestroy(
    hwnd: HWND,
    _msg: UINT,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as LPVOID;
    if !raw.is_null() {
        let inner = take_rc_from_pointer(raw);
        let mut inner = inner.borrow_mut();
        inner.events.dispatch(WindowEvent::Destroyed);
        inner.hwnd = HWindow(null_mut());
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    }

    None
}

fn no_native_title_bar(decorations: WindowDecorations) -> bool {
    decorations == WindowDecorations::RESIZE
        || decorations.contains(WindowDecorations::INTEGRATED_BUTTONS)
}

unsafe fn wm_nccalcsize(hwnd: HWND, _msg: UINT, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let inner = match inner.try_borrow() {
        Ok(inner) => inner,
        Err(_) => {
            // We've been called recursively and the upper levels
            // own the borrow. Just take the default action
            return None;
        }
    };

    let no_native_title_bar = no_native_title_bar(inner.config.window_decorations);

    if !(wparam == 1 && no_native_title_bar) {
        return None;
    }

    if inner.saved_placement.is_none() {
        let dpi = inner.get_effective_dpi() as u32;
        let frame_x = GetSystemMetricsForDpi(SM_CXFRAME, dpi);
        let frame_y = GetSystemMetricsForDpi(SM_CYFRAME, dpi);
        let padding = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);

        let params = (lparam as *mut NCCALCSIZE_PARAMS).as_mut().unwrap();

        let requested_client_rect = &mut params.rgrc[0];

        requested_client_rect.right -= frame_x + padding;
        requested_client_rect.left += frame_x + padding;

        let is_maximized = get_window_state(hwnd) == WindowState::MAXIMIZED;

        // Handle bugged top window border on Windows 10
        if *IS_WIN10 {
            if is_maximized {
                requested_client_rect.top += frame_y + padding;
                requested_client_rect.bottom -= frame_y + padding - 2;
            } else {
                requested_client_rect.top += 1;
                requested_client_rect.bottom -= frame_y - padding;
            }
        } else {
            requested_client_rect.bottom -= frame_y + padding;

            if is_maximized {
                requested_client_rect.top += frame_y + padding;
            }
        }
    }

    Some(0)
}

unsafe fn wm_nchittest(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let inner = match inner.try_borrow() {
        Ok(inner) => inner,
        Err(_) => {
            // We've been called recursively and the upper levels
            // own the borrow. Just take the default action
            return None;
        }
    };

    let no_native_title_bar = no_native_title_bar(inner.config.window_decorations);
    if !no_native_title_bar {
        return None;
    }

    // Let the default procedure handle resizing areas
    let result = DefWindowProcW(hwnd, msg, wparam, lparam);

    if matches!(
        result,
        HTNOWHERE
            | HTRIGHT
            | HTLEFT
            | HTTOPLEFT
            | HTTOP
            | HTTOPRIGHT
            | HTBOTTOMRIGHT
            | HTBOTTOM
            | HTBOTTOMLEFT
    ) {
        return Some(result);
    }

    // The adjustment in NCCALCSIZE messes with the detection
    // of the top hit area so manually fixing that.
    let dpi = inner.get_effective_dpi() as u32;
    let frame_x = GetSystemMetricsForDpi(SM_CXFRAME, dpi) as isize;
    let frame_y = GetSystemMetricsForDpi(SM_CYFRAME, dpi) as isize;
    let padding = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) as isize;

    let coords = mouse_coords(lparam);
    let screen_point = ScreenPoint::new(coords.x, coords.y);
    let cursor_point = screen_to_client(hwnd, screen_point);
    let is_maximized = get_window_state(hwnd) == WindowState::MAXIMIZED;

    // check if mouse is in any of the resize areas (HTTOP, HTBOTTOM, etc)

    let mut client_rect = RECT::default();
    let client_rect_is_valid =
        GetClientRect(hwnd, &mut client_rect) == winapi::shared::minwindef::TRUE;

    // Since we are eating the bottom window frame to deal with a Windows 10 bug,
    // we detect resizing in the window client area as a workaround
    if !is_maximized
        && *IS_WIN10
        && client_rect_is_valid
        && cursor_point.y >= (client_rect.bottom as isize) - (frame_y + padding)
    {
        if cursor_point.x <= (frame_x + padding) {
            return Some(HTBOTTOMLEFT);
        } else if cursor_point.x >= (client_rect.right as isize) - (frame_x + padding) {
            return Some(HTBOTTOMRIGHT);
        } else {
            return Some(HTBOTTOM);
        }
    }

    if !is_maximized && cursor_point.y >= 0 && cursor_point.y < frame_y {
        if cursor_point.x <= (frame_x + padding) {
            return Some(HTTOPLEFT);
        } else if cursor_point.x >= (client_rect.right as isize) - (frame_x + padding) {
            return Some(HTTOPRIGHT);
        } else {
            return Some(HTTOP);
        }
    }

    if let Some(coords) = inner.window_drag_position {
        if coords == screen_point && inner.saved_placement.is_none() {
            return Some(HTCAPTION);
        }
    }

    let use_snap_layouts = !*IS_WIN10;
    if use_snap_layouts {
        if let Some(max) = inner.maximize_button_position {
            if max.contains(screen_point) {
                return Some(HTMAXBUTTON);
            }
        }
    }

    Some(HTCLIENT)
}

fn get_window_state(hwnd: HWND) -> WindowState {
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as _,
        ..Default::default()
    };

    let placement =
        if unsafe { GetWindowPlacement(hwnd, &mut placement) } == winapi::shared::minwindef::TRUE {
            placement.showCmd as i32
        } else {
            0
        };

    match placement {
        SW_SHOWMAXIMIZED => WindowState::MAXIMIZED,
        SW_SHOWMINIMIZED => WindowState::HIDDEN,
        _ => unsafe {
            let mut rect = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);

            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi);

            if mi.rcMonitor.left == rect.left
                && mi.rcMonitor.top == rect.top
                && mi.rcMonitor.right == rect.right
                && mi.rcMonitor.bottom == rect.bottom
            {
                WindowState::FULL_SCREEN
            } else {
                WindowState::default()
            }
        },
    }
}

/// "Blur behind" is the old vista term for a cool blurring
/// effect that the DWM could enable.  Subsequent windows
/// versions have removed the blurring.  We use this call
/// to tell DWM that we set proper alpha channel info as
/// a result of rendering our window content.
fn enable_blur_behind(hwnd: HWND) {
    use winapi::shared::minwindef::*;
    use winapi::um::dwmapi::*;
    use winapi::um::wingdi::*;

    unsafe {
        let region = CreateRectRgn(0, 0, -1, -1);

        let bb = DWM_BLURBEHIND {
            dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
            fEnable: TRUE,
            hRgnBlur: region,
            fTransitionOnMaximized: FALSE,
        };

        DwmEnableBlurBehindWindow(hwnd, &bb);

        DeleteObject(region as _);
    }
}

fn apply_theme(hwnd: HWND) -> Option<LRESULT> {
    // Check for OS app theme, and set window attributes accordingly.
    // Note that the MS terminal app uses the logic found here for this stuff:
    // https://github.com/microsoft/terminal/blob/9b92986b49bed8cc41fde4d6ef080921c41e6d9e/src/interactivity/win32/windowtheme.cpp#L62
    use winapi::um::dwmapi::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute};
    use winapi::um::uxtheme::MARGINS;

    #[allow(non_snake_case)]
    type WINDOWCOMPOSITIONATTRIB = u32;
    const WCA_USEDARKMODECOLORS: WINDOWCOMPOSITIONATTRIB = 26;

    #[allow(non_snake_case)]
    #[repr(C)]
    pub struct WINDOWCOMPOSITIONATTRIBDATA {
        Attrib: WINDOWCOMPOSITIONATTRIB,
        pvData: PVOID,
        cbData: winapi::shared::basetsd::SIZE_T,
    }

    shared_library!(User32,
        pub fn SetWindowCompositionAttribute(hwnd: HWND, attrib: *mut WINDOWCOMPOSITIONATTRIBDATA) -> BOOL,
    );

    const DWMWA_USE_IMMERSIVE_DARK_MODE: DWORD = 20;
    const DWMWA_MICA_EFFECT: DWORD = 1029;
    const DWMWA_SYSTEMBACKDROP_TYPE: DWORD = 38;

    #[allow(non_camel_case_types)]
    #[allow(dead_code)]
    #[derive(PartialEq, Eq)]
    #[repr(C)]
    enum ACCENT_STATE {
        ACCENT_DISABLED = 0,
        ACCENT_ENABLE_BLURBEHIND = 3,
        ACCENT_ENABLE_ACRYLICBLURBEHIND = 4,
    }

    #[allow(non_snake_case)]
    #[repr(C)]
    struct ACCENT_POLICY {
        AccentState: u32,
        AccentFlags: u32,
        GradientColour: u32,
        AnimationId: u32,
    }

    #[allow(non_camel_case_types)]
    #[allow(dead_code)]
    #[repr(C)]
    enum DWM_SYSTEMBACKDROP_TYPE {
        DWMSBT_AUTO = 0,
        DWMSBT_NONE = 1,
        DWMSBT_MAINWINDOW = 2,      // Mica
        DWMSBT_TRANSIENTWINDOW = 3, // Acrylic
        DWMSBT_TABBEDWINDOW = 4,    // Tabbed
    }

    unsafe {
        mark_title_font_stale();

        let appearance = get_appearance();
        let theme_string = if appearance == Appearance::Dark {
            "DarkMode_Explorer"
        } else {
            ""
        };

        SetWindowTheme(
            hwnd as _,
            wide_string(theme_string).as_slice().as_ptr(),
            std::ptr::null_mut(),
        );

        let mut enabled: BOOL = if appearance == Appearance::Dark { 1 } else { 0 };
        DwmSetWindowAttribute(
            hwnd as _,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &enabled as *const _ as *const _,
            std::mem::size_of_val(&enabled) as u32,
        );

        if let Ok(user) = User32::open(std::path::Path::new("user32.dll")) {
            (user.SetWindowCompositionAttribute)(
                hwnd,
                &mut WINDOWCOMPOSITIONATTRIBDATA {
                    Attrib: WCA_USEDARKMODECOLORS,
                    pvData: &mut enabled as *mut _ as _,
                    cbData: std::mem::size_of_val(&enabled) as _,
                },
            );
        };

        if let Some(inner) = rc_from_hwnd(hwnd) {
            let mut inner = inner.borrow_mut();

            // Set Acrylic or Mica system Backdrop
            let pv_attribute = match inner.config.win32_system_backdrop {
                SystemBackdrop::Auto => DWM_SYSTEMBACKDROP_TYPE::DWMSBT_AUTO,
                SystemBackdrop::Disable => DWM_SYSTEMBACKDROP_TYPE::DWMSBT_NONE,
                SystemBackdrop::Acrylic => DWM_SYSTEMBACKDROP_TYPE::DWMSBT_TRANSIENTWINDOW,
                SystemBackdrop::Mica => DWM_SYSTEMBACKDROP_TYPE::DWMSBT_MAINWINDOW,
                SystemBackdrop::Tabbed => DWM_SYSTEMBACKDROP_TYPE::DWMSBT_TABBEDWINDOW,
            };

            let margins = match inner.config.window_decorations {
                WindowDecorations::TITLE => -1,
                _ => 0,
            };

            DwmExtendFrameIntoClientArea(
                hwnd,
                &MARGINS {
                    cxLeftWidth: margins,
                    cxRightWidth: margins,
                    cyTopHeight: margins,
                    cyBottomHeight: margins,
                },
            );

            // Apply Acrylic or Mica Backdrop
            if *IS_WIN11_22H2 {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_SYSTEMBACKDROP_TYPE,
                    &pv_attribute as *const _ as _,
                    std::mem::size_of_val(&pv_attribute) as u32,
                );
            } else {
                let mut colour = inner.config.win32_acrylic_accent_color.to_srgb_u8();
                colour.3 = if colour.3 == 0 { 1 } else { colour.3 }; // acrylic doesn't like to have 0 alpha

                let mut policy = ACCENT_POLICY {
                    AccentState: if inner.config.win32_system_backdrop == SystemBackdrop::Acrylic {
                        ACCENT_STATE::ACCENT_ENABLE_ACRYLICBLURBEHIND as _
                    } else {
                        ACCENT_STATE::ACCENT_DISABLED as _
                    },
                    AccentFlags: if inner.config.win32_system_backdrop == SystemBackdrop::Acrylic {
                        2
                    } else {
                        0
                    },
                    GradientColour: (colour.0 as u32)
                        | (colour.1 as u32) << 8
                        | (colour.2 as u32) << 16
                        | (colour.3 as u32) << 24,
                    AnimationId: 0,
                };

                if let Ok(user) = User32::open(std::path::Path::new("user32.dll")) {
                    (user.SetWindowCompositionAttribute)(
                        hwnd,
                        &mut WINDOWCOMPOSITIONATTRIBDATA {
                            Attrib: 0x13,
                            pvData: &mut policy as *mut _ as _,
                            cbData: std::mem::size_of_val(&policy) as _,
                        },
                    );
                }

                if !*IS_WIN10 && !*IS_WIN11_22H2 {
                    // For build versions less than 22h2 but are still win11
                    let mica_enabled: u32 =
                        if inner.config.win32_system_backdrop == SystemBackdrop::Mica {
                            1
                        } else {
                            0
                        };
                    DwmSetWindowAttribute(
                        hwnd,
                        DWMWA_MICA_EFFECT,
                        &mica_enabled as *const _ as _,
                        std::mem::size_of_val(&mica_enabled) as u32,
                    );
                }
            }

            if appearance != inner.appearance {
                inner.appearance = appearance;
                inner
                    .events
                    .dispatch(WindowEvent::AppearanceChanged(appearance));
            }
        }
    }

    None
}

unsafe fn wm_enter_exit_size_move(
    hwnd: HWND,
    msg: UINT,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    let mut should_size = false;
    if let Some(inner) = rc_from_hwnd(hwnd) {
        let mut inner = inner.borrow_mut();
        inner.in_size_move = msg == WM_ENTERSIZEMOVE;
        should_size = !inner.in_size_move;
    }

    if should_size {
        wm_size(hwnd, 0, 0, 0)?;
    }

    Some(0)
}

/// We handle WM_WINDOWPOSCHANGED and dispatch directly to our wm_size as it
/// is a bit more efficient than letting DefWindowProcW parse this and
/// trigger WM_SIZE.
unsafe fn wm_windowposchanged(
    hwnd: HWND,
    _msg: UINT,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    // let pos = &*(lparam as *const WINDOWPOS);
    wm_size(hwnd, 0, 0, 0)?;
    Some(0)
}

unsafe fn wm_size(hwnd: HWND, _msg: UINT, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
    let mut should_paint = false;
    let mut should_pump = false;

    if let Some(inner) = rc_from_hwnd(hwnd) {
        let mut inner = inner.borrow_mut();
        should_paint = inner.check_and_call_resize_if_needed();
        should_pump = inner.in_size_move;
    }

    if should_paint {
        wm_paint(hwnd, 0, 0, 0)?;
        if should_pump {
            crate::spawn::SPAWN_QUEUE.run();
        }
    }

    None
}

unsafe fn wm_set_focus(
    hwnd: HWND,
    _msg: UINT,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let mut inner = inner.borrow_mut();
    // fork: another window of this thread may have moved the shared input
    // context while we were in the background
    inner.replay_ime_window_position();
    inner.events.dispatch(WindowEvent::FocusChanged(true));
    None
}

unsafe fn wm_kill_focus(
    hwnd: HWND,
    _msg: UINT,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    rc_from_hwnd(hwnd)?
        .borrow_mut()
        .events
        .dispatch(WindowEvent::FocusChanged(false));
    None
}

/// fork: 首帧尚未 present 时把更新区域填成终端背景色，避免窗口在 GPU
/// 管线就绪前露出白帧。依赖 per-pixel alpha 的配置（系统 backdrop 材质、
/// 半透明窗口）直接跳过：GDI 写 alpha=0，会破坏这些效果
fn fill_unpresented_background(config: &ConfigHandle, hdc: HDC, rc: &RECT) {
    if !matches!(
        config.win32_system_backdrop,
        SystemBackdrop::Auto | SystemBackdrop::Disable
    ) || config.window_background_opacity < 1.0
    {
        return;
    }

    // 与 term::color::ColorPalette::default() 一致：未配置时背景为黑色
    let background = config
        .resolved_palette
        .background
        .map(SrgbaTuple::from)
        .unwrap_or_default();
    let (red, green, blue, _) = background.as_rgba_u8();

    unsafe {
        // 创建窗口时 enable_blur_behind 让 DWM 尊重表面的 alpha 通道，而
        // GDI 自身绘制写的是 alpha=0（会被合成成透明），所以改用 1x1 不透明
        // DIB 经 AlphaBlend 拉伸填充
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as _;
        info.bmiHeader.biWidth = 1;
        info.bmiHeader.biHeight = 1;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut winapi::ctypes::c_void = null_mut();
        let dib = CreateDIBSection(hdc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if dib.is_null() || bits.is_null() {
            return;
        }
        *(bits as *mut u32) = 0xff00_0000 | (red as u32) << 16 | (green as u32) << 8 | blue as u32;

        let mem_dc = CreateCompatibleDC(hdc);
        if mem_dc.is_null() {
            DeleteObject(dib as _);
            return;
        }
        let old_bitmap = SelectObject(mem_dc, dib as _);
        AlphaBlend(
            hdc,
            rc.left,
            rc.top,
            rect_width(rc),
            rect_height(rc),
            mem_dc,
            0,
            0,
            1,
            1,
            BLENDFUNCTION {
                BlendOp: AC_SRC_OVER,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA,
            },
        );
        SelectObject(mem_dc, old_bitmap);
        DeleteDC(mem_dc);
        DeleteObject(dib as _);
    }
}

/// fork: PerMonitorV2 下 DefWindowProc 不会替窗口调整尺寸；采用系统建议的
/// RECT，让窗口跨不同缩放的显示器时保持物理尺寸与行列数。SetWindowPos
/// 触发的 WM_WINDOWPOSCHANGED 照常经 wm_size 派发带新 DPI 的 Resized。
/// 自管全屏（saved_placement）时尺寸由我们钉死，不干预
unsafe fn wm_dpichanged(hwnd: HWND, _msg: UINT, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if lparam == 0 {
        return None;
    }
    if let Some(inner) = rc_from_hwnd(hwnd) {
        if let Ok(inner) = inner.try_borrow() {
            if inner.saved_placement.is_some() {
                return None;
            }
        }
    }
    let suggested = &*(lparam as *const RECT);
    log::trace!(
        "WM_DPICHANGED dpi={} suggested rect left={} top={} width={} height={}",
        LOWORD(wparam as u32),
        suggested.left,
        suggested.top,
        rect_width(suggested),
        rect_height(suggested)
    );
    SetWindowPos(
        hwnd,
        null_mut(),
        suggested.left,
        suggested.top,
        rect_width(suggested),
        rect_height(suggested),
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
    Some(0)
}

/// fork: 显示模式（分辨率/刷新率/显示器增减）变化后重新读取刷新率。
/// 返回 None 让 DefWindowProc 照常处理
unsafe fn wm_displaychange(
    hwnd: HWND,
    _msg: UINT,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    if let Some(inner) = rc_from_hwnd(hwnd) {
        // 广播消息可能在别的分支持有借用时同步送达，借不到就留给下次
        // 移动/尺寸变化时的 refresh_monitor_info(false) 兜底
        if let Ok(mut inner) = inner.try_borrow_mut() {
            inner.refresh_monitor_info(true);
        }
    }
    None
}

/// fork: 睡眠唤醒后显示驱动可能已经重置（Optimus 切换、独显重上电、
/// TDR），DWM 还在合成唤醒前的旧帧。主动失效窗口并派发 NeedRepaint，
/// 让下一帧的 is_context_lost 检测与 gpu_recovery 重建尽早发生，而不是
/// 等用户敲键才发现窗口黑掉。PBT_APMRESUMEAUTOMATIC 每次唤醒都发，
/// PBT_APMRESUMESUSPEND 只在用户触发的唤醒时额外发一次，多画一帧无妨
unsafe fn wm_powerbroadcast(
    hwnd: HWND,
    _msg: UINT,
    wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    if matches!(wparam, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND) {
        log::debug!("resumed from suspend (PBT {wparam:#x}); repainting");
        InvalidateRect(hwnd, null(), 0);
        if let Some(inner) = rc_from_hwnd(hwnd) {
            // 广播消息可能在别的分支持有借用时同步送达，借不到就靠上面
            // 的 InvalidateRect 产生的 WM_PAINT 兜底
            if let Ok(mut inner) = inner.try_borrow_mut() {
                inner.events.dispatch(WindowEvent::NeedRepaint);
            }
        }
    }
    None
}

/// 帧率上下限：0 会除零，超过 1000 已无意义且会让定时器空转
const MIN_FPS: u64 = 1;
const MAX_FPS: u64 = 1000;

/// fork: 计算节流用的最小帧间隔。`follows_display` 打开且读到有效刷新率
/// （>1Hz，0/1 是驱动的占位值）时用显示器刷新率，否则用 config.max_fps；
/// 两者都 clamp 到 1..=1000。用浮点求倒数，避免整除毫秒把 60fps 截成
/// 16ms（实际 62.5fps）、144fps 截成 6ms（166fps）
pub(crate) fn effective_frame_interval(
    config_max_fps: u64,
    follows_display: bool,
    monitor_hz: Option<u32>,
) -> std::time::Duration {
    let fps = match monitor_hz {
        Some(hz) if follows_display && hz > 1 => hz as u64,
        _ => config_max_fps,
    };
    let fps = fps.clamp(MIN_FPS, MAX_FPS);
    std::time::Duration::from_secs_f64(1.0 / fps as f64)
}

#[cfg(test)]
mod frame_interval_tests {
    use super::effective_frame_interval;
    use std::time::Duration;

    fn close_to(actual: Duration, expected: Duration) -> bool {
        let a = actual.as_secs_f64();
        let e = expected.as_secs_f64();
        (a - e).abs() < 1e-9
    }

    #[test]
    fn uses_config_when_switch_is_off() {
        let d = effective_frame_interval(60, false, Some(165));
        assert!(close_to(d, Duration::from_secs_f64(1.0 / 60.0)), "{d:?}");
    }

    #[test]
    fn follows_display_when_available() {
        let d = effective_frame_interval(60, true, Some(165));
        assert!(close_to(d, Duration::from_secs_f64(1.0 / 165.0)), "{d:?}");
    }

    #[test]
    fn falls_back_to_config_when_display_rate_unknown() {
        let d = effective_frame_interval(144, true, None);
        assert!(close_to(d, Duration::from_secs_f64(1.0 / 144.0)), "{d:?}");
    }

    #[test]
    fn placeholder_display_rates_are_ignored() {
        for hz in [0, 1] {
            let d = effective_frame_interval(60, true, Some(hz));
            assert!(
                close_to(d, Duration::from_secs_f64(1.0 / 60.0)),
                "{hz}: {d:?}"
            );
        }
    }

    #[test]
    fn zero_config_fps_clamps_to_one_frame_per_second() {
        let d = effective_frame_interval(0, false, None);
        assert!(close_to(d, Duration::from_secs(1)), "{d:?}");
    }

    #[test]
    fn huge_values_clamp_to_one_millisecond() {
        let d = effective_frame_interval(u64::MAX, false, None);
        assert!(close_to(d, Duration::from_millis(1)), "{d:?}");
        let d = effective_frame_interval(60, true, Some(u32::MAX));
        assert!(close_to(d, Duration::from_millis(1)), "{d:?}");
    }

    #[test]
    fn interval_is_not_truncated_to_whole_milliseconds() {
        // 1000/60 整除会得到 16ms；真实间隔应是 16.666…ms
        let d = effective_frame_interval(60, false, None);
        assert!(
            d > Duration::from_millis(16) && d < Duration::from_millis(17),
            "{d:?}"
        );
    }
}

unsafe fn wm_paint(hwnd: HWND, _msg: UINT, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let mut inner = inner.borrow_mut();

    if inner.paint_throttled {
        // fork: 节流期间必须把更新区域验证掉，否则 Win32 会在消息队列
        // 空闲时反复生成 WM_PAINT，主线程在节流窗口内空转跑满一个核。
        // invalidated 保持为真，由定时器回调在节流结束后补发 InvalidateRect
        inner.invalidated = true;
        ValidateRect(hwnd, null());
        return Some(0);
    }

    // fork: 节流 deadline 从本帧开始计
    inner.frame_start = Instant::now();

    let mut ps = PAINTSTRUCT {
        fErase: 0,
        fIncUpdate: 0,
        fRestore: 0,
        hdc: std::ptr::null_mut(),
        rcPaint: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rgbReserved: [0; 32],
    };
    let _ = BeginPaint(hwnd, &mut ps);
    if !inner.first_frame_presented {
        fill_unpresented_background(&inner.config, ps.hdc, &ps.rcPaint);
    }
    EndPaint(hwnd, &mut ps);

    inner.invalidated = false;
    // Ask the app to repaint in a bit
    inner.events.dispatch(WindowEvent::NeedRepaint);

    inner.paint_throttled = true;
    inner.schedule_paint_throttle();

    Some(0)
}

fn mods_and_buttons(wparam: WPARAM) -> (Modifiers, MouseButtons) {
    let mut modifiers = Modifiers::default();
    let mut buttons = MouseButtons::default();
    if wparam & MK_CONTROL != 0 {
        modifiers |= Modifiers::CTRL;
    }
    if wparam & MK_SHIFT != 0 {
        modifiers |= Modifiers::SHIFT;
    }
    if unsafe { GetKeyState(VK_MENU) } as u16 & 0x8000 != 0 {
        modifiers |= Modifiers::ALT;
    }
    if wparam & MK_LBUTTON != 0 {
        buttons |= MouseButtons::LEFT;
    }
    if wparam & MK_MBUTTON != 0 {
        buttons |= MouseButtons::MIDDLE;
    }
    if wparam & MK_RBUTTON != 0 {
        buttons |= MouseButtons::RIGHT;
    }
    // TODO: XBUTTON1 and XBUTTON2?
    (modifiers, buttons)
}

fn mouse_coords(lparam: LPARAM) -> Point {
    let point = MAKEPOINTS(lparam as _);
    Point::new(point.x as _, point.y as _)
}

fn nc_mouse_coords(hwnd: HWND, lparam: LPARAM) -> Point {
    let point = MAKEPOINTS(lparam as _);
    let point = ScreenPoint::new(point.x as _, point.y as _);
    screen_to_client(hwnd, point)
}

fn screen_to_client(hwnd: HWND, point: ScreenPoint) -> Point {
    let mut point = POINT {
        x: point.x.try_into().unwrap(),
        y: point.y.try_into().unwrap(),
    };
    unsafe { ScreenToClient(hwnd, &mut point as *mut _) };
    Point::new(point.x.try_into().unwrap(), point.y.try_into().unwrap())
}

fn client_to_screen(hwnd: HWND, point: Point) -> ScreenPoint {
    let mut point = POINT {
        x: point.x.try_into().unwrap(),
        y: point.y.try_into().unwrap(),
    };
    unsafe { ClientToScreen(hwnd, &mut point as *mut _) };
    ScreenPoint::new(point.x.try_into().unwrap(), point.y.try_into().unwrap())
}

fn apply_mouse_cursor(cursor: Option<CursorIcon>) {
    match cursor {
        None => unsafe {
            SetCursor(null_mut());
        },
        Some(cursor) => unsafe {
            SetCursor(LoadCursorW(null_mut(), mouse_cursor_id(cursor)));
        },
    }
}

fn mouse_cursor_id(cursor: CursorIcon) -> LPCWSTR {
    match cursor {
        CursorIcon::Cell | CursorIcon::Crosshair => IDC_CROSS,
        CursorIcon::EResize
        | CursorIcon::EwResize
        | CursorIcon::WResize
        | CursorIcon::ColResize => IDC_SIZEWE,
        CursorIcon::Grab | CursorIcon::Grabbing | CursorIcon::Pointer => IDC_HAND,
        CursorIcon::Help | CursorIcon::ContextMenu => IDC_HELP,
        CursorIcon::Move | CursorIcon::AllScroll | CursorIcon::AllResize => IDC_SIZEALL,
        CursorIcon::NResize
        | CursorIcon::NsResize
        | CursorIcon::SResize
        | CursorIcon::RowResize => IDC_SIZENS,
        CursorIcon::NeResize | CursorIcon::NeswResize | CursorIcon::SwResize => IDC_SIZENESW,
        CursorIcon::NoDrop | CursorIcon::NotAllowed => IDC_NO,
        CursorIcon::NwResize | CursorIcon::NwseResize | CursorIcon::SeResize => IDC_SIZENWSE,
        CursorIcon::Progress => IDC_APPSTARTING,
        CursorIcon::Text | CursorIcon::VerticalText => IDC_IBEAM,
        CursorIcon::Wait => IDC_WAIT,
        _ => IDC_ARROW,
    }
}

#[test]
fn cursor_icons_use_windows_cursors() {
    assert_eq!(mouse_cursor_id(CursorIcon::Pointer), IDC_HAND);
    assert_eq!(mouse_cursor_id(CursorIcon::NsResize), IDC_SIZENS);
    assert_eq!(mouse_cursor_id(CursorIcon::Text), IDC_IBEAM);
}

unsafe fn mouse_button(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    // To support dragging the window, capture when the left
    // button goes down and release when it goes up.
    // Without this, the drag state can be confused when dragging
    // the mouse up outside of the client area.
    if msg == WM_LBUTTONDOWN {
        SetCapture(hwnd);
    } else if msg == WM_LBUTTONUP {
        ReleaseCapture();
    }
    let (modifiers, mouse_buttons) = mods_and_buttons(wparam);
    let coords = mouse_coords(lparam);
    let event = MouseEvent {
        kind: match msg {
            WM_LBUTTONDOWN => MouseEventKind::Press(MousePress::Left),
            WM_LBUTTONUP => MouseEventKind::Release(MousePress::Left),
            WM_RBUTTONDOWN => MouseEventKind::Press(MousePress::Right),
            WM_RBUTTONUP => MouseEventKind::Release(MousePress::Right),
            WM_MBUTTONDOWN => MouseEventKind::Press(MousePress::Middle),
            WM_MBUTTONUP => MouseEventKind::Release(MousePress::Middle),
            _ => return None,
        },
        coords,
        screen_coords: client_to_screen(hwnd, coords),
        mouse_buttons,
        modifiers,
    };
    inner
        .borrow_mut()
        .events
        .dispatch(WindowEvent::MouseEvent(event));
    Some(0)
}

unsafe fn nc_mouse_button(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;

    let no_native_title_bar = no_native_title_bar(inner.borrow().config.window_decorations);
    if !no_native_title_bar {
        // Don't mess with this event unless we're doing our own custom
        // titlebar
        return None;
    }

    // To support dragging the window, capture when the left
    // button goes down and release when it goes up.
    // Without this, the drag state can be confused when dragging
    // the mouse up outside of the client area.

    if msg == WM_LBUTTONDOWN {
        SetCapture(hwnd);
    } else if msg == WM_LBUTTONUP {
        ReleaseCapture();
    }

    if wparam != HTMAXBUTTON as usize {
        return None;
    }

    let (modifiers, mouse_buttons) = mods_and_buttons(0);
    let coords = nc_mouse_coords(hwnd, lparam);

    let event = MouseEvent {
        kind: match msg {
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => MouseEventKind::Press(MousePress::Left),
            _ => return None,
        },
        coords,
        screen_coords: client_to_screen(hwnd, coords),
        mouse_buttons,
        modifiers,
    };
    inner
        .borrow_mut()
        .events
        .dispatch(WindowEvent::MouseEvent(event));
    Some(0)
}

unsafe fn mouse_move(hwnd: HWND, _msg: UINT, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let mut inner = inner.borrow_mut();

    if !inner.track_mouse_leave {
        inner.track_mouse_leave = true;

        let mut trk = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };

        inner.track_mouse_leave = TrackMouseEvent(&mut trk) == winapi::shared::minwindef::TRUE;
    }

    let (modifiers, mouse_buttons) = mods_and_buttons(wparam);
    let coords = mouse_coords(lparam);
    let event = MouseEvent {
        kind: MouseEventKind::Move,
        coords,
        screen_coords: client_to_screen(hwnd, coords),
        mouse_buttons,
        modifiers,
    };

    inner.events.dispatch(WindowEvent::MouseEvent(event));
    Some(0)
}

unsafe fn nc_mouse_move(hwnd: HWND, _msg: UINT, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let mut inner = inner.borrow_mut();

    if !inner.track_mouse_leave {
        inner.track_mouse_leave = true;

        let mut trk = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE | TME_NONCLIENT,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };

        inner.track_mouse_leave = TrackMouseEvent(&mut trk) == winapi::shared::minwindef::TRUE;
    }

    if wparam != HTMAXBUTTON as usize {
        return None;
    }

    let (modifiers, mouse_buttons) = mods_and_buttons(0);
    let coords = nc_mouse_coords(hwnd, lparam);

    let event = MouseEvent {
        kind: MouseEventKind::Move,
        coords,
        screen_coords: client_to_screen(hwnd, coords),
        mouse_buttons,
        modifiers,
    };

    inner.events.dispatch(WindowEvent::MouseEvent(event));
    inner.events.dispatch(WindowEvent::NeedRepaint);

    Some(0)
}

unsafe fn mouse_leave(hwnd: HWND, _msg: UINT, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let mut inner = inner.borrow_mut();

    inner.track_mouse_leave = false;
    inner.events.dispatch(WindowEvent::MouseLeave);

    Some(0)
}

/// fork: how far one wheel notch scrolls, as configured in the Mouse control
/// panel and reported by SystemParametersInfoW
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WheelScroll {
    /// this many lines (vertical wheel) or characters (horizontal wheel)
    Units(i32),
    /// WHEEL_PAGESCROLL: one notch scrolls a whole screen. The registry
    /// spells it "-1", which upstream multiplied in and so scrolled one
    /// line in the wrong direction.
    Page,
}

impl WheelScroll {
    fn from_spi(value: UINT) -> Self {
        if value == WHEEL_PAGESCROLL {
            Self::Page
        } else {
            Self::Units(value.min(i16::MAX as UINT) as i32)
        }
    }
}

/// Windows' default for both the vertical and the horizontal wheel
const DEFAULT_WHEEL_SCROLL: WheelScroll = WheelScroll::Units(3);

fn query_wheel_scroll(action: UINT) -> WheelScroll {
    let mut value: UINT = 0;
    let ok = unsafe { SystemParametersInfoW(action, 0, &mut value as *mut UINT as PVOID, 0) };
    if ok == 0 {
        DEFAULT_WHEEL_SCROLL
    } else {
        WheelScroll::from_spi(value)
    }
}

thread_local! {
    /// fork: (vertical, horizontal) wheel settings; None until first use and
    /// again after WM_SETTINGCHANGE, so a changed setting is picked up
    /// without restarting
    static WHEEL_SCROLL: std::cell::Cell<Option<(WheelScroll, WheelScroll)>> =
        std::cell::Cell::new(None);
}

fn wheel_scroll_settings() -> (WheelScroll, WheelScroll) {
    WHEEL_SCROLL.with(|cell| match cell.get() {
        Some(settings) => settings,
        None => {
            let settings = (
                query_wheel_scroll(SPI_GETWHEELSCROLLLINES),
                query_wheel_scroll(SPI_GETWHEELSCROLLCHARS),
            );
            cell.set(Some(settings));
            settings
        }
    })
}

fn invalidate_wheel_scroll_settings() {
    WHEEL_SCROLL.with(|cell| cell.set(None));
}

/// fork: rough count of text rows (or columns) that fit in `client_px`
/// pixels, used to turn WHEEL_PAGESCROLL into "one screen". The window
/// layer does not know the real cell metrics, so the cell size is estimated
/// from the configured font size and line height (a cell is about 1.2em
/// tall and 0.6em wide); the GUI clamps the viewport, so the estimate only
/// affects how far a page scroll moves.
fn estimate_page_units(
    client_px: i32,
    dpi: f64,
    font_size: f64,
    line_height: f64,
    horizontal: bool,
) -> i32 {
    let em_px = font_size * dpi / 72.0;
    let cell_px = if horizontal {
        em_px * 0.6
    } else {
        em_px * 1.2 * line_height
    };
    if !(cell_px.is_finite() && cell_px > 0.0) {
        return 1;
    }
    ((client_px.max(0) as f64 / cell_px).floor() as i32).max(1)
}

/// fork: turn a raw wheel delta into whole scroll units, carrying the
/// fraction to the next event (high resolution wheels and touchpads send
/// less than WHEEL_DELTA per message). Same rules as upstream, including
/// dropping the carried fraction when the direction changes, but done in
/// i32 so large settings or a page scroll cannot overflow.
/// Returns (units to scroll, new remainder).
fn accumulate_wheel(delta: i32, units_per_notch: i32, remainder: i32) -> (i32, i32) {
    let wheel_delta = WHEEL_DELTA as i32;
    let scaled = delta.saturating_mul(units_per_notch);
    let mut position = scaled / wheel_delta;
    let fraction = scaled % wheel_delta;
    let mut remainder = if remainder.signum() != fraction.signum() {
        0
    } else {
        remainder
    };
    remainder += fraction;
    position += remainder / wheel_delta;
    remainder %= wheel_delta;
    (position, remainder)
}

#[cfg(test)]
mod wheel_tests {
    use super::*;

    #[test]
    fn spi_values_map_to_wheel_scroll() {
        assert_eq!(WheelScroll::from_spi(3), WheelScroll::Units(3));
        assert_eq!(WheelScroll::from_spi(0), WheelScroll::Units(0));
        assert_eq!(WheelScroll::from_spi(WHEEL_PAGESCROLL), WheelScroll::Page);
        assert_eq!(
            WheelScroll::from_spi(1_000_000),
            WheelScroll::Units(i16::MAX as i32)
        );
    }

    #[test]
    fn whole_notches_scroll_whole_units() {
        assert_eq!(accumulate_wheel(120, 3, 0), (3, 0));
        assert_eq!(accumulate_wheel(-240, 3, 0), (-6, 0));
    }

    #[test]
    fn fractions_carry_over() {
        let (pos, rem) = accumulate_wheel(20, 3, 0);
        assert_eq!((pos, rem), (0, 60));
        let (pos, rem) = accumulate_wheel(20, 3, rem);
        assert_eq!((pos, rem), (1, 0));
    }

    #[test]
    fn direction_change_drops_the_carried_fraction() {
        assert_eq!(accumulate_wheel(-20, 3, 60), (0, -60));
    }

    #[test]
    fn large_settings_do_not_overflow() {
        assert_eq!(accumulate_wheel(120, i16::MAX as i32, 0).0, i16::MAX as i32);
        assert_eq!(
            accumulate_wheel(i16::MIN as i32, i32::MAX, 0).0,
            i32::MIN / 120
        );
    }

    #[test]
    fn page_estimate_scales_with_font_and_dpi() {
        // 12pt at 96dpi is a 16px em; rows are ~19.2px, columns ~9.6px
        assert_eq!(estimate_page_units(960, 96.0, 12.0, 1.0, false), 50);
        assert_eq!(estimate_page_units(960, 96.0, 12.0, 1.0, true), 100);
        assert_eq!(estimate_page_units(960, 144.0, 12.0, 1.0, false), 33);
        assert_eq!(estimate_page_units(960, 96.0, 12.0, 2.0, false), 25);
    }

    #[test]
    fn page_estimate_is_at_least_one() {
        assert_eq!(estimate_page_units(0, 96.0, 12.0, 1.0, false), 1);
        assert_eq!(estimate_page_units(-5, 96.0, 12.0, 1.0, false), 1);
        assert_eq!(estimate_page_units(500, 96.0, 0.0, 1.0, false), 1);
    }
}

impl WindowInner {
    /// fork: rows (or columns) in one screen, for WHEEL_PAGESCROLL
    fn wheel_page_units(&self, horizontal: bool) -> i32 {
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(self.hwnd.0, &mut rect);
        }
        let client_px = if horizontal {
            rect_width(&rect)
        } else {
            rect_height(&rect)
        };
        estimate_page_units(
            client_px,
            self.get_effective_dpi() as f64,
            self.config.font_size,
            self.config.line_height,
            horizontal,
        )
    }
}

unsafe fn mouse_wheel(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let (modifiers, mouse_buttons) = mods_and_buttons(wparam);
    // Wheel events return screen coordinates!
    let coords = mouse_coords(lparam);
    let screen_coords = ScreenPoint::new(coords.x, coords.y);
    let coords = screen_to_client(hwnd, screen_coords);
    let delta = GET_WHEEL_DELTA_WPARAM(wparam) as i32;
    let horizontal = msg == WM_MOUSEHWHEEL;
    let (vertical_setting, horizontal_setting) = wheel_scroll_settings();
    let setting = if horizontal {
        horizontal_setting
    } else {
        vertical_setting
    };

    let mut inner = inner.borrow_mut();
    let units_per_notch = match setting {
        WheelScroll::Units(units) => units,
        WheelScroll::Page => inner.wheel_page_units(horizontal),
    };
    let remainder = if horizontal {
        &mut inner.hscroll_remainder
    } else {
        &mut inner.vscroll_remainder
    };
    let (position, new_remainder) = accumulate_wheel(delta, units_per_notch, *remainder);
    *remainder = new_remainder;
    log::trace!(
        "mouse_wheel horizontal={} delta={} units_per_notch={} remainder={} pos={}",
        horizontal,
        delta,
        units_per_notch,
        new_remainder,
        position
    );
    if position == 0 {
        return Some(0);
    }
    let position = position.clamp(i16::MIN as i32, i16::MAX as i32) as i16;

    let event = MouseEvent {
        kind: if horizontal {
            MouseEventKind::HorzWheel(position)
        } else {
            MouseEventKind::VertWheel(position)
        },
        coords,
        screen_coords,
        mouse_buttons,
        modifiers,
    };
    inner.events.dispatch(WindowEvent::MouseEvent(event));
    Some(0)
}

/// Helper for managing the IME Manager
struct ImmContext {
    hwnd: HWND,
    imc: HIMC,
}

impl ImmContext {
    /// Obtain the IMM context; it will be released automatically
    /// when dropped
    pub fn get(hwnd: HWND) -> Self {
        Self {
            hwnd,
            imc: unsafe { ImmGetContext(hwnd) },
        }
    }

    /// Set the position of the IME candidate window relative to the cursor.
    pub fn set_candidate_window_position(&self, cursor: Rect) {
        let mut cf = CANDIDATEFORM {
            dwIndex: 0,
            // Don't draw the IME candidate window on the cursor
            // to prevent the window from hiding composition (preedit) string
            dwStyle: CFS_EXCLUDE,
            // cursor position the IME candidate window bases on
            ptCurrentPos: POINT {
                x: cursor.origin.x.max(0) as i32,
                y: cursor.origin.y.max(0) as i32,
            },
            // cursor rectangle the IME candidate window excludes
            rcArea: RECT {
                left: cursor.min_x().max(0) as i32,
                top: cursor.min_y().max(0) as i32,
                right: cursor.max_x().max(0) as i32,
                bottom: cursor.max_y().max(0) as i32,
            },
        };
        unsafe {
            ImmSetCandidateWindow(self.imc, &mut cf);
        }
    }

    /// Set the position of the IME composition window relative to the cursor.
    pub fn set_composition_window_position(&self, cursor: Rect) {
        let mut cf = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: POINT {
                x: cursor.origin.x.max(0) as i32,
                y: cursor.origin.y.max(0) as i32,
            },
            rcArea: RECT::default(),
        };
        unsafe {
            ImmSetCompositionWindow(self.imc, &mut cf);
        }
    }

    pub fn get_str(&self, which: DWORD) -> Result<String, OsString> {
        // This returns a size in bytes even though it is for a buffer of u16!
        let byte_size =
            unsafe { ImmGetCompositionStringW(self.imc, which, std::ptr::null_mut(), 0) };
        if byte_size > 0 {
            let word_size = byte_size as usize / 2;
            let mut wide_buf = vec![0u16; word_size];
            unsafe {
                ImmGetCompositionStringW(
                    self.imc,
                    which,
                    wide_buf.as_mut_ptr() as *mut _,
                    byte_size as u32,
                )
            };
            OsString::from_wide(&wide_buf).into_string()
        } else {
            Ok(String::new())
        }
    }
}

impl Drop for ImmContext {
    fn drop(&mut self) {
        unsafe {
            ImmReleaseContext(self.hwnd, self.imc);
        }
    }
}

unsafe fn ime_set_context(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let use_system_rendering = {
        let inner = rc_from_hwnd(hwnd)?;
        let inner = inner.borrow();
        inner.config.ime_preedit_rendering == ImePreeditRendering::System
    };

    if use_system_rendering {
        return None;
    }

    // Don't show system CompositionWindow because application itself draws it.
    // Note: DefWindowProcW may trigger other window messages, so we must
    // release the borrow before calling it.
    let lparam = lparam & !(ISC_SHOWUICOMPOSITIONWINDOW as LPARAM);
    let result = DefWindowProcW(hwnd, msg, wparam, lparam);
    Some(result)
}

/// fork: place the IME windows before the IME shows them for a new
/// composition. Returns None so that DefWindowProc carries on as before.
unsafe fn ime_start_composition(
    hwnd: HWND,
    _msg: UINT,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    if let Some(inner) = rc_from_hwnd(hwnd) {
        if let Ok(mut inner) = inner.try_borrow_mut() {
            inner.replay_ime_window_position();
        }
    }
    None
}

unsafe fn ime_end_composition(
    hwnd: HWND,
    _msg: UINT,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    // IME was cancelled
    let inner = rc_from_hwnd(hwnd)?;
    let mut inner = inner.borrow_mut();

    if inner.config.ime_preedit_rendering == ImePreeditRendering::System {
        return None;
    }

    inner
        .events
        .dispatch(WindowEvent::AdviseDeadKeyStatus(DeadKeyStatus::None));
    Some(1)
}

unsafe fn ime_composition(
    hwnd: HWND,
    _msg: UINT,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let mut inner = inner.borrow_mut();

    if inner.config.ime_preedit_rendering == ImePreeditRendering::System {
        return None;
    }

    let imc = ImmContext::get(hwnd);

    let lparam = lparam as DWORD;

    if lparam == 0 {
        // IME was cancelled
        inner
            .events
            .dispatch(WindowEvent::AdviseDeadKeyStatus(DeadKeyStatus::None));
        return Some(1);
    }

    if lparam & GCS_RESULTSTR == 0 {
        // No finished result; continue with the default
        // processing
        if let Ok(composing) = imc.get_str(GCS_COMPSTR) {
            inner
                .events
                .dispatch(WindowEvent::AdviseDeadKeyStatus(DeadKeyStatus::Composing(
                    composing,
                )));
        }
        // We will show the composing string ourselves.
        // Suppress the default composition display.
        return Some(1);
    }

    match imc.get_str(GCS_RESULTSTR) {
        Ok(s) if !s.is_empty() => {
            let key = KeyEvent {
                key: KeyCode::Composed(s),
                modifiers: Modifiers::NONE,
                leds: KeyboardLedStatus::empty(),
                repeat_count: 1,
                key_is_down: true,
                raw: None,
                win32_uni_char: None,
            };
            inner
                .events
                .dispatch(WindowEvent::AdviseDeadKeyStatus(DeadKeyStatus::None));
            inner.events.dispatch(WindowEvent::KeyEvent(key));

            // fork: the IME can commit a result and start the next
            // composition in the same message (typing on after a commit);
            // pick up the new composition string instead of dropping it
            if lparam & GCS_COMPSTR != 0 {
                if let Ok(composing) = imc.get_str(GCS_COMPSTR) {
                    if !composing.is_empty() {
                        inner.events.dispatch(WindowEvent::AdviseDeadKeyStatus(
                            DeadKeyStatus::Composing(composing),
                        ));
                    }
                }
            }

            return Some(1);
        }
        Ok(_) => {}
        Err(_) => eprintln!("cannot represent IME as unicode string!?"),
    };
    None
}

/// Holds information about the current keyboard layout.
/// This is used to determine whether the layout includes
/// an AltGr key or just has a regular Right-Alt key,
/// as well as to build out information about dead keys.
struct KeyboardLayoutInfo {
    layout: HKL,
    has_alt_gr: bool,
    dead_keys: HashMap<(Modifiers, u8), DeadKey>,
}

#[derive(Debug)]
struct DeadKey {
    dead_char: char,
    _vk: u8,
    _mods: Modifiers,
    map: HashMap<(Modifiers, u8), char>,
}

#[derive(Debug)]
enum ResolvedDeadKey {
    InvalidDeadKey,
    Combined(char),
    InvalidCombination(char),
}

impl KeyboardLayoutInfo {
    pub fn new() -> Self {
        Self {
            layout: std::ptr::null_mut(),
            has_alt_gr: false,
            dead_keys: HashMap::new(),
        }
    }

    unsafe fn clear_key_state() {
        let mut out = [0u16; 16];
        let state = [0u8; 256];
        let scan = MapVirtualKeyW(VK_DECIMAL as _, MAPVK_VK_TO_VSC);
        // keep clocking the state to clear out its effects
        while ToUnicode(
            VK_DECIMAL as _,
            scan,
            state.as_ptr(),
            out.as_mut_ptr(),
            out.len() as i32,
            0,
        ) < 0
        {}
    }

    /// Probe to detect whether an AltGr key is present.
    /// This is done by synthesizing a keyboard state with control and alt
    /// pressed and then testing the virtual key presses.  If we find that
    /// one of these yields a single unicode character output then we assume that
    /// it does have AltGr.
    unsafe fn probe_alt_gr(&mut self) {
        self.has_alt_gr = false;

        let mut state = [0u8; 256];
        state[VK_CONTROL as usize] = 0x80;
        state[VK_MENU as usize] = 0x80;

        for vk in 0..=255u32 {
            if vk == VK_PACKET as u32 {
                // Avoid false positives
                continue;
            }

            let mut out = [0u16; 16];
            let ret = ToUnicode(vk, 0, state.as_ptr(), out.as_mut_ptr(), out.len() as i32, 0);
            if ret == 1 {
                self.has_alt_gr = true;
                break;
            }

            if ret == -1 {
                // Dead key.
                // keep clocking the state to clear out its effects
                while ToUnicode(vk, 0, state.as_ptr(), out.as_mut_ptr(), out.len() as i32, 0) < 0 {}
            }
        }
    }

    fn apply_mods(mods: Modifiers, state: &mut [u8; 256]) {
        if mods.contains(Modifiers::SHIFT) {
            state[VK_SHIFT as usize] = 0x80;
        }
        if mods.contains(Modifiers::CTRL) || mods.contains(Modifiers::RIGHT_ALT) {
            state[VK_CONTROL as usize] = 0x80;
        }
        if mods.contains(Modifiers::RIGHT_ALT) || mods.contains(Modifiers::ALT) {
            state[VK_MENU as usize] = 0x80;
        }
    }

    /// Probe the keymap to figure out which keys are dead keys
    unsafe fn probe_dead_keys(&mut self) {
        self.dead_keys.clear();

        let shift_states = [
            Modifiers::NONE,
            Modifiers::SHIFT,
            Modifiers::SHIFT | Modifiers::CTRL,
            Modifiers::ALT,
            Modifiers::RIGHT_ALT, // AltGr
        ];

        for &mods in &shift_states {
            let mut state = [0u8; 256];
            Self::apply_mods(mods, &mut state);

            for vk in 0..=255u32 {
                if vk == VK_PACKET as u32 {
                    // Avoid false positives
                    continue;
                }

                let scan = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC);

                Self::clear_key_state();
                let mut out = [0u16; 16];
                let ret = ToUnicode(
                    vk,
                    scan,
                    state.as_ptr(),
                    out.as_mut_ptr(),
                    out.len() as i32,
                    0,
                );

                if ret != -1 {
                    continue;
                }

                // Found a Dead key.
                let dead_char = std::char::from_u32_unchecked(out[0] as u32);

                let mut map = HashMap::new();

                for &smod in &shift_states {
                    let mut second_state = [0u8; 256];
                    Self::apply_mods(smod, &mut second_state);

                    for ik in 0..=255u32 {
                        // Re-initiate the dead key starting state
                        Self::clear_key_state();
                        if ToUnicode(
                            vk,
                            scan,
                            state.as_ptr(),
                            out.as_mut_ptr(),
                            out.len() as i32,
                            0,
                        ) != -1
                        {
                            continue;
                        }

                        let scan = MapVirtualKeyW(ik, MAPVK_VK_TO_VSC);

                        let ret = ToUnicode(
                            ik,
                            scan,
                            second_state.as_ptr(),
                            out.as_mut_ptr(),
                            out.len() as i32,
                            0,
                        );

                        if ret == 1 {
                            // Found a combination
                            let c = std::char::from_u32_unchecked(out[0] as u32);
                            // clock through again to get the base
                            ToUnicode(
                                ik,
                                scan,
                                second_state.as_ptr(),
                                out.as_mut_ptr(),
                                out.len() as i32,
                                0,
                            );
                            let base = std::char::from_u32_unchecked(out[0] as u32);

                            if ((smod == Modifiers::CTRL)
                                || (smod == Modifiers::CTRL | Modifiers::SHIFT))
                                && c == base
                                && (c as u32) < 0x20
                            {
                                continue;
                            }

                            log::trace!(
                                "{:?}: {:?} {:?} + {:?} {:?} -> {:?} base={:?}",
                                dead_char,
                                mods,
                                vk,
                                smod,
                                ik,
                                c,
                                base
                            );

                            map.insert((smod, ik as u8), c);
                        }
                    }
                }

                self.dead_keys.insert(
                    (mods, vk as u8),
                    DeadKey {
                        dead_char,
                        _mods: mods,
                        _vk: vk as u8,
                        map,
                    },
                );
            }
        }
        Self::clear_key_state();
    }

    unsafe fn update(&mut self) {
        let current_layout = GetKeyboardLayout(0);
        if current_layout == self.layout {
            // Avoid recomputing this if the layout hasn't changed
            return;
        }

        let mut saved_state = [0u8; 256];
        if GetKeyboardState(saved_state.as_mut_ptr()) == 0 {
            return;
        }

        self.probe_alt_gr();
        self.probe_dead_keys();
        log::trace!("dead_keys: {:#x?}", self.dead_keys);

        SetKeyboardState(saved_state.as_mut_ptr());
        self.layout = current_layout;
    }

    pub fn has_alt_gr(&mut self) -> bool {
        unsafe {
            self.update();
        }
        self.has_alt_gr
    }

    /// Similar to Modifiers::remove_positional_mods except that it preserves
    /// RIGHT_ALT
    fn fixup_mods(mods: Modifiers) -> Modifiers {
        mods - (Modifiers::LEFT_SHIFT
            | Modifiers::RIGHT_SHIFT
            | Modifiers::LEFT_CTRL
            | Modifiers::RIGHT_CTRL
            | Modifiers::LEFT_ALT)
    }

    pub fn is_dead_key_leader(&mut self, mods: Modifiers, vk: u32) -> Option<char> {
        unsafe {
            self.update();
        }
        if vk <= (u8::MAX as u32) {
            self.dead_keys
                .get(&(Self::fixup_mods(mods), vk as u8))
                .map(|dead| dead.dead_char)
        } else {
            None
        }
    }

    pub fn resolve_dead_key(
        &mut self,
        leader: (Modifiers, u32),
        key: (Modifiers, u32),
    ) -> ResolvedDeadKey {
        unsafe {
            self.update();
        }
        if leader.1 <= (u8::MAX as u32) && key.1 <= (u8::MAX as u32) {
            if let Some(dead) = self
                .dead_keys
                .get(&(Self::fixup_mods(leader.0), leader.1 as u8))
            {
                if let Some(c) = dead
                    .map
                    .get(&(Self::fixup_mods(key.0), key.1 as u8))
                    .map(|&c| c)
                {
                    ResolvedDeadKey::Combined(c)
                } else {
                    ResolvedDeadKey::InvalidCombination(dead.dead_char)
                }
            } else {
                ResolvedDeadKey::InvalidDeadKey
            }
        } else {
            ResolvedDeadKey::InvalidDeadKey
        }
    }
}

/// Generate a MSG and call TranslateMessage upon it
unsafe fn translate_message(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) {
    TranslateMessage(&MSG {
        hwnd,
        message: msg,
        wParam: wparam,
        lParam: lparam,
        pt: POINT { x: 0, y: 0 },
        time: GetTickCount(),
    });
}

unsafe fn key(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let mut inner = inner.borrow_mut();
    let repeat = (lparam & 0xffff) as u16;
    let scan_code = ((lparam >> 16) & 0xff) as u8;
    let releasing = (lparam & (1 << 31)) != 0;
    let ime_active = wparam == VK_PROCESSKEY as WPARAM;
    let phys_code = super::keycodes::vkey_to_phys(wparam);

    let alt_pressed = (lparam & (1 << 29)) != 0;
    let is_extended = (lparam & (1 << 24)) != 0;
    let was_down = (lparam & (1 << 30)) != 0;
    let label = match msg {
        WM_CHAR => "WM_CHAR",
        WM_IME_CHAR => "WM_IME_CHAR",
        WM_KEYDOWN => "WM_KEYDOWN",
        WM_KEYUP => "WM_KEYUP",
        WM_SYSKEYUP => "WM_SYSKEYUP",
        WM_SYSKEYDOWN => "WM_SYSKEYDOWN",
        WM_SYSCHAR => "WM_SYSCHAR",
        WM_DEADCHAR => "WM_DEADCHAR",
        _ => "WAT",
    };
    log::trace!(
        "{} c=`{}` repeat={} scan={} is_extended={} alt_pressed={} was_down={} \
             releasing={} IME={} dead_pending={:?}",
        label,
        wparam,
        repeat,
        scan_code,
        is_extended,
        alt_pressed,
        was_down,
        releasing,
        ime_active,
        inner.dead_pending,
    );

    if ime_active {
        // If the IME is active, allow Windows to perform default processing
        // to drive it forwards.  It will generate a call to `ime_composition`
        // or `ime_endcomposition` when it completes.

        if msg == WM_KEYDOWN {
            // Release the borrow before calling translate_message:
            // TranslateMessage can trigger other window messages (like WM_SIZE)
            // via CtfImeCreateInputContext, which would otherwise cause a
            // RefCell borrow conflict while inner is still borrowed.
            drop(inner);
            // Explicitly allow the built-in translation to occur for the IME
            translate_message(hwnd, msg, wparam, lparam);
            return Some(0);
        }

        return None;
    }

    if msg == WM_DEADCHAR {
        // Ignore WM_DEADCHAR; we only care about the resultant WM_CHAR
        return Some(0);
    }

    let keys = {
        let mut keys = [0u8; 256];
        GetKeyboardState(keys.as_mut_ptr());
        keys
    };

    let mut modifiers = Modifiers::default();
    if keys[VK_SHIFT as usize] & 0x80 != 0 {
        modifiers |= Modifiers::SHIFT;
    }
    if keys[VK_LSHIFT as usize] & 0x80 != 0 {
        modifiers |= Modifiers::LEFT_SHIFT;
    }
    if keys[VK_RSHIFT as usize] & 0x80 != 0 {
        modifiers |= Modifiers::RIGHT_SHIFT;
    }
    if keys[VK_LCONTROL as usize] & 0x80 != 0 {
        modifiers |= Modifiers::LEFT_CTRL;
    }
    if keys[VK_RCONTROL as usize] & 0x80 != 0 {
        modifiers |= Modifiers::RIGHT_CTRL;
    }
    modifiers.set(Modifiers::ENHANCED_KEY, is_extended);

    if inner.keyboard_info.has_alt_gr()
        && (keys[VK_RMENU as usize] & 0x80 != 0)
        && (keys[VK_CONTROL as usize] & 0x80 != 0)
    {
        // AltGr is pressed; while AltGr is on the RHS of the keyboard
        // is not the same thing as right-alt.
        // Windows sets RMENU and CONTROL to indicate AltGr and we
        // have to keep these in the key state in order for ToUnicode
        // to map the key correctly.
        // We set RIGHT_ALT as a hint to ourselves that AltGr is in
        // use (we use regular ALT otherwise) so that our dead key
        // resolution can do the right thing.
        modifiers |= Modifiers::RIGHT_ALT;
    } else if inner.keyboard_info.has_alt_gr()
        && inner.config.treat_left_ctrlalt_as_altgr
        && (keys[VK_MENU as usize] & 0x80 != 0)
        && (keys[VK_CONTROL as usize] & 0x80 != 0)
    {
        // When running inside a VNC session, VNC emulates the AltGr keypresses
        // by sending plain VK_MENU (rather than VK_RMENU) + VK_CONTROL.
        // For compatibility with that the option `treat_left_ctrlalt_as_altgr` allows
        // to treat MENU+CONTROL as equivalent to RMENU+CONTROL (AltGr) even though it is
        // technically a lossy transformation.
        //
        // We only do that when the keyboard layout has AltGr and the option is enabled,
        // so that we don't screw things up by default or for other keyboard layouts.
        // See issue #392 & #472 for some more context.
        modifiers |= Modifiers::RIGHT_ALT;
    } else {
        if keys[VK_CONTROL as usize] & 0x80 != 0 {
            modifiers |= Modifiers::CTRL;
        }
        if keys[VK_MENU as usize] & 0x80 != 0 {
            modifiers |= Modifiers::ALT;
        }
    }
    if keys[VK_LWIN as usize] & 0x80 != 0 || keys[VK_RWIN as usize] & 0x80 != 0 {
        modifiers |= Modifiers::SUPER;
    }

    let mut leds = KeyboardLedStatus::empty();
    if keys[VK_CAPITAL as usize] & 1 != 0 {
        leds |= KeyboardLedStatus::CAPS_LOCK;
    }
    if keys[VK_NUMLOCK as usize] & 1 != 0 {
        leds |= KeyboardLedStatus::NUM_LOCK;
    }

    let handled_raw = Handled::new();
    let raw_key_event = RawKeyEvent {
        key: match phys_code {
            Some(phys) => KeyCode::Physical(phys),
            None => KeyCode::RawCode(wparam as _),
        },
        phys_code,
        raw_code: wparam as _,
        scan_code: scan_code as _,
        leds,
        modifiers,
        repeat_count: 1,
        key_is_down: !releasing,
        handled: handled_raw.clone(),
    };

    let (key, win32_uni_char) = if msg == WM_IME_CHAR || msg == WM_CHAR {
        // If we were sent a character by the IME, some other apps,
        // or by ourselves via TranslateMessage, then take that
        // value as-is.
        (
            Some(KeyCode::Char(std::char::from_u32_unchecked(wparam as u32))),
            None,
        )
    } else {
        // Otherwise we're dealing with a raw key message.
        // ToUnicode has frustrating statefulness so we take care to
        // call it only when we think it will give consistent results.

        inner
            .events
            .dispatch(WindowEvent::RawKeyEvent(raw_key_event.clone()));
        if handled_raw.is_handled() {
            // Cancel any pending dead key
            if inner.dead_pending.take().is_some() {
                inner
                    .events
                    .dispatch(WindowEvent::AdviseDeadKeyStatus(DeadKeyStatus::None));
            }
            log::trace!("raw key was handled; not processing further");
            return Some(0);
        }

        let is_modifier_only = phys_code.map(|p| p.is_modifier()).unwrap_or(false);
        if is_modifier_only {
            // If this event is only modifiers then don't ask the system
            // for further resolution, as we don't want ToUnicode to
            // perturb its inscrutable global state.
            // Modifier-only keypresses are reported as NUL when using win32 input mode.
            (phys_code.map(|p| p.to_key_code()), Some('\x00'))
        } else {
            // If we think this might be a dead key, process it for ourselves.
            // Our KeyboardLayoutInfo struct probed the layout for the key
            // combinations that start a dead key sequence, as well as those
            // that are valid end states for dead keys, so we can resolve
            // these for ourselves in a couple of quick hash lookups.
            let vk = wparam as u32;

            if releasing && inner.dead_pending.is_some() {
                // Don't care about key-up events while processing dead keys
                return Some(0);
            }

            // If we previously had the start of a dead key...
            let dead = if let Some(leader) = inner.dead_pending.take() {
                inner
                    .events
                    .dispatch(WindowEvent::AdviseDeadKeyStatus(DeadKeyStatus::None));
                // look to see how the current event resolves it
                match inner
                    .keyboard_info
                    .resolve_dead_key(leader, (modifiers, vk))
                {
                    // Valid combination produces a single character
                    ResolvedDeadKey::Combined(c) => Some(KeyCode::Char(c)),
                    ResolvedDeadKey::InvalidCombination(c) => {
                        // An invalid combination results in the deferred
                        // keypress triggering the original key first,
                        // and then we process the current key.

                        // Emit an event for the leader of the failed
                        // dead key combination
                        let key = KeyEvent {
                            key: KeyCode::Char(c),
                            modifiers,
                            leds,
                            repeat_count: 1,
                            key_is_down: !releasing,
                            win32_uni_char: Some(c),
                            raw: Some(RawKeyEvent {
                                scan_code: 0,
                                ..raw_key_event.clone()
                            }),
                        }
                        .normalize_shift()
                        .resurface_positional_modifier_key()
                        .normalize_ctrl();

                        inner.events.dispatch(WindowEvent::KeyEvent(key.clone()));

                        // And then we'll perform normal processing on the
                        // current key press
                        if let Some(new_dead_char) =
                            inner.keyboard_info.is_dead_key_leader(modifiers, vk)
                        {
                            if new_dead_char != c {
                                // Happens to be the start of its own new,
                                // different, dead key sequence
                                inner.dead_pending.replace((modifiers, vk));
                                return Some(0);
                            }

                            // They pressed the same dead key twice,
                            // emit the underlying char again and call
                            // it done.
                            // <https://github.com/wezterm/wezterm/issues/1729>
                            inner.events.dispatch(WindowEvent::KeyEvent(key.clone()));
                            return Some(0);
                        }

                        // We don't know; allow normal ToUnicode processing
                        None
                    }

                    // We thought we had a dead key last time around,
                    // but this time it didn't resolve.  Most likely
                    // because the keyboard layout changed in the middle
                    // of the keypress.
                    // We're effectively swallowing the original dead
                    // key event here, but we could potentially re-process
                    // the original and current one here if needed.
                    // Seems like a real edge case.
                    ResolvedDeadKey::InvalidDeadKey => None,
                }
            } else if let Some(c) = inner.keyboard_info.is_dead_key_leader(modifiers, vk) {
                if releasing {
                    // Don't care about key-up events while processing dead keys
                    return Some(0);
                }

                // They pressed a dead key.
                // If they want dead key processing, then record that and
                // wait for a subsequent keypress.
                if inner.config.use_dead_keys {
                    inner.dead_pending.replace((modifiers, vk));
                    inner.events.dispatch(WindowEvent::AdviseDeadKeyStatus(
                        DeadKeyStatus::Composing(c.to_string()),
                    ));
                    return Some(0);
                }
                // They don't want dead keys; just return the base character
                Some(KeyCode::Char(c))
            } else {
                // Not a dead key as far as we know
                None
            };

            if dead.is_some() {
                (dead, None)
            } else {
                // We get here for the various UP (but not DOWN as we shortcircuit
                // those above) messages.
                // We perform conversion to unicode for ourselves,
                // rather than calling TranslateMessage to do it for us,
                // so that we have tighter control over the key processing.
                let mut out = [0u16; 16];

                let win32_uni_char = {
                    let res = ToUnicode(
                        wparam as u32,
                        scan_code as u32,
                        keys.as_ptr(),
                        out.as_mut_ptr(),
                        out.len() as i32,
                        0,
                    );

                    match res {
                        1 => Some(std::char::from_u32_unchecked(out[0] as u32)),
                        0 => Some('\x00'),
                        _ => None,
                    }
                };

                let mut keys = keys;
                // If control is pressed, clear that out and remember it in our
                // own set of modifiers.
                // We used to also remove shift from this set, but it impacts
                // handling of eg: ctrl+shift+' (which is equivalent to ctrl+" in a US English
                // layout.
                // The shift normalization is now handled by the normalize_shift() method.
                if modifiers.contains(Modifiers::CTRL) {
                    keys[VK_CONTROL as usize] = 0;
                    keys[VK_LCONTROL as usize] = 0;
                    keys[VK_RCONTROL as usize] = 0;
                }

                let res = ToUnicode(
                    wparam as u32,
                    scan_code as u32,
                    keys.as_ptr(),
                    out.as_mut_ptr(),
                    out.len() as i32,
                    0,
                );

                let key = match res {
                    1 => Some(KeyCode::Char(std::char::from_u32_unchecked(out[0] as u32))),
                    // No mapping, so use our raw info
                    0 => {
                        log::trace!(
                            "ToUnicode had no mapping for {:?} wparam={}",
                            phys_code,
                            wparam
                        );
                        phys_code.map(|p| p.to_key_code())
                    }
                    _ => {
                        // dead key: if our dead key mapping in KeyboardLayoutInfo was
                        // correct, we shouldn't be able to get here as we should have
                        // landed in the dead key case above.
                        // If somehow we do get here, we don't have a valid mapping
                        // as -1 indicates the start of a dead key sequence,
                        // and any other n > 1 indicates an ambiguous expansion.
                        // Either way, indicate that we don't have a valid result.
                        log::error!(
                            "unexpected dead key expansion: \
                             modifiers={:?} vk={:?} res={} releasing={} {:?}",
                            modifiers,
                            vk,
                            res,
                            releasing,
                            out
                        );
                        KeyboardLayoutInfo::clear_key_state();
                        None
                    }
                };

                (key, win32_uni_char)
            }
        }
    };

    if let Some(key) = key {
        // FIXME: verify this behavior: Urgh, special case for ctrl and non-latin layouts.
        // In order to avoid a situation like #678, if CTRL is the only
        // modifier and we've got composed text, then discard the composed
        // text.
        let key = KeyEvent {
            key,
            modifiers,
            leds,
            repeat_count: repeat,
            key_is_down: !releasing,
            win32_uni_char,
            raw: Some(raw_key_event),
        }
        .normalize_shift();

        // Special case for ALT-space to show the system menu, and
        // ALT-F4 to close the window.
        if key.modifiers == Modifiers::ALT
            && (key.key == KeyCode::Char(' ') || key.key == KeyCode::Function(4))
        {
            translate_message(hwnd, msg, wparam, lparam);
            return None;
        }

        inner.events.dispatch(WindowEvent::KeyEvent(key));
        return Some(0);
    }
    None
}

unsafe fn drop_files(hwnd: HWND, _msg: UINT, wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
    let inner = rc_from_hwnd(hwnd)?;
    let h_drop = wparam as HDROP;

    // Get the number of files dropped
    let file_count = DragQueryFileW(h_drop, 0xFFFFFFFF, null_mut(), 0);

    let mut filenames: Vec<PathBuf> = Vec::with_capacity(file_count as usize);

    for idx in 0..file_count {
        // The returned size of buffer is in characters, not including the terminating null character
        let buf_size = DragQueryFileW(h_drop, idx, null_mut(), 0);
        if buf_size > 0 {
            // Windows will truncate the filename and add null terminator if space isn't enough
            let buf_size = buf_size as usize + 1;
            let mut wide_buf = vec![0u16; buf_size];
            DragQueryFileW(h_drop, idx, wide_buf.as_mut_ptr(), wide_buf.len() as u32);
            wide_buf.pop(); // Drops the null terminator
            filenames.push(OsString::from_wide(&wide_buf).into());
        }
    }

    let mut inner = inner.borrow_mut();
    inner.events.dispatch(WindowEvent::DroppedFile(filenames));

    DragFinish(h_drop);
    Some(0)
}

unsafe fn do_wnd_proc(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_NCCREATE => wm_nccreate(hwnd, msg, wparam, lparam),
        WM_NCDESTROY => wm_ncdestroy(hwnd, msg, wparam, lparam),
        WM_NCCALCSIZE => wm_nccalcsize(hwnd, msg, wparam, lparam),
        WM_NCHITTEST => wm_nchittest(hwnd, msg, wparam, lparam),
        WM_PAINT => wm_paint(hwnd, msg, wparam, lparam),
        WM_ENTERSIZEMOVE | WM_EXITSIZEMOVE => wm_enter_exit_size_move(hwnd, msg, wparam, lparam),
        WM_WINDOWPOSCHANGED => wm_windowposchanged(hwnd, msg, wparam, lparam),
        WM_SETFOCUS => wm_set_focus(hwnd, msg, wparam, lparam),
        WM_KILLFOCUS => wm_kill_focus(hwnd, msg, wparam, lparam),
        WM_DEADCHAR | WM_KEYDOWN | WM_KEYUP | WM_SYSCHAR | WM_CHAR | WM_IME_CHAR | WM_SYSKEYUP
        | WM_SYSKEYDOWN => key(hwnd, msg, wparam, lparam),
        WM_SIZING => {
            // Allow events to be processed during live resize
            crate::spawn::SPAWN_QUEUE.run();
            None
        }
        WM_SETTINGCHANGE => {
            // fork: the wheel scroll settings may be what changed
            invalidate_wheel_scroll_settings();
            apply_theme(hwnd)
        }
        WM_DWMCOMPOSITIONCHANGED => apply_theme(hwnd),
        WM_DISPLAYCHANGE => wm_displaychange(hwnd, msg, wparam, lparam),
        WM_POWERBROADCAST => wm_powerbroadcast(hwnd, msg, wparam, lparam),
        WM_DPICHANGED => wm_dpichanged(hwnd, msg, wparam, lparam),
        WM_IME_SETCONTEXT => ime_set_context(hwnd, msg, wparam, lparam),
        WM_IME_STARTCOMPOSITION => ime_start_composition(hwnd, msg, wparam, lparam),
        WM_IME_COMPOSITION => ime_composition(hwnd, msg, wparam, lparam),
        WM_IME_ENDCOMPOSITION => ime_end_composition(hwnd, msg, wparam, lparam),
        WM_MOUSEMOVE => mouse_move(hwnd, msg, wparam, lparam),
        WM_MOUSELEAVE => mouse_leave(hwnd, msg, wparam, lparam),
        WM_MOUSEHWHEEL | WM_MOUSEWHEEL => mouse_wheel(hwnd, msg, wparam, lparam),
        WM_LBUTTONDBLCLK | WM_RBUTTONDBLCLK | WM_MBUTTONDBLCLK | WM_LBUTTONDOWN | WM_LBUTTONUP
        | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP => {
            mouse_button(hwnd, msg, wparam, lparam)
        }
        WM_DROPFILES => drop_files(hwnd, msg, wparam, lparam),
        WM_ERASEBKGND => Some(1),
        WM_CLOSE => {
            if let Some(inner) = rc_from_hwnd(hwnd) {
                let mut inner = inner.borrow_mut();
                inner.events.dispatch(WindowEvent::CloseRequested);
                // Don't let it close
                return Some(0);
            }
            None
        }
        _ => {
            if matches!(
                msg,
                WM_NCMOUSEMOVE | WM_NCMOUSELEAVE | WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK
            ) {
                let use_snap_layouts = !*IS_WIN10;
                if use_snap_layouts {
                    return match msg {
                        WM_NCMOUSEMOVE => nc_mouse_move(hwnd, msg, wparam, lparam),
                        WM_NCMOUSELEAVE => mouse_leave(hwnd, msg, wparam, lparam),
                        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
                            nc_mouse_button(hwnd, msg, wparam, lparam)
                        }
                        _ => None,
                    };
                }
            }

            None
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match std::panic::catch_unwind(|| {
        do_wnd_proc(hwnd, msg, wparam, lparam)
            .unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam))
    }) {
        Ok(result) => result,
        Err(e) => {
            log::error!("caught {:?}", e);
            std::process::exit(1)
        }
    }
}
