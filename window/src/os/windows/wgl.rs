use super::*;
use glium::backend::Backend;
use std::cell::RefCell;
use std::ffi::CStr;
use std::io::Error as IoError;
use std::os::raw::c_void;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use winapi::shared::windef::*;
use winapi::um::libloaderapi::{GetModuleHandleW, *};
use winapi::um::wingdi::*;
use winapi::um::winuser::*;

pub mod ffi {
    include!(concat!(env!("OUT_DIR"), "/wgl_bindings.rs"));
}
pub mod ffiextra {
    include!(concat!(env!("OUT_DIR"), "/wgl_extra_bindings.rs"));
}

struct WglWrapper {
    lib: libloading::Library,
    wgl: ffi::Wgl,
    ext: Option<ffiextra::Wgl>,
}

type GetProcAddressFunc =
    unsafe extern "system" fn(*const std::os::raw::c_char) -> *const std::os::raw::c_void;

impl Drop for WglWrapper {
    fn drop(&mut self) {
        log::trace!("dropping WglWrapper and libloading {:?}", self.lib);
    }
}

thread_local! {
    /// fork: WGL 探测结果（opengl32 句柄、函数表、扩展表）每线程只做一次；
    /// 原实现每建一个窗口都创建隐藏探测窗口且不销毁，泄漏 HWND + DC
    static PROBED_WGL: RefCell<Option<Rc<WglWrapper>>> = RefCell::new(None);
}

impl WglWrapper {
    fn load() -> anyhow::Result<Rc<Self>> {
        if let Some(wgl) = PROBED_WGL.with(|probed| probed.borrow().clone()) {
            return Ok(wgl);
        }
        let wgl = Self::probe()?;
        PROBED_WGL.with(|probed| probed.borrow_mut().replace(Rc::clone(&wgl)));
        Ok(wgl)
    }

    /// 用一个临时隐藏窗口建基础上下文探测 WGL 扩展，探测完销毁窗口
    fn probe() -> anyhow::Result<Rc<Self>> {
        let class_name = wide_string("wezterm wgl extension probing window");
        let h_inst = unsafe { GetModuleHandleW(null()) };
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
            lpfnWndProc: Some(DefWindowProcW),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: h_inst,
            hIcon: null_mut(),
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

        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1024,
                768,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if hwnd.is_null() {
            let err = IoError::last_os_error();
            anyhow::bail!("CreateWindowExW: {}", err);
        }

        let result = Self::probe_with_window(hwnd);
        unsafe {
            DestroyWindow(hwnd);
        }
        result
    }

    fn probe_with_window(hwnd: HWND) -> anyhow::Result<Rc<Self>> {
        let mut state = GlState::create_basic(Rc::new(WglWrapper::create()?), hwnd)?;

        unsafe {
            state.make_current();
        }

        // 探测阶段 GlState 是这个 Rc 的唯一持有者，可以就地装载扩展表
        if let Some(wgl) = state.wgl.as_mut().and_then(Rc::get_mut) {
            let _ = wgl.load_ext();
        }

        state.make_not_current();

        let hdc = state.hdc;
        let wgl = state.into_wrapper();
        unsafe {
            ReleaseDC(hwnd, hdc);
        }
        Ok(wgl)
    }

    fn create() -> anyhow::Result<Self> {
        if crate::configuration::prefer_swrast() {
            let mesa_dir = std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join("mesa");
            let mesa_dir = wide_string(mesa_dir.to_str().unwrap());

            unsafe {
                AddDllDirectory(mesa_dir.as_ptr());
                SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
            }
        }

        let lib = unsafe { libloading::Library::new("opengl32.dll") }.map_err(|e| {
            log::error!("{:?}", e);
            e
        })?;
        log::trace!("loaded {:?}", lib);

        let get_proc_address: libloading::Symbol<GetProcAddressFunc> =
            unsafe { lib.get(b"wglGetProcAddress\0")? };
        let wgl = ffi::Wgl::load_with(|s: &'static str| {
            let sym_name = std::ffi::CString::new(s).expect("symbol to be cstring compatible");
            if let Ok(sym) = unsafe { lib.get(sym_name.as_bytes_with_nul()) } {
                return *sym;
            }
            unsafe { get_proc_address(sym_name.as_ptr()) }
        });
        Ok(Self {
            lib,
            wgl,
            ext: None,
        })
    }

    fn load_ext(&mut self) -> anyhow::Result<()> {
        let get_proc_address: libloading::Symbol<GetProcAddressFunc> =
            unsafe { self.lib.get(b"wglGetProcAddress\0")? };

        self.ext
            .replace(ffiextra::Wgl::load_with(|s: &'static str| {
                let sym_name = std::ffi::CString::new(s).expect("symbol to be cstring compatible");
                if let Ok(sym) = unsafe { self.lib.get(sym_name.as_bytes_with_nul()) } {
                    return *sym;
                }
                unsafe { get_proc_address(sym_name.as_ptr()) }
            }));

        Ok(())
    }

    // fork: WGL swaps are vsync-throttled by default, so SwapBuffers blocks
    // inside the wndProc paint path and stalls the whole message loop on
    // vblank. Mirror window/src/egl.rs's SwapInterval(0) here and leave
    // frame pacing to the existing max_fps mechanism. Silently skip when
    // the driver has no WGL_EXT_swap_control.
    fn disable_vsync(&self) {
        if let Some(ext) = self.ext.as_ref() {
            if ext.SwapIntervalEXT.is_loaded() {
                let res = unsafe { ext.SwapIntervalEXT(0) };
                log::trace!("wglSwapIntervalEXT(0) -> {}", res);
            }
        }
    }
}

pub struct GlState {
    wgl: Option<Rc<WglWrapper>>,
    hdc: HDC,
    rc: ffi::types::HGLRC,
}

/// fork: 像素格式档位。纯 2D 文本渲染用不到多重采样与深度/模板缓冲，
/// 先请求 Lean（0 样本、0 深度/模板，省显存带宽，部分驱动上 MSAA 还会
/// 拖慢 SwapBuffers），驱动不给再回退上游原来的 Legacy 组合
#[derive(Clone, Copy, Debug)]
enum PixelFormatProfile {
    Lean,
    Legacy,
}

impl PixelFormatProfile {
    const ORDER: [PixelFormatProfile; 2] = [PixelFormatProfile::Lean, PixelFormatProfile::Legacy];

    fn depth_bits(self) -> i32 {
        match self {
            Self::Lean => 0,
            Self::Legacy => 24,
        }
    }

    fn stencil_bits(self) -> i32 {
        match self {
            Self::Lean => 0,
            Self::Legacy => 8,
        }
    }

    fn samples(self) -> i32 {
        match self {
            Self::Lean => 0,
            Self::Legacy => 4,
        }
    }
}

fn has_extension(extensions: &str, wanted: &str) -> bool {
    extensions.split(' ').find(|&ext| ext == wanted).is_some()
}

impl GlState {
    fn into_wrapper(mut self) -> Rc<WglWrapper> {
        self.delete();
        self.wgl.take().unwrap()
    }

    pub fn create(window: HWND) -> anyhow::Result<Self> {
        let wgl = WglWrapper::load()?;

        if let Some(ext) = wgl.ext.as_ref() {
            let hdc = unsafe { GetDC(window) };

            fn cstr(data: *const i8) -> String {
                let data = unsafe { CStr::from_ptr(data).to_bytes().to_vec() };
                String::from_utf8(data).unwrap()
            }

            let extensions = if ext.GetExtensionsStringARB.is_loaded() {
                unsafe { cstr(ext.GetExtensionsStringARB(hdc as *const _)) }
            } else if ext.GetExtensionsStringEXT.is_loaded() {
                unsafe { cstr(ext.GetExtensionsStringEXT()) }
            } else {
                "".to_owned()
            };
            log::trace!("opengl extensions: {:?}", extensions);

            if has_extension(&extensions, "WGL_ARB_pixel_format") {
                return match Self::create_ext(Rc::clone(&wgl), extensions, hdc) {
                    Ok(state) => Ok(state),
                    Err(err) => {
                        log::warn!(
                            "failed to created extended OpenGL context \
                            ({}), fall back to basic",
                            err
                        );
                        Self::create_basic(wgl, window)
                    }
                };
            }
        }

        Self::create_basic(wgl, window)
    }

    /// 按档位组装 ChoosePixelFormatARB 的属性表
    fn pixel_format_attribs(extensions: &str, profile: PixelFormatProfile) -> Vec<i32> {
        use ffiextra::*;

        let mut attribs: Vec<i32> = vec![
            DRAW_TO_WINDOW_ARB as i32,
            1,
            SUPPORT_OPENGL_ARB as i32,
            1,
            DOUBLE_BUFFER_ARB as i32,
            1,
            PIXEL_TYPE_ARB as i32,
            TYPE_RGBA_ARB as i32,
            COLOR_BITS_ARB as i32,
            24,
            ALPHA_BITS_ARB as i32,
            8,
            DEPTH_BITS_ARB as i32,
            profile.depth_bits(),
            STENCIL_BITS_ARB as i32,
            profile.stencil_bits(),
        ];

        if profile.samples() > 0 {
            attribs.extend_from_slice(&[
                SAMPLE_BUFFERS_ARB as i32,
                1,
                SAMPLES_ARB as i32,
                profile.samples(),
            ]);
        }

        if has_extension(extensions, "WGL_ARB_framebuffer_sRGB") {
            log::trace!("will request FRAMEBUFFER_SRGB_CAPABLE_ARB");
            attribs.push(FRAMEBUFFER_SRGB_CAPABLE_ARB as i32);
            attribs.push(1);
        } else if has_extension(extensions, "WGL_EXT_framebuffer_sRGB") {
            log::trace!("will request FRAMEBUFFER_SRGB_CAPABLE_EXT");
            attribs.push(FRAMEBUFFER_SRGB_CAPABLE_EXT as i32);
            attribs.push(1);
        }

        attribs.push(0);
        attribs
    }

    /// 依次尝试各档位，返回第一个驱动接受的像素格式 id
    fn choose_pixel_format_arb(
        wgl: &WglWrapper,
        extensions: &str,
        hdc: HDC,
    ) -> anyhow::Result<i32> {
        let mut last_err = None;
        for profile in PixelFormatProfile::ORDER {
            let attribs = Self::pixel_format_attribs(extensions, profile);
            let mut format_id = 0;
            let mut num_formats = 0;

            let res = unsafe {
                wgl.ext.as_ref().unwrap().ChoosePixelFormatARB(
                    hdc as _,
                    attribs.as_ptr(),
                    null(),
                    1,
                    &mut format_id,
                    &mut num_formats,
                )
            };
            if res != 0 && num_formats > 0 {
                log::trace!(
                    "ChoosePixelFormatARB accepted {profile:?} profile: format {format_id}"
                );
                return Ok(format_id);
            }
            let err = if res == 0 {
                format!("ChoosePixelFormatARB returned 0 for {profile:?} profile")
            } else {
                format!("ChoosePixelFormatARB returned 0 formats for {profile:?} profile")
            };
            log::debug!("{err}; trying next profile");
            last_err.replace(err);
        }
        anyhow::bail!("{}", last_err.unwrap_or_default())
    }

    fn create_ext(wgl: Rc<WglWrapper>, extensions: String, hdc: HDC) -> anyhow::Result<Self> {
        use ffiextra::*;

        let format_id = Self::choose_pixel_format_arb(&wgl, &extensions, hdc)?;

        let mut pfd: PIXELFORMATDESCRIPTOR = unsafe { std::mem::zeroed() };

        let res = unsafe {
            DescribePixelFormat(
                hdc,
                format_id,
                std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as _,
                &mut pfd,
            )
        };
        if res == 0 {
            anyhow::bail!(
                "DescribePixelFormat function failed: {}",
                std::io::Error::last_os_error()
            );
        }

        let res = unsafe { SetPixelFormat(hdc, format_id, &pfd) };
        if res == 0 {
            anyhow::bail!(
                "SetPixelFormat function failed: {}",
                std::io::Error::last_os_error()
            );
        }

        let mut attribs = vec![
            CONTEXT_MAJOR_VERSION_ARB as i32,
            4,
            CONTEXT_MINOR_VERSION_ARB as i32,
            5,
            CONTEXT_PROFILE_MASK_ARB as i32,
            CONTEXT_CORE_PROFILE_BIT_ARB as i32,
        ];

        if has_extension(&extensions, "WGL_ARB_create_context_robustness") {
            log::trace!("requesting robustness features");
            attribs.push(CONTEXT_RESET_NOTIFICATION_STRATEGY_ARB as i32);
            attribs.push(LOSE_CONTEXT_ON_RESET_ARB as i32);
            attribs.push(CONTEXT_FLAGS_ARB as i32);
            attribs.push(CONTEXT_ROBUST_ACCESS_BIT_ARB as i32);
        }
        attribs.push(0);

        let rc = unsafe {
            wgl.ext
                .as_ref()
                .unwrap()
                .CreateContextAttribsARB(hdc as _, null(), attribs.as_ptr())
        };

        if rc.is_null() {
            let err = unsafe { winapi::um::errhandlingapi::GetLastError() };
            anyhow::bail!(
                "CreateContextAttribsARB failed, GetLastError={} {:x}",
                err,
                err
            );
        }

        unsafe {
            wgl.wgl.MakeCurrent(hdc as *mut _, rc);
        }

        wgl.disable_vsync();

        Ok(Self {
            wgl: Some(wgl),
            rc,
            hdc,
        })
    }

    fn basic_pixel_format_descriptor(profile: PixelFormatProfile) -> PIXELFORMATDESCRIPTOR {
        PIXELFORMATDESCRIPTOR {
            nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
            nVersion: 1,
            dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
            iPixelType: PFD_TYPE_RGBA,
            cColorBits: 24,
            cRedBits: 0,
            cRedShift: 0,
            cGreenBits: 0,
            cGreenShift: 0,
            cBlueBits: 0,
            cBlueShift: 0,
            cAlphaBits: 8,
            cAlphaShift: 0,
            cAccumBits: 0,
            cAccumRedBits: 0,
            cAccumGreenBits: 0,
            cAccumBlueBits: 0,
            cAccumAlphaBits: 0,
            cDepthBits: profile.depth_bits() as u8,
            cStencilBits: profile.stencil_bits() as u8,
            cAuxBuffers: 0,
            iLayerType: PFD_MAIN_PLANE,
            bReserved: 0,
            dwLayerMask: 0,
            dwVisibleMask: 0,
            dwDamageMask: 0,
        }
    }

    fn create_basic(wgl: Rc<WglWrapper>, window: HWND) -> anyhow::Result<Self> {
        let hdc = unsafe { GetDC(window) };

        // 同样先试 0 深度/模板，ChoosePixelFormat 返回 0 再回退上游组合
        for profile in PixelFormatProfile::ORDER {
            let pfd = Self::basic_pixel_format_descriptor(profile);
            let format = unsafe { ChoosePixelFormat(hdc, &pfd) };
            if format == 0 {
                log::debug!("ChoosePixelFormat found nothing for {profile:?} profile");
                continue;
            }
            unsafe {
                SetPixelFormat(hdc, format, &pfd);
            }
            break;
        }

        let rc = unsafe { wgl.wgl.CreateContext(hdc as *mut _) };
        unsafe {
            wgl.wgl.MakeCurrent(hdc as *mut _, rc);
        }

        wgl.disable_vsync();

        Ok(Self {
            wgl: Some(wgl),
            rc,
            hdc,
        })
    }

    fn make_not_current(&self) {
        if let Some(wgl) = self.wgl.as_ref() {
            unsafe {
                wgl.wgl.MakeCurrent(self.hdc as *mut _, std::ptr::null());
            }
        }
    }

    fn delete(&mut self) {
        self.make_not_current();
        if let Some(wgl) = self.wgl.as_ref() {
            unsafe {
                wgl.wgl.DeleteContext(self.rc);
            }
        }
    }
}

impl Drop for GlState {
    fn drop(&mut self) {
        self.delete();
    }
}

unsafe impl glium::backend::Backend for GlState {
    fn resize(&self, _: (u32, u32)) {
        todo!()
    }

    fn swap_buffers(&self) -> Result<(), glium::SwapBuffersError> {
        unsafe {
            SwapBuffers(self.hdc);
        }
        Ok(())
    }

    unsafe fn get_proc_address(&self, symbol: &str) -> *const c_void {
        let sym_name = std::ffi::CString::new(symbol).expect("symbol to be cstring compatible");
        if let Ok(sym) = self
            .wgl
            .as_ref()
            .unwrap()
            .lib
            .get(sym_name.as_bytes_with_nul())
        {
            //eprintln!("{} -> {:?}", symbol, sym);
            return *sym;
        }
        let res = self
            .wgl
            .as_ref()
            .unwrap()
            .wgl
            .GetProcAddress(sym_name.as_ptr()) as *const c_void;
        // eprintln!("{} -> {:?}", symbol, res);
        res
    }

    fn get_framebuffer_dimensions(&self) -> (u32, u32) {
        unimplemented!();
    }

    fn is_current(&self) -> bool {
        unsafe { self.wgl.as_ref().unwrap().wgl.GetCurrentContext() == self.rc }
    }

    unsafe fn make_current(&self) {
        self.wgl
            .as_ref()
            .unwrap()
            .wgl
            .MakeCurrent(self.hdc as *mut _, self.rc);
    }
}
