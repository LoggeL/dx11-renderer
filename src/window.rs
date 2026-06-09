use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::core::{w, Result, HSTRING};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect,
    GetCursorPos, GetForegroundWindow, LoadCursorW, PeekMessageW, PostQuitMessage,
    RegisterClassExW, SetCursorPos, SetWindowTextW, ShowCursor, TranslateMessage, CS_HREDRAW,
    CS_OWNDC, CS_VREDRAW, CW_USEDEFAULT, IDC_ARROW, MSG, PM_REMOVE, WINDOW_EX_STYLE, WM_DESTROY,
    WM_QUIT, WM_SIZE, WNDCLASSEXW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

static RESIZED: AtomicBool = AtomicBool::new(false);
static RESIZE_PACKED: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            RESIZE_PACKED.store(lp.0 as u32, Ordering::Relaxed);
            RESIZED.store(true, Ordering::Release);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

pub struct Window {
    pub hwnd: HWND,
}

impl Window {
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self> {
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

            let instance = GetModuleHandleW(None)?;
            let class = w!("dx11_renderer_window");

            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);

            let mut rect = RECT {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            };
            AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW, false)?;

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                &HSTRING::from(title),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                None,
                None,
                Some(instance.into()),
                None,
            )?;

            Ok(Self { hwnd })
        }
    }

    /// Pumps pending messages. Returns false when the window was closed.
    pub fn pump(&self) -> bool {
        let mut msg = MSG::default();
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return false;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
    }

    pub fn take_resize(&self) -> Option<(u32, u32)> {
        if RESIZED.swap(false, Ordering::Acquire) {
            let p = RESIZE_PACKED.load(Ordering::Relaxed);
            Some((p & 0xffff, (p >> 16) & 0xffff))
        } else {
            None
        }
    }

    pub fn client_size(&self) -> (u32, u32) {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
        }
        (
            (rect.right - rect.left).max(0) as u32,
            (rect.bottom - rect.top).max(0) as u32,
        )
    }

    pub fn focused(&self) -> bool {
        unsafe { GetForegroundWindow() == self.hwnd }
    }

    pub fn set_title(&self, title: &str) {
        unsafe {
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(title));
        }
    }
}

/// Polled keyboard state with edge detection.
pub struct Input {
    cur: [bool; 256],
    prev: [bool; 256],
}

impl Input {
    pub fn new() -> Self {
        Self {
            cur: [false; 256],
            prev: [false; 256],
        }
    }

    pub fn update(&mut self, focused: bool) {
        self.prev = self.cur;
        if !focused {
            self.cur = [false; 256];
            return;
        }
        for i in 1..256 {
            self.cur[i] = unsafe { GetAsyncKeyState(i as i32) } as u16 & 0x8000 != 0;
        }
    }

    pub fn down(&self, vk: VIRTUAL_KEY) -> bool {
        self.cur[vk.0 as usize & 255]
    }

    pub fn pressed(&self, vk: VIRTUAL_KEY) -> bool {
        let i = vk.0 as usize & 255;
        self.cur[i] && !self.prev[i]
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

/// Right-mouse-button mouse look: hides the cursor, pins it in place and
/// reports per-frame deltas while the button is held.
pub struct MouseLook {
    active: bool,
    anchor: POINT,
}

impl MouseLook {
    pub fn new() -> Self {
        Self {
            active: false,
            anchor: POINT::default(),
        }
    }

    pub fn update(&mut self, input: &Input) -> (f32, f32) {
        let rmb = input.down(VK_RBUTTON);
        unsafe {
            if rmb && !self.active {
                self.active = true;
                let _ = GetCursorPos(&mut self.anchor);
                ShowCursor(false);
            } else if !rmb && self.active {
                self.active = false;
                ShowCursor(true);
            }
            if self.active {
                let mut p = POINT::default();
                let _ = GetCursorPos(&mut p);
                let _ = SetCursorPos(self.anchor.x, self.anchor.y);
                return ((p.x - self.anchor.x) as f32, (p.y - self.anchor.y) as f32);
            }
        }
        (0.0, 0.0)
    }
}

impl Default for MouseLook {
    fn default() -> Self {
        Self::new()
    }
}
