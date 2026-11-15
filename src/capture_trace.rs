//! Opt-in diagnostics store stages and timing, with capture contents and paths excluded.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct CaptureTrace {
    file: PathBuf,
    id: u128,
    mode: &'static str,
    started: Instant,
}

impl CaptureTrace {
    pub fn new(data: &Path, mode: &'static str) -> Self {
        Self {
            file: data.join("capture-diagnostics.log"),
            id: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros(),
            mode,
            started: Instant::now(),
        }
    }

    pub fn stage(&self, event: &str) {
        static LOCK: Mutex<()> = Mutex::new(());
        let Ok(_lock) = LOCK.lock() else {
            return;
        };
        if fs::metadata(&self.file).is_ok_and(|file| file.len() >= 256 * 1024) {
            let _ = fs::rename(&self.file, self.file.with_extension("previous.log"));
        }
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
        {
            let _ = writeln!(
                file,
                "{} {} +{}ms {event}",
                self.id,
                self.mode,
                self.started.elapsed().as_millis()
            );
        }
    }
}
