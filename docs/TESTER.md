# Kuma Capture 0.1.0 Windows test checklist

1. Start on Windows 11 x64. Run `KumaCaptureSetup.exe`, then open Kuma Capture from Start. Alternatively, extract the whole portable ZIP to a writable folder. The portable build requires the Microsoft Visual C++ x64 runtime if it is absent on the PC.
2. Save PNG and JPEG screenshots via all four capture modes. Test region dragging, cursor inclusion, image/paste, active-window bounds, negative display coordinates and mixed-DPI monitors.
3. Select a display and arm replay. Save one early clip and two consecutive clips after the buffer fills. Confirm that the buffer stays armed, replay files play, and only the selected display is recorded. Trigger the same hotkey while a fullscreen app has focus.
4. Test manual recording alongside replay. Check video resolution and audio sync in the output. Toggle the mic and desktop audio independently and confirm the meter follows the selected device. Try an unavailable hardware encoder and check that software fallback succeeds.
5. Disconnect the selected display with replay armed. Confirm that capture pauses and Kuma displays an interruption. Test reconnecting, changing monitors and approving a replay reset.
6. Check Settings sidebar keyboard navigation, Advanced disclosure, file path preview, date folders, duplicate filenames, hotkey conflicts, and preference persistence after restart. Test title bar drag, double-click maximize, resize edges and the Windows Snap Layout hover menu.
7. Save while offline. For each provider you have a test account for, configure a test destination, upload manually and automatically, then force a failure and retry. Confirm the local file survives and the remote file is private according to the provider's permissions.
8. Test closing to the tray, restoring, and quitting. Use an empty test account and nonsensitive footage. For a failure, capture the Kuma status text, Windows version, GPU/driver, reproduction steps, and relevant OBS log excerpts after removing paths or secrets. OBS logs are under `obs/config/obs-studio/logs` beside Kuma Capture.

The downloadable binaries are unsigned prerelease builds. A Linux cross-build and automated checks are available. The hands-on Windows checks above are pending.
