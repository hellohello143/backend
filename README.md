# Kuma Capture build prompt

Build **Kuma Capture**, a Rilakkuma-themed Windows 11 x64 desktop app for screenshots, instant replay clips, and manual screen recording. Use [hellohello143/backend](https://github.com/hellohello143/backend) as the project repository. Deliver a working Windows executable for a tester, source code, reproducible build instructions, and a short tester checklist.

## Architecture

- Use Rust for the desktop shell, settings, global hotkeys, tray, history, and coordination between capture engines. Choose a native desktop UI stack with accessible controls and support for a custom title bar.
- Fork [ShareX](https://github.com/ShareX/ShareX) and [OBS Studio](https://github.com/obsproject/obs-studio) and pin both to known commits. Reuse ShareX’s C# capture and uploader components and OBS’s native display capture, encoding, and replay components. First prove the Rust shell can produce a real screenshot and a playable replay from the forks. Use thin native bindings or bundled helpers based on that proof.
- Provide one set of settings and one capture history. Preserve upstream notices and document the licenses, corresponding source, and build requirements for everything shipped.

## App layout and settings

- Give the app **Capture** and **History** views and an obvious **Settings** button. Show the selected monitor, replay status, and common shortcuts on Capture. Show thumbnails and open, reveal in Explorer, copy, and upload actions in History.
- Give Settings a clickable sidebar with **General**, **Hotkeys**, **Screenshots**, **Clips & Recording**, **Audio**, **Files & Folders**, **Uploads**, and **Appearance**. Show one category at a time and remember the last one. Support keyboard navigation, visible focus, and Windows display scaling.
- Put common controls first. Show monitor selection, clip length, and quality presets directly. Put bitrate, encoder, buffer memory limit, and similar fine-tuning behind a clearly labeled **Advanced** button within the relevant category. Start those sections collapsed and preserve values when they close.
- Use short labels, consistent spacing, and a reset action per category. Show hotkey conflicts beside the relevant shortcut. Apply ordinary changes immediately and confirm before a change discards the active replay buffer or interrupts a recording.

## Screenshots, clips, and hotkeys

- Support screenshot capture of a region, the active window, one monitor, or the entire desktop. Offer PNG and JPEG, JPEG quality, optional cursor capture, clipboard copy, and configurable after-capture actions.
- **Clips and manual recordings always use the entire selected monitor.** Default to the primary display and offer a monitor picker. Changing monitors resets the replay buffer. Pause and show a warning if the selected monitor disconnects. Make it clear that clips include everything visible on that display, including when the user switches apps.
- Offer 15-, 30-, 60-, and 120-second replay presets, a custom duration, and a memory cap. Default to 30 seconds. Add tray commands to arm, stop, and save replay. Show when the buffer is filling, report the actual duration of early saves, and keep the buffer running after a clip is saved.
- Default to H.264 at up to 1080p and 60 FPS, preserving aspect ratio. Offer resolution, FPS, bitrate or encoder quality, hardware encoding with a software fallback, and playable MP4 output. Allow independent system audio and mic capture, device selection, and level meters. Default the mic to off.
- Add configurable global hotkeys for each screenshot mode, saving and toggling replay, and starting or stopping recording. Provide sensible defaults and a reset action. Verify the save-replay hotkey with a fullscreen app focused.

## Local files and cloud storage

- Give screenshots, clips, and recordings separate save locations and filename templates. Default to `{type}_{date:yyyy-MM-dd}_{time:HH-mm-ss-fff}` in local time, for example `clip_2026-11-14_21-04-08-127.mp4`. Support monitor, resolution, duration, and sequence tokens. Preview the resolved filename and full path. Offer no date folders, `yyyy/MM`, and `yyyy-MM`, plus custom subfolders. Sanitize Windows filenames, constrain paths to the chosen folder, and handle simultaneous saves without overwriting files.
- Support local folders, including folders synced by desktop cloud clients, and Windows network shares. For direct uploads, offer **Google Drive, OneDrive, Dropbox, S3-compatible buckets** such as Amazon S3 and Cloudflare R2, **WebDAV** such as Nextcloud, and **SFTP**. Reuse ShareX uploader code where it fits. Keep **ShareX-compatible custom HTTP uploaders** in Advanced so users can connect their own upload or file-bin service.
- Let users save multiple named destination profiles and choose a default per capture type. Provide **Send to…** in History, optional automatic upload, remote folder or bucket-prefix selection, and a connection test. Support copying a link when the destination provides one. Default to local saves and private uploads. Make publicly shareable links an explicit action where supported.
- Save locally before uploading. Show progress and failures, keep the local copy, and let users retry from History. Store credentials in Windows Credential Manager or equivalent OS-backed storage. Keep credentials out of logs and exported settings. Use a shared upload interface and verify large-file handling, including resumable or multipart uploads where the provider supports them.

## Theme and title bar

- Use warm cream, honey brown, and soft strawberry pink, with rounded surfaces and custom Rilakkuma illustrations. Carry the theme through the app icon, tray, Capture screen, notifications, and empty states. Keep larger illustrations off dense settings pages and make the art replaceable so a distributed build can ship assets cleared for use.
- Build a Discord-style custom title bar with a draggable area and standard minimize, maximize or restore, and close buttons. Preserve Snap Layouts, double-click maximize, resize borders, keyboard access, and high-DPI behavior. Keep replay and recording status visible in the tray when the window closes.

## Windows executable and tester handoff

- Deliver a single portable `KumaCapture.exe` if the runtime can be bundled reliably. If OBS modules or runtime DLLs need companion files, deliver a finished `KumaCaptureSetup.exe` plus a complete portable ZIP. A tester should be able to run the app on a clean Windows 11 x64 machine without separately installing OBS, ShareX, or developer tools. Bundle dependencies for offline installation where practical. For any setup download, pin the version, use the official source, verify its checksum or signature, and document what requires internet.
- Configure GitHub Actions to build and package on Windows, run automated tests, and produce downloadable artifacts. Attach the versioned executable or installer, any portable ZIP, checksums, and the tester checklist to a GitHub prerelease when ready for hands-on testing.
- Automate checks for filename expansion, collisions, settings persistence, destination routing, and upload retry. Use test endpoints or test accounts for uploads. Record which providers passed live tests.
- Have the Windows tester check installation and launch, fullscreen hotkeys, mixed-DPI monitors, monitor disconnects, replay timing, repeated clips, audio sync, offline saves, Google Drive and other configured uploads, and settings navigation with Advanced collapsed. Include expected results and a way to collect logs with secrets removed. Track Windows and hardware checks separately from CI so a successful build is clearly distinguishable from a tested release.
