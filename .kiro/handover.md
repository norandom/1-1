# 1-1 maintainer handover

**18 November 2026, end of day.** 1-1 (pronounced “zero”) is an RDP client for coding and monitoring remote agents. The latest release is **v0.1.0-alpha.5**. The maintainer tested it on Windows and on Linux (Hyprland, Wayland), and found no regressions. Folder sharing comes next, through attaching the drive to sessions (task 5.2), before the usability items below. [Releases](https://github.com/norandom/1-1/releases)

## Releases

- **alpha.5** (`c84ab4b`, current feed). Fixes the Wayland exit with protocol error 71 ([#1](https://github.com/norandom/1-1/issues/1), closed). The native wgpu renderer drew under the web view, which already draws every frame. It now starts only with `ONE_ONE_NATIVE_RENDERER=1`. This release also adds the folder sharing preview (the choice is saved, but no session offers the folder yet) and a Linux `.deb`.
- **alpha.4** (`832ca8c`). Pause display, credential test, server identity review, sidebar Disconnect, resizable and hideable sidebar, per-host bottom bar, and fixes for the window-snapping drag and a resize crash.
- **Published assets:** each release has the Windows x64 installer plus the Linux `.deb` and AppImage, with signatures and `SHA256SUMS.txt`. `latest.json` on `preview-channel` lists Windows x64 and the AppImage. A `.deb` cannot replace itself, so `.deb` users update by installing the next release's `.deb`. Windows ARM64 and macOS are built in CI but not published.
- **Releasing:**
  1. Bump the five version files and push.
  2. When CI is green, push a `v*` tag. The tag triggers `preview-build.yml`, which exports the signed AppImage and the `.deb` together.
  3. Verify each asset's SHA-256 against its `build-info.json`, and check that the `.sig` key ID matches the updater key in `tauri.conf.json`.
  4. Create the release, then run `scripts/update-manifest.mjs` and upload `latest.json`.

  GitHub's macOS arm64 runners often fail to start a job. Rerun that job, or release on green code checks when macOS is not shipped.

## Where to resume

1. **Fix the task 4.1 review findings**, then mark 4.1 done. Code is in `src-tauri/src/rdp/folder_share.rs`; `BoundRoot` and `LinkSafeBackend` are not wired into sessions yet.
   - *Blocker:* a create of `name:stream` on a multiply-linked file passes the link check, because the stream doesn't exist yet. New streams attach to the file itself. Strip `:stream` before counting links.
   - Count links fail-closed: only "not found" forwards the request.
   - Refuse a `FileRenameInformation` that replaces a multiply-linked destination, and refuse remote hard-link creation (`FileLinkInformation`).
   - After binding, re-check every guard's attributes and compare the root guard's final path with the expected path. A junction swapped in between two guard opens can otherwise pass.
   - Note in the design that a same-user `DefineDosDevice` remap of the drive letter is out of scope.
   - In the refusal test, decode the reply's status, completion ID and file ID. Add tests for streams, rename-overwrite and the binding race.
2. **Continue folder sharing:** 4.2 read-only and permission enforcement, 4.3 scoped create, copy, rename and delete, 5.1 a bounded file worker off the input path, 5.2 attaching the drive. The maintainer's main past pain was RDP file transfers making the mouse and typing slow. Keep input traffic ahead of file replies and measure typing latency during copies (6.2). [Folder-sharing tasks](specs/rdp-folder-sharing/tasks.md)
3. **Usability backlog, after 5.2:**
   - **Native pointer, as Royal TS and mstsc do it:** set `pointer_software_rendering` to false, show the remote pointer bitmap as the CSS cursor over the canvas, and hide and restore it cleanly at the window edge. The local pointer currently stays visible next to the drawn one.
   - **HiDPI:** `rdp/client.rs` sends `desktop_scale_factor: 0` while requesting physical pixels, so the remote UI is tiny on a MacBook. Add a per-host Scaling option: Automatic (device pixel ratio × 100), 100%, or Low resolution.
   - **Clipboard line endings:** convert CRLF to LF for the Linux and macOS clipboards, and back to CRLF when sending to the remote. Wayland currently gets an extra `\r`.
   - **Command line:** `1-1 --host <saved name|address[:port]>` connects directly, asks for the password if none is saved, and opens only the session with the sidebar hidden.
   - **CPU:** about 2% per connection on an i9 at high resolution with RDP 6.1 compression. Measure after the pointer change.
4. **Open acceptance checks from 17 November:**
   - Windows and FreeRDP shrink and restore (11.4).
   - Clipboard Resync in both directions after resize and reconnect.
   - Separate active and unattended bandwidth comparisons. [Bandwidth research](specs/windows-clearcodec-royalts-e2e/bandwidth-research.md)
5. **Note for the server side:** hypr_rdp ignores Suppress Output. 1-1 detects this after about 8 seconds and falls back to live updates. Windows and xfreerdp honor pause.

## Spec status

- **Session actions:** tasks 1 and 2 are complete. Task 3 (Windows acceptance runs) is open. A click or scroll within 5 seconds after Pause does not resume, and pause refusal is detected after about 8 seconds. [Tasks](specs/rdp-session-actions/tasks.md)
- **Folder sharing:** tasks 1–3 are complete: server identity, Windows fixture, grant store, and folder picker with grant controls. Task 4.1 is implemented, with the review findings above open. Recorded decisions (18 November):
  - An already-open handle that a local program moves outside the folder stays usable until it is closed.
  - Multiply-linked files are read-only through the share.
  - NTFS refuses to move a folder that holds an open file.
- **windows-clearcodec-royalts-e2e:** 62 of 77 numbered subtasks are complete. Open work is the 11.x end-to-end harness, interoperability, DPI/text fidelity, performance and release evidence. [Task list](specs/windows-clearcodec-royalts-e2e/tasks.md)

## CI and local builds

- **CI times:** a full run takes about 9 minutes with the Rust build cache. Docs-only pushes are skipped. The macOS cold-launch spike runs only on a manual dispatch with `macos_cold_launch` checked. Intel macOS builds only on a manual dispatch.
- **Linux coverage:** Linux CI compiles and tests the non-Windows code paths. Keep test paths portable.
- **Contract checks:** `pwsh -NoProfile -File scripts/Invoke-CI.Tests.ps1` passes 249 of 249.
- **Local preview:** `pwsh -NoProfile -File scripts/Invoke-WindowsPreview.ps1` builds into `D:/BuildArtifacts/cargo-target/release/rdp-client.exe`. Copy it under `D:/BuildArtifacts/drift-rdp/releases/` before testing; later builds overwrite the target. Launch it with stderr redirected and `RUST_BACKTRACE=1` to capture a panic.
- **Logs:** the app log is `%LOCALAPPDATA%/com.drift.app/logs/1-1.log`. It keeps five 2 MB files and records panics with a backtrace.
- **Storage:** run native PowerShell from the repository and reuse the configured Cargo target directory. [Storage script](../scripts/Invoke-BuildStorage.ps1)
