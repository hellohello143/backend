use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    UI::{
        HiDpi::GetDpiForWindow,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

pub fn attach(cc: &eframe::CreationContext<'_>) {
    let Ok(handle) = cc.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(window) = handle.as_raw() else {
        return;
    };
    unsafe {
        SetWindowSubclass(window.hwnd.get() as HWND, Some(chrome_proc), 1, 0);
    }
}

unsafe extern "system" fn chrome_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    if message == WM_NCHITTEST {
        let mut bounds: RECT = std::mem::zeroed();
        if GetWindowRect(window, &mut bounds) != 0 {
            let x = (lparam as u16 as i16) as i32;
            let y = ((lparam >> 16) as u16 as i16) as i32;
            let scale = GetDpiForWindow(window).max(96) as f32 / 96.0;
            let px = |point: f32| (point * scale).round() as i32;
            if IsZoomed(window) == 0 {
                let left = x < bounds.left + px(6.0);
                let right = x >= bounds.right - px(6.0);
                let top = y < bounds.top + px(6.0);
                let bottom = y >= bounds.bottom - px(6.0);
                let hit = match (left, right, top, bottom) {
                    (true, _, true, _) => HTTOPLEFT,
                    (_, true, true, _) => HTTOPRIGHT,
                    (true, _, _, true) => HTBOTTOMLEFT,
                    (_, true, _, true) => HTBOTTOMRIGHT,
                    (true, _, _, _) => HTLEFT,
                    (_, true, _, _) => HTRIGHT,
                    (_, _, true, _) => HTTOP,
                    (_, _, _, true) => HTBOTTOM,
                    _ => HTCLIENT,
                };
                if hit != HTCLIENT {
                    return hit as LRESULT;
                }
            }
            let mut origin = POINT { x: 0, y: 0 };
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(window, &mut origin);
            if y < origin.y + px(39.0)
                && y >= origin.y
                && x >= bounds.right - px(84.0)
                && x < bounds.right - px(42.0)
            {
                return HTMAXBUTTON as LRESULT;
            }
        }
    } else if message == WM_NCLBUTTONUP && wparam == HTMAXBUTTON as WPARAM {
        ShowWindow(
            window,
            if IsZoomed(window) == 0 {
                SW_MAXIMIZE
            } else {
                SW_RESTORE
            },
        );
        return 0;
    } else if message == WM_NCDESTROY {
        RemoveWindowSubclass(window, Some(chrome_proc), id);
    }
    DefSubclassProc(window, message, wparam, lparam)
}

pub fn with_label(response: egui::Response, label: &str) -> egui::Response {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.on_hover_text(label)
}
