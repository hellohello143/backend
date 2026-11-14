// The desktop-DC capture workflow is adapted from ShareX.ScreenCaptureLib/Screenshot.cs.
// ShareX copyright (c) 2007–2026 ShareX Team. GPL-3.0-or-later.
use crate::model::{self, Kind, Media, NameInfo, Settings};
use anyhow::{Context, Result};
use chrono::Local;
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::borrow::Cow;
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{BOOL, HWND, LPARAM, RECT},
    Graphics::Gdi::*,
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
