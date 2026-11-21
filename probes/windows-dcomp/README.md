# Probe: native renderer on Windows through DirectComposition

Question: can 1-1 show remote frames on Windows through a GPU layer below
the web view, as it does on Hyprland ([Linux probe](../native-subsurface/)),
without forking wry? Tauri creates WebView2 with the windowed controller;
wry does not expose the composition controller.

Answer: yes. A DirectComposition target created on the main window with
`topmost = false` is composed **below** the window's child windows, and
WebView2's are among them. With the window transparent and the page leaving
a transparent hole, the layer shows through and the page's overlays draw
above it. [Results](results-2026-11-21.md).

The probe is a minimal Tauri app with 1-1's stack (versions from
`src-tauri/Cargo.lock`, copied here as the lockfile):

- `PROBE_MODE=layer` (default): on the main thread, `DCompositionCreateDevice2`,
  `CreateTargetForHwnd(hwnd, false)`, one visual as the target's root.
  wgpu 24 (DX12) creates its surface from that visual
  (`SurfaceTargetUnsafe::CompositionVisual`); wgpu calls `SetContent` but not
  `Commit`, so the probe commits after configuring and after each move.
  A render thread presents a pattern with Fifo at the hole's size.
  DirectComposition objects are not thread-safe: configure, offset and commit
  stay on the main thread, and a lock keeps a resize out of a frame in flight.
- `PROBE_MODE=canvas`: the page draws the same pattern into a canvas with two
  full-size `putImageData` calls per frame, the current 1-1 path without IPC.
- `PROBE_UPLOAD=full` (default), `strip` or `clear`, as in the Linux probe.
- `PROBE_THREAD=render`: the render thread configures and commits instead of the main thread, as 1-1 does. The layer shows either way (checked 21 November 2026), so DirectComposition calls only need serializing, not the creating thread.

```powershell
cd probes/windows-dcomp
cargo build --release
$env:PROBE_MODE = 'layer'; & "$env:CARGO_TARGET_DIR/release/probe-windows-dcomp.exe"
$env:PROBE_MODE = 'canvas'; & "$env:CARGO_TARGET_DIR/release/probe-windows-dcomp.exe"
pwsh -NoProfile -File ../../scripts/windows/Measure-AppCpu.ps1 -Seconds 20 -Name probe -ProcessName probe-windows-dcomp
```

Not production code: `strip` writes into a fresh swap-chain image whose
other rows are undefined, and the probe has one layer for one window.
