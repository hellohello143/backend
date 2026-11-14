use crate::capture;
use image::RgbaImage;
use std::{
    path::PathBuf,
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{
            GetDpiForWindow, SetThreadDpiAwarenessContext,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        },
        WindowsAndMessaging::*,
    },
};

const PREVIEW_CLASS: &[u16] = &[75, 117, 109, 97, 80, 114, 101, 118, 105, 101, 119, 0];
const SCREENSHOT_SOUND: &[u8] = include_bytes!("../assets/capture-chime.wav");
static PREVIEW_GENERATION: AtomicUsize = AtomicUsize::new(0);
static PREVIEW_WINDOW: AtomicUsize = AtomicUsize::new(0);

pub fn play_sound() {
    unsafe {
        PlaySoundW(
            SCREENSHOT_SOUND.as_ptr().cast(),
            null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

struct DesktopImage {
    pixels: RgbaImage,
    bgra: Vec<u8>,
}
impl DesktopImage {
    fn new(pixels: RgbaImage) -> Self {
        let mut bgra = pixels.as_raw().clone();
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        Self { pixels, bgra }
    }
    unsafe fn paint(&self, dc: HDC, dest: RECT) {
        let width = self.pixels.width() as i32;
        let height = self.pixels.height() as i32;
        let mut bitmap: BITMAPINFO = std::mem::zeroed();
        bitmap.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bitmap.bmiHeader.biWidth = width;
        bitmap.bmiHeader.biHeight = -height;
        bitmap.bmiHeader.biPlanes = 1;
        bitmap.bmiHeader.biBitCount = 32;
        StretchDIBits(
            dc,
            dest.left,
            dest.top,
            dest.right - dest.left,
            dest.bottom - dest.top,
            0,
            0,
            width,
            height,
            self.bgra.as_ptr().cast(),
            &bitmap,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

fn cursor_point() -> POINT {
    let mut point = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut point);
    }
    point
}

fn work_area(point: POINT) -> RECT {
    unsafe {
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST), &mut info) == 0 {
            capture::virtual_desktop()
        } else {
            info.rcWork
        }
    }
}

fn register(class_name: &[u16], procedure: WNDPROC) -> bool {
    unsafe {
        let mut class: WNDCLASSW = std::mem::zeroed();
        class.lpfnWndProc = procedure;
        class.hInstance = GetModuleHandleW(null());
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        class.lpszClassName = class_name.as_ptr();
        RegisterClassW(&class) != 0 || windows_sys::Win32::Foundation::GetLastError() == 1410
    }
}

fn message_loop(window: HWND) {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while IsWindow(window) != 0 && GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if IsWindow(window) != 0 {
            DestroyWindow(window);
        }
    }
}

unsafe fn attach_state<T>(window: HWND, message: u32, lparam: LPARAM) -> *mut T {
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    GetWindowLongPtrW(window, GWLP_USERDATA) as *mut T
}

struct Preview {
    image: DesktopImage,
    path: PathBuf,
    generation: usize,
    width: i32,
    height: i32,
}

pub fn dismiss_preview() {
    PREVIEW_GENERATION.fetch_add(1, Ordering::Relaxed);
    let window = PREVIEW_WINDOW.swap(0, Ordering::AcqRel) as HWND;
    if !window.is_null() {
        unsafe {
            let mut result = 0;
            SendMessageTimeoutW(window, WM_CLOSE, 0, 0, SMTO_ABORTIFHUNG, 80, &mut result);
            windows_sys::Win32::Graphics::Dwm::DwmFlush();
        }
    }
}

pub fn show_preview(image: RgbaImage, path: PathBuf) {
    let generation = PREVIEW_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        static CLASS: OnceLock<bool> = OnceLock::new();
        if !CLASS.get_or_init(|| register(PREVIEW_CLASS, Some(preview_proc))) {
            return;
        }
        let area = work_area(cursor_point());
        let mut preview = Box::new(Preview {
            image: DesktopImage::new(image),
            path,
            generation,
            width: 268,
            height: 176,
        });
        let window = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PREVIEW_CLASS.as_ptr(),
            wide("Kuma Capture screenshot preview").as_ptr(),
            WS_POPUP | WS_BORDER,
            area.right - 286,
            area.bottom - 194,
            268,
            176,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            (&mut *preview as *mut Preview).cast(),
        );
        if window.is_null() {
            return;
        }
        if PREVIEW_GENERATION.load(Ordering::Relaxed) != generation {
            DestroyWindow(window);
            return;
        }
        PREVIEW_WINDOW.store(window as usize, Ordering::Release);
        SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE);
        let dpi = GetDpiForWindow(window).max(96) as i32;
        preview.width = 268 * dpi / 96;
        preview.height = 176 * dpi / 96;
        let gap = 18 * dpi / 96;
        SetWindowPos(
            window,
            HWND_TOPMOST,
            area.right - preview.width - gap,
            area.bottom - preview.height - gap,
            preview.width,
            preview.height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        SetTimer(window, 1, 4500, None);
        SetTimer(window, 2, 150, None);
        message_loop(window);
    });
}

unsafe extern "system" fn preview_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = attach_state::<Preview>(window, message, lparam);
    if state.is_null() {
        return DefWindowProcW(window, message, wparam, lparam);
    }
    let state = &mut *state;
    match message {
        WM_NCDESTROY => {
            let _ = PREVIEW_WINDOW.compare_exchange(
                window as usize,
                0,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
            DefWindowProcW(window, message, wparam, lparam)
        }
        WM_TIMER
            if wparam == 1 || PREVIEW_GENERATION.load(Ordering::Relaxed) != state.generation =>
        {
            PostMessageW(window, WM_CLOSE, 0, 0);
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_LBUTTONUP => {
            let x = (lparam as u16 as i16) as i32;
            let y = ((lparam >> 16) as u16 as i16) as i32;
            let header = (state.height * 34 / 176).max(28);
            if x < state.width - header || y > header {
                let _ = std::process::Command::new("explorer.exe")
                    .arg(&state.path)
                    .spawn();
            }
            PostMessageW(window, WM_CLOSE, 0, 0);
            0
        }
        WM_RBUTTONUP => {
            PostMessageW(window, WM_CLOSE, 0, 0);
            0
        }
        WM_PAINT => {
            let mut paint: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(window, &mut paint);
            let mut area: RECT = std::mem::zeroed();
            GetClientRect(window, &mut area);
            let cream = CreateSolidBrush(0x00EAF3FF);
            FillRect(dc, &area, cream);
            DeleteObject(cream);
            let header = (state.height * 34 / 176).max(28);
            let mut text_area = RECT {
                left: 10,
                top: 0,
                right: area.right - header,
                bottom: header,
            };
            let font = GetStockObject(DEFAULT_GUI_FONT);
            let old_font = SelectObject(dc, font);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x002B3657);
            DrawTextW(
                dc,
                wide("Screenshot saved").as_ptr(),
                -1,
                &mut text_area,
                DT_SINGLELINE | DT_VCENTER,
            );
            let mut close = RECT {
                left: area.right - header,
                top: 0,
                right: area.right,
                bottom: header,
            };
            DrawTextW(
                dc,
                wide("×").as_ptr(),
                -1,
                &mut close,
                DT_SINGLELINE | DT_CENTER | DT_VCENTER,
            );
            SelectObject(dc, old_font);
            let (w, h) = state.image.pixels.dimensions();
            let scale = ((area.right - 16) as f32 / w as f32)
                .min((area.bottom - header - 8) as f32 / h as f32);
            let width = ((w as f32 * scale) as i32).max(1);
            let height = ((h as f32 * scale) as i32).max(1);
            let left = (area.right - width) / 2;
            state.image.paint(
                dc,
                RECT {
                    left,
                    top: header,
                    right: left + width,
                    bottom: header + height,
                },
            );
            EndPaint(window, &paint);
            0
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}
