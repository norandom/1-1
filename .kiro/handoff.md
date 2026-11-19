# 1-1 follow-ups

**19 November 2026, after v0.1.0-beta.1.** This lists the work that is open, in the agreed order, for continuing on Linux. The release history and the release procedure are in [handover.md](handover.md).

## Where things stand

- **Released:** v0.1.0-beta.1 (tag on `2e90e87`). Windows x64, Linux `.deb` and AppImage, macOS Apple silicon. The update feed serves Windows x64, macOS and the AppImage.
- **Shared folders work only when 1-1 runs on Windows.** The remote side can be Windows or xrdp. 1-1 on Linux and macOS cannot share a local folder yet (follow-up 1).
- **`master` is green** except the `macos-bundle` CI job, which GitHub's runners often cancel before it starts. Rerun it, or rely on the tag build's macOS job.
- **Not in the repository:** `AGENTS.md`, `.kiro/steering/`, `.opencode/` and a few spec drafts exist only on the Windows workstation. They hold the agent workflow rules and the Windows execution policy. Copy them over or commit them if you want the same agent setup on Linux.

## Follow-ups, in order

### 1. Shared folders on Linux and macOS

The maintainer asked about this right after beta.1; decide whether it goes before the Nix flake.

What is shared between platforms already: grant store (`store/share_grants.rs`), commands and dialog, server identity check, worker (`rdp/folder_share/worker.rs`), drive channel wrapper (`channel.rs`), status (`status.rs`), sliced writes and the File transfers setting, bottom-bar status.

What is Windows-only and needs a Unix counterpart:

| Piece | Windows code | Unix work |
|---|---|---|
| Folder picker and identity | `commands/folder_share.rs`, `mod native` under `cfg(windows)`: IFileOpenDialog, volume and file ID, NTFS/ReFS only | A picker (XDG desktop portal on Linux, NSOpenPanel on macOS); identity from device and inode; decide which filesystems qualify |
| Binding the folder | `rdp/folder_share.rs` `BoundRoot`: guard handles on the root and every ancestor that deny rename and delete | Unix has no share modes. Hold directory file descriptors and resolve everything relative to the root descriptor (`openat` with `O_NOFOLLOW`, or `openat2` with `RESOLVE_BENEATH` on Linux), then re-check device and inode |
| Share policy | `ShareBackend` in `mod windows_policy`: no privilege-enabling requests, multiply linked files read-only, share root cannot be removed, `/` normalized | Port the rules: `st_nlink > 1` is read-only for changes, refuse removing or renaming the root, refuse symlink traversal. Privileges do not apply |
| Drive backend | `ironrdp-rdpdr-native` Windows backend through `WindowsRdpdrBackendFactory` | The same crate has a Unix backend (`src/nix`, `NixRdpdrBackend`). Read how it confines paths before trusting it |
| Attach | `rdp/folder_share/attach.rs` under `cfg(windows)` | A Unix `start()` that builds the Unix backend behind the same `WorkerBackend` and `DriveChannel` |

Other notes:

- `worker` is compiled under `cfg(any(windows, test))` and `session::share_ready` returns `None` elsewhere. Widen both when Unix attaches.
- Mirror the Windows tests: `rdp/folder_share/tests.rs` and `tests/policy/operations.rs` drive the real backend through the policy (read-only mode, in-folder operations, a junction swapped in, hard links, forward-slash paths).
- The spec is `specs/rdp-folder-sharing/`; `tasks.md` has dated notes for every task and the real-host findings.

### 2. Nix flake

Agreed as the next packaging step; the maintainer will maintain a later nixpkgs package.

- Add `flake.nix` with a package (`nix run github:norandom/1-1`) and a dev shell. A Tauri 2 app needs the Rust crates and the npm packages pinned by hash (`cargoHash`, `npmDepsHash`), WebKitGTK 4.1 and the usual GTK inputs.
- Add a CI job on a Linux runner that builds the flake. The hashes go stale whenever `Cargo.lock` or `package-lock.json` changes, and that job is what catches it.
- Later: a pull request to `NixOS/nixpkgs` built from the flake's recipe. Declare the licence correctly (1-1 is a fork of Drift).

### 3. APT repository on GitHub Pages

- The release workflow builds the `.deb` already. Add a job that builds a signed repository index and publishes it to GitHub Pages, so `apt install` and `apt upgrade` work. This also closes the gap that a `.deb` install cannot update itself.
- Needs two things from the maintainer: GitHub Pages enabled for the repository, and a GPG signing key whose private half is stored in the Actions secrets.

### 4. Smoother sessions during copies

The maintainer accepted the current behavior as good enough, not perfect.

- Measured on Windows: the server reads in 1 MiB pieces and the uplink (about 1.5 MB/s there) stays saturated during a copy. Replies are written in slices (16 or 64 KiB, at TPKT boundaries) and the socket send buffer is capped (32 or 128 KiB) per the host's File transfers setting.
- The session loop still waits on the network while a slice is written, and input shares the socket with file data. The next step is a separate writer with two queues that always sends input and acknowledgements before file data, so the loop never blocks on a file write.
- Diagnostics exist: one `folder share, last …` log line per second during transfers, and `ONE_ONE_SHARE_TRACE=1` logs every request and reply.

### 5. Folder sharing, task 6

Record the acceptance results in the spec (`validation.md`): hosts, copies both ways with hashes, and typing latency during a copy (6.2). The real-host runs of 19 November are noted in `tasks.md` under 5.2.

### 6. Usability backlog

- Native pointer as mstsc and Royal TS do it: use the remote pointer shape as the local cursor and hide it cleanly at the window edge.
- `1-1 --host <name|address>` to connect directly; `1-1 import` exists and shows how arguments reach the window (`commands/launch.rs`, single-instance plugin).
- CPU use per connection (about 2% on an i9); measure after the pointer change.

## To verify on real machines

- **Linux password persistence:** save a password, log out and in, connect. Needs KWallet or GNOME Keyring running.
- **macOS `.dmg`:** never opened on a real Mac. It is not notarized; if macOS calls it damaged even after removing the quarantine flag, the bundle needs ad-hoc signing.
- **HiDPI scaling** on the MacBook (Automatic should give 200%).
- **hypr_rdp:** it sent nothing on the drive channel, so it probably lacks drive redirection (server side, the maintainer's own project).

## Things that cost time and are easy to forget

- **Windows starts a shared drive only if the client registers the audio channel (`rdpsnd`).** 1-1 attaches it with a no-op handler whenever a drive is offered.
- **xrdp sends some paths with forward slashes.** The share turns `/` into `\` before the backend sees them.
- **A hung remote Explorer can leave the remote drive redirector stuck.** Signing out or restarting the remote machine clears it; it looked like a client bug for an hour.
- **Dependencies:** new or updated crates and npm packages must be at least two weeks old. Check the release date before pinning.
- **Releases:** follow the steps in [handover.md](handover.md). Download release artifacts into a folder other than `releases/v<version>` on the Windows workstation, because the local preview script stages its frontend there.
- **CI:** pushes that touch only Markdown, `docs/` or `.kiro/` skip the CI workflow. CodeQL's default setup runs on every push to `master`, including these, and `[skip ci]` does not stop it; cancel the run, or move CodeQL to an advanced setup with `paths-ignore`.
