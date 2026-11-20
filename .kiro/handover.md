# 1-1 maintainer handover

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

1. **Native renderer on Linux (next, agreed 20 November).** Every remote screen update still costs one full-window GTK 3 software repaint (`cairo_fill` → `pixman_fill` on the main thread; about 34 MB per frame at Hyprland scale 1.6, since GTK 3 draws at scale 2). Plan:
   - A half-day spike probe: a wgpu `wl_subsurface` placed below a transparent Tauri web view, a 60 Hz test pattern under a transparent HTML hole, main-thread CPU measured with the profiler. It must show that GTK 3 and WebKitGTK cooperate with our own subsurface. The existing `renderer/gpu.rs` (`ONE_ONE_NATIVE_RENDERER=1`) only draws a full-window quad onto GTK's own surface, which caused [#1](https://github.com/norandom/1-1/issues/1) (protocol error 71).
   - If the spike works, a spec `rdp-native-renderer-linux`: positioning with the canvas (fit, native, scrolling, scaling), one frame per tab, web view overlays above the GPU layer, and fallback to the canvas path.
   - A one-minute check to run first: `GDK_BACKEND=x11 rdp-client` (XWayland), to see whether scrolling feels lighter.
2. **Linux CPU findings (20 November, `45cce78`).** See [Profiling on Linux](../docs/profiling-linux.md).
   - Idle main thread was about 80%: endless animations (the connected-card pulse, the connecting and reconnecting spinners, a full-window backdrop blur). Removed; idle is now 0%. Keep looping animations out of the session and dashboard views.
   - The pointer lagged because IronRDP composited it into every frame. Done: the native pointer. `pointer_software_rendering` is false, `rdp/remote_pointer.rs` publishes the shape as a PNG data URL in `SessionInfo.pointer`, and `src/lib/remote-cursor.ts` scales it to the canvas as the CSS cursor (128 px cap; the Windows I-beam's inverted pixels show as a checker pattern). The maintainer confirmed that the mouse feels better.
   - WebKitGTK's web process segfaults at exit (`libgbm`/`dri_gbm.so` freeing in exit handlers). It is in `coredumpctl` since 18 November too, so it predates these changes, and it does not affect sessions. Possible workaround: `WEBKIT_DISABLE_DMABUF_RENDERER=1`, not measured.
3. **Folder sharing from Linux and macOS (`ec5fead`).** Spec requirement 5 and tasks 7.1–7.4 are done.
   - The folder is opened without following links, checked by device and inode, and held open. The upstream `NixRdpdrBackend` reaches it through `/proc/self/fd/N` (`/dev/fd/N` on macOS), so a rename or a replacement at the old path cannot redirect the share.
   - Read-only, hard-link and privilege rules run in `ShareBackend` before upstream. `DriveRouter` gives each folder its own backend. Network file systems are refused.
   - Open: 7.5 (try the picker in a desktop session, update `docs/user-guide/`), 7.6 (from the Linux client against Windows and xrdp: read-only refusal, hashes in both directions, several folders, typing during copies; record in `validation.md`), a macOS compile, and the README line "Shared folders (Windows build)", which stays until 7.6 passes. [Folder-sharing tasks](specs/rdp-folder-sharing/tasks.md)
4. **Esc (`d48888f`).** Esc now goes to the remote (vim, terminals). It still closes 1-1's own menus and dialogs. Disconnect is on the toolbar and the tab.
5. **Graphics for terminal work.** The spec `rdp-terminal-graphics` is approved and not started: measure the large EGFX cache (`ONE_ONE_EGFX_CACHE=large`) before adopting it, use QOI/QOIZ only where the host negotiates them, no new UI switches. RemoteFX, NSCodec, UDP, the glyph cache and H.264 are not adopted. [Spec](specs/rdp-terminal-graphics/)
6. **H.264, parked.** The local branch `spike/h264-hw-decode` (`5097efa`, not pushed) holds `probes/gst-h264-decode/`: GStreamer VA-API decoding at about 5 ms per 1080p frame on Intel, bit-exact once the SPS is rewritten (`vah264dec` holds a picture back when the DPB size is 1). Open: a real-host probe, a codec spec, and per-region AVC420 conversion upstream. The default stays ClearCodec, with no switch yet.
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
- **windows-clearcodec-royalts-e2e:** 62 of 77 numbered subtasks are complete. Open work is the 11.x end-to-end harness, interoperability, DPI/text fidelity, performance and release evidence. [Task list](specs/windows-clearcodec-royalts-e2e/tasks.md)

## CI and local builds

- **CI times:** a full run takes about 9 minutes with the Rust build cache. Docs-only pushes are skipped. The macOS cold-launch spike runs only on a manual dispatch with `macos_cold_launch` checked. Intel macOS builds only on a manual dispatch.
- **Linux coverage:** Linux CI compiles and tests the non-Windows code paths. Keep test paths portable.
- **Security scanning:** CodeQL default setup covers Actions, JavaScript/TypeScript and Rust. The 65 initial alerts were dismissed on 19 November as test fixtures or false positives. Remaining Dependabot alerts: `minimatch` in the typescript-eslint chain (fix newer than the two-week age rule) and `glib` 0.18 from Tauri's GTK stack.
- **Contract checks:** `pwsh -NoProfile -File scripts/Invoke-CI.Tests.ps1` passes 249 of 249.
- **Local preview:** `pwsh -NoProfile -File scripts/Invoke-WindowsPreview.ps1` builds into `D:/BuildArtifacts/cargo-target/release/rdp-client.exe`. Copy it under `D:/BuildArtifacts/drift-rdp/releases/` before testing; later builds overwrite the target. Launch it with stderr redirected and `RUST_BACKTRACE=1` to capture a panic.
- **Logs:** the app log is `%LOCALAPPDATA%/com.drift.app/logs/1-1.log`. It keeps five 2 MB files and records panics with a backtrace.
- **Linux local builds:** the maintainer's shell exports `CC=icx` and `CXX=icpx` (Intel compilers), and `ring` then fails to link (`_intel_fast_memcpy`). Build with `CC=gcc CXX=g++ npx tauri build --bundles deb`, and install with `sudo apt install --reinstall ./src-tauri/target/release/bundle/deb/1-1_<version>_amd64.deb` after quitting 1-1 (the single-instance check hands a new launch to the running window). `cargo-nextest` is in `~/.local/bin` (prebuilt). The Linux log is `~/.local/share/com.drift.app/logs/1-1.log`.
- **Linux profiling:** `scripts/linux/profile.sh start --wait`, then `dump` and `report`; one-time `sudo scripts/linux/profile-setup.sh` (already done on the maintainer's machine). [Profiling on Linux](../docs/profiling-linux.md)
- **Storage:** run native PowerShell from the repository and reuse the configured Cargo target directory. [Storage script](../scripts/Invoke-BuildStorage.ps1)
