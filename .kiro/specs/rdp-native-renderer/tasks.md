# Native renderer implementation plan

Tasks 1–2 build the parts that need no GPU or window: the path choice, per-session frame stores and the session's frame sink. Tasks 3–4 bring the Wayland layer and the render thread into the app. Task 5 makes the page transparent over the session and connects layout. Task 6 handles failures and fallback, task 7 measures and verifies on the reference machine, task 8 prepares H.264, and task 9 covers macOS and Windows. Each leaf is one to three hours. The canvas path keeps working after every task.

- [x] 1. Choose the display path
- [x] 1.1 Decide native or canvas from startup facts
  - `renderer/display_path.rs`: `choose(facts) -> (Path, Reason)` with the reasons from the design; `ONE_ONE_RENDERER` parsing (`canvas`, `native`, unset). Pure function, unit tests for every reason and the override.
  - _Requirements: 1.1, 1.2, 1.5, 6.4_
- [x] 1.2 Collect the facts and expose the path
  - On Linux: GDK display type (Wayland or X11), advertised `wl_compositor` and `wl_subcompositor`. Log the chosen path and reason once. `get_display_path` command; `SessionInfo.displayPath` and `displayPathReason`; shown in the Performance HUD.
  - _Depends: 1.1_
  - _Requirements: 1.4_
  - Done 21 November 2026 (`renderer/display_path.rs`, `src/lib/display-path.ts`, HUD "Renderer" line). Two adjustments, also noted in the design: the path is app-wide state (`get_display_path` plus a `display-path` event) instead of a `SessionInfo` field, since every session switches together; and automatic mode keeps the canvas (`OptInOnly`) until task 7.3 passes, so `ONE_ONE_RENDERER=native` opts in during development. The GPU adapter is tried only when every other fact allows the layer.

- [x] 2. Frames for the native path in the session actor
- [x] 2.1 One frame store per session, with damage
  - `SharedFrame` records the union of damage since the last take and offers `take_damage()` (copies only that rectangle, under the lock) and a full take for a session switch. Created per session, dropped with it; the app-wide `SharedFrame` goes away. Unit tests: merging, full refresh on resize, take after take.
  - _Requirements: 4.1, 4.2, 4.3, 3.1_
- [x] 2.2 `FrameSink` in `present_image_update`
  - `Canvas` keeps today's path exactly; `Native` writes to the session's store and wakes the renderer, bypassing `FrameDelivery`. A stall is "no take for 5 s while visible". Switching `Native` → `Canvas` resets delivery and sends one full frame. Session tests with a recording channel: no packets on the native path, one full frame after a switch, no reconnect.
  - _Depends: 2.1_
  - _Requirements: 1.3, 1.6, 3.1_
  - Done 21 November 2026. `SharedFrame` keeps its buffer authoritative and tracks damage (`take_damage`, `take_full`); this also fixes the pinned defect B3-10, where `publish()` swapped the buffer and lost earlier rects. `renderer/frame_stores.rs` registers each native session's store weakly. On the native path `present_image_update` writes only the store; `SessionCommand::UseCanvas` switches a session to the canvas with one full frame (also accepted while connecting or reconnecting). Session tests now run the canvas path by default, with native twins for the store checks and a switch test. The stall check for the native path moves to 4.2, since it needs the render thread. The old opt-in renderer (`ONE_ONE_NATIVE_RENDERER`) is no longer fed until 4.1 replaces it.

- [ ] 3. The Wayland layer
- [ ] 3.1 `NativeLayer` below the web view
  - From the probe: GDK's `wl_display` and `wl_surface`, a desynchronized subsurface placed below, empty input region, full opaque region, buffer scale from GDK. Covers the web view: origin from `translate_coordinates`, size from the web view's `size-allocate`, `queue_draw` after each position change. Thread-local on GTK's main thread.
  - _Depends: 1.2_
  - _Requirements: 1.1, 2.2, 2.6_
- [ ] 3.2 Transparent window only on the native path
  - Create the main window with `WebviewWindowBuilder` in `setup` (from today's `tauri.conf.json` settings), `transparent(true)` only when the path is native. Check that Windows, macOS and the Linux canvas path create the same window as before.
  - _Depends: 1.2_
  - _Requirements: 1.2, 1.6_
- [ ] 3.3 Fractional scale where offered
  - With `wp_fractional_scale_v1` and `wp_viewporter`: render at the exact fractional size, viewport destination at the logical size. Without them: integer buffer scale.
  - _Depends: 3.1_
  - _Requirements: 2.3_

- [ ] 4. The render thread
- [ ] 4.1 Persistent texture, damage uploads, draw with rectangles
  - Rework `renderer/gpu.rs`: surface from the `NativeLayer` handles, one texture for the visible session, `take_damage` → `write_texture` → submit, clear to the background color, quad with source and destination rectangles, nearest at 1:1 and linear when scaled. Present only on change; FIFO, latency 2.
  - _Depends: 2.1, 3.1_
  - _Requirements: 2.1, 3.1, 3.5, 4.4_
- [ ] 4.2 The visible session
  - Backend view state per window: session id, destination and source rectangles. On a session change: full upload from that session's store. `null` view: draw only the background once, then stay idle. Stall on the native path: the visible session's store not taken for 5 s sets `display_stalled` (moved here from 2.2).
  - _Depends: 4.1_
  - _Requirements: 2.4, 4.1_

- [ ] 5. The page over the layer
- [ ] 5.1 `native-layer` styling
  - `<html class="native-layer">` from `get_display_path`. Audit every ancestor of the session canvas and make their backgrounds transparent under the class; panels beside the session (tab strip, sidebar, toolbar, status bar, dialogs) stay opaque. The canvas stays in the layout but draws nothing.
  - _Depends: 1.2_
  - _Requirements: 2.5, 2.7_
- [ ] 5.2 Report the view from `SessionCanvas`
  - `native_view(session_id, dest, source)` from `layoutSurface`, clipped to the scroll container; on mount, `ResizeObserver`, scroll, and `null` on unmount, tab switch and the screens that replace the canvas. Frontend tests for fit, native, scrolled and clipped rectangles and for `null`.
  - _Depends: 4.2, 5.1_
  - _Requirements: 2.2, 2.4, 2.7_

- [ ] 6. Failures and fallback
- [ ] 6.1 Runtime fallback to the canvas
  - Render-thread errors as in the design (reconfigure on `Lost`/`Outdated`; `RuntimeFailure` on device loss, out of memory, three failures). On `RuntimeFailure`: every session's sink switches to `Canvas` with a full frame, the frontend drops `native-layer`, the reason appears in the HUD. Test with an injected failure.
  - _Depends: 2.2, 4.1, 5.1_
  - _Requirements: 1.3, 1.6_
- [ ] 6.2 Regression guard
  - Frontend test: no `repeat: Infinity` or CSS `infinite` animation under `src/components/session` and `src/components/dashboard`. Rust test: `build_connector_config` keeps `pointer_software_rendering` false.
  - _Requirements: 6.3_

- [ ] 7. Verify and measure on the reference machine
- [ ] 7.1 Pixel parity
  - `ONE_ONE_RENDERER_CAPTURE=1` reads back the presented region; compare with the canvas at 1:1 (exact) and in fit mode (within linear filtering).
  - _Depends: 5.2_
  - _Requirements: 2.1_
- [ ] 7.2 Behavior checklist with the maintainer
  - Resize, maximize, fullscreen, sidebar and bottom bar, tab switches, dashboard, dialogs and HUD over the session, reconnect, pause, disconnect, minimize, another workspace, several sessions, input and pointer. Record in `validation.md`.
  - _Depends: 5.2, 6.1_
  - _Requirements: 2.2–2.7, 4.1–4.4_
- [ ] 7.3 Reference measurement
  - `scripts/linux/profile.sh`, 30 s per scenario and path: idle, scrolling terminal, typing with echo, full-screen update. Main-thread, web-process and whole-app CPU, frame rate, decode-to-present latency. Targets: main thread ≤ 5%, whole app at most half of the canvas, latency not worse. Record in `validation.md`.
  - _Depends: 7.2_
  - _Requirements: 3.2, 3.3, 3.4, 6.1, 6.2_
- [ ] 7.4 Other desktops before release
  - GNOME and KDE on Wayland (native), an X11 session and `GDK_BACKEND=x11` (canvas with the right reason), a machine without a usable Vulkan or GL driver (canvas).
  - _Depends: 7.3_
  - _Requirements: 1.2, 6.2_

- [ ] 8. Leave room for H.264
- [ ] 8.1 `FrameSource` with a YUV input
  - `FrameSource::{Rgba, Yuv420}`; NV12 and I420 planes as R8 and RG8 textures, a conversion shader (BT.709 limited, full range optional). Unit test with a synthetic frame against the CPU conversion. No session produces `Yuv420`.
  - _Depends: 4.1_
  - _Requirements: 7.1, 7.3_
- [ ] 8.2 Record the zero-copy route
  - Note in the H.264 spike branch: VA-API `vaExportSurfaceHandle`, Vulkan DMA-BUF import extensions, `wgpu-hal` `texture_from_raw` and `create_texture_from_hal`, and what wgpu version changes would simplify it.
  - _Requirements: 7.2_

- [ ] 9. macOS and Windows
- [ ] 9.1 Verify the 20 November fixes there
  - Native pointer on WKWebView and WebView2: size and hotspot on HiDPI, I-beam, hidden pointer; no endless animation in a quick profile.
  - _Requirements: 5.1_
- [ ] 9.2 Measure the canvas path and decide
  - `docs/profiling-macos.md` (Instruments) and `docs/profiling-windows.md` (Windows Performance Recorder) with the scenarios of 7.3. Apply the rule of requirement 5.3; record the decision and numbers. A platform that qualifies gets a spec amendment for its `NativeLayer`.
  - _Depends: 7.3, 9.1_
  - _Requirements: 5.2, 5.3, 5.4, 6.1_
