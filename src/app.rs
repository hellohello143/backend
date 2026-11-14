use crate::{
    capture, desktop,
    model::{self, DateFolders, Kind, Media, NameInfo, Settings, UploadProfile, ACTIONS},
    obs::Engine,
    shortcut, uploads,
};
use anyhow::{bail, Context, Result};
use chrono::Local;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, Vec2,
    ViewportCommand,
};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use image::RgbaImage;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};
use windows_sys::Win32::{
    Foundation::HWND,
    System::Threading::{CreateMutexW, GetCurrentProcessId},
    UI::WindowsAndMessaging::GetWindowThreadProcessId,
};

const CATEGORIES: [&str; 8] = [
    "General",
    "Hotkeys",
    "Screenshots",
    "Clips & Recording",
    "Audio",
    "Files & Folders",
    "Uploads",
    "Appearance",
];
const BROWN: Color32 = Color32::from_rgb(88, 55, 43);
const HONEY: Color32 = Color32::from_rgb(191, 116, 60);
const PINK: Color32 = Color32::from_rgb(252, 210, 213);
const PAPER: Color32 = Color32::from_rgb(255, 248, 237);

pub fn run() -> Result<()> {
    // A single process owns the tray, settings files and private OBS profile.
    let mutex_name = "Local\\KumaCaptureDesktop\0"
        .encode_utf16()
        .collect::<Vec<_>>();
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    if mutex.is_null() {
        bail!("Could not reserve the desktop app instance.");
    }
    if unsafe { windows_sys::Win32::Foundation::GetLastError() } == 183 {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(mutex) };
        bail!("Kuma Capture is already running. Open it from the Windows tray.");
    }
    let root = std::env::current_exe()?
        .parent()
        .context("Missing program folder")?
        .to_owned();
    let app_data = std::env::var_os("LOCALAPPDATA").context("Missing Windows profile folder")?;
    let data = PathBuf::from(app_data).join("KumaCapture");
    fs::create_dir_all(&data)?;
    let icon = icon_pixels();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Kuma Capture")
            .with_inner_size([980.0, 690.0])
            .with_min_inner_size([720.0, 500.0])
            .with_decorations(false)
            .with_icon(egui::IconData {
                rgba: icon,
                width: 64,
                height: 64,
            }),
        ..Default::default()
    };
    let result = eframe::run_native(
        "Kuma Capture",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, root, data)))),
    );
    unsafe { windows_sys::Win32::Foundation::CloseHandle(mutex) };
    result.map_err(|e| anyhow::anyhow!("{e}"))
}

enum Work {
    Replay(Settings),
    Recording(Settings),
    SaveReplay(Settings),
    Devices(Settings),
    Screenshot(RgbaImage, String, Settings),
    Reset,
    Quit,
}

#[derive(Clone, Default)]
struct CaptureStatus {
    replay: Option<Instant>,
    recording: Option<Instant>,
    fallback: bool,
    devices: [Vec<(String, String)>; 2],
    meters: [f32; 2],
}

enum Report {
    Saved(Media, Option<String>),
    Snip(Result<Option<desktop::Selection>, String>, Settings),
    Status(CaptureStatus),
    Done(Result<(), String>),
    Upload(PathBuf, String, Result<String, String>),
    Connection(Result<(), String>),
}

fn worker(root: PathBuf, rx: Receiver<Work>, tx: Sender<Report>, ctx: egui::Context) {
    let mut engine = Engine::new(root);
    let mut current = Settings::default();
    let status = |engine: &Engine| CaptureStatus {
        replay: engine.replay,
        recording: engine.recording,
        fallback: engine.fallback,
        devices: engine.devices.clone(),
        meters: engine.meters,
    };
    loop {
        let work = match rx.recv_timeout(Duration::from_millis(180)) {
            Ok(work) => Some(work),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let Some(work) = work else {
            match engine.tick(&current) {
                Ok(Some(media)) => {
                    let _ = tx.send(Report::Status(status(&engine)));
                    ctx.request_repaint();
                    let _ = tx.send(Report::Saved(
                        media,
                        Some("Recording saved after the monitor disconnected.".into()),
                    ));
                }
                Err(error) => {
                    engine.stop();
                    let _ = tx.send(Report::Status(status(&engine)));
                    let _ = tx.send(Report::Done(Err(error.to_string())));
                    ctx.request_repaint();
                }
                _ => {}
            }
            if engine.replay.is_some()
                || engine.recording.is_some()
                || !engine.meters.iter().all(|level| *level == 0.0)
            {
                let _ = tx.send(Report::Status(status(&engine)));
                ctx.request_repaint();
            }
            continue;
        };
        let send_media = |(media, warning): (Media, Option<String>)| {
            let _ = tx.send(Report::Saved(media, warning));
        };
        let result = match work {
            Work::Replay(settings) => {
                current = settings;
                engine.toggle_replay(&current)
            }
            Work::Recording(settings) => {
                current = settings;
                engine.toggle_recording(&current).map(|media| {
                    if let Some(media) = media {
                        send_media((media, None));
                    }
                })
            }
            Work::SaveReplay(settings) => {
                current = settings;
                engine
                    .save_replay(&current)
                    .map(|media| send_media((media, None)))
            }
            Work::Devices(settings) => engine.inspect_devices(&settings),
            Work::Screenshot(image, label, settings) => {
                let preview = settings
                    .screenshot_preview
                    .then(|| image::imageops::thumbnail(&image, 500, 340));
                capture::save(image, &label, &settings).map(|(media, warning)| {
                    if settings.screenshot_sound {
                        desktop::play_sound();
                    }
                    if let Some(image) = preview {
                        desktop::show_preview(image, media.path.clone());
                    }
                    send_media((media, warning));
                })
            }
            Work::Quit => break,
            Work::Reset => {
                engine.stop();
                Ok(())
            }
        };
        let _ = tx.send(Report::Status(status(&engine)));
        let _ = tx.send(Report::Done(result.map_err(|error| format!("{error:#}"))));
        ctx.request_repaint();
    }
}

#[derive(PartialEq)]
enum Page {
    Capture,
    History,
    Settings,
}
struct Tray {
    _icon: TrayIcon,
    open: MenuItem,
    region: MenuItem,
    arm: MenuItem,
    save: MenuItem,
    record: MenuItem,
    quit: MenuItem,
}

impl Tray {
    fn new() -> Result<Self> {
        let menu = Menu::new();
        let open = MenuItem::new("Open Kuma Capture", true, None);
        let region = MenuItem::new("Region screenshot", true, None);
        let arm = MenuItem::new("Arm instant replay", true, None);
        let save = MenuItem::new("Save clip", false, None);
        let record = MenuItem::new("Start recording", true, None);
        let quit = MenuItem::new("Quit", true, None);
        menu.append_items(&[&open, &region, &arm, &save, &record, &quit])?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("Kuma Capture · Idle")
            .with_menu(Box::new(menu))
            .with_icon(Icon::from_rgba(icon_pixels(), 64, 64)?)
            .build()?;
        Ok(Self {
            _icon: icon,
            open,
            region,
            arm,
            save,
            record,
            quit,
        })
    }
    fn status(&self, state: &CaptureStatus) {
        self.arm.set_text(if state.replay.is_some() {
            "Stop instant replay"
        } else {
            "Arm instant replay"
        });
        self.save.set_enabled(state.replay.is_some());
        self.record.set_text(if state.recording.is_some() {
            "Stop recording"
        } else {
            "Start recording"
        });
        let label = if state.recording.is_some() {
            "Recording"
        } else if state.replay.is_some() {
            "Replay armed"
        } else {
            "Idle"
        };
        let _ = self
            ._icon
            .set_tooltip(Some(format!("Kuma Capture · {label}")));
    }
}

struct App {
    settings: Settings,
    persisted: String,
    data: PathBuf,
    root: PathBuf,
    history: Vec<Media>,
    page: Page,
    advanced: [bool; 8],
    status: CaptureStatus,
    work: Sender<Work>,
    worker_thread: Option<thread::JoinHandle<()>>,
    reports: Receiver<Report>,
    report_sender: Sender<Report>,
    busy: bool,
    message: String,
    alert: bool,
    pending: Option<Settings>,
    previous_window: usize,
    tray: Option<Tray>,
    hotkeys: Option<GlobalHotKeyManager>,
    registered: Vec<(HotKey, usize)>,
    hotkey_errors: [String; 7],
    snipping: bool,
    hotkey_recorder: Option<shortcut::Recorder>,
    thumbnails: HashMap<PathBuf, egui::TextureHandle>,
    remotes: Vec<(String, String)>,
    upload_active: Vec<PathBuf>,
    file_category: Kind,
    filter: Option<Kind>,
    quit: bool,
    theme: Option<bool>,
    window_maximized: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, root: PathBuf, data: PathBuf) -> Self {
        crate::chrome::attach(cc);
        let mut message = String::new();
        let mut settings: Settings =
            model::read_json(&data.join("settings.json")).unwrap_or_else(|error| {
                message = format!("{error:#}");
                Settings::default()
            });
        for profile in &mut settings.profiles {
            match uploads::read_secret(&format!("KumaCapture/destination/{}", profile.id)) {
                Ok(Some(endpoint)) => profile.endpoint = endpoint,
                Ok(None) => {}
                Err(error) => message = format!("Could not load a storage credential: {error}"),
            }
        }
        let history = model::read_json(&data.join("history.json")).unwrap_or_default();
        let (work, rx) = mpsc::channel();
        let (report_sender, reports) = mpsc::channel();
        let worker_root = root.clone();
        let sender = report_sender.clone();
        let ctx = cc.egui_ctx.clone();
        let worker_thread = thread::spawn(move || worker(worker_root, rx, sender, ctx));
        let tray = Tray::new().ok();
        let hotkeys = GlobalHotKeyManager::new().ok();
        let remotes = uploads::discover(&root, &data.join("remotes.conf")).unwrap_or_default();
        let mut app = Self {
            persisted: serde_json::to_string(&settings).unwrap_or_default(),
            settings,
            root,
            data,
            history,
            page: Page::Capture,
            advanced: [false; 8],
            status: Default::default(),
            work,
            worker_thread: Some(worker_thread),
            reports,
            report_sender,
            busy: false,
            alert: !message.is_empty(),
            message,
            pending: None,
            previous_window: 0,
            tray,
            hotkeys,
            registered: Vec::new(),
            hotkey_errors: Default::default(),
            snipping: false,
            hotkey_recorder: None,
            thumbnails: HashMap::new(),
            remotes,
            upload_active: Vec::new(),
            file_category: Kind::Screenshot,
            filter: None,
            quit: false,
            theme: None,
            window_maximized: false,
        };
        app.register_hotkeys();
        app
    }

    fn notice(&mut self, message: impl Into<String>, alert: bool) {
        self.message = message.into();
        self.alert = alert;
    }
    fn send(&mut self, work: Work) {
        if self.busy {
            self.notice("One capture operation is still running.", true);
            return;
        }
        self.busy = self.work.send(work).is_ok();
        if self.busy {
            self.notice("Working…", false);
        } else {
            self.notice("The capture worker stopped.", true);
        }
    }

    fn register_hotkeys(&mut self) {
        let Some(manager) = &self.hotkeys else {
            self.hotkey_errors
                .fill("Windows hotkeys are unavailable.".into());
            return;
        };
        for (hotkey, _) in self.registered.drain(..) {
            let _ = manager.unregister(hotkey);
        }
        self.hotkey_errors.fill(String::new());
        for (index, configuration) in self.settings.hotkeys.iter().enumerate() {
            if configuration.trim().is_empty() {
                continue;
            }
            match configuration.parse::<HotKey>() {
                Ok(hotkey) => match manager.register(hotkey) {
                    Ok(()) => self.registered.push((hotkey, index)),
                    Err(_) => self.hotkey_errors[index] = "This shortcut is already in use.".into(),
                },
                Err(_) => self.hotkey_errors[index] = "Record a supported Windows shortcut.".into(),
            }
        }
    }

    fn remember_settings(&mut self) {
        let serialized = serde_json::to_string(&self.settings).unwrap_or_default();
        if serialized == self.persisted {
            return;
        }
        match model::write_json(&self.data.join("settings.json"), &self.settings) {
            Ok(()) => self.persisted = serialized,
            Err(error) => self.notice(format!("Could not save settings: {error}"), true),
        }
    }

    fn remember_history(&mut self) {
        if let Err(error) = model::write_json(&self.data.join("history.json"), &self.history) {
            self.notice(format!("Could not save history: {error}"), true);
        }
    }

    fn upload(&mut self, profile: UploadProfile, path: PathBuf, ctx: &egui::Context) {
        if self.upload_active.contains(&path) {
            return;
        }
        if let Some(item) = self.history.iter_mut().find(|item| item.path == path) {
            item.upload = format!("Uploading to {}…", profile.name);
            item.upload_profile = profile.id.clone();
            item.link.clear();
        }
        self.upload_active.push(path.clone());
        self.remember_history();
        let root = self.root.clone();
        let config = self.data.join("remotes.conf");
        let tx = self.report_sender.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result =
                uploads::upload(&root, &config, &profile, &path).map_err(|e| e.to_string());
            let _ = tx.send(Report::Upload(path, profile.name, result));
            ctx.request_repaint();
        });
    }

    fn connection(&mut self, profile: UploadProfile, ctx: &egui::Context) {
        self.notice("Checking the destination…", false);
        let root = self.root.clone();
        let config = self.data.join("remotes.conf");
        let tx = self.report_sender.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = uploads::test(&root, &config, &profile).map_err(|e| e.to_string());
            let _ = tx.send(Report::Connection(result));
            ctx.request_repaint();
        });
    }

    fn collect_reports(&mut self, ctx: &egui::Context) {
        while let Ok(report) = self.reports.try_recv() {
            match report {
                Report::Snip(result, mut settings) => {
                    self.snipping = false;
                    match result {
                        Ok(Some(selection)) => {
                            settings.copy_screenshot = selection.copy_image;
                            self.send(Work::Screenshot(
                                selection.image,
                                selection.label.into(),
                                settings,
                            ));
                        }
                        Ok(None) => {}
                        Err(error) => self.notice(error, true),
                    }
                }
                Report::Saved(item, warning) => {
                    let profile = self
                        .settings
                        .profiles
                        .iter()
                        .find(|profile| {
                            profile.id == self.settings.folders[item.kind.id()].auto_upload
                        })
                        .cloned();
                    let path = item.path.clone();
                    let duration = item
                        .duration
                        .map(|seconds| format!(" · {seconds}s"))
                        .unwrap_or_default();
                    self.notice(format!("{} saved{duration}", item.kind.label()), false);
                    self.history.insert(0, item);
                    if self.history.len() > 500 {
                        self.history.truncate(500);
                    }
                    self.remember_history();
                    if let Some(profile) = profile {
                        self.upload(profile, path, ctx);
                    }
                    if let Some(warning) = warning {
                        self.notice(warning, true);
                    }
                }
                Report::Status(status) => {
                    if self.status.replay != status.replay
                        || self.status.recording != status.recording
                    {
                        if let Some(tray) = &self.tray {
                            tray.status(&status);
                        }
                    }
                    self.status = status;
                }
                Report::Done(result) => {
                    self.busy = false;
                    if let Err(error) = result {
                        self.notice(error, true);
                    } else if self.message == "Working…" {
                        self.notice(
                            if self.status.fallback {
                                "Capture started with the software encoder."
                            } else {
                                "Ready"
                            },
                            false,
                        );
                    }
                }
                Report::Upload(path, name, result) => {
                    self.upload_active.retain(|active| active != &path);
                    if let Some(item) = self.history.iter_mut().find(|item| item.path == path) {
                        match result {
                            Ok(link) => {
                                item.upload = format!("Uploaded to {name}");
                                item.link = link;
                            }
                            Err(error) => item.upload = format!("Failed: {error}"),
                        }
                    }
                    self.remember_history();
                }
                Report::Connection(result) => match result {
                    Ok(()) => self.notice("Destination is reachable.", false),
                    Err(error) => self.notice(error, true),
                },
            }
        }
    }

    fn open(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn handle_action(&mut self, action: usize, ctx: &egui::Context) {
        if self.busy {
            self.notice("One capture operation is still running.", true);
            return;
        }
        let settings = self.settings.clone();
        if self.snipping {
            return;
        }
        match action {
            4 => self.send(Work::SaveReplay(settings)),
            5 => self.send(Work::Replay(settings)),
            6 => self.send(Work::Recording(settings)),
            0 => {
                self.snipping = true;
                desktop::dismiss_preview();
                let tx = self.report_sender.clone();
                let ctx = ctx.clone();
                thread::spawn(move || {
                    let result = desktop::snip(settings.cursor, settings.copy_screenshot)
                        .map_err(|error| error.to_string());
                    let _ = tx.send(Report::Snip(result, settings));
                    ctx.request_repaint();
                });
            }
            1..=3 => {
                desktop::dismiss_preview();
                let result = match action {
                    1 => capture::active_window(self.previous_window as HWND)
                        .map(|rect| (rect, "Window".into())),
                    2 => capture::selected_display(&settings.monitor)
                        .map(|display| (display.rect, display.label)),
                    _ => Ok((capture::virtual_desktop(), "Desktop".into())),
                };
                match result
                    .and_then(|(rect, label)| Ok((capture::screen(rect, settings.cursor)?, label)))
                {
                    Ok((image, label)) => self.send(Work::Screenshot(image, label, settings)),
                    Err(error) => self.notice(error.to_string(), true),
                }
            }
            _ => {}
        }
    }

    fn inputs(&mut self, ctx: &egui::Context) {
        let hwnd = capture::foreground();
        let mut pid = 0;
        if !hwnd.is_null() {
            unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
            if pid != 0 && pid != unsafe { GetCurrentProcessId() } {
                self.previous_window = hwnd as usize;
            }
        }
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == HotKeyState::Pressed && self.hotkey_recorder.is_none() {
                if let Some(action) = self
                    .registered
                    .iter()
                    .find(|(key, _)| key.id() == event.id)
                    .map(|(_, action)| *action)
                {
                    self.handle_action(action, ctx);
                }
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let Some(tray) = self.tray.as_ref() else {
                continue;
            };
            if event.id == *tray.open.id() {
                self.open(ctx);
            } else if event.id == *tray.region.id() {
                self.handle_action(0, ctx);
            } else if event.id == *tray.arm.id() {
                self.handle_action(5, ctx);
            } else if event.id == *tray.save.id() {
                self.handle_action(4, ctx);
            } else if event.id == *tray.record.id() {
                self.handle_action(6, ctx);
            } else if event.id == *tray.quit.id() {
                self.request_quit(ctx);
            }
        }
        while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
            if matches!(event, tray_icon::TrayIconEvent::DoubleClick { .. }) {
                self.open(ctx);
            }
        }
    }

    fn request_quit(&mut self, ctx: &egui::Context) {
        if self.status.recording.is_some() || self.busy {
            self.notice("Finish saving or stop the recording before quitting.", true);
            self.open(ctx);
        } else {
            self.quit = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn title_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("window chrome")
            .exact_height(39.0)
            .show(ctx, |ui| {
                let boundary = ui.max_rect();
                let text = ui.visuals().text_color();
                let painter = ui.painter();
                let logo =
                    Rect::from_min_size(boundary.min + Vec2::new(12.0, 7.0), Vec2::splat(24.0));
                paint_bear(painter, logo);
                painter.text(
                    boundary.min + Vec2::new(43.0, 20.0),
                    Align2::LEFT_CENTER,
                    "Kuma Capture",
                    FontId::proportional(14.0),
                    text,
                );
                let controls = 126.0;
                let drag = Rect::from_min_max(
                    boundary.min,
                    Pos2::new(boundary.right() - controls, boundary.bottom()),
                );
                let response =
                    ui.interact(drag, egui::Id::new("title drag"), Sense::click_and_drag());
                if response.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                self.window_maximized = ctx
                    .input(|input| input.viewport().maximized)
                    .unwrap_or(self.window_maximized);
                if response.double_clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!self.window_maximized));
                }
                for (i, (label, glyph)) in [
                    ("Minimize", "–"),
                    (
                        "Maximize or restore",
                        if self.window_maximized { "❐" } else { "□" },
                    ),
                    ("Close", "×"),
                ]
                .into_iter()
                .enumerate()
                {
                    let rect = Rect::from_min_size(
                        Pos2::new(
                            boundary.right() - controls + 42.0 * i as f32,
                            boundary.top(),
                        ),
                        Vec2::new(42.0, 39.0),
                    );
                    if crate::chrome::with_label(
                        ui.put(
                            rect,
                            egui::Button::new(RichText::new(glyph).size(19.0)).frame(false),
                        ),
                        label,
                    )
                    .clicked()
                    {
                        match i {
                            0 => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                            1 => ctx.send_viewport_cmd(ViewportCommand::Maximized(
                                !self.window_maximized,
                            )),
                            _ => ctx.send_viewport_cmd(ViewportCommand::Close),
                        }
                    }
                }
            });
    }

    fn page_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("pages").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for (label, page) in [
                    ("Capture", Page::Capture),
                    ("History", Page::History),
                    ("Settings", Page::Settings),
                ] {
                    if ui.selectable_label(self.page == page, label).clicked() {
                        self.page = page;
                    }
                }
            });
            ui.add_space(5.0);
        });
    }

    fn capture_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let status = if let Some(started) = self.status.recording {
            format!("Recording · {}s", started.elapsed().as_secs())
        } else if let Some(started) = self.status.replay {
            let elapsed = started.elapsed().as_secs();
            let target = self.settings.replay_seconds as u64;
            if elapsed < target {
                format!("Buffering · {elapsed}/{target}s")
            } else {
                "Replay ready".into()
            }
        } else {
            "Replay off".into()
        };
        let display = capture::selected_display(&self.settings.monitor)
            .map(|display| display.label)
            .unwrap_or_else(|_| "Selected display disconnected".into());
        ui.add_space(10.0);
        card(ui, |ui| {
            ui.set_min_height(124.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Kuma Capture").size(32.0).strong());
                    ui.label(display);
                    ui.add_space(6.0);
                    ui.label(RichText::new(status).strong().color(HONEY));
                });
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(138.0, 108.0), Sense::hover());
                    paint_bear(ui.painter(), rect);
                    ui.painter().text(
                        rect.min + Vec2::new(13.0, 22.0),
                        Align2::CENTER_CENTER,
                        "✦",
                        FontId::proportional(16.0),
                        HONEY,
                    );
                    ui.painter().text(
                        rect.right_top() + Vec2::new(-12.0, 31.0),
                        Align2::CENTER_CENTER,
                        "✦",
                        FontId::proportional(19.0),
                        HONEY,
                    );
                });
            });
        });
        ui.add_space(12.0);
        ui.columns(2, |columns| {
            card(&mut columns[0], |ui| {
                ui.heading("Screenshots");
                ui.add_space(5.0);
                for action in 0..4 {
                    if action_button(
                        ui,
                        ACTIONS[action],
                        &self.settings.hotkeys[action],
                        !self.busy,
                    ) {
                        self.handle_action(action, &ctx);
                    }
                }
            });
            card(&mut columns[1], |ui| {
                ui.heading("Clips & recording");
                ui.add_space(5.0);
                let arm = if self.status.replay.is_some() {
                    "Stop replay"
                } else {
                    "Arm replay"
                };
                let record = if self.status.recording.is_some() {
                    "Stop recording"
                } else {
                    "Start recording"
                };
                if action_button(ui, arm, &self.settings.hotkeys[5], !self.busy) {
                    self.handle_action(5, &ctx);
                }
                if action_button(
                    ui,
                    "Save clip",
                    &self.settings.hotkeys[4],
                    !self.busy && self.status.replay.is_some(),
                ) {
                    self.handle_action(4, &ctx);
                }
                if action_button(ui, record, &self.settings.hotkeys[6], !self.busy) {
                    self.handle_action(6, &ctx);
                }
                ui.add_space(4.0);
                ui.label("Video includes the entire selected display.");
            });
        });
    }

    fn history_view(&mut self, ui: &mut egui::Ui) {
        let mut upload = None;
        ui.horizontal(|ui| {
            ui.heading("History");
            ui.separator();
            ui.selectable_value(&mut self.filter, None, "All");
            for kind in Kind::ALL {
                ui.selectable_value(&mut self.filter, Some(kind), kind.label());
            }
        });
        ui.add_space(8.0);
        let mut count = 0;
        for media in self
            .history
            .iter()
            .filter(|item| self.filter.is_none_or(|kind| kind == item.kind))
        {
            count += 1;
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    if media.kind == Kind::Screenshot {
                        if !self.thumbnails.contains_key(&media.path) {
                            if let Ok(image) = image::open(&media.path) {
                                let thumbnail = image.thumbnail(100, 65).to_rgba8();
                                let pixels = egui::ColorImage::from_rgba_unmultiplied(
                                    [thumbnail.width() as usize, thumbnail.height() as usize],
                                    thumbnail.as_raw(),
                                );
                                self.thumbnails.insert(
                                    media.path.clone(),
                                    ui.ctx().load_texture(
                                        media.path.to_string_lossy(),
                                        pixels,
                                        egui::TextureOptions::LINEAR,
                                    ),
                                );
                            }
                        }
                        if let Some(texture) = self.thumbnails.get(&media.path) {
                            ui.image((texture.id(), texture.size_vec2()));
                        }
                    }
                    ui.vertical(|ui| {
                        let name = media.path.file_name().unwrap_or_default().to_string_lossy();
                        ui.label(RichText::new(name).strong());
                        let duration = media
                            .duration
                            .map(|seconds| format!(" · {seconds}s"))
                            .unwrap_or_default();
                        ui.small(format!(
                            "{} · {}{duration}",
                            media.timestamp.format("%b %d, %H:%M"),
                            media.dimensions
                        ));
                        if !media.upload.is_empty() {
                            ui.small(&media.upload);
                        }
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Open").clicked() {
                                let _ = Command::new("explorer").arg(&media.path).spawn();
                            }
                            if ui.button("Show in Explorer").clicked() {
                                let _ = Command::new("explorer")
                                    .arg(format!("/select,{}", media.path.display()))
                                    .spawn();
                            }
                            if ui.button("Copy path").clicked() {
                                ui.ctx()
                                    .copy_text(media.path.to_string_lossy().into_owned());
                            }
                            if !media.link.is_empty() && ui.button("Copy link").clicked() {
                                ui.ctx().copy_text(media.link.clone());
                            }
                            if !self.upload_active.contains(&media.path) {
                                if media.upload.starts_with("Failed:") {
                                    if let Some(profile) = self
                                        .settings
                                        .profiles
                                        .iter()
                                        .find(|item| item.id == media.upload_profile)
                                    {
                                        if ui.button("Retry").clicked() {
                                            upload = Some((profile.clone(), media.path.clone()));
                                        }
                                    }
                                }
                                if !self.settings.profiles.is_empty() {
                                    ui.menu_button("Send to…", |ui| {
                                        for profile in &self.settings.profiles {
                                            if ui.button(&profile.name).clicked() {
                                                upload =
                                                    Some((profile.clone(), media.path.clone()));
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                }
                            }
                        });
                    });
                });
            });
            ui.add_space(5.0);
        }
        if count == 0 {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(92.0), Sense::hover());
            paint_bear(ui.painter(), rect);
            ui.label("Captures appear here after you save them.");
        }
        if let Some((profile, path)) = upload {
            self.upload(profile, path, ui.ctx());
        }
    }

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        let old = self.settings.clone();
        let ctx = ui.ctx().clone();
        let category = self.settings.last_category.min(CATEGORIES.len() - 1);
        let width = ui.available_width();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(146.0, ui.available_height()),
                Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.heading("Settings");
                    ui.add_space(8.0);
                    for (id, name) in CATEGORIES.iter().enumerate() {
                        ui.selectable_value(&mut self.settings.last_category, id, *name);
                    }
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                Vec2::new((width - 170.0).max(420.0), ui.available_height()),
                Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width((width - 174.0).max(420.0));
                    let category = self.settings.last_category.min(CATEGORIES.len() - 1);
                    egui::ScrollArea::vertical()
                        .id_salt(category)
                        .show(ui, |ui| {
                            ui.heading(CATEGORIES[category]);
                            ui.add_space(8.0);
                            match category {
                                0 => {
                                    ui.checkbox(
                                        &mut self.settings.close_to_tray,
                                        "Keep Kuma running in the tray when closed",
                                    );
                                    ui.separator();
                                    ui.label(format!("Kuma Capture {}", env!("CARGO_PKG_VERSION")));
                                    ui.hyperlink_to(
                                        "Check for updates on GitHub",
                                        "https://github.com/hellohello143/backend/releases",
                                    );
                                }
                                1 => self.hotkey_settings(ui),
                                2 => {
                                    ui.horizontal(|ui| {
                                        ui.label("Image format");
                                        ui.selectable_value(
                                            &mut self.settings.screenshot_jpeg,
                                            false,
                                            "PNG",
                                        );
                                        ui.selectable_value(
                                            &mut self.settings.screenshot_jpeg,
                                            true,
                                            "JPEG",
                                        );
                                    });
                                    if self.settings.screenshot_jpeg {
                                        ui.add(
                                            egui::Slider::new(
                                                &mut self.settings.jpeg_quality,
                                                40..=100,
                                            )
                                            .text("JPEG quality"),
                                        );
                                    }
                                    ui.checkbox(&mut self.settings.cursor, "Include cursor");
                                    if ui
                                        .checkbox(
                                            &mut self.settings.screenshot_preview,
                                            "Show a preview after saving",
                                        )
                                        .changed()
                                        && !self.settings.screenshot_preview
                                    {
                                        desktop::dismiss_preview();
                                    }
                                    ui.checkbox(
                                        &mut self.settings.screenshot_sound,
                                        "Play a capture chime",
                                    );
                                    ui.checkbox(
                                        &mut self.settings.copy_screenshot,
                                        "Copy screenshot image to clipboard",
                                    );
                                    ui.checkbox(
                                        &mut self.settings.copy_path,
                                        "Copy saved file path to clipboard",
                                    );
                                }
                                3 => self.video_settings(ui),
                                4 => self.audio_settings(ui),
                                5 => self.file_settings(ui),
                                6 => self.upload_settings(ui),
                                7 => {
                                    ui.checkbox(&mut self.settings.dark, "Dark cocoa theme");
                                }
                                _ => {}
                            }
                            ui.add_space(15.0);
                            if ui.button("Restore category defaults").clicked() {
                                self.reset_category(category);
                            }
                        });
                },
            );
        });
        let capture_changed = old.monitor != self.settings.monitor
            || old.replay_seconds != self.settings.replay_seconds
            || old.replay_mb != self.settings.replay_mb
            || old.resolution != self.settings.resolution
            || old.fps != self.settings.fps
            || old.encoder != self.settings.encoder
            || old.quality != self.settings.quality
            || old.bitrate != self.settings.bitrate
            || old.desktop_audio != self.settings.desktop_audio
            || old.microphone != self.settings.microphone
            || old.desktop_device != self.settings.desktop_device
            || old.mic_device != self.settings.mic_device
            || old.cursor != self.settings.cursor;
        if capture_changed {
            if self.status.recording.is_some() || self.busy {
                self.settings = old;
                self.notice("Stop the recording or wait for the current operation before changing capture settings.", true);
            } else if self.status.replay.is_some() {
                self.pending = Some(self.settings.clone());
                self.settings = old;
            } else {
                self.send(Work::Reset);
            }
        }
        if category != self.settings.last_category {
            ctx.request_repaint();
        }
    }

    fn reset_category(&mut self, category: usize) {
        let defaults = Settings::default();
        match category {
            0 => self.settings.close_to_tray = defaults.close_to_tray,
            1 => {
                self.settings.hotkeys = defaults.hotkeys;
                self.register_hotkeys();
            }
            2 => {
                self.settings.screenshot_jpeg = defaults.screenshot_jpeg;
                self.settings.jpeg_quality = defaults.jpeg_quality;
                self.settings.cursor = defaults.cursor;
                self.settings.copy_screenshot = defaults.copy_screenshot;
                self.settings.copy_path = defaults.copy_path;
                self.settings.screenshot_preview = defaults.screenshot_preview;
                self.settings.screenshot_sound = defaults.screenshot_sound;
            }
            3 => {
                self.settings.monitor = defaults.monitor;
                self.settings.replay_seconds = defaults.replay_seconds;
                self.settings.replay_mb = defaults.replay_mb;
                self.settings.resolution = defaults.resolution;
                self.settings.fps = defaults.fps;
                self.settings.encoder = defaults.encoder;
                self.settings.quality = defaults.quality;
                self.settings.bitrate = defaults.bitrate;
            }
            4 => {
                self.settings.desktop_audio = defaults.desktop_audio;
                self.settings.microphone = defaults.microphone;
                self.settings.desktop_device = defaults.desktop_device;
                self.settings.mic_device = defaults.mic_device;
            }
            5 => self.settings.folders = defaults.folders,
            6 => {
                for profile in &self.settings.profiles {
                    let _ = uploads::save_secret(
                        &format!("KumaCapture/destination/{}", profile.id),
                        "",
                    );
                }
                self.settings.profiles.clear();
                for folder in &mut self.settings.folders {
                    folder.auto_upload.clear();
                }
            }
            7 => self.settings.dark = defaults.dark,
            _ => {}
        }
    }

    fn hotkey_settings(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("hotkeys")
            .num_columns(3)
            .spacing([16.0, 10.0])
            .show(ui, |ui| {
                for (index, action) in ACTIONS.iter().enumerate() {
                    ui.label(*action);
                    let key = &self.settings.hotkeys[index];
                    let label = if key.is_empty() { "Set shortcut" } else { key };
                    if ui
                        .button(label)
                        .on_hover_text("Click to record a shortcut")
                        .clicked()
                    {
                        if let Some(manager) = &self.hotkeys {
                            for (hotkey, _) in self.registered.drain(..) {
                                let _ = manager.unregister(hotkey);
                            }
                        }
                        self.hotkey_recorder = Some(shortcut::Recorder::new(index));
                    }
                    if ui
                        .add_enabled(!key.is_empty(), egui::Button::new("Clear"))
                        .clicked()
                    {
                        self.settings.hotkeys[index].clear();
                        self.register_hotkeys();
                    }
                    ui.end_row();
                    if !self.hotkey_errors[index].is_empty() {
                        ui.label("");
                        ui.colored_label(Color32::RED, &self.hotkey_errors[index]);
                        ui.label("");
                        ui.end_row();
                    }
                }
            });
    }

    fn record_hotkey(&mut self, ctx: &egui::Context) {
        let Some(recorder) = &mut self.hotkey_recorder else {
            return;
        };
        let index = recorder.index;
        let modifiers = recorder.modifiers();
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("record shortcut")).show(ctx, |ui| {
            ui.heading(ACTIONS[index]);
            ui.label("Press a key or hold Ctrl, Alt, Shift, or Windows while pressing a key.");
            if !modifiers.is_empty() {
                ui.label(format!("{} + …", modifiers.join(" + ")));
            }
            cancel = ui.button("Cancel").clicked();
        });
        ctx.request_repaint_after(Duration::from_millis(16));
        let input = if cancel {
            Some(shortcut::Input::Cancel)
        } else if ctx.input(|input| input.focused) {
            recorder.poll(ctx)
        } else {
            Some(shortcut::Input::Cancel)
        };
        if let Some(input) = input {
            self.hotkey_recorder = None;
            let previous = self.settings.hotkeys[index].clone();
            let mut conflict = None;
            if let shortcut::Input::Chord(chord) = input {
                if let Ok(proposed) = chord.parse::<HotKey>() {
                    conflict =
                        self.settings
                            .hotkeys
                            .iter()
                            .enumerate()
                            .find_map(|(other, binding)| {
                                let same = binding
                                    .parse::<HotKey>()
                                    .is_ok_and(|existing| existing.id() == proposed.id());
                                (other != index && same)
                                    .then(|| format!("Already assigned to {}.", ACTIONS[other]))
                            });
                    if conflict.is_none() {
                        self.settings.hotkeys[index] = chord;
                    }
                }
            }
            self.register_hotkeys();
            if !self.hotkey_errors[index].is_empty() && self.settings.hotkeys[index] != previous {
                conflict = Some(self.hotkey_errors[index].clone());
                self.settings.hotkeys[index] = previous;
                self.register_hotkeys();
            }
            if let Some(error) = conflict {
                self.hotkey_errors[index] = error;
            }
            while GlobalHotKeyEvent::receiver().try_recv().is_ok() {}
        }
    }

    fn advanced(&mut self, ui: &mut egui::Ui, category: usize) -> bool {
        ui.add_space(8.0);
        let text = if self.advanced[category] {
            "Hide advanced"
        } else {
            "Advanced"
        };
        if ui.button(text).clicked() {
            self.advanced[category] = !self.advanced[category];
        }
        self.advanced[category]
    }

    fn video_settings(&mut self, ui: &mut egui::Ui) {
        let displays = capture::displays();
        let label = displays
            .iter()
            .find(|item| item.id == self.settings.monitor)
            .map_or(
                if self.settings.monitor.is_empty() {
                    "Primary display"
                } else {
                    "Disconnected display"
                },
                |item| item.label.as_str(),
            );
        egui::ComboBox::from_id_salt("monitor")
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.settings.monitor, String::new(), "Primary display");
                for display in displays {
                    ui.selectable_value(&mut self.settings.monitor, display.id, display.label);
                }
            });
        ui.label("Clips and recordings always use the entire display.");
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Clip length");
            for value in [15, 30, 60, 120] {
                ui.selectable_value(
                    &mut self.settings.replay_seconds,
                    value,
                    format!("{value}s"),
                );
            }
        });
        ui.horizontal(|ui| {
            ui.label("Quality");
            ui.selectable_value(&mut self.settings.quality, "Small".into(), "Balanced");
            ui.selectable_value(&mut self.settings.quality, "HQ".into(), "High");
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Resolution");
            for value in [480, 720, 1080, 1440, 2160] {
                ui.selectable_value(&mut self.settings.resolution, value, format!("{value}p"));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Frame rate");
            for fps in [24, 30, 48, 60, 120] {
                ui.selectable_value(&mut self.settings.fps, fps, format!("{fps} FPS"));
            }
        });
        if self.advanced(ui, 3) {
            ui.add(
                egui::Slider::new(&mut self.settings.replay_seconds, 5..=600)
                    .text("Custom length (seconds)"),
            );
            ui.add(
                egui::Slider::new(&mut self.settings.replay_mb, 32..=8192)
                    .logarithmic(true)
                    .text("Replay memory (MB)"),
            );
            egui::ComboBox::from_id_salt("encoder")
                .selected_text(encoder_label(&self.settings.encoder))
                .show_ui(ui, |ui| {
                    for id in ["x264", "nvenc", "qsv", "amd"] {
                        ui.selectable_value(
                            &mut self.settings.encoder,
                            id.into(),
                            encoder_label(id),
                        );
                    }
                });
            ui.add(
                egui::DragValue::new(&mut self.settings.bitrate)
                    .range(0..=100_000)
                    .suffix(" kbps"),
            );
            ui.small(
                "A custom bitrate uses the software encoder. Set 0 to use the quality preset.",
            );
        }
    }

    fn audio_settings(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.settings.desktop_audio, "Capture system audio");
        ui.checkbox(&mut self.settings.microphone, "Capture microphone");
        for (index, (name, device)) in [
            ("System audio device", &mut self.settings.desktop_device),
            ("Microphone device", &mut self.settings.mic_device),
        ]
        .into_iter()
        .enumerate()
        {
            ui.label(name);
            let devices = &self.status.devices[index];
            let selection = devices
                .iter()
                .find(|item| item.1 == *device)
                .map_or("Windows default", |item| item.0.as_str());
            egui::ComboBox::from_id_salt(index)
                .selected_text(selection)
                .show_ui(ui, |ui| {
                    ui.selectable_value(device, "default".into(), "Windows default");
                    for (label, id) in devices {
                        ui.selectable_value(device, id.clone(), label);
                    }
                });
            ui.add(egui::ProgressBar::new(self.status.meters[index]).desired_width(200.0));
        }
        if ui
            .add_enabled(!self.busy, egui::Button::new("Find audio devices"))
            .clicked()
        {
            self.send(Work::Devices(self.settings.clone()));
        }
        ui.small("Meters are active during replay and recording.");
    }

    fn file_settings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for kind in Kind::ALL {
                ui.selectable_value(&mut self.file_category, kind, kind.label());
            }
        });
        let kind = self.file_category;
        let folder = &mut self.settings.folders[kind.id()];
        ui.add_space(6.0);
        ui.label("Save folder");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut folder.folder)
                    .desired_width(290.0)
                    .hint_text("Default folder"),
            );
            if ui.button("Browse…").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    folder.folder = path.to_string_lossy().into_owned();
                }
            }
        });
        ui.label("Subfolder");
        ui.text_edit_singleline(&mut folder.subfolder);
        ui.horizontal_wrapped(|ui| {
            ui.label("Date folders");
            ui.selectable_value(&mut folder.date_folders, DateFolders::None, "None");
            ui.selectable_value(&mut folder.date_folders, DateFolders::YearMonth, "yyyy/MM");
            ui.selectable_value(&mut folder.date_folders, DateFolders::Combined, "yyyy-MM");
        });
        ui.label("Filename template");
        ui.add(egui::TextEdit::singleline(&mut folder.template).desired_width(360.0));
        ui.small("{type}  {date:yyyy-MM-dd}  {time:HH-mm-ss-fff}  {monitor}  {resolution}  {duration}  {seq}");
        let ext = if kind != Kind::Screenshot {
            "mp4"
        } else if self.settings.screenshot_jpeg {
            "jpg"
        } else {
            "png"
        };
        let example = NameInfo {
            kind,
            timestamp: Local::now(),
            monitor: "Display 1",
            dimensions: "1920x1080",
            duration: if kind == Kind::Screenshot {
                None
            } else {
                Some(30)
            },
            ext,
        };
        match model::full_path(folder, &example, 1) {
            Ok(path) => {
                ui.label(RichText::new(path.to_string_lossy()).monospace());
            }
            Err(error) => {
                ui.colored_label(Color32::RED, error.to_string());
            }
        }
    }

    fn upload_settings(&mut self, ui: &mut egui::Ui) {
        let config = self.data.join("remotes.conf");
        ui.label("Set up a storage connection, then add a destination profile.");
        ui.horizontal(|ui| {
            if ui.button("Set up storage…").clicked() {
                if let Err(error) = uploads::configure(&self.root, &config) {
                    self.notice(error.to_string(), true);
                }
            }
            if ui.button("Refresh connections").clicked() {
                match uploads::discover(&self.root, &config) {
                    Ok(remotes) => self.remotes = remotes,
                    Err(error) => self.notice(error.to_string(), true),
                }
            }
            if ui.button("Add destination").clicked() {
                self.settings.profiles.push(UploadProfile {
                    id: format!("{:016x}", rand::random::<u64>()),
                    name: "New destination".into(),
                    ..Default::default()
                });
            }
        });
        ui.small("Google Drive, OneDrive, Dropbox, S3-compatible buckets, WebDAV and SFTP");
        let advanced = self.advanced(ui, 6);
        let mut delete = None;
        let mut test = None;
        let mut secret_error = None;
        for (i, profile) in self.settings.profiles.iter_mut().enumerate() {
            ui.add_space(5.0);
            card(ui, |ui| {
                ui.push_id(profile.id.clone(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Profile");
                        ui.text_edit_singleline(&mut profile.name);
                    });
                    egui::ComboBox::from_id_salt("remote")
                        .selected_text(if profile.remote.is_empty() {
                            "Choose storage connection"
                        } else {
                            &profile.remote
                        })
                        .show_ui(ui, |ui| {
                            for (name, kind) in &self.remotes {
                                let provider = uploads::PROVIDERS
                                    .iter()
                                    .find(|(_, id)| *id == kind)
                                    .map_or(kind.as_str(), |(label, _)| *label);
                                ui.selectable_value(
                                    &mut profile.remote,
                                    name.clone(),
                                    format!("{name} · {provider}"),
                                );
                            }
                        });
                    ui.horizontal(|ui| {
                        ui.label("Remote folder or prefix");
                        ui.text_edit_singleline(&mut profile.folder);
                    });
                    if advanced {
                        ui.horizontal(|ui| {
                            ui.label("Custom HTTPS PUT URL");
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut profile.endpoint)
                                        .password(true),
                                )
                                .changed()
                            {
                                if let Err(error) = uploads::save_secret(
                                    &format!("KumaCapture/destination/{}", profile.id),
                                    &profile.endpoint,
                                ) {
                                    secret_error = Some(error);
                                }
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Test connection").clicked() {
                            test = Some(profile.clone());
                        }
                        if ui.button("Remove").clicked() {
                            delete = Some(i);
                        }
                    });
                });
            });
        }
        if let Some(index) = delete {
            let removed = self.settings.profiles.remove(index);
            let _ = uploads::save_secret(&format!("KumaCapture/destination/{}", removed.id), "");
            for folder in &mut self.settings.folders {
                if folder.auto_upload == removed.id {
                    folder.auto_upload.clear();
                }
            }
        }
        if let Some(error) = secret_error {
            self.notice(error.to_string(), true);
        }
        if let Some(profile) = test {
            self.connection(profile, ui.ctx());
        }
        ui.add_space(8.0);
        ui.heading("Automatic upload");
        for kind in Kind::ALL {
            let id = &mut self.settings.folders[kind.id()].auto_upload;
            let selected = self
                .settings
                .profiles
                .iter()
                .find(|profile| profile.id == *id)
                .map_or("Off", |profile| profile.name.as_str());
            egui::ComboBox::from_id_salt(("auto upload", kind.id()))
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.selectable_value(id, String::new(), "Off");
                    for profile in &self.settings.profiles {
                        ui.selectable_value(id, profile.id.clone(), &profile.name);
                    }
                });
            ui.small(kind.label());
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.work.send(Work::Quit);
        if let Some(worker) = self.worker_thread.take() {
            let _ = worker.join();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(110));
        if self.theme != Some(self.settings.dark) {
            theme(ctx, self.settings.dark);
            self.theme = Some(self.settings.dark);
        }
        self.inputs(ctx);
        self.collect_reports(ctx);
        let recording_shortcut = self.hotkey_recorder.is_some();
        if ctx.input(|input| input.viewport().close_requested()) && !self.quit {
            if self.settings.close_to_tray && self.tray.is_some() {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            } else if self.status.recording.is_some() || self.busy {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.notice("Stop recording or finish saving before quitting.", true);
            }
        }
        self.title_bar(ctx);
        self.page_bar(ctx);
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(2.0);
            if !self.message.is_empty() {
                let color = if self.alert {
                    Color32::from_rgb(185, 66, 71)
                } else {
                    ui.visuals().text_color()
                };
                ui.colored_label(color, &self.message);
            }
            ui.add_space(2.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| match self.page {
            Page::Capture => {
                egui::ScrollArea::vertical().show(ui, |ui| self.capture_view(ui));
            }
            Page::History => {
                egui::ScrollArea::vertical().show(ui, |ui| self.history_view(ui));
            }
            Page::Settings => self.settings_view(ui),
        });
        if self.pending.is_some() {
            egui::Modal::new(egui::Id::new("confirm replay reset")).show(ctx, |ui| {
                ui.heading("Reset the replay buffer?");
                ui.label("Applying this change clears the current replay.");
                ui.horizontal(|ui| {
                    if ui.button("Apply and rearm").clicked() {
                        if let Some(settings) = self.pending.take() {
                            self.settings = settings.clone();
                            let _ = self.work.send(Work::Reset);
                            self.send(Work::Replay(settings));
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending = None;
                    }
                });
            });
        }
        if recording_shortcut {
            self.record_hotkey(ctx);
        }
        self.remember_settings();
    }
}

fn action_button(ui: &mut egui::Ui, name: &str, key: &str, enabled: bool) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        clicked = ui.add_enabled(enabled, egui::Button::new(name)).clicked();
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            ui.small(key);
        });
    });
    clicked
}

fn card(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .corner_radius(CornerRadius::same(12))
        .inner_margin(12)
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            contents(ui);
        });
}

fn encoder_label(id: &str) -> &'static str {
    match id {
        "nvenc" => "NVIDIA NVENC",
        "qsv" => "Intel Quick Sync",
        "amd" => "AMD hardware",
        _ => "Software x264",
    }
}

fn theme(ctx: &egui::Context, dark: bool) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = if dark {
        Color32::from_rgb(42, 31, 29)
    } else {
        PAPER
    };
    visuals.extreme_bg_color = if dark {
        Color32::from_rgb(58, 43, 39)
    } else {
        Color32::from_rgb(255, 253, 245)
    };
    visuals.override_text_color = Some(if dark {
        Color32::from_rgb(250, 234, 222)
    } else {
        BROWN
    });
    visuals.selection.bg_fill = if dark {
        Color32::from_rgb(103, 65, 67)
    } else {
        PINK
    };
    visuals.widgets.inactive.bg_fill = if dark {
        Color32::from_rgb(71, 49, 42)
    } else {
        Color32::from_rgb(250, 230, 211)
    };
    visuals.widgets.hovered.bg_fill = if dark {
        Color32::from_rgb(112, 70, 73)
    } else {
        PINK
    };
    ctx.set_visuals(visuals);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(9.0, 8.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    ctx.set_style(style);
}

fn icon_pixels() -> Vec<u8> {
    let mut pixels = vec![0; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let inside = |cx: f32, cy: f32, rx: f32, ry: f32| {
                ((x as f32 - cx) / rx).powi(2) + ((y as f32 - cy) / ry).powi(2) < 1.0
            };
            let color = if inside(22.0, 32.0, 2.6, 3.1)
                || inside(42.0, 32.0, 2.6, 3.1)
                || inside(32.0, 38.0, 3.6, 2.4)
            {
                [75, 45, 36, 255]
            } else if inside(32.0, 42.0, 11.0, 8.3) {
                [255, 242, 217, 255]
            } else if inside(12.0, 17.0, 7.0, 7.0) || inside(52.0, 17.0, 7.0, 7.0) {
                [251, 185, 177, 255]
            } else if inside(32.0, 35.0, 24.7, 20.0)
                || inside(12.0, 17.0, 11.0, 11.0)
                || inside(52.0, 17.0, 11.0, 11.0)
            {
                [190, 120, 75, 255]
            } else {
                [0, 0, 0, 0]
            };
            pixels[(y * 64 + x) * 4..][..4].copy_from_slice(&color);
        }
    }
    pixels
}

fn paint_bear(painter: &egui::Painter, bounds: Rect) {
    let centre = bounds.center();
    let unit = bounds.width().min(bounds.height()) / 64.0;
    let point = |x, y| centre + Vec2::new(x * unit, y * unit);
    let cocoa = Color32::from_rgb(186, 117, 72);
    for x in [-20.0, 20.0] {
        painter.circle_filled(point(x, -15.0), 11.0 * unit, cocoa);
        painter.circle_filled(point(x, -15.0), 6.7 * unit, PINK);
    }
    painter.add(egui::epaint::EllipseShape::filled(
        point(0.0, 4.0),
        Vec2::new(25.0, 20.0) * unit,
        cocoa,
    ));
    painter.add(egui::epaint::EllipseShape::filled(
        point(0.0, 11.0),
        Vec2::new(11.0, 8.0) * unit,
        PAPER,
    ));
    for x in [-10.0, 10.0] {
        painter.circle_filled(point(x, 1.0), 2.6 * unit, BROWN);
    }
    painter.circle_filled(point(0.0, 8.0), 2.8 * unit, BROWN);
    painter.line_segment(
        [point(0.0, 10.0), point(-3.2, 13.0)],
        Stroke::new(1.1 * unit, BROWN),
    );
    painter.line_segment(
        [point(0.0, 10.0), point(3.2, 13.0)],
        Stroke::new(1.1 * unit, BROWN),
    );
}
