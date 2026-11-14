# Kuma Capture

Kuma Capture is a Windows 11 x64 screenshot, full-display replay and recording app. This version is a Windows tester prerelease.

## Run the tester build

Run `KumaCaptureSetup.exe`, then launch Kuma Capture from Start. The installer includes the capture engine, storage runtime and Microsoft Visual C++ x64 redistributable. The portable ZIP can be extracted to a writable folder and opened with `KumaCapture.exe`; on a clean system, use the installer to set up the Microsoft runtime. See [the tester checklist](docs/TESTER.md) for expected results.

The **Capture** screen has screenshot controls, replay and manual recording. Replay starts when you click **Arm replay**. The default **Ctrl+Shift+5** saves a clip. Use Settings to change shortcuts, the capture display, quality, filenames and upload defaults. All video capture uses the entire selected display.

In **Settings → Uploads**, open **Set up storage…** to configure a connection, refresh connections, and then add a destination. Credentials in rclone's config are encrypted using a key stored in Windows Credential Manager. A simple custom HTTPS PUT profile is available under **Advanced**. Upload is optional and local output is always written first.

## Build on Windows

Install Rust 1.100.0 with the MSVC build tools, Python 3.12 or newer, and NSIS 3.11 or newer. From the repo root:

```powershell
cargo test --locked
cargo build --release --locked
python packaging/package.py --exe target/release/kuma-capture.exe --nsis "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
```

The packaging script verifies pinned downloads and writes the installer, portable ZIP and checksums to `dist`. GitHub Actions builds on Windows and packages artifacts, and version tags can publish a prerelease.

## Implementation and verification

- The Rust app includes a ShareX-derived screenshot workflow and launches a pinned OBS Studio runtime using its authenticated WebSocket API. Storage integrations use a pinned rclone runtime. See the [third-party notes](docs/THIRD-PARTY.md) for licensing and exact upstream revisions.
- This version contains a Rust ShareX adaptation and bundled upstream OBS, without separately published GitHub forks. Advanced custom upload is HTTPS PUT, with no `.sxcu` import. Video thumbnails and explicit public-link creation are pending.
- Windows display capture, hardware encoders, audio devices, Snap Layouts, and live storage providers require Windows tester verification. The app uses a custom illustrated bear and a Discord-style title bar.
