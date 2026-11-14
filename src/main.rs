#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod chrome;
mod model;
#[cfg(windows)]
mod obs;
#[cfg(windows)]
mod uploads;

#[cfg(windows)]
fn main() {
    if std::env::args().any(|argument| argument == "--version") {
        return;
    }
    if let Err(error) = app::run() {
        let title = "Kuma Capture\0".encode_utf16().collect::<Vec<_>>();
        let text = format!("{error:#}\0").encode_utf16().collect::<Vec<_>>();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                0x10,
            )
        };
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Kuma Capture targets Windows.");
}
