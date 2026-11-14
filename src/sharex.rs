use anyhow::{bail, Context, Result};
use image::RgbaImage;
use std::{fs, os::windows::process::CommandExt, path::Path, process::Command};

pub fn capture(
    root: &Path,
    data: &Path,
    mode: &str,
    bounds: String,
    cursor: bool,
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
    let status = Command::new(&engine)
        .current_dir(&runtime)
        .env("DOTNET_STARTUP_HOOKS", hook)
        .env("KUMA_SHAREX_OUTPUT", &output)
        .env("KUMA_SHAREX_MODE", mode)
        .env("KUMA_SHAREX_BOUNDS", bounds)
        .env("KUMA_SHAREX_CURSOR", if cursor { "1" } else { "0" })
        .creation_flags(0x0800_0000)
        .status()
        .context("Start the ShareX capture engine")?;
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
