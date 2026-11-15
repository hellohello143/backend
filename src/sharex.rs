use crate::capture_trace::CaptureTrace;
use anyhow::{bail, Context, Result};
use image::RgbaImage;
use std::{fs, os::windows::process::CommandExt, path::Path, process::Command};

pub fn capture(
    root: &Path,
    data: &Path,
    mode: &str,
    bounds: String,
    cursor: bool,
    trace: Option<&CaptureTrace>,
) -> Result<Option<RgbaImage>> {
    let runtime = root.join("sharex");
    let engine = runtime.join("ShareX.exe");
    let hook = runtime.join("KumaShareX.dll");
    if !engine.is_file() || !hook.is_file() {
        bail!("The ShareX capture runtime is missing. Run the current Kuma Capture installer.");
    }
    let staging = tempfile::Builder::new()
        .prefix("sharex-")
        .tempdir_in(data)?;
    let output = staging.path().join("capture.png");
    let stages = staging.path().join("stages.txt");
    let mut command = Command::new(&engine);
    command
        .current_dir(&runtime)
        .env("DOTNET_STARTUP_HOOKS", hook)
        .env("KUMA_SHAREX_OUTPUT", &output)
        .env("KUMA_SHAREX_MODE", mode)
        .env("KUMA_SHAREX_BOUNDS", bounds)
        .env("KUMA_SHAREX_CURSOR", if cursor { "1" } else { "0" })
        .creation_flags(0x0800_0000);
    if let Some(trace) = trace {
        command.env("KUMA_SHAREX_STAGES", &stages);
        trace.stage("sharex_start");
    }
    let status = command
        .status()
        .context("Start the ShareX capture engine")?;
    if let (Some(trace), Ok(events)) = (trace, fs::read_to_string(stages)) {
        for stage in events.lines() {
            let Some((elapsed, event)) = stage.split_once(' ') else {
                continue;
            };
            if elapsed.parse::<u32>().is_ok()
                && matches!(
                    event,
                    "frame_requested"
                        | "frame_acquired"
                        | "overlay_requested"
                        | "overlay_visible"
                        | "selection_complete"
                        | "png_written"
                )
            {
                trace.stage(&format!("sharex +{elapsed}ms {event}"));
            }
        }
    }
    if !status.success() {
        let message = fs::read_to_string(output.with_extension("error"))
            .unwrap_or_else(|_| status.to_string());
        bail!("ShareX capture failed: {message}");
    }
    if output.is_file() {
        return Ok(Some(image::open(output)?.into_rgba8()));
    }
    if mode != "region" {
        bail!("ShareX did not produce a screenshot.");
    }
    Ok(None)
}
