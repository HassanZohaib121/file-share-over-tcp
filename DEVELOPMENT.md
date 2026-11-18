# Development

## Live Slint preview

Open `ui/app.slint` in VS Code and run **Slint: Show Preview** from the Command Palette. The recommended Slint extension provides live preview as you edit. For a desktop preview window, run the **Slint: Live desktop preview** task.

To preview on Android, install the Slint Viewer app on the device, open its remote-viewer mode, then select that viewer from the Slint preview's **Remote** menu. Keep the device and development computer on the same network.

## Android app watcher

Install Android SDK Platform-Tools and the NDK, make `adb` available on `PATH`, and configure `ANDROID_HOME` and `ANDROID_NDK_ROOT`. Connect a device with USB debugging enabled, then run **Android: Watch, rebuild, and relaunch** from **Terminal > Run Task**.

The watcher builds and launches the app once, then rebuilds and relaunches it when Rust, Slint, or Cargo inputs change. Rust changes require an app restart; use the live Slint preview for immediate UI iteration. With multiple connected devices, run `scripts/watch-android.ps1 -Device <serial>` from PowerShell.
