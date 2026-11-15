#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(any(windows, test))]
type Version = (u32, u32, u32);

#[cfg(any(windows, test))]
fn version(text: &str) -> Option<Version> {
    let mut parts = text.trim().split('.');
    let result = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(result)
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn compares_release_numbers_and_rejects_unversioned_builds() {
        assert!(version("0.10.0") > version("0.9.9"));
        assert_eq!(version("0.3.0"), Some((0, 3, 0)));
        for invalid in ["0.3", "0.3.0.0", "0.3.0-beta", "tester-abc", ""] {
            assert_eq!(version(invalid), None);
        }
    }
}

#[cfg(windows)]
#[path = "../design.rs"]
#[allow(dead_code)]
mod design;

#[cfg(windows)]
mod windows {
    use crate::design;
    use anyhow::{bail, Context, Result};
    use eframe::egui::{self, RichText};
    use std::sync::mpsc;

    use serde::Deserialize;
    use sha2::{Digest, Sha256};
    use std::{
        fs::{self, File},
        io::{Read, Write},
        path::{Path, PathBuf},
        process::Command,
        ptr::{null, null_mut},
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, HANDLE},
        System::{
            Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY},
            Threading::{CreateMutexW, OpenEventW, OpenMutexW, SetEvent},
        },
        UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK},
    };

    const REPO: &str = "hellohello143/backend";
    const INSTALLER: &str = "KumaCaptureSetup.exe";
    const MANIFEST: &str = "KumaCaptureUpdate.json";
    const MAX_INSTALLER: u64 = 1_000_000_000;
    const INSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Kuma Capture";
    use super::{version, Version};

    #[derive(Deserialize)]
    struct Release {
        draft: bool,
        tag_name: String,
        assets: Vec<Asset>,
    }

    #[derive(Deserialize)]
    struct Asset {
        name: String,
        size: u64,
        digest: Option<String>,
        browser_download_url: String,
    }

    #[derive(Deserialize)]
    struct Manifest {
        version: String,
        installer: String,
        sha256: String,
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    pub fn message(text: &str, error: bool) {
        unsafe {
            MessageBoxW(
                null_mut(),
                wide(text).as_ptr(),
                wide("Kuma Capture Updater").as_ptr(),
                MB_OK
                    | if error {
                        MB_ICONERROR
                    } else {
                        MB_ICONINFORMATION
                    },
            );
        }
    }

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    fn registry(name: &str) -> Option<String> {
        let mut buffer = [0u16; 8192];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                wide(INSTALL_KEY).as_ptr(),
                wide(name).as_ptr(),
                RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
                null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != 0 {
            return None;
        }
        let size = buffer.iter().position(|v| *v == 0)?;
        String::from_utf16(&buffer[..size]).ok()
    }

    fn install_location() -> Option<PathBuf> {
        let location = registry("InstallLocation").map(PathBuf::from).or_else(|| {
            let uninstaller = registry("UninstallString")?;
            Path::new(uninstaller.trim().trim_matches('"'))
                .parent()
                .map(Path::to_owned)
        })?;
        (location.is_absolute() && location.join("KumaCapture.exe").is_file()).then_some(location)
    }

    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new()
            .user_agent("KumaCaptureUpdater")
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(60))
            .build()
    }

    fn asset_url(release: &Release, asset: &Asset) -> Result<String> {
        let tag = &release.tag_name;
        if !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        {
            bail!("Invalid release tag.");
        }
        let expected = format!(
            "https://github.com/{REPO}/releases/download/{tag}/{}",
            asset.name
        );
        if asset.browser_download_url != expected {
            bail!("The update asset is outside the Kuma Capture repository.");
        }
        Ok(expected)
    }

    fn available(
        agent: &ureq::Agent,
        installed: Version,
    ) -> Result<Option<(Version, String, String, u64)>> {
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
        let response = agent
            .get(&url)
            .set("Accept", "application/vnd.github+json")
            .call()
            .context("Check GitHub for releases")?;
        let releases: Vec<Release> =
            serde_json::from_reader(response.into_reader().take(2 * 1024 * 1024))?;
        let mut best = None;
        for release in releases.iter().filter(|release| !release.draft) {
            let Some(meta) = release
                .assets
                .iter()
                .find(|asset| asset.name == MANIFEST && asset.size <= 4096)
            else {
                continue;
            };
            let Some(setup) = release
                .assets
                .iter()
                .find(|asset| asset.name == INSTALLER && (1..=MAX_INSTALLER).contains(&asset.size))
            else {
                continue;
            };
            let response = agent
                .get(&asset_url(release, meta)?)
                .call()
                .context("Read the update manifest")?;
            let meta: Manifest = serde_json::from_reader(response.into_reader().take(4096))?;
            let Some(candidate) = version(&meta.version) else {
                continue;
            };
            if candidate <= installed
                || best
                    .as_ref()
                    .is_some_and(|(current, _, _, _)| *current >= candidate)
            {
                continue;
            }
            if meta.installer != INSTALLER
                || meta.sha256.len() != 64
                || !meta.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                bail!("The update manifest is invalid.");
            }
            if let Some(digest) = &setup.digest {
                if !digest.eq_ignore_ascii_case(&format!("sha256:{}", meta.sha256)) {
                    bail!("GitHub and the update manifest report different installer checksums.");
                }
            }
            best = Some((
                candidate,
                asset_url(release, setup)?,
                meta.sha256,
                setup.size,
            ));
        }
        Ok(best)
    }

    fn download(
        agent: &ureq::Agent,
        url: &str,
        path: &Path,
        hash: &str,
        size: u64,
        mut progress: impl FnMut(f32),
    ) -> Result<()> {
        let response = agent
            .get(url)
            .call()
            .context("Download the Kuma Capture installer")?;
        let mut input = response.into_reader().take(size + 1);
        let mut file = File::create(path)?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0; 128 * 1024];
        let mut written = 0;
        let mut last_percent = 0;
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            file.write_all(&buffer[..count])?;
            digest.update(&buffer[..count]);
            written += count as u64;
            let percent = (written * 100 / size).min(100);
            if percent > last_percent {
                progress(percent as f32 / 100.);
                last_percent = percent;
            }
        }
        file.sync_all()?;
        if written != size || !format!("{:x}", digest.finalize()).eq_ignore_ascii_case(hash) {
            bail!("The downloaded installer failed verification.");
        }
        Ok(())
    }

    fn app_running() -> bool {
        let mutex =
            unsafe { OpenMutexW(0x0010_0000, 0, wide(r"Local\KumaCaptureDesktop").as_ptr()) };
        if mutex.is_null() {
            return false;
        }
        drop(Handle(mutex));
        true
    }

    fn close_app() -> Result<bool> {
        if !app_running() {
            return Ok(false);
        }
        let event = unsafe { OpenEventW(2, 0, wide(r"Local\KumaCaptureUpdateExit").as_ptr()) };
        if event.is_null() {
            message(
                "Quit Kuma Capture from its system tray icon, then click OK to install the update.",
                false,
            );
        } else {
            let event = Handle(event);
            if unsafe { SetEvent(event.0) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let until = Instant::now() + Duration::from_secs(60);
        while app_running() && Instant::now() < until {
            thread::sleep(Duration::from_millis(300));
        }
        if app_running() {
            bail!("Kuma Capture is still running. Finish any capture or upload, quit from the tray, then reopen the updater.");
        }
        Ok(true)
    }

    pub fn run() -> Result<()> {
        let exe = std::env::current_exe()?;
        let local = PathBuf::from(
            std::env::var_os("LOCALAPPDATA").context("Find Windows application data")?,
        );
        let work = local.join("KumaCapture").join("updater");
        fs::create_dir_all(&work)?;
        let shadow = work.join(format!(
            "KumaCaptureUpdater-{}.exe",
            env!("CARGO_PKG_VERSION")
        ));
        if exe != shadow {
            let staging = tempfile::Builder::new().prefix("run-").tempdir_in(&work)?;
            let copy = staging.path().join("updater.exe");
            fs::copy(&exe, &copy)?;
            if !shadow.is_file() || fs::read(&shadow)? != fs::read(&copy)? {
                fs::rename(&copy, &shadow)?;
            }
            Command::new(&shadow)
                .spawn()
                .context("Start the independent update process")?;
            return Ok(());
        }
        let mutex =
            Handle(unsafe { CreateMutexW(null(), 0, wide(r"Local\KumaCaptureUpdater").as_ptr()) });
        if mutex.0.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        if unsafe { GetLastError() } == 183 {
            return Ok(());
        }

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([440., 260.])
                .with_resizable(false),
            ..Default::default()
        };
        eframe::run_native(
            "Kuma Capture Updater",
            options,
            Box::new(move |cc| {
                design::install_fonts(&cc.egui_ctx);
                let dark = fs::read(local.join("KumaCapture/settings.json"))
                    .ok()
                    .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
                    .and_then(|settings| settings["dark"].as_bool())
                    .unwrap_or(false);
                design::theme(&cc.egui_ctx, dark);
                let ctx = cc.egui_ctx.clone();
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let publish = |state| {
                        let _ = tx.send(state);
                        ctx.request_repaint();
                    };
                    let result = update(&work, &publish);
                    publish(match result {
                        Ok(message) => Stage::Finished(message),
                        Err(error) => Stage::Failed(format!("{error:#}")),
                    });
                });
                Ok(Box::new(UpdateWindow {
                    rx,
                    stage: Stage::Checking,
                }))
            }),
        )
        .map_err(|error| anyhow::anyhow!("{error}"))
    }

    enum Stage {
        Checking,
        Downloading(f32),
        Waiting,
        Installing,
        Finished(String),
        Failed(String),
    }

    struct UpdateWindow {
        rx: mpsc::Receiver<Stage>,
        stage: Stage,
    }

    impl eframe::App for UpdateWindow {
        fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
            while let Ok(stage) = self.rx.try_recv() {
                self.stage = stage;
            }
            if matches!(self.stage, Stage::Installing)
                && ctx.input(|i| i.viewport().close_requested())
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(design::Palette::new(ctx.style().visuals.dark_mode).bg).inner_margin(egui::Margin::same(19)))
                .show(ctx, |ui| {
                    let p = design::Palette::ui(ui);
                    ui.horizontal(|ui| {
                        let (area, _) = ui.allocate_exact_size(egui::Vec2::splat(47.), egui::Sense::hover());
                        design::bear(ui.painter(), area, matches!(self.stage, Stage::Finished(_)));
                        ui.label(design::heading("Kuma Capture Updater", 25.));
                    });
                    ui.add_space(8.);
                    let (title, hint) = match &self.stage {
                        Stage::Checking => ("Checking for updates", "Looking for a newer release on GitHub."),
                        Stage::Downloading(_) => ("Downloading update", "Kuma will verify the download before installing."),
                        Stage::Waiting => ("Waiting for Kuma Capture", "Finish any active capture or upload. Older versions may need to be closed from the tray."),
                        Stage::Installing => ("Installing update", "Your captures and preferences are available after the update."),
                        Stage::Finished(message) => ("Ready", message.as_str()),
                        Stage::Failed(message) => ("Update could not finish", message.as_str()),
                    };
                    ui.label(design::heading(title, 21.));
                    ui.label(RichText::new(hint).color(p.muted));
                    ui.add_space(8.);
                    match &self.stage {
                        Stage::Downloading(progress) => { ui.add(egui::ProgressBar::new(*progress).fill(p.accent).show_percentage()); }
                        Stage::Finished(_) | Stage::Failed(_) => {
                            if ui.button("Close").clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                        }
                        _ => { ui.spinner(); ctx.request_repaint_after(Duration::from_millis(55)); }
                    }
                });
        }
    }

    fn update(work: &Path, publish: &impl Fn(Stage)) -> Result<String> {
        let location = install_location().context("Kuma Capture setup could not be found. If you use the portable ZIP, download the new ZIP from GitHub Releases.")?;
        let installed = registry("DisplayVersion")
            .as_deref()
            .and_then(version)
            .unwrap_or((0, 1, 0));
        let agent = agent();
        let Some((new, url, hash, size)) = available(&agent, installed)? else {
            return Ok("Kuma Capture is already up to date.".into());
        };
        let folder = tempfile::Builder::new()
            .prefix("download-")
            .tempdir_in(work)?;
        let installer = folder.path().join(INSTALLER);
        publish(Stage::Downloading(0.));
        download(&agent, &url, &installer, &hash, size, |fraction| {
            publish(Stage::Downloading(fraction))
        })?;
        publish(Stage::Waiting);
        let running = close_app()?;
        publish(Stage::Installing);
        let status = Command::new(&installer)
            .arg("/S")
            .status()
            .context("Run verified Kuma Capture setup")?;
        if !status.success() {
            bail!("Kuma Capture setup did not complete ({status}).");
        }
        if registry("DisplayVersion").as_deref().and_then(version) != Some(new) {
            bail!("Setup exited, but the installed version could not be verified.");
        }
        if running {
            Command::new(location.join("KumaCapture.exe"))
                .spawn()
                .context("Reopen Kuma Capture")?;
        }
        Ok(format!(
            "Kuma Capture {}.{}.{} is installed.",
            new.0, new.1, new.2
        ))
    }
}

#[cfg(windows)]
fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        return;
    }
    if let Err(error) = windows::run() {
        windows::message(&format!("{error:#}"), true);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Kuma Capture Updater targets Windows.");
}
