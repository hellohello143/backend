#![cfg_attr(not(windows), allow(dead_code))]
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const ACTIONS: [&str; 7] = [
    "Region screenshot",
    "Active window screenshot",
    "Monitor screenshot",
    "All displays screenshot",
    "Save replay",
    "Toggle replay",
    "Toggle recording",
];
const DEFAULT_KEYS: [&str; 7] = [
    "Ctrl+Shift+1",
    "Ctrl+Shift+2",
    "Ctrl+Shift+3",
    "Ctrl+Shift+4",
    "Ctrl+Shift+5",
    "Ctrl+Shift+6",
    "Ctrl+Shift+7",
];
pub const DEFAULT_TEMPLATE: &str = "{type}_{date:yyyy-MM-dd}_{time:HH-mm-ss-fff}";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Screenshot,
    Clip,
    Recording,
}
impl Kind {
    pub const ALL: [Self; 3] = [Self::Screenshot, Self::Clip, Self::Recording];
    pub fn id(self) -> usize {
        self as usize
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Screenshot => "Screenshot",
            Self::Clip => "Clip",
            Self::Recording => "Recording",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DateFolders {
    #[default]
    None,
    YearMonth,
    Combined,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Destination {
    pub folder: String,
    pub subfolder: String,
    pub template: String,
    pub date_folders: DateFolders,
    pub auto_upload: String,
}
impl Default for Destination {
    fn default() -> Self {
        Self {
            folder: String::new(),
            subfolder: String::new(),
            template: DEFAULT_TEMPLATE.into(),
            date_folders: DateFolders::None,
            auto_upload: String::new(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UploadProfile {
    pub id: String,
    pub name: String,
    pub remote: String,
    pub folder: String,
    #[serde(skip)]
    pub endpoint: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub folders: [Destination; 3],
    pub hotkeys: [String; 7],
    pub monitor: String,
    pub replay_seconds: u32,
    pub replay_mb: u32,
    pub resolution: u32,
    pub fps: u32,
    pub encoder: String,
    pub quality: String,
    pub bitrate: u32,
    pub desktop_audio: bool,
    pub microphone: bool,
    pub desktop_device: String,
    pub mic_device: String,
    pub screenshot_jpeg: bool,
    pub jpeg_quality: u8,
    pub cursor: bool,
    pub copy_screenshot: bool,
    pub copy_path: bool,
    pub screenshot_preview: bool,
    pub screenshot_sound: bool,
    pub close_to_tray: bool,
    pub dark: bool,
    pub last_category: usize,
    pub profiles: Vec<UploadProfile>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folders: Default::default(),
            hotkeys: DEFAULT_KEYS.map(str::to_owned),
            monitor: String::new(),
            replay_seconds: 30,
            replay_mb: 512,
            resolution: 1080,
            fps: 60,
            encoder: "x264".into(),
            quality: "Small".into(),
            bitrate: 0,
            desktop_audio: true,
            microphone: false,
            desktop_device: "default".into(),
            mic_device: "default".into(),
            screenshot_jpeg: false,
            jpeg_quality: 90,
            cursor: false,
            copy_screenshot: false,
            copy_path: false,
            screenshot_preview: true,
            screenshot_sound: true,
            close_to_tray: true,
            dark: false,
            last_category: 0,
            profiles: Vec::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Media {
    pub kind: Kind,
    pub path: PathBuf,
    pub timestamp: DateTime<Local>,
    pub dimensions: String,
    pub duration: Option<u64>,
    pub upload: String,
    pub link: String,
    #[serde(default)]
    pub upload_profile: String,
}

pub fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("Read {}", path.display()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn write_json(path: &Path, data: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(data)?;
    let temp = path.with_extension("tmp");
    let mut file = File::create(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, path)?;
    Ok(())
}

pub struct NameInfo<'a> {
    pub kind: Kind,
    pub timestamp: DateTime<Local>,
    pub monitor: &'a str,
    pub dimensions: &'a str,
    pub duration: Option<u64>,
    pub ext: &'a str,
}

pub fn default_root() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("Videos")
        .join("Kuma Capture")
}

pub fn full_path(dest: &Destination, info: &NameInfo<'_>, sequence: u32) -> Result<PathBuf> {
    let mut path = if dest.folder.trim().is_empty() {
        default_root().join(format!("{}s", info.kind.label()))
    } else {
        let path = PathBuf::from(dest.folder.trim());
        if !path.is_absolute() {
            bail!("Choose an absolute save folder.");
        }
        path
    };
    for part in dest
        .subfolder
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
    {
        if matches!(part, "." | "..") || safe_name(part) != part {
            bail!("Use valid Windows names for each subfolder.");
        }
        path.push(part);
    }
    match dest.date_folders {
        DateFolders::None => {}
        DateFolders::YearMonth => {
            path.push(info.timestamp.format("%Y").to_string());
            path.push(info.timestamp.format("%m").to_string());
        }
        DateFolders::Combined => path.push(info.timestamp.format("%Y-%m").to_string()),
    }
    let template = if dest.template.trim().is_empty() {
        DEFAULT_TEMPLATE
    } else {
        &dest.template
    };
    let mut name = String::new();
    let mut remaining = template;
    while let Some((before, rest)) = remaining.split_once('{') {
        name.push_str(before);
        let (token, tail) = rest
            .split_once('}')
            .context("Close the filename token with }.")?;
        remaining = tail;
        let value = match token {
            "type" => info.kind.label().to_lowercase(),
            "monitor" => info.monitor.to_owned(),
            "resolution" => info.dimensions.to_owned(),
            "duration" => info
                .duration
                .map_or_else(|| "NA".into(), |seconds| seconds.to_string()),
            "seq" => format!("{sequence:03}"),
            "date:yyyy-MM-dd" => info.timestamp.format("%Y-%m-%d").to_string(),
            "date:yyyyMMdd" => info.timestamp.format("%Y%m%d").to_string(),
            "date:yyyy-MM" => info.timestamp.format("%Y-%m").to_string(),
            "time:HH-mm-ss-fff" => info.timestamp.format("%H-%M-%S-%3f").to_string(),
            "time:HHmmss" => info.timestamp.format("%H%M%S").to_string(),
            _ => bail!("Unknown filename token {{{token}}}."),
        };
        name.push_str(&value);
    }
    name.push_str(remaining);
    let safe = safe_name(&name);
    let suffix = if sequence > 1 && !template.contains("{seq}") {
        format!("_{sequence:03}")
    } else {
        String::new()
    };
    Ok(path.join(format!("{safe}{suffix}.{}", info.ext)))
}

fn safe_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .take(110)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim().trim_end_matches([' ', '.']);
    let base = clean
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let numbered = ["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(prefix).is_some_and(|rest| {
            matches!(
                rest,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    if clean.is_empty() {
        "capture".into()
    } else if numbered
        || matches!(
            base.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
    {
        format!("_{clean}")
    } else {
        clean.to_owned()
    }
}

pub fn save_unique(
    dest: &Destination,
    info: &NameInfo<'_>,
    mut write: impl FnMut(&mut File) -> Result<()>,
) -> Result<PathBuf> {
    for sequence in 1..=9999 {
        let path = full_path(dest, info, sequence)?;
        fs::create_dir_all(path.parent().context("Missing save folder")?)?;
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let result = write(&mut file).and_then(|()| Ok(file.sync_all()?));
                drop(file);
                if let Err(error) = result {
                    let _ = fs::remove_file(&path);
                    return Err(error);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).with_context(|| format!("Create {}", path.display())),
        }
    }
    bail!("Too many files with the same name.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> NameInfo<'static> {
        NameInfo {
            kind: Kind::Clip,
            timestamp: DateTime::parse_from_rfc3339("2025-03-09T17:22:40.123Z")
                .unwrap()
                .with_timezone(&Local),
            monitor: "../LAN",
            dimensions: "1920x1080",
            duration: Some(30),
            ext: "mp4",
        }
    }

    #[test]
    fn date_folders_and_tokens() {
        let sample = info();
        let dest = Destination {
            date_folders: DateFolders::YearMonth,
            ..Default::default()
        };
        let path = full_path(&dest, &sample, 2).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            sample
                .timestamp
                .format("clip_%Y-%m-%d_%H-%M-%S-%3f_002.mp4")
                .to_string()
        );
        assert_eq!(
            path.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
            sample.timestamp.format("%m").to_string()
        );
    }

    #[test]
    fn rejects_traversal_and_sanitizes_names() {
        let dest = Destination {
            subfolder: "../outside".into(),
            ..Default::default()
        };
        assert!(full_path(&dest, &info(), 1).is_err());
        let dest = Destination {
            template: "CON".into(),
            ..Default::default()
        };
        assert_eq!(
            full_path(&dest, &info(), 1).unwrap().file_name().unwrap(),
            "_CON.mp4"
        );
    }

    #[test]
    fn reserves_unique_files_and_cleans_up_write_failures() {
        let root = std::env::temp_dir().join(format!("kuma-test-{}", std::process::id()));
        let dest = Destination {
            folder: root.to_string_lossy().into(),
            ..Default::default()
        };
        let write = |file: &mut File| Ok(file.write_all(b"media")?);
        let first = save_unique(&dest, &info(), write).unwrap();
        let second = save_unique(&dest, &info(), write).unwrap();
        assert_ne!(first, second);
        assert!(save_unique(&dest, &info(), |_| bail!("write failed")).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preferences_survive_schema_additions() {
        let settings: Settings = serde_json::from_str(r#"{"replay_seconds":60}"#).unwrap();
        assert_eq!(settings.replay_seconds, 60);
        assert_eq!(settings.fps, 60);
        assert!(settings.screenshot_preview);
        assert!(settings.screenshot_sound);
        assert_eq!(settings.hotkeys[4], "Ctrl+Shift+5");
    }
}
