# Native renderer for remote frames

## Introduction

On Linux, every frame that the web view shows makes GTK 3 repaint the whole window in software, on the thread that also carries mouse and keyboard input. Measured on 20 and 21 November 2026 ([profiling notes](../../../docs/profiling-linux.md)), remote screen updates keep that thread at 20–66% and delay each update by roughly 20 ms. Two fixes from 20 November already removed the idle cost (endless animations) and the pointer cost (native pointer). What remains is the remote picture itself.

The [subsurface probe](../../../probes/native-subsurface/results-2026-11-21.md) showed a way out on Wayland: a GPU layer in its own subsurface below a transparent web view. The same 60 Hz full-frame stream cost 28% CPU instead of 84%, and the main thread dropped from 34% to 1%. HTML overlays stayed on top and the web view kept all input.

The maintainer approved these requirements on 21 November 2026. This spec brings that layer into 1-1 on Linux, keeps today's canvas path as the fallback, and decides from measurements whether the same approach and the 20 November fixes pay off on macOS and Windows.

## Boundary

- **In scope:** showing decoded remote frames through a native GPU layer on Linux Wayland; choosing between the native layer and the canvas at runtime; measuring macOS and Windows with their own profilers; adopting the native layer there only where the measurements justify it.
- **Unchanged:** RDP decoding and codecs (ClearCodec stays the default), the frame data the session produces, input handling, the native pointer, and all UI that is not the remote picture. Toolbars, banners, dialogs and menus stay in the web view.
- **Out of scope:** decoding H.264 (parked; requirement 7 only keeps the renderer ready for it), new graphics settings in the UI, and a native renderer for X11 sessions. X11 and XWayland use the canvas.

## Requirements

### 1. Choose the display path, with today's canvas as the fallback

1. When a Linux session starts in a Wayland desktop session on a supported compositor and the native layer can be created, 1-1 shall show the remote picture through the native layer. Hyprland is the only supported compositor (maintainer decision, 21 November 2026); elsewhere 1-1 keeps the canvas unless `ONE_ONE_RENDERER=native` asks for the layer.
2. If the desktop session is X11, 1-1 runs under XWayland, the compositor lacks a required Wayland interface, no usable GPU adapter is found, or creating the layer fails, 1-1 shall show the remote picture through the web canvas exactly as before, without an error dialog and with the session usable.
3. If the native layer fails while a session is running (lost device, lost surface, a protocol error it can recover from), 1-1 shall switch that session to the canvas without dropping the connection or losing the current picture.
4. When 1-1 chooses or changes the display path, it shall log the path and the reason once, and show the active path in the session's diagnostics (the Performance HUD).
5. Settings shall offer the choice Automatic, GPU layer or Web canvas (amended 21 November 2026 at the maintainer's request; it replaces "no setting in the UI"). With Web canvas, 1-1 shall use the canvas; with GPU layer, 1-1 shall try the native layer, also on an untested compositor, and fall back only on failure; with Automatic, 1-1 chooses as in 1.1. The choice applies at the next start, and Settings shows the path in use and its reason. The environment variable `ONE_ONE_RENDERER` (`auto`, `native`, `canvas`), when set, overrides the setting for testing.
6. The native layer shall never stop a session that the canvas could show.

### 2. Show the same picture

1. While the native layer is active, the remote picture shall match the canvas path pixel for pixel at the same size, in every display mode (fit, native with scrolling) and scaling setting, without visible tearing or stale regions after an update.
2. When the window is resized, maximized, made fullscreen, or the sidebar or bottom bar is shown or hidden, the picture shall follow the session area within one frame of the web view's layout change, and shall not appear outside it.
3. On a display with fractional scaling, the picture shall be as sharp as the canvas path at the same scale or sharper.
4. When the user switches tabs, opens the dashboard or another view, the native layer of a session that is not on screen shall not be visible; when the session returns, its current picture shall appear without waiting for a server update.
5. While dialogs, menus, toolbars, banners and the Performance HUD are open over a session, they shall appear above the remote picture as they do today.
6. Mouse, keyboard, wheel and clipboard input, the native pointer and focus shall behave exactly as with the canvas.
7. While the display is paused, reconnecting, or the session has ended, the native layer shall show what the canvas would show in the same state, or be hidden so that the web view's state screen is visible.

### 3. Keep input responsive

1. While the native layer is active, remote screen updates shall not run on the thread that handles window events and input.
2. With the native layer, the main thread's CPU during remote screen updates shall be at most 5% of one core in the reference measurement (requirement 6.2), compared with today's 20–66%.
3. For the same remote activity, the whole app (1-1 and its web process) shall use less CPU with the native layer than with the canvas, by at least half in the reference measurement.
4. The time from a decoded update to its presentation shall not exceed the canvas path's, measured as in requirement 6.2.
5. When the remote picture does not change, the native layer shall present nothing and use no measurable CPU.

### 4. Several sessions and the session lifecycle

1. When several sessions are open, each shall keep its own picture; switching tabs shall show the right session's picture without mixing frames between sessions.
2. When a session reconnects, changes desktop size or reactivates, the native layer shall adopt the new size and show the new picture, as the canvas does today.
3. When a session closes, its native layer and GPU resources shall be released.
4. When the window is minimized or on a hidden workspace, the native layer shall not present frames.

### 5. macOS and Windows: measure, then adopt where it pays

1. The 20 November fixes shall apply on macOS and Windows: no endless animations in the session and dashboard views, and the native pointer shown as the system cursor with the correct size and hotspot on HiDPI displays (WKWebView and WebView2).
2. Before any native layer is built for macOS or Windows, a measurement on each platform (Instruments on macOS, Windows Performance Recorder on Windows) shall record main-thread and web-process CPU and per-update cost of the canvas path, with the same scenarios as requirement 6.2.
3. If that measurement shows the canvas path costs at least twice the CPU of the probe's native layer, or delays input noticeably, the spec shall be extended for that platform: a Metal layer below a transparent WKWebView on macOS, a DirectComposition visual below a transparent WebView2 on Windows. The choice and fallback rules of requirement 1 shall apply there too.
4. If the measurement shows no such cost, the platform shall keep the canvas, and the measurement shall be recorded as the reason.

### 5a. Windows: the native layer (amendment of 21 November 2026, approved by the maintainer)

The Windows measurement ([profiling](../../../docs/profiling-windows.md)) met the rule of 5.3: typing costs 28% and a window drag 68% of one core on the canvas path, and the [DirectComposition probe](../../../probes/windows-dcomp/results-2026-11-21.md) drew the same full-frame stream for 9.9% where the canvas took 44%, before any IPC. CPU was measured with `scripts/windows/Measure-AppCpu.ps1` (per-process and main-thread processor time); Windows Performance Recorder is documented for call stacks but was not needed for the decision.

1. When 1-1 starts on Windows and the layer can be created (a DX12 adapter, a DirectComposition device and target for the main window), 1-1 shall show the remote picture through a GPU layer below the web view, with the web view transparent over the session area. Automatic chooses the layer on Windows.
2. Requirements 1.2 to 1.6, 2, 3.1, 3.5, 4 and 6.3 to 6.4 shall apply on Windows as on Linux: every failure to create the layer falls back to the canvas without a dialog, a runtime failure switches all sessions to the canvas without reconnecting, overlays stay above the picture, input stays with the web view, and a hidden session or a minimized window presents nothing.
3. On displays with a scale other than 100%, and when the window moves between monitors with different scales, the picture shall be at least as sharp as the canvas path.
4. The reference measurement of 6.2 shall be repeated on the maintainer's Windows workstation for both paths. Targets: main thread at most 5% during remote updates, and the whole app at most half of the canvas path for the same activity.

### 6. Measurement and regression guard

1. The profiling tools for Linux (`scripts/linux/profile.sh`) and the equivalent steps for macOS and Windows shall be documented in `docs/`, so that each measurement can be repeated.
2. The reference measurement shall record, for the canvas and for the native layer on the same machine and session: an idle desktop, a scrolling terminal, typing with echo, and a full-screen update (a video or window drag); per scenario the main-thread CPU, the web-process CPU, the whole app's CPU, and the frame rate. The maintainer's Linux workstation (Hyprland, scale 1.6) is the reference machine and the only test environment: GNOME, KDE and X11 are not supported targets (maintainer decision, 21 November 2026), so they are not checked; requirement 1.1 keeps them on the canvas.
3. Tests shall fail if a session or dashboard view adds an animation that repeats forever, or if the session again composites the pointer into frames.
4. Automated tests shall cover the choice between the native layer and the canvas, including every fallback reason in requirement 1.2 and the runtime switch in 1.3, without a GPU.

### 7. Leave room for H.264

H.264 is parked on the branch `spike/h264-hw-decode`: GStreamer decodes with VA-API on Linux, and later Media Foundation and VideoToolbox. On the canvas path, every hardware-decoded frame would have to be read back from the GPU, converted from YUV to RGBA on the CPU and repainted by GTK. The native layer is the path that keeps hardware decoding cheap, so its design must not close that door.

1. The native layer's frame input shall accept, besides RGBA regions from the CPU, a decoded YUV frame (NV12 or I420) to be converted on the GPU, without a second renderer or a change to how sessions choose the display path.
2. The design shall name how a decoder's GPU buffer (a DMA-BUF from VA-API on Linux) could be imported without a copy, and what that needs beyond the current wgpu version. Building that import belongs to the H.264 spec.
3. Until H.264 ships, requirements 7.1 and 7.2 shall add no code path that sessions use, so ClearCodec behavior stays unchanged.
