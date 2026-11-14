// The desktop-DC capture workflow is adapted from ShareX.ScreenCaptureLib/Screenshot.cs.
// ShareX copyright (c) 2007–2026 ShareX Team. GPL-3.0-or-later.
use crate::model::{self, Kind, Media, NameInfo, Settings};
use anyhow::{bail, Context, Result};
use chrono::Local;
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::borrow::Cow;
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{BOOL, HWND, LPARAM, POINT, RECT},
    Graphics::{Dwm::DwmGetWindowAttribute, Gdi::*},
    UI::WindowsAndMessaging::*,
};

#[derive(Clone)]
pub struct Display {
    pub id: String,
    pub label: String,
    pub rect: RECT,
    pub primary: bool,
}

pub fn displays() -> Vec<Display> {
    unsafe extern "system" fn visit(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let displays = unsafe { &mut *(data as *mut Vec<Display>) };
        let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) } != 0 {
            let rect = info.monitorInfo.rcMonitor;
            let mut device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
            device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
            let has_device =
                unsafe { EnumDisplayDevicesW(info.szDevice.as_ptr(), 0, &mut device, 1) } != 0;
            let name = wide(&info.szDevice);
            let device_name = if has_device {
                wide(&device.DeviceString)
            } else {
                name.clone()
            };
            let id = if has_device {
                wide(&device.DeviceID)
            } else {
                name
            };
            let primary = info.monitorInfo.dwFlags & 1 != 0;
            let role = if primary { " (primary)" } else { "" };
            displays.push(Display {
                id,
                label: format!(
                    "{device_name} · {} × {}{role}",
                    rect.right - rect.left,
                    rect.bottom - rect.top
                ),
                rect,
                primary,
            });
        }
        1
    }
    let mut displays: Vec<Display> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(visit),
            &mut displays as *mut Vec<Display> as LPARAM,
        )
    };
    displays.sort_by_key(|display| !display.primary);
    displays
}

fn wide(chars: &[u16]) -> String {
    let length = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..length])
}

pub fn selected_display(id: &str) -> Result<Display> {
    displays()
        .into_iter()
        .find(|display| {
            if id.is_empty() {
                display.primary
            } else {
                display.id == id
            }
        })
        .context("The selected monitor is unavailable. Select a connected display.")
}

pub fn virtual_desktop() -> RECT {
    unsafe {
        let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        RECT {
            left,
            top,
            right: left + GetSystemMetrics(SM_CXVIRTUALSCREEN),
            bottom: top + GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

pub fn active_window(previous: HWND) -> Result<RECT> {
    let hwnd = if previous.is_null() {
        unsafe { GetForegroundWindow() }
    } else {
        previous
    };
    if hwnd.is_null() {
        bail!("No active window.");
    }
    window_bounds(hwnd)
}

fn window_bounds(hwnd: HWND) -> Result<RECT> {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            9,
            (&mut rect as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if result < 0 && unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        bail!("Could not read the active window bounds.");
    }
    let desktop = virtual_desktop();
    rect.left = rect.left.max(desktop.left);
    rect.top = rect.top.max(desktop.top);
    rect.right = rect.right.min(desktop.right);
    rect.bottom = rect.bottom.min(desktop.bottom);
    Ok(rect)
}

pub fn visible_windows() -> Vec<RECT> {
    unsafe extern "system" fn visit(hwnd: HWND, context: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd) != 0
            && IsIconic(hwnd) == 0
            && GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW == 0
        {
            let mut cloaked = 0u32;
            DwmGetWindowAttribute(hwnd, 14, (&mut cloaked as *mut u32).cast(), 4);
            if cloaked == 0 && GetWindowTextLengthW(hwnd) > 0 {
                if let Ok(rect) = window_bounds(hwnd) {
                    (&mut *(context as *mut Vec<RECT>)).push(rect);
                }
            }
        }
        1
    }
    let mut result = Vec::new();
    unsafe {
        EnumWindows(Some(visit), (&mut result as *mut Vec<RECT>) as LPARAM);
    }
    result
}

pub struct CapturedCursor {
    icon: HICON,
    spot: POINT,
}

impl CapturedCursor {
    pub fn get() -> Option<Self> {
        unsafe {
            let mut cursor: CURSORINFO = std::mem::zeroed();
            cursor.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut cursor) == 0 || cursor.flags & CURSOR_SHOWING == 0 {
                return None;
            }
            let icon = CopyIcon(cursor.hCursor);
            (!icon.is_null()).then_some(Self {
                icon,
                spot: cursor.ptScreenPos,
            })
        }
    }

    pub fn paint(&self, target: &mut RgbaImage, rect: RECT) {
        unsafe {
            let Ok((surface, bits)) = Surface::new(target.width() as i32, target.height() as i32)
            else {
                return;
            };
            let bgra = std::slice::from_raw_parts_mut(bits.cast::<u8>(), target.as_raw().len());
            for (bgra, rgba) in bgra
                .chunks_exact_mut(4)
                .zip(target.as_raw().chunks_exact(4))
            {
                bgra.copy_from_slice(&[rgba[2], rgba[1], rgba[0], 255]);
            }
            draw_icon(surface.memory, rect, self.icon, self.spot);
            GdiFlush();
            for (rgba, bgra) in target
                .as_mut()
                .chunks_exact_mut(4)
                .zip(bgra.chunks_exact(4))
            {
                rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], 255]);
            }
        }
    }
}

impl Drop for CapturedCursor {
    fn drop(&mut self) {
        unsafe {
            DestroyIcon(self.icon);
        }
    }
}

struct Surface {
    desktop: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Surface {
    unsafe fn new(width: i32, height: i32) -> Result<(Self, *mut u8)> {
        let mut surface = Surface {
            desktop: GetDC(null_mut()),
            memory: null_mut(),
            bitmap: null_mut(),
            previous: null_mut(),
        };
        if surface.desktop.is_null() {
            bail!("Could not access the desktop.");
        }
        surface.memory = CreateCompatibleDC(surface.desktop);
        if surface.memory.is_null() {
            bail!("Could not create a capture surface.");
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        surface.bitmap = CreateDIBSection(
            surface.desktop,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            null_mut(),
            0,
        );
        if surface.bitmap.is_null() || bits.is_null() {
            bail!("Could not allocate the capture bitmap.");
        }
        surface.previous = SelectObject(surface.memory, surface.bitmap);
        if surface.previous.is_null() {
            bail!("Could not select the capture bitmap.");
        }
        Ok((surface, bits.cast()))
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if !self.previous.is_null() {
                SelectObject(self.memory, self.previous);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.memory.is_null() {
                DeleteDC(self.memory);
            }
            if !self.desktop.is_null() {
                ReleaseDC(null_mut(), self.desktop);
            }
        }
    }
}

pub fn screen(rect: RECT, cursor: bool) -> Result<RgbaImage> {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width < 1 || height < 1 || width as u64 * height as u64 > 120_000_000 {
        bail!("Invalid capture dimensions.");
    }
    unsafe {
        let (surface, bits) = Surface::new(width, height)?;
        if BitBlt(
            surface.memory,
            0,
            0,
            width,
            height,
            surface.desktop,
            rect.left,
            rect.top,
            SRCCOPY | CAPTUREBLT,
        ) == 0
        {
            bail!("Windows could not capture this desktop.");
        }
        if cursor {
            draw_cursor(surface.memory, rect);
        }
        GdiFlush();
        let mut rgba =
            std::slice::from_raw_parts(bits.cast::<u8>(), width as usize * height as usize * 4)
                .to_vec();
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        RgbaImage::from_raw(width as u32, height as u32, rgba).context("Invalid capture bitmap")
    }
}

unsafe fn draw_cursor(dc: HDC, rect: RECT) {
    let mut cursor: CURSORINFO = std::mem::zeroed();
    cursor.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
    if GetCursorInfo(&mut cursor) == 0 || cursor.flags & CURSOR_SHOWING == 0 {
        return;
    }
    draw_icon(dc, rect, cursor.hCursor, cursor.ptScreenPos);
}

unsafe fn draw_icon(dc: HDC, rect: RECT, handle: HICON, position: POINT) {
    let mut icon: ICONINFO = std::mem::zeroed();
    if GetIconInfo(handle, &mut icon) != 0 {
        DrawIconEx(
            dc,
            position.x - rect.left - icon.xHotspot as i32,
            position.y - rect.top - icon.yHotspot as i32,
            handle,
            0,
            0,
            0,
            null_mut(),
            DI_NORMAL,
        );
        if !icon.hbmColor.is_null() {
            DeleteObject(icon.hbmColor);
        }
        if !icon.hbmMask.is_null() {
            DeleteObject(icon.hbmMask);
        }
    }
}

pub fn foreground() -> HWND {
    unsafe { GetForegroundWindow() }
}

pub fn save(image: RgbaImage, label: &str, settings: &Settings) -> Result<(Media, Option<String>)> {
    let now = Local::now();
    let dimensions = format!("{}x{}", image.width(), image.height());
    let ext = if settings.screenshot_jpeg {
        "jpg"
    } else {
        "png"
    };
    let info = NameInfo {
        kind: Kind::Screenshot,
        timestamp: now,
        monitor: label,
        dimensions: &dimensions,
        duration: None,
        ext,
    };
    let image = DynamicImage::ImageRgba8(image);
    let path = model::save_unique(&settings.folders[0], &info, |file| {
        if settings.screenshot_jpeg {
            image::codecs::jpeg::JpegEncoder::new_with_quality(file, settings.jpeg_quality)
                .encode_image(&image)?;
        } else {
            image.write_to(file, ImageFormat::Png)?;
        }
        Ok(())
    })?;
    let clipboard = (|| -> Result<()> {
        if settings.copy_screenshot {
            let mut clipboard = arboard::Clipboard::new()?;
            let image = image
                .as_rgba8()
                .context("Screenshot pixels are unavailable")?;
            clipboard.set_image(arboard::ImageData {
                width: image.width() as usize,
                height: image.height() as usize,
                bytes: Cow::Borrowed(image.as_raw()),
            })?;
        } else if settings.copy_path {
            arboard::Clipboard::new()?.set_text(path.to_string_lossy().into_owned())?;
        }
        Ok(())
    })();
    Ok((
        Media {
            kind: Kind::Screenshot,
            path,
            timestamp: now,
            dimensions,
            duration: None,
            upload: String::new(),
            link: String::new(),
            upload_profile: String::new(),
        },
        clipboard.err().map(|e| e.to_string()),
    ))
}
