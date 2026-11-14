use crate::capture;
use anyhow::{bail, Result};
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
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus},
        WindowsAndMessaging::*,
    },
};

const SNIP_CLASS: &[u16] = &[75, 117, 109, 97, 83, 110, 105, 112, 0];
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

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Rectangle,
    Window,
    Display,
    Desktop,
}
const MODES: [Mode; 4] = [Mode::Rectangle, Mode::Window, Mode::Display, Mode::Desktop];

pub struct Selection {
    pub image: RgbaImage,
    pub label: &'static str,
    pub copy_image: bool,
}

struct Snip {
    bounds: RECT,
    desktop: DesktopImage,
    dark: DesktopImage,
    windows: Vec<RECT>,
    monitors: Vec<RECT>,
    toolbar: RECT,
    buttons: Vec<HWND>,
    mode: Mode,
    anchor: Option<POINT>,
    point: POINT,
    result: Option<(RECT, &'static str)>,
    include_cursor: bool,
    copy_image: bool,
    cursor: Option<capture::CapturedCursor>,
}

impl Snip {
    fn selection(&self) -> Option<RECT> {
        match self.mode {
            Mode::Rectangle => self.anchor.map(|start| RECT {
                left: start.x.min(self.point.x),
                top: start.y.min(self.point.y),
                right: start.x.max(self.point.x),
                bottom: start.y.max(self.point.y),
            }),
            Mode::Window => self
                .windows
                .iter()
                .copied()
                .find(|rect| inside(*rect, self.point)),
            Mode::Display => self
                .monitors
                .iter()
                .copied()
                .find(|rect| inside(*rect, self.point)),
            Mode::Desktop => Some(self.bounds),
        }
    }
}

fn inside(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn client(rect: RECT, bounds: RECT) -> RECT {
    RECT {
        left: rect.left - bounds.left,
        top: rect.top - bounds.top,
        right: rect.right - bounds.left,
        bottom: rect.bottom - bounds.top,
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

fn register(class_name: &[u16], procedure: WNDPROC, crosshair: bool) -> bool {
    unsafe {
        let mut class: WNDCLASSW = std::mem::zeroed();
        class.lpfnWndProc = procedure;
        class.hInstance = GetModuleHandleW(null());
        class.hCursor = LoadCursorW(null_mut(), if crosshair { IDC_CROSS } else { IDC_ARROW });
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

pub fn snip(include_cursor: bool, copy_image: bool) -> Result<Option<Selection>> {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let bounds = capture::virtual_desktop();
    let pixels = capture::screen(bounds, false)?;
    let cursor = capture::CapturedCursor::get();
    let mut dark = pixels.clone();
    for pixel in dark.pixels_mut() {
        for channel in &mut pixel.0[..3] {
            *channel = (*channel as u16 * 54 / 100) as u8;
        }
    }
    let point = cursor_point();
    let area = client(work_area(point), bounds);
    let mut state = Box::new(Snip {
        bounds,
        desktop: DesktopImage::new(pixels),
        dark: DesktopImage::new(dark),
        windows: capture::visible_windows(),
        monitors: capture::displays()
            .into_iter()
            .map(|display| display.rect)
            .collect(),
        toolbar: area,
        buttons: Vec::new(),
        mode: Mode::Rectangle,
        anchor: None,
        point,
        result: None,
        include_cursor,
        copy_image,
        cursor,
    });
    static CLASS: OnceLock<bool> = OnceLock::new();
    if !CLASS.get_or_init(|| register(SNIP_CLASS, Some(snip_proc), true)) {
        bail!("Could not register the snipping overlay.");
    }
    unsafe {
        let window = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            SNIP_CLASS.as_ptr(),
            wide("Kuma Capture snipping overlay").as_ptr(),
            WS_POPUP,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            (&mut *state as *mut Snip).cast(),
        );
        if window.is_null() {
            bail!("Could not open the snipping overlay.");
        }
        SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE);
        create_toolbar(window, &mut state);
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        SetFocus(window);
        message_loop(window);
    }
    Ok(state.result.map(|(selection, label)| {
        let rect = client(selection, bounds);
        let mut image = image::imageops::crop_imm(
            &state.desktop.pixels,
            rect.left as u32,
            rect.top as u32,
            (rect.right - rect.left) as u32,
            (rect.bottom - rect.top) as u32,
        )
        .to_image();
        if state.include_cursor {
            if let Some(cursor) = state.cursor.take() {
                cursor.paint(&mut image, selection);
            }
        }
        Selection {
            image,
            label,
            copy_image: state.copy_image,
        }
    }))
}

unsafe fn create_toolbar(window: HWND, state: &mut Snip) {
    let dpi = GetDpiForWindow(window).max(96) as i32;
    let px = |value: i32| value * dpi / 96;
    let area = state.toolbar;
    let width = px(532).min(area.right - area.left - 12);
    let left = area.left + (area.right - area.left - width) / 2;
    let top = area.top + px(12);
    state.toolbar = RECT {
        left,
        top,
        right: left + width,
        bottom: top + px(48),
    };
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let labels = [
        "Rectangle",
        "Window",
        "Display",
        "All screens",
        "Cursor",
        "Copy",
        "Cancel",
    ];
    let weights = [83, 72, 69, 89, 70, 66, 67];
    let mut x = left + px(8);
    for (index, (label, weight)) in labels.into_iter().zip(weights).enumerate() {
        let w = (width - px(16)) * weight / 516;
        let style = if index < 4 {
            BS_RADIOBUTTON
        } else if index < 6 {
            BS_AUTOCHECKBOX
        } else {
            BS_PUSHBUTTON
        };
        let button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide(label).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | style as u32,
            x,
            top + px(8),
            w - 2,
            px(31),
            window,
            (index + 1) as _,
            GetModuleHandleW(null()),
            null(),
        );
        SendMessageW(button, WM_SETFONT, font as usize, 1);
        state.buttons.push(button);
        x += w;
    }
    for (index, checked) in [(0, true), (4, state.include_cursor), (5, state.copy_image)] {
        SendMessageW(state.buttons[index], BM_SETCHECK, checked as usize, 0);
    }
}

unsafe fn attach_state<T>(window: HWND, message: u32, lparam: LPARAM) -> *mut T {
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    GetWindowLongPtrW(window, GWLP_USERDATA) as *mut T
}

unsafe extern "system" fn snip_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = attach_state::<Snip>(window, message, lparam);
    if state.is_null() {
        return DefWindowProcW(window, message, wparam, lparam);
    }
    let state = &mut *state;
    match message {
        WM_COMMAND => {
            let control = (wparam & 0xFFFF) as usize;
            match control {
                1..=4 => {
                    state.mode = MODES[control - 1];
                    state.anchor = None;
                    for (index, button) in state.buttons[..4].iter().enumerate() {
                        SendMessageW(*button, BM_SETCHECK, (index + 1 == control) as usize, 0);
                    }
                }
                5 => state.include_cursor = SendMessageW(state.buttons[4], BM_GETCHECK, 0, 0) != 0,
                6 => state.copy_image = SendMessageW(state.buttons[5], BM_GETCHECK, 0, 0) != 0,
                7 => {
                    PostMessageW(window, WM_CLOSE, 0, 0);
                    return 0;
                }
                _ => return DefWindowProcW(window, message, wparam, lparam),
            }
            SetFocus(window);
            InvalidateRect(window, null(), 0);
            0
        }
        WM_KEYDOWN if wparam == 0x1B => {
            PostMessageW(window, WM_CLOSE, 0, 0);
            0
        }
        WM_KEYDOWN if (0x31..=0x34).contains(&wparam) => {
            PostMessageW(window, WM_COMMAND, wparam - 0x30, 0);
            0
        }
        WM_KEYDOWN if wparam == 0x0D && state.mode != Mode::Rectangle => {
            finish_selection(window, state);
            0
        }
        WM_RBUTTONDOWN | WM_DISPLAYCHANGE => {
            PostMessageW(window, WM_CLOSE, 0, 0);
            0
        }
        WM_LBUTTONDOWN => {
            state.point = cursor_point();
            if state.mode == Mode::Rectangle {
                state.anchor = Some(state.point);
                SetCapture(window);
            } else {
                finish_selection(window, state);
            }
            0
        }
        WM_MOUSEMOVE => {
            state.point = cursor_point();
            InvalidateRect(window, null(), 0);
            0
        }
        WM_LBUTTONUP if state.anchor.is_some() => {
            state.point = cursor_point();
            finish_selection(window, state);
            state.anchor = None;
            ReleaseCapture();
            InvalidateRect(window, null(), 0);
            0
        }
        WM_CAPTURECHANGED => {
            state.anchor = None;
            0
        }
        WM_PAINT => {
            let mut paint: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(window, &mut paint);
            let full = client(state.bounds, state.bounds);
            state.dark.paint(dc, full);
            if let Some(selection) = state.selection() {
                let bright = client(selection, state.bounds);
                let saved = SaveDC(dc);
                IntersectClipRect(dc, bright.left, bright.top, bright.right, bright.bottom);
                state.desktop.paint(dc, full);
                RestoreDC(dc, saved);
                let pink = CreateSolidBrush(0x00C9B7FF);
                FrameRect(dc, &bright, pink);
                DeleteObject(pink);
                if state.anchor.is_some()
                    && bright.right > bright.left
                    && bright.bottom > bright.top
                {
                    let mut label = RECT {
                        left: bright.left + 4,
                        right: bright.left + 150,
                        top: (bright.top - 23).max(0),
                        bottom: bright.top.max(23),
                    };
                    let size = wide(&format!(
                        "{} × {}",
                        bright.right - bright.left,
                        bright.bottom - bright.top
                    ));
                    SetBkMode(dc, OPAQUE as i32);
                    SetBkColor(dc, 0x00E8F2FF);
                    SetTextColor(dc, 0x002B3657);
                    DrawTextW(
                        dc,
                        size.as_ptr(),
                        -1,
                        &mut label,
                        DT_SINGLELINE | DT_VCENTER,
                    );
                }
            }
            let cream = CreateSolidBrush(0x00E8F2FF);
            FillRect(dc, &state.toolbar, cream);
            DeleteObject(cream);
            EndPaint(window, &paint);
            0
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

unsafe fn finish_selection(window: HWND, state: &mut Snip) {
    if let Some(mut rect) = state.selection() {
        rect.left = rect.left.max(state.bounds.left);
        rect.top = rect.top.max(state.bounds.top);
        rect.right = rect.right.min(state.bounds.right);
        rect.bottom = rect.bottom.min(state.bounds.bottom);
        if rect.right > rect.left && rect.bottom > rect.top {
            state.result = Some((
                rect,
                match state.mode {
                    Mode::Rectangle => "Region",
                    Mode::Window => "Window",
                    Mode::Display => "Display",
                    Mode::Desktop => "Desktop",
                },
            ));
            PostMessageW(window, WM_CLOSE, 0, 0);
        }
    }
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
        if !CLASS.get_or_init(|| register(PREVIEW_CLASS, Some(preview_proc), false)) {
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
