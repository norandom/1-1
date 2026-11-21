# 1-1 maintainer handover

**Next: the MacBook (agreed 21 November 2026, evening).** Master is pushed (`bca3708`), no release. In this order:
1. **Build and run** on the Mac: `npm ci`, then `npx tauri build --bundles app` or `npx tauri dev`. The macOS build compiles in CI but has never been run from source on a Mac; the `.dmg` was never opened either (not notarized: if macOS calls it damaged after `xattr -dr com.apple.quarantine`, the bundle needs ad-hoc signing).
2. **Renderer 9.1, macOS half:** the native pointer in WKWebView (size and hotspot on Retina, the I-beam's checker pattern, a hidden pointer), no endless animation at idle.
3. **Renderer 9.2, macOS half:** measure the canvas path with the four scenarios (idle, scrolling terminal, typing with echo, window drag) and write `docs/profiling-macos.md`. `Measure-AppCpu.ps1` is Windows-only; on macOS use Instruments (Time Profiler, plus the WebContent process) or a `ps`-based per-second sampler for 1-1 and its `com.apple.WebKit.WebContent` and `GPU` processes. Apply the rule of requirement 5.3 against the Linux layer's about 6% and the Windows numbers below. If it qualifies, the sketch is a `CAMetalLayer` below the WKWebView's layer with `drawsBackground` off, wgpu on Metal: the same split as `native_layer/dcomp.rs` (a probe first, as for Windows).
4. **Keyboard on macOS:** the layout from TIS (status bar, "Detected" in the host's Keyboard section), German Mac layouts reported as German (closest match), Cmd as Ctrl (Super as Ctrl), and whether macOS takes Cmd chords (Cmd+Q, Cmd+Tab, Cmd+Space) before 1-1 sees them.
5. **HiDPI scaling:** Automatic should give 200% on Retina.
6. **Shared folders from the macOS build** (`ec5fead`, tasks 7.1–7.4): never run on a Mac. Picker (NSOpenPanel), a read-only and a read-write share against Windows or xrdp, `/dev/fd/N` access.

CI note from 21 November: GitHub reads `[skip ci]` from the **last** commit of a push only. A push of code commits that ends with a `[skip ci]` docs commit runs no CI at all; start it with `gh workflow run ci.yml --ref master`. Linux CI denies warnings: anything used only on Windows or macOS needs `cfg_attr(not(...), allow(dead_code))`, as the first run after the Windows native layer showed (`bca3708`).

**21 November 2026, evening (Windows workstation).** First part of "Where to resume", item 0, done against the xrdp host; pushed as `baa98dd` and `bca3708`.
- **Keyboard on Windows, fixed.** The layout is the high word of the HKL, not the input language in the low word; English (Germany) on the German layout gave `0x2000`, no layout at all. Variant layouts (`0xFnnn`) are found by "Layout Id" in the registry. A layout servers lack (the maintainer's German (Magic Utilities), `A0010407`) is reported as its language's base layout, closest match. Verified: `00000407 (German, ClosestMatch)`.
- **xrdp resize, fixed with a host setting.** xrdp resizes by deactivation-reactivation, and in between sends PDUs upstream's sequence rejects. 1-1 now holds every PDU that is not part of the sequence (fast-path, static channels, slow-path updates) and replays it in order once the new share is active (`belongs_to_activation`, `HeldPdus` in `rdp/session.rs`). With Bulk compression on, xrdp also compresses the finalization PDUs themselves, and the sequence cannot reach the session's decompressor (private in `ActiveStage`): the session still reconnects, and the log says so and names the workaround. With the host's Bulk compression Off, resizes reactivate in place; verified by the maintainer, shared folder still accepted. The real fix is upstream: connection finalization decompressing with the session's history.
- **CPU on Windows:** `scripts/windows/Measure-AppCpu.ps1` samples main thread, app and WebView2 processes per second, no admin needed. Idle xrdp session: whole app 2.1% of one core (main thread 0.2%, WebView2 renderer 1.4%), so no endless animation.
- **Native pointer on Windows:** fine (maintainer). Renderer task 9.1, Windows half, done.
- **Canvas measured, then a native layer for Windows** ([profiling](../docs/profiling-windows.md)). The canvas cost 28% of one core for typing and 68% for a window drag, most of it moving frames through the Tauri channel (WebView2 browser process, main thread). The maintainer chose the native layer. [Probe](../probes/windows-dcomp/results-2026-11-21.md): a DirectComposition target with `topmost = false` on the main window is composed below WebView2's child windows, so no wry fork. Spec amendment 5a approved (Automatic chooses the layer on Windows), tasks 10.1–10.3 implemented (`renderer/native_layer/dcomp.rs`). Measured on the layer: drag 26.7% (main thread 1.5%), typing 15.4% (the rest is key-event IPC).
- **Still open:** renderer 10.4 one by one (snap, a monitor with another scale, several sessions, `ONE_ONE_RENDERER_FAIL_AFTER`); the Windows key: Windows opens the Start menu before 1-1 sees it, so Super as Ctrl needs a low-level keyboard hook while a session has focus, as mstsc does; batching key-event IPC (typing); the 1 s resize delay against a Windows host; macOS 9.1/9.2.

**21 November 2026, afternoon.** Master is pushed (`ef27194`), CI green on Linux, Windows and macOS, no release. New since the morning: the keyboard layout feature (spec `rdp-keyboard-layout`, verified by the maintainer against Windows) and a slower, smarter remote resize (1 s, and none for a panel that slides in and out). **Next: a session on the Windows workstation** (see "Where to resume", item 0).

**21 November 2026.** The native renderer works and is the default on Hyprland. Pushed on 21 November (`2763812`), unreleased; `spike/h264-hw-decode` (`b27d9d8`) is local too. Remote screens on Linux now draw through a GPU layer below the web view: with the remote screen changing, the main thread averages about 1.5% instead of 15% (peaks 66%), and the whole app about 6% of one core. Also new today: the Renderer setting, an automatic fallback to the canvas, the end of WebKit's crash at quit, YUV input ready for H.264, and a measurement that rejected LTO. Next: spec group 9 (macOS and Windows) needs the MacBook and the Windows workstation; then pushing and a release.

**20 November 2026.** Five commits on master since beta.1, unreleased and not pushed (`ec5fead`..`0104322`): folder sharing from the Linux and macOS builds (tasks 7.1–7.4), Esc goes to the remote, the Linux CPU and pointer fixes, the Linux profiler, and the rdp-terminal-graphics spec. H.264 is parked on the local branch `spike/h264-hw-decode` (not pushed). Next, agreed with the maintainer: a spike for the native renderer on Linux (see "Where to resume"). The Nix flake and APT repository from 19 November are still open.

**19 November 2026, evening.** The latest release is **v0.1.0-beta.1** (tag on `2e90e87`): folder sharing works end to end on Windows and xrdp hosts, with several folders per host and the File transfers setting; hosts copy as a command line. Assets: Windows x64 installer, Linux `.deb` and AppImage, macOS `.dmg` and update archive; the feed lists Windows x64, macOS arm64 and the AppImage. Next, agreed with the maintainer: a Nix flake checked in CI (the maintainer will maintain a later nixpkgs package), then a signed APT repository on GitHub Pages. See the folder-sharing notes below.

**19 November 2026.** 1-1 (pronounced “zero”) is an RDP client for coding and monitoring remote agents. The latest release is **v0.1.0-alpha.7**: per-host HiDPI scaling, per-host password saving and the first macOS `.dmg`. The maintainer still has to confirm that Linux passwords survive a logout (KWallet) and that scaling looks right on the MacBook. Folder sharing comes next, through attaching the drive to sessions (task 5.2), before the usability items below. [Releases](https://github.com/norandom/1-1/releases)

## Releases

- **alpha.7** (`20e4e4c`, current feed).
  - Per-host Scaling: Automatic (default, desktop scale = device pixel ratio × 100), Custom percentage (100–300%), Off (100%), Low resolution. Sent at connect and on every Display Control resize.
  - Per-host `savePassword` (default on). Off: no keychain entry, password asked at connect and held by the session. A host with no saved password also gets the prompt.
  - Linux keychain errors name KWallet and GNOME Keyring.
  - Assets add the macOS arm64 `.dmg` (not notarized) and the macOS update archive `1-1.app.tar.gz` with its `.sig`. The feed lists Windows x64, macOS arm64 and the AppImage.
- **alpha.6** (`705fecd`).
  - Security: Tauri 2.12.5 (IPC origin confusion), tauri-plugin-updater 2.14.1 with zip 9.3.0 (critical), the shadcn CLI package removed (its stylesheet is vendored in `src/styles/shadcn-tailwind.css`), and npm, serde_with and rand fixes.
  - Linux passwords use the Secret Service (keyring feature `linux-native-sync-persistent`) instead of the kernel keyring alone, which lost them at logout.
  - Clipboard text is LF on Linux and macOS and CRLF toward the remote.
  - Bundle targets are listed explicitly (NSIS, deb, AppImage, app, DMG). With `"all"`, Tauri 2.12 rejected the pre-release version for RPM.
- **alpha.5** (`c84ab4b`). Fixes the Wayland exit with protocol error 71 ([#1](https://github.com/norandom/1-1/issues/1), closed). The native wgpu renderer drew under the web view, which already draws every frame. It now starts only with `ONE_ONE_NATIVE_RENDERER=1`. This release also adds the folder sharing preview (the choice is saved, but no session offers the folder yet) and a Linux `.deb`.
- **alpha.4** (`832ca8c`). Pause display, credential test, server identity review, sidebar Disconnect, resizable and hideable sidebar, per-host bottom bar, and fixes for the window-snapping drag and a resize crash.
- **Published assets:** each release has the Windows x64 installer plus the Linux `.deb` and AppImage, with signatures and `SHA256SUMS.txt`. `latest.json` on `preview-channel` lists Windows x64 and the AppImage. A `.deb` cannot replace itself, so `.deb` users update by installing the next release's `.deb`. Windows ARM64 and macOS are built in CI but not published.
- **Releasing:**
  1. Bump the five version files and push.
  2. When CI is green, push a `v*` tag. The tag triggers `preview-build.yml`, which exports the signed AppImage and the `.deb` together.
  3. Verify each asset's SHA-256 against its `build-info.json`, and check that the `.sig` key ID matches the updater key in `tauri.conf.json`.
  4. Create the release, then run `scripts/update-manifest.mjs` on a folder that holds only the published platforms (`windows-x86_64`, `darwin-aarch64` without the `.dmg`, `linux-x86_64` without the `.deb`), and upload `latest.json`. The release assets are the Windows installer and `.sig`, the AppImage and `.sig`, the `.deb`, the `.dmg`, `1-1.app.tar.gz` and `.sig`, and `SHA256SUMS.txt`.

  GitHub's macOS arm64 runners often fail to start a job. Rerun that job, or release on green code checks when macOS is not shipped.

## Where to resume

0. **Next: the Windows workstation (agreed 21 November).** Build with `pwsh -NoProfile -File scripts/Invoke-WindowsPreview.ps1` (see "CI and local builds"). Check, in this order:
   - **Native renderer group 9, Windows part** ([tasks](specs/rdp-native-renderer/tasks.md), 9.1 and 9.2):
     - The native pointer in WebView2: the remote pointer's size and hotspot on a HiDPI display, the I-beam's inverted pixels (checker pattern), and a hidden pointer.
     - No endless animation: CPU near zero with an idle session in Task Manager.
     - Measure the canvas path with Windows Performance Recorder (CPU and GPU profiles), using the scenarios of task 7.3: idle, a scrolling terminal, typing with echo, and a full-screen update. Write the steps as `docs/profiling-windows.md`.
     - Decide with requirement 5.3: a native layer only if the canvas costs at least twice the probe's native CPU or delays input noticeably. On Windows that would be the WebView2 composition controller with a DirectComposition swap chain below the web view, which wry does not expose: the largest of the three platforms.
   - **Keyboard on a Windows client** (spec `rdp-keyboard-layout`): the layout detected from `GetKeyboardLayout` (status bar item, and "Detected" in the host's Keyboard section), symbols, Super as Ctrl. Watch for Windows itself taking Win+key chords before 1-1 sees them: Win+V is clipboard history and Win+L locks the screen, so Super as Ctrl may need its own handling there.
   - **The 1 s resize delay** (`0670aa9`): revealing the status bar should no longer resize the remote. Window drags resize once, a second after release.
   - **Folder sharing** still works from the Windows build (unchanged since beta.1; the Linux work moved shared code around).
0b. **Keyboard layout (spec `rdp-keyboard-layout`, 21 November).** Done and verified by the maintainer against Windows; input methods (4.3) deferred. [Spec](specs/rdp-keyboard-layout/), [validation](specs/rdp-keyboard-layout/validation.md)
   - **Automatic (default):** at connect, 1-1 detects the local layout and reports its Windows layout ID, so Windows and xrdp start the session with the client's layout. Mac variants map to the base layout (German Macintosh → German, "closest match").
     - Linux Wayland: `keyboard/detect.rs` reads the compiled keymap from `wl_keyboard`; on Hyprland its symbols line looks like `pc_de(mac_nodeadkeys)_us_2_…`, and the active group comes from `wl_keyboard.modifiers`.
     - X11: `setxkbmap -query`. Windows: `GetKeyboardLayout`. macOS: Carbon TIS on the main thread.
     - Table: `keyboard/klid.rs`. Resolution and source: `keyboard/report.rs`.
   - **Typed characters** go as Unicode (`SessionCommand::SendText`); shortcuts, named keys, Space and the keypad stay scancodes (`src/lib/keyboard-input.ts`). **Super as Ctrl**: a Super chord becomes the Ctrl chord, and Super alone stays the Windows key.
   - **Per host:** Keyboard section in the host settings (Layout, Keys: Automatic or Physical keys, Super as Ctrl), and `--keyboard-layout`, `--keyboard-keys`, `--super-as-ctrl` on `1-1 import`. The status bar shows the reported layout (`DE`), with its source on hover.
   - **Deferred, 4.3:** input methods and dead-key layouts need a hidden text field holding keyboard focus during a session (WebKit composes only into editable elements), as xterm.js does. The note is in the tasks.
1. **Native renderer (spec `rdp-native-renderer`, 21 November).** Groups 1–8 are done; group 7 (verification) was skipped by the maintainer, whose sessions and `validation.md` stand in for it. [Spec](specs/rdp-native-renderer/), [probe](../probes/native-subsurface/results-2026-11-21.md)
   - **How it works:** a wgpu layer in a desynchronized `wl_subsurface` on GDK's Wayland connection, placed below GTK's surface (`renderer/native_layer.rs`). The page is transparent over the session (`html.native-layer`), and the layer clears to the page background. Each native session writes into its own `SharedFrame` with damage tracking, and the render thread (`renderer/gpu.rs`) uploads only damage and presents only on change. The page reports the visible session and its rectangles through `native_view`. Wayland fractional scaling is used where the compositor offers it.
   - **Choice:** Settings, Display, Renderer (Automatic, GPU layer, Web canvas), applied at the next start; `ONE_ONE_RENDERER` overrides it. Automatic chooses the layer only on Hyprland. GNOME, KDE and X11 are not supported and not tested (maintainer decision); X11 and XWayland always use the canvas.
   - **Fallback:** if the layer fails while running, all sessions switch to the canvas with one full frame, without reconnecting, and the HUD shows the reason. `ONE_ONE_RENDERER_FAIL_AFTER=N` injects a failure after N frames.
   - **Guards:** a frontend test fails on looping animations in the session and dashboard views; a Rust test fails if the pointer is composited into frames again.
   - **H.264 groundwork:** `renderer/yuv.rs` converts YUV 4:2:0 (I420, NV12) on the GPU, exact against `ironrdp-egfx` for MS-RDPEGFX's full-range BT.709. Nothing produces YUV yet.
   - **Open:** group 9: on macOS and Windows, verify the native pointer and the removed animations, then measure the canvas with Instruments and Windows Performance Recorder and decide per platform.
   - **Removed:** the old opt-in renderer (`ONE_ONE_NATIVE_RENDERER`) and the B3-10 defect in `SharedFrame::publish`.
2. **Linux CPU findings (20 November, `45cce78`).** See [Profiling on Linux](../docs/profiling-linux.md).
   - Idle main thread was about 80%: endless animations (the connected-card pulse, the connecting and reconnecting spinners, a full-window backdrop blur). Removed; idle is now 0%. Keep looping animations out of the session and dashboard views.
   - The pointer lagged because IronRDP composited it into every frame. Done: the native pointer. `pointer_software_rendering` is false, `rdp/remote_pointer.rs` publishes the shape as a PNG data URL in `SessionInfo.pointer`, and `src/lib/remote-cursor.ts` scales it to the canvas as the CSS cursor (128 px cap; the Windows I-beam's inverted pixels show as a checker pattern). The maintainer confirmed that the mouse feels better.
   - WebKitGTK's web process segfaulted at every quit (`libgbm`/`dri_gbm.so` freeing in its exit handlers, since at least 18 November). Fixed on 21 November (`5b3fafc`, `webkit_exit.rs`): 1-1 ends its own web processes when the window is about to close or the app is asked to exit. The network process, which holds browser storage, is left alone. Confirmed on the maintainer's machine.
3. **Folder sharing from Linux and macOS (`ec5fead`).** Spec requirement 5 and tasks 7.1–7.4 are done.
   - The folder is opened without following links, checked by device and inode, and held open. The upstream `NixRdpdrBackend` reaches it through `/proc/self/fd/N` (`/dev/fd/N` on macOS), so a rename or a replacement at the old path cannot redirect the share.
   - Read-only, hard-link and privilege rules run in `ShareBackend` before upstream. `DriveRouter` gives each folder its own backend. Network file systems are refused.
   - Open: 7.5 (try the picker in a desktop session, update `docs/user-guide/`), 7.6 (from the Linux client against Windows and xrdp: read-only refusal, hashes in both directions, several folders, typing during copies; record in `validation.md`), a macOS compile, and the README line "Shared folders (Windows build)", which stays until 7.6 passes. [Folder-sharing tasks](specs/rdp-folder-sharing/tasks.md)
4. **Esc (`d48888f`).** Esc now goes to the remote (vim, terminals). It still closes 1-1's own menus and dialogs. Disconnect is on the toolbar and the tab.
5. **Graphics for terminal work.** The spec `rdp-terminal-graphics` is approved and not started: measure the large EGFX cache (`ONE_ONE_EGFX_CACHE=large`) before adopting it, use QOI/QOIZ only where the host negotiates them, no new UI switches. RemoteFX, NSCodec, UDP, the glyph cache and H.264 are not adopted. [Spec](specs/rdp-terminal-graphics/)
6. **H.264, parked.** The local branch `spike/h264-hw-decode` (`b27d9d8`, not pushed) holds `probes/gst-h264-decode/`: GStreamer VA-API decoding at about 5 ms per 1080p frame on Intel, bit-exact once the SPS is rewritten (`vah264dec` holds a picture back when the DPB size is 1). `ZERO-COPY.md` there describes the route from VA-API DMA-BUFs through Vulkan into the native layer's YUV converter. For text, AVC420 is worse than ClearCodec: 4:2:0 halves color resolution (colored text and ClearType smear) and H.264 is lossy; if H.264 comes, AVC444 (two 4:2:0 pictures recombined on the GPU) is the mode for text. Open: a real-host probe, a codec spec. The default stays ClearCodec, with no switch yet.
7. **Usability backlog:**
   - **Command line:** `1-1 --host <saved name|address[:port]>` connects directly, asks for the password if none is saved, and opens only the session with the sidebar hidden.
   - ~~Native pointer~~, ~~HiDPI~~, ~~Clipboard line endings~~, ~~Update notice~~: done.
8. **Open acceptance checks from 17 November:**
   - Windows and FreeRDP shrink and restore (11.4).
   - Clipboard Resync in both directions after resize and reconnect.
   - Separate active and unattended bandwidth comparisons. [Bandwidth research](specs/windows-clearcodec-royalts-e2e/bandwidth-research.md)
9. **Note for the server side:** hypr_rdp ignores Suppress Output. 1-1 detects this after about 8 seconds and falls back to live updates. Windows and xfreerdp honor pause. hypr_rdp does not seem to implement drive redirection.

Earlier folder-sharing notes (19 November): tasks 1 to 5.3 were tested against Windows and xrdp. xrdp sends forward-slash paths. Windows starts the drive only when the audio channel (`rdpsnd`) is registered. Copies stalled the session until replies were sliced and the send buffer capped (per-host **File transfers** setting). Diagnostics: one `folder share, last` log line per second during transfers; `ONE_ONE_SHARE_TRACE=1` logs each request. Open from then: task 6 (acceptance record, typing-latency measurement).

## Spec status

- **Session actions:** tasks 1 and 2 are complete. Task 3 (Windows acceptance runs) is open. A click or scroll within 5 seconds after Pause does not resume, and pause refusal is detected after about 8 seconds. [Tasks](specs/rdp-session-actions/tasks.md)
- **Folder sharing:** tasks 1–5.3 and 7.1–7.4 are complete (Windows on 19 November, Linux and macOS on 20 November). Open: 6 (acceptance and typing latency), 7.5 and 7.6. Recorded decisions (18 November):
  - An already-open handle that a local program moves outside the folder stays usable until it is closed.
  - Multiply-linked files are read-only through the share.
  - NTFS refuses to move a folder that holds an open file.
- **rdp-terminal-graphics:** approved on 20 November, tasks 1–4 not started.
- **rdp-keyboard-layout:** approved and done on 21 November, verified against Windows; 4.3 (input methods) deferred.
- **rdp-native-renderer:** approved on 21 November. Groups 1–8 done (7 skipped by decision); group 9 (macOS and Windows) open. Requirement 1.5 was amended for the Settings choice, and task 7.4 replaced by "Hyprland only". [Validation](specs/rdp-native-renderer/validation.md)
- **windows-clearcodec-royalts-e2e:** 62 of 77 numbered subtasks are complete. Open work is the 11.x end-to-end harness, interoperability, DPI/text fidelity, performance and release evidence. [Task list](specs/windows-clearcodec-royalts-e2e/tasks.md)

## CI and local builds

- **CI times:** a full run takes about 9 minutes with the Rust build cache. Docs-only pushes are skipped. The macOS cold-launch spike runs only on a manual dispatch with `macos_cold_launch` checked. Intel macOS builds only on a manual dispatch.
- **Linux coverage:** Linux CI compiles and tests the non-Windows code paths. Keep test paths portable.
- **Security scanning:** CodeQL default setup covers Actions, JavaScript/TypeScript and Rust. The 65 initial alerts were dismissed on 19 November as test fixtures or false positives. Remaining Dependabot alerts: `minimatch` in the typescript-eslint chain (fix newer than the two-week age rule) and `glib` 0.18 from Tauri's GTK stack.
- **Contract checks:** `pwsh -NoProfile -File scripts/Invoke-CI.Tests.ps1` passes 249 of 249.
- **Local preview:** `pwsh -NoProfile -File scripts/Invoke-WindowsPreview.ps1` builds into `D:/BuildArtifacts/cargo-target/release/rdp-client.exe`. Copy it under `D:/BuildArtifacts/drift-rdp/releases/` before testing; later builds overwrite the target. Launch it with stderr redirected and `RUST_BACKTRACE=1` to capture a panic.
- **Logs:** the app log is `%LOCALAPPDATA%/com.drift.app/logs/1-1.log`. It keeps five 2 MB files and records panics with a backtrace.
- **Linux local builds:** the maintainer's shell exports `CC=icx` and `CXX=icpx` (Intel compilers), and `ring` then fails to link (`_intel_fast_memcpy`). Build with `CC=gcc CXX=g++ npx tauri build --bundles deb`, and install with `sudo apt install --reinstall ./src-tauri/target/release/bundle/deb/1-1_<version>_amd64.deb` after quitting 1-1 (the single-instance check hands a new launch to the running window). `cargo-nextest` is in `~/.local/bin` (prebuilt). The Linux log is `~/.local/share/com.drift.app/logs/1-1.log`.
- **Linux profiling:** `scripts/linux/profile.sh start --wait`, then `dump` and `report`; one-time `sudo scripts/linux/profile-setup.sh` (already done on the maintainer's machine). The rolling buffer holds well under a second of a busy thread, so planned scenarios use `record 15 <name>` (straight to disk). [Profiling on Linux](../docs/profiling-linux.md)
- **Testing a build next to the running app:** do not use `dbus-run-session` to get around the single-instance check: it crashes `xdg-desktop-portal-hyprland` when the private session ends (five crash reports on 21 November). Quit 1-1 instead.
- **Release profile:** Cargo's defaults on purpose. Thin LTO, with or without one codegen unit, did not make decoding faster (ClearCodec 1080p 9.0 ms either way); one codegen unit makes the binary 26% smaller at five times the rebuild time. Benchmark: `cargo test --release --lib bench_ -- --ignored --nocapture`; numbers in the profiling doc.
- **Storage:** run native PowerShell from the repository and reuse the configured Cargo target directory. [Storage script](../scripts/Invoke-BuildStorage.ps1)
