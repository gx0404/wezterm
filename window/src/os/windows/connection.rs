//! The connection to the GUI subsystem
use super::{FrameTick, HWindow, WindowInner};
use crate::connection::ConnectionOps;
use crate::screen::{ScreenInfo, Screens};
use crate::spawn::*;
use crate::{Appearance, ScreenRect};
use anyhow::Context;
use config::ConfigHandle;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::time::{Duration, Instant};
use winapi::shared::minwindef::*;
use winapi::shared::ntdef::LARGE_INTEGER;
use winapi::shared::windef::*;
use winapi::shared::winerror::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
use winapi::um::handleapi::CloseHandle;
use winapi::um::shellscalingapi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use winapi::um::synchapi::{CancelWaitableTimer, CreateWaitableTimerExW, SetWaitableTimerEx};
use winapi::um::winbase::{INFINITE, WAIT_OBJECT_0};
use winapi::um::wingdi::{
    DEVMODEW, DISPLAY_DEVICEW, DM_DISPLAYFREQUENCY, QDC_ONLY_ACTIVE_PATHS, QDC_VIRTUAL_MODE_AWARE,
};
use winapi::um::winnt::{HANDLE, TIMER_ALL_ACCESS};
use winapi::um::winuser::*;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME,
};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

pub struct Connection {
    event_handle: HANDLE,
    /// fork: 高精度帧定时器，驱动所有窗口的 max_fps 节流 deadline；
    /// Win10 1803 以前创建失败为 None，此时 wm_paint 回退 async_io 定时器
    frame_timer: Option<FrameTimer>,
    pub(crate) windows: RefCell<HashMap<HWindow, Rc<RefCell<WindowInner>>>>,
    pub(crate) gl_connection: RefCell<Option<Rc<crate::egl::GlConnection>>>,
}

/// winapi 0.3.9 未收录的 CreateWaitableTimerExW 标志（Win10 1803+）：
/// 定时器不受 15.6ms 系统时钟节拍量化，精度到 0.5ms 以内
const CREATE_WAITABLE_TIMER_HIGH_RESOLUTION: DWORD = 0x0000_0002;

/// 定时器实际唤醒可能比 deadline 早几十微秒；容差内视为已到期，避免为
/// 这点差值再武装一次定时器
const FRAME_DEADLINE_SLACK: Duration = Duration::from_micros(250);

/// 窗口正被借用（极少见）时无法读其 deadline，稍后重试的间隔
const FRAME_TICK_RETRY: Duration = Duration::from_millis(1);

/// fork: 高精度可等待定时器句柄 + 当前武装的到期时刻
struct FrameTimer {
    handle: HANDLE,
    armed: Cell<Option<Instant>>,
}

impl FrameTimer {
    fn new() -> Option<Self> {
        let handle = unsafe {
            CreateWaitableTimerExW(
                null_mut(),
                null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS,
            )
        };
        if handle.is_null() {
            log::info!(
                "CreateWaitableTimerExW(HIGH_RESOLUTION) failed ({}); \
                 frame pacing falls back to async-io timers",
                std::io::Error::last_os_error()
            );
            return None;
        }
        Some(Self {
            handle,
            armed: Cell::new(None),
        })
    }

    /// 把定时器设到 deadline。lpDueTime 负值表示相对时间（100ns 单位）；
    /// 已过期的 deadline 用 1 tick 立即触发
    fn arm(&self, deadline: Instant) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let ticks = (remaining.as_nanos() / 100).max(1) as i64;
        let mut due: LARGE_INTEGER = unsafe { std::mem::zeroed() };
        unsafe {
            *due.QuadPart_mut() = -ticks;
        }
        let ok =
            unsafe { SetWaitableTimerEx(self.handle, &due, 0, None, null_mut(), null_mut(), 0) };
        if ok == 0 {
            log::error!(
                "SetWaitableTimerEx failed: {}",
                std::io::Error::last_os_error()
            );
            self.armed.set(None);
            return;
        }
        self.armed.set(Some(deadline));
    }

    fn cancel(&self) {
        unsafe {
            CancelWaitableTimer(self.handle);
        }
        self.armed.set(None);
    }
}

impl Drop for FrameTimer {
    fn drop(&mut self) {
        self.cancel();
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

pub(crate) fn get_appearance() -> Appearance {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.open_subkey("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize") {
        Ok(theme) => {
            let light = theme.get_value::<u32, _>("AppsUseLightTheme").unwrap_or(1) == 1;
            if light {
                Appearance::Light
            } else {
                Appearance::Dark
            }
        }
        _ => Appearance::Light,
    }
}

impl ConnectionOps for Connection {
    fn terminate_message_loop(&self) {
        unsafe {
            PostQuitMessage(0);
        }
    }

    fn get_appearance(&self) -> Appearance {
        get_appearance()
    }

    fn name(&self) -> String {
        "Windows".to_string()
    }

    fn run_message_loop(&self) -> anyhow::Result<()> {
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        loop {
            SPAWN_QUEUE.run();

            let res = unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) };
            if res != 0 {
                if msg.message == WM_QUIT {
                    // Clear our state before we exit, otherwise we can
                    // trigger `drop` handlers during shutdown and that
                    // can have bad interactions
                    self.windows.borrow_mut().clear();
                    return Ok(());
                }

                unsafe {
                    // We don't want to call TranslateMessage here
                    // unconditionally.  Instead, we perform translation
                    // in a handful of special cases in window.rs.
                    DispatchMessageW(&mut msg);
                }
            } else {
                self.wait_message();
            }
        }
    }

    fn beep(&self) {
        unsafe {
            MessageBeep(MB_OK);
        }
    }

    fn screens(&self) -> anyhow::Result<Screens> {
        let mut info = ScreenInfoHelper::new()?;
        info.enumerate();

        let main = info
            .primary
            .ok_or_else(|| anyhow::anyhow!("There is no primary monitor configured!?"))?;
        let active = info.active.unwrap_or_else(|| main.clone());

        Ok(Screens {
            main,
            active,
            by_name: info.by_name,
            virtual_rect: info.virtual_rect,
        })
    }
}

impl Connection {
    pub(crate) fn create_new() -> anyhow::Result<Self> {
        let event_handle = SPAWN_QUEUE.event_handle.0;
        Ok(Self {
            event_handle,
            frame_timer: FrameTimer::new(),
            windows: RefCell::new(HashMap::new()),
            gl_connection: RefCell::new(None),
        })
    }

    /// fork: 同时等待 spawn 队列事件与帧定时器。返回值 WAIT_OBJECT_0+i
    /// 对应第 i 个句柄，WAIT_OBJECT_0+count 表示有新消息到达
    fn wait_message(&self) {
        let mut handles: [HANDLE; 2] = [self.event_handle, null_mut()];
        let mut count: DWORD = 1;
        if let Some(timer) = &self.frame_timer {
            handles[1] = timer.handle;
            count = 2;
        }
        let res = unsafe {
            MsgWaitForMultipleObjects(
                count,
                handles.as_ptr(),
                0,
                INFINITE,
                QS_ALLEVENTS | QS_ALLINPUT | QS_ALLPOSTMESSAGE,
            )
        };
        if count == 2 && res == WAIT_OBJECT_0 + 1 {
            self.frame_timer_fired();
        }
    }

    /// fork: wm_paint 调用，请求帧定时器不晚于 deadline 唤醒主线程；已武装
    /// 的时刻更早时不动，到期扫描会接着处理更晚的窗口。
    /// 返回 false 表示没有高精度定时器，调用方走 async_io 回退路径
    pub(crate) fn request_frame_deadline(&self, deadline: Instant) -> bool {
        let Some(timer) = &self.frame_timer else {
            return false;
        };
        match timer.armed.get() {
            Some(armed) if armed <= deadline => {}
            _ => timer.arm(deadline),
        }
        true
    }

    /// 帧定时器到期：遍历窗口，已到期的结束节流并补发重绘，再按剩余最近的
    /// deadline 重设定时器。此处在消息循环里而非 wndProc 内，正常情况下没有
    /// 未释放的 RefCell 借用；借不到的窗口稍后重试，避免永远卡在节流态
    fn frame_timer_fired(&self) {
        let Some(timer) = &self.frame_timer else {
            return;
        };
        timer.armed.set(None);

        let now = Instant::now();
        let expiry_cutoff = now + FRAME_DEADLINE_SLACK;
        let windows: Vec<Rc<RefCell<WindowInner>>> =
            self.windows.borrow().values().map(Rc::clone).collect();

        fn earliest(current: Option<Instant>, candidate: Instant) -> Option<Instant> {
            Some(current.map_or(candidate, |c| c.min(candidate)))
        }

        let mut next_deadline: Option<Instant> = None;
        let mut to_invalidate: Vec<HWND> = Vec::new();

        for window in windows {
            let mut inner = match window.try_borrow_mut() {
                Ok(inner) => inner,
                Err(_) => {
                    next_deadline = earliest(next_deadline, now + FRAME_TICK_RETRY);
                    continue;
                }
            };
            match inner.frame_timer_tick(expiry_cutoff) {
                FrameTick::Idle => {}
                FrameTick::Pending(deadline) => {
                    next_deadline = earliest(next_deadline, deadline);
                }
                FrameTick::Expired { invalidate } => {
                    if invalidate {
                        to_invalidate.push(inner.hwnd());
                    }
                }
            }
        }

        for hwnd in to_invalidate {
            unsafe {
                InvalidateRect(hwnd, null(), 0);
            }
        }

        if let Some(deadline) = next_deadline {
            timer.arm(deadline);
        }
    }

    pub(crate) fn get_window(&self, handle: HWindow) -> Option<Rc<RefCell<WindowInner>>> {
        self.windows.borrow().get(&handle).map(Rc::clone)
    }

    pub(crate) fn with_window_inner<
        R,
        F: FnOnce(&mut WindowInner) -> anyhow::Result<R> + Send + 'static,
    >(
        window: HWindow,
        f: F,
    ) -> promise::Future<R>
    where
        R: Send + 'static,
    {
        let mut prom = promise::Promise::new();
        let future = prom.get_future().unwrap();
        promise::spawn::spawn_into_main_thread(async move {
            if let Some(handle) = Connection::get()
                .expect("Connection::init has not been called")
                .get_window(window)
            {
                let mut inner = handle.borrow_mut();
                prom.result(f(&mut inner));
            }
        })
        .detach();

        future
    }
}

pub(crate) struct ScreenInfoHelper {
    primary: Option<ScreenInfo>,
    active: Option<ScreenInfo>,
    by_name: HashMap<String, ScreenInfo>,
    virtual_rect: ScreenRect,
    active_handle: HMONITOR,
    friendly_names: HashMap<String, String>,
    gdi_to_adapater: HashMap<String, String>,
    config: ConfigHandle,
}

impl ScreenInfoHelper {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            primary: None,
            active: None,
            by_name: HashMap::new(),
            virtual_rect: euclid::rect(0, 0, 0, 0),
            active_handle: unsafe { MonitorFromWindow(GetFocus(), MONITOR_DEFAULTTONEAREST) },
            friendly_names: gdi_display_name_to_friendly_monitor_names()?,
            gdi_to_adapater: gdi_display_name_to_adapter_names(),
            config: config::configuration(),
        })
    }

    pub fn enumerate(&mut self) {
        unsafe extern "system" fn callback(
            mon: HMONITOR,
            _hdc: HDC,
            _rect: *mut RECT,
            data: LPARAM,
        ) -> i32 {
            let info: &mut ScreenInfoHelper = &mut *(data as *mut ScreenInfoHelper);
            let mut mi: MONITORINFOEXW = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            GetMonitorInfoW(mon, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO);

            let max_fps = display_refresh_rate(&mi).map(|hz| hz as usize);

            let monitor_name = info.monitor_name(&mi);

            let mut effective_dpi = None;

            if let Some(dpi) = info.config.dpi_by_screen.get(&monitor_name).copied() {
                effective_dpi.replace(dpi);
            } else if let Some(dpi) = info.config.dpi {
                effective_dpi.replace(dpi);
            } else {
                let mut dpi_x = 0;
                let mut dpi_y = 0;
                GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
                if dpi_x != 0 {
                    effective_dpi.replace(dpi_x as f64);
                }
            }

            let screen_info = ScreenInfo {
                name: monitor_name.clone(),
                rect: euclid::rect(
                    mi.rcMonitor.left as isize,
                    mi.rcMonitor.top as isize,
                    mi.rcMonitor.right as isize - mi.rcMonitor.left as isize,
                    mi.rcMonitor.bottom as isize - mi.rcMonitor.top as isize,
                ),
                scale: 1.0,
                max_fps,
                effective_dpi,
            };

            info.virtual_rect = info.virtual_rect.union(&screen_info.rect);

            if mi.dwFlags & MONITORINFOF_PRIMARY == MONITORINFOF_PRIMARY {
                info.primary.replace(screen_info.clone());
            }
            if mon == info.active_handle {
                info.active.replace(screen_info.clone());
            }

            info.by_name.insert(monitor_name, screen_info);

            winapi::shared::ntdef::TRUE.into()
        }

        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(callback),
                self as *mut _ as LPARAM,
            );
        }
    }

    pub fn monitor_name(&self, mi: &MONITORINFOEXW) -> String {
        unsafe {
            let monitor_name = wstr(&mi.szDevice);
            let friendly_name = match self.friendly_names.get(&monitor_name) {
                Some(name) => name.to_string(),
                None => {
                    // Fall back to EnumDisplayDevicesW.
                    // It likely has a terribly generic name like "Generic PnP Monitor".
                    let mut display_device: DISPLAY_DEVICEW = std::mem::zeroed();
                    display_device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;

                    if EnumDisplayDevicesW(mi.szDevice.as_ptr(), 0, &mut display_device, 0) != 0 {
                        wstr(&display_device.DeviceString)
                    } else {
                        "Unknown".to_string()
                    }
                }
            };

            let adapter_name = match self.gdi_to_adapater.get(&monitor_name) {
                Some(name) => name.to_string(),
                None => "Unknown".to_string(),
            };

            // "\\.\DISPLAY1" -> "DISPLAY1"
            let monitor_name = if let Some(name) = monitor_name.strip_prefix("\\\\.\\") {
                name.to_string()
            } else {
                monitor_name
            };

            let monitor_name = format!("{monitor_name}: {friendly_name} on {adapter_name}");

            monitor_name
        }
    }
}

/// fork: 读取显示器当前模式的刷新率（Hz）。驱动给的 0/1 是「默认/未知」
/// 占位值，与读取失败一样按 None 处理，由调用方回退 max_fps
pub(crate) fn display_refresh_rate(mi: &MONITORINFOEXW) -> Option<u32> {
    unsafe {
        let mut devmode: DEVMODEW = std::mem::zeroed();
        devmode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(mi.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut devmode) != 0
            && (devmode.dmFields & DM_DISPLAYFREQUENCY) != 0
            && devmode.dmDisplayFrequency > 1
        {
            Some(devmode.dmDisplayFrequency)
        } else {
            None
        }
    }
}

/// fork: 按 HMONITOR 读取刷新率；mon 为空或 GetMonitorInfoW 失败时为 None
pub(crate) fn monitor_refresh_rate(mon: HMONITOR) -> Option<u32> {
    if mon.is_null() {
        return None;
    }
    unsafe {
        let mut mi: MONITORINFOEXW = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(mon, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO) == 0 {
            return None;
        }
        display_refresh_rate(&mi)
    }
}

/// Convert a UCS2 wide char string to a Rust String
fn wstr(slice: &[u16]) -> String {
    let len = slice.iter().position(|&c| c == 0).unwrap_or(0);
    OsString::from_wide(&slice[0..len])
        .to_string_lossy()
        .to_string()
}

/// Build a mapping of GDI paths like `\\.\DISPLAY6` to the name of the associated
/// display adapter eg: `NVIDIA GeForce RTX 3080 Ti`.
fn gdi_display_name_to_adapter_names() -> HashMap<String, String> {
    let mut map = HashMap::new();

    let mut display_device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
    display_device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;

    for n in 0.. {
        if unsafe { EnumDisplayDevicesW(std::ptr::null(), n, &mut display_device, 0) } == 0 {
            break;
        }
        let adapter_name = wstr(&display_device.DeviceString);
        let gdi_name = wstr(&display_device.DeviceName);

        map.insert(gdi_name, adapter_name);
    }
    map
}

/// Build a mapping of GDI paths like `\\.\DISPLAY6` to the corresponding friendly name of
/// the associated monitor eg: `Gigabyte M32U`.
fn gdi_display_name_to_friendly_monitor_names() -> anyhow::Result<HashMap<String, String>> {
    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = vec![];
    let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![];
    let mut map = HashMap::new();

    let flags = QDC_ONLY_ACTIVE_PATHS | QDC_VIRTUAL_MODE_AWARE;

    loop {
        let mut path_count = 0u32;
        let mut mode_count = 0u32;

        let result = unsafe {
            GetDisplayConfigBufferSizes(flags, &mut path_count as *mut _, &mut mode_count as *mut _)
        };

        if result != ERROR_SUCCESS as i32 {
            return Err(std::io::Error::last_os_error()).context("GetDisplayConfigBufferSizes");
        }

        unsafe {
            paths.resize_with(path_count as usize, || std::mem::zeroed());
            modes.resize_with(mode_count as usize, || std::mem::zeroed());
        }

        let result = unsafe {
            QueryDisplayConfig(
                flags,
                &mut path_count as *mut _,
                paths.as_mut_ptr(),
                &mut mode_count as &mut _,
                modes.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };

        // Shrink down if fewer paths than were requested were
        // returned to us
        unsafe {
            paths.resize_with(path_count as usize, || std::mem::zeroed());
            modes.resize_with(mode_count as usize, || std::mem::zeroed());
        }

        if result == ERROR_INSUFFICIENT_BUFFER as i32 {
            continue;
        }

        if result != ERROR_SUCCESS as i32 {
            return Err(std::io::Error::last_os_error()).context("QueryDisplayConfig");
        }

        break;
    }

    for path in &paths {
        let mut target_name: DISPLAYCONFIG_TARGET_DEVICE_NAME = unsafe { std::mem::zeroed() };

        target_name.header.adapterId = path.targetInfo.adapterId;
        target_name.header.id = path.targetInfo.id;
        target_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        target_name.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;

        let result = unsafe { DisplayConfigGetDeviceInfo(&mut target_name.header) };
        if result != ERROR_SUCCESS as i32 {
            return Err(std::io::Error::last_os_error())
                .context("DisplayConfigGetDeviceInfo DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME");
        }

        let mut source_name: DISPLAYCONFIG_SOURCE_DEVICE_NAME = unsafe { std::mem::zeroed() };
        source_name.header.adapterId = path.targetInfo.adapterId;
        source_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source_name.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;

        let result = unsafe { DisplayConfigGetDeviceInfo(&mut source_name.header) };
        if result != ERROR_SUCCESS as i32 {
            return Err(std::io::Error::last_os_error())
                .context("DisplayConfigGetDeviceInfo DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME");
        }

        let name = wstr(&target_name.monitorFriendlyDeviceName);
        let gdi_name = wstr(&source_name.viewGdiDeviceName);

        map.insert(gdi_name, name);
    }
    Ok(map)
}
