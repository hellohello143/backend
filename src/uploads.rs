use crate::model::UploadProfile;
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::{distributions::Alphanumeric, Rng, RngCore};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use std::{fs, ptr::null_mut};
use windows_sys::Win32::Security::Credentials::*;
use xsalsa20poly1305::{
    aead::{Aead, KeyInit},
    XSalsa20Poly1305,
};

const STORAGE_KEY: &str = "KumaCapture/rclone";

pub fn read_secret(name: &str) -> Result<Option<String>> {
    let target: Vec<_> = name.encode_utf16().chain([0]).collect();
    let mut result = null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut result) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(1168) {
            return Ok(None);
        }
        return Err(error.into());
    }
    let value = unsafe {
        let entry = &*result;
        let bytes = if entry.CredentialBlobSize == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(entry.CredentialBlob, entry.CredentialBlobSize as usize)
        };
        let value = String::from_utf8(bytes.to_vec());
        CredFree(result.cast());
        value?
    };
    Ok(Some(value))
}

pub fn save_secret(name: &str, text: &str) -> Result<()> {
    let mut target: Vec<_> = name.encode_utf16().chain([0]).collect();
    if text.is_empty() {
        let deleted = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if deleted == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(1168) {
                return Err(error.into());
            }
        }
        return Ok(());
    }
    if text.len() > 2560 {
        bail!("This value exceeds the Windows credential limit.");
    }
    let mut bytes = text.as_bytes().to_vec();
    let mut entry: CREDENTIALW = unsafe { std::mem::zeroed() };
    entry.Type = CRED_TYPE_GENERIC;
    entry.TargetName = target.as_mut_ptr();
    entry.CredentialBlob = bytes.as_mut_ptr();
    entry.CredentialBlobSize = bytes.len() as u32;
    entry.Persist = CRED_PERSIST_LOCAL_MACHINE;
    if unsafe { CredWriteW(&entry, 0) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn storage_password(config: &Path) -> Result<String> {
    let secret = match read_secret(STORAGE_KEY)? {
        Some(secret) => secret,
        None if config.exists() => {
            bail!("The Windows credential for this encrypted storage config is missing.")
        }
        None => {
            let secret: String = rand::thread_rng()
                .sample_iter(&Alphanumeric)
                .take(48)
                .map(char::from)
                .collect();
            save_secret(STORAGE_KEY, &secret)?;
            secret
        }
    };
    if !config.exists() {
        let mut nonce = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let key = Sha256::digest(format!("[{secret}][rclone-config]"));
        let cipher = XSalsa20Poly1305::new_from_slice(&key)
            .map_err(|_| anyhow::anyhow!("Invalid storage key"))?;
        let sealed = cipher
            .encrypt((&nonce).into(), &b""[..])
            .map_err(|_| anyhow::anyhow!("Could not encrypt storage preferences"))?;
        let mut data = nonce.to_vec();
        data.extend(sealed);
        fs::write(
            config,
            format!(
                "# Encrypted rclone configuration File\n\nRCLONE_ENCRYPT_V0:\n{}",
                STANDARD.encode(data)
            ),
        )?;
    }
    Ok(secret)
}

const NO_WINDOW: u32 = 0x0800_0000;
pub const PROVIDERS: &[(&str, &str)] = &[
    ("Google Drive", "drive"),
    ("OneDrive", "onedrive"),
    ("Dropbox", "dropbox"),
    ("S3 / Cloudflare R2", "s3"),
    ("WebDAV / Nextcloud", "webdav"),
    ("SFTP", "sftp"),
];

fn program(root: &Path) -> Result<PathBuf> {
    let executable = root.join("rclone").join("rclone.exe");
    if !executable.is_file() {
        bail!("The storage runtime is missing. Install the complete Kuma Capture package.");
    }
    Ok(executable)
}

fn command(root: &Path, config: &Path) -> Result<Command> {
    let mut command = Command::new(program(root)?);
    command
        .arg("--config")
        .arg(config)
        .env("RCLONE_CONFIG_PASS", storage_password(config)?)
        .creation_flags(NO_WINDOW);
    Ok(command)
}

#[derive(Deserialize)]
struct Remote {
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

pub fn discover(root: &Path, config: &Path) -> Result<Vec<(String, String)>> {
    if !config.exists() {
        return Ok(Vec::new());
    }
    let output = command(root, config)?
        .args(["listremotes", "--json"])
        .output()?;
    if !output.status.success() {
        bail!("Could not read configured storage remotes.");
    }
    let remotes: Vec<Remote> = serde_json::from_slice(&output.stdout)?;
    Ok(remotes
        .into_iter()
        .filter(|remote| PROVIDERS.iter().any(|(_, kind)| *kind == remote.kind))
        .map(|remote| (remote.name, remote.kind))
        .collect())
}

pub fn configure(root: &Path, config: &Path) -> Result<()> {
    let mut command = command(root, config)?;
    command.arg("config").creation_flags(0x0000_0010);
    command
        .spawn()
        .context("Open the interactive storage setup")?;
    Ok(())
}

fn remote_path(profile: &UploadProfile) -> Result<String> {
    if profile.remote.is_empty() || profile.remote.contains([':', '/', '\\', '\r', '\n']) {
        bail!("Choose a configured storage remote.");
    }
    let folder = profile.folder.trim_matches('/');
    if folder.split('/').any(|part| part == "." || part == "..") {
        bail!("Choose a remote folder without traversal segments.");
    }
    Ok(format!("{}:{folder}", profile.remote))
}

pub fn test(root: &Path, config: &Path, profile: &UploadProfile) -> Result<()> {
    if !profile.endpoint.is_empty() {
        let url = parse_endpoint(profile)?;
        let response = agent().head(&url).call();
        return match response {
            Ok(_) | Err(ureq::Error::Status(405 | 501, _)) => Ok(()),
            Err(ureq::Error::Status(code, _)) => {
                bail!("The upload endpoint replied with HTTP {code}.")
            }
            Err(_) => bail!("Could not reach the upload endpoint over HTTPS."),
        };
    }
    let status = command(root, config)?
        .args([
            "lsd",
            &remote_path(profile)?,
            "--max-depth",
            "1",
            "--contimeout",
            "15s",
            "--timeout",
            "30s",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        bail!("Cannot list this destination. Check its path and credentials.");
    }
    Ok(())
}

pub fn upload(root: &Path, config: &Path, profile: &UploadProfile, path: &Path) -> Result<String> {
    if !profile.endpoint.is_empty() {
        let url = parse_endpoint(profile)?;
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        let result = agent()
            .put(&url)
            .set("Content-Type", "application/octet-stream")
            .set("Content-Length", &size.to_string())
            .send(file);
        return match result {
            Ok(response) => Ok(response
                .header("Location")
                .filter(|value| value.starts_with("https://"))
                .unwrap_or_default()
                .to_owned()),
            Err(ureq::Error::Status(code, _)) => {
                bail!("The upload endpoint replied with HTTP {code}.")
            }
            Err(_) => bail!("Could not upload to this HTTPS endpoint."),
        };
    }
    let destination = remote_path(profile)?;
    let status = command(root, config)?
        .arg("copy")
        .arg(path)
        .arg(destination)
        .args([
            "--immutable",
            "--retries",
            "3",
            "--contimeout",
            "15s",
            "--timeout",
            "45s",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        bail!("Upload failed. Check the destination, credentials, and available storage.");
    }
    Ok(String::new())
}

fn parse_endpoint(profile: &UploadProfile) -> Result<String> {
    let url = profile.endpoint.trim();
    let authority = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next());
    if authority
        .is_none_or(|host| host.is_empty() || host.contains('@') || host.contains(['?', '#', '\\']))
        || url.contains(['\r', '\n'])
    {
        bail!("Enter an HTTPS upload endpoint.");
    }
    Ok(url.to_owned())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(45))
        .timeout_write(Duration::from_secs(45))
        .build()
}
