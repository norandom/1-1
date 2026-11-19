<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="80" />
</p>

<h1 align="center">1-1</h1>

<p align="center">
  <strong>1-1 is a free cross-platform RDP client for Windows, Linux, and macOS. For Windows RDP, IronRDP, XfreeRDP etc. On top of that it's uniquely designed for the AI era.</strong>
  <br />
  Pronounced "zero."
  <br />
  Built with Rust, Tauri, React, and IronRDP.
</p>

<p align="center">
  <a href="#install">Install</a> &middot;
  <a href="#features">Features</a> &middot;
  <a href="#why-this-fork">Why this fork</a> &middot;
  <a href="#in-progress">In progress</a> &middot;
  <a href="#build-from-source">Build</a> &middot;
  <a href="#architecture">Architecture</a>
</p>

<br />

<p align="center">
  <img src=".github/assets/drift-session.png" width="800" alt="1-1, connected to a Windows machine" />
</p>

<br />

> **Fork notice: no warranty, no support.** 1-1 is a personal fork of [Drift](https://github.com/CREVIOS/drift-rdp), kept for my own use and shared as is. All kudos for the foundation go to the Drift author. If something breaks, you keep both halves. If you need a supported client, use upstream Drift.

## Install

Download the newest installer from [Releases](https://github.com/norandom/1-1/releases), on the `preview-channel` prerelease:

- **Windows:** per-user NSIS installer, no admin rights needed. Builds for x64 and ARM64. The installer downloads the WebView2 Evergreen bootstrapper at install time, so the first install needs an internet connection. On an offline machine, install the [WebView2 Evergreen Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) first.
- **macOS:** `.dmg`, Apple Silicon.
- **Linux:** `.AppImage` or `.deb`.

After installation, 1-1 checks the preview update feed once per launch and from Settings. Installing an update is always your explicit choice, and it is blocked while a session is active, so it never interrupts remote work.

No release yet, or you want to build your own installer? See [Build from source](#build-from-source).

## Unsigned builds, warnings, donations

I do not hold a Windows or macOS developer certificate, so the installers are not Authenticode-signed and the macOS build is not notarized. Expect Windows SmartScreen ("Windows protected your PC", click "More info" then "Run anyway") and the macOS Gatekeeper warning (right-click the app and choose "Open"). That is normal for an unsigned build, not a sign of malware. Every installer's SHA-256 hash is listed in the release manifest, so you can verify what you downloaded before running it. The update-feed signatures described under [Build from source](#build-from-source) are separate and do not change any of this.

No donations, please. This is just a fork, shared as is with no warranty and no support. If you want to support RDP client development financially, support upstream [Drift](https://github.com/CREVIOS/drift-rdp) instead.

## Why this fork

I used RoyalTS for a long time and eventually had enough of it: the splash screen is slow, it crashes on Windows, bandwidth use never got optimized, and development moved too slowly for my taste. So I forked Drift and added what I was missing.

The split I work with: [warpeer](https://github.com/norandom/warpeer) is my SSH client for Linux boxes, 1-1 is my RDP client for Windows machines. Both run cross-platform; each just targets its own protocol and OS.

Most of my work in this fork went into ClearCodec. I made it the preferred codec over H.264 because H.264 is not sharp enough for text-heavy work, and text is exactly what I stare at when an agent codes inside a VM.

1-1 focuses on remote coding and agentic workflows: sharp code and terminal text, responsive input, a reliable clipboard, and long-running sessions that do not eat your bandwidth.

## Features

**Connecting**

- RDP to Windows machines on port 3389, with TLS plus NLA/CredSSP authentication via IronRDP
- Verified against three server classes: Windows RDP Server (Windows 11, NLA mandatory), FreeRDP/xfreerdp servers on Linux, and IronRDP-based Rust servers on Linux
- Keyboard, mouse, and trackpad scroll forwarding; configurable resolution and quality per host
- Up to 10 simultaneous sessions in tabs, with auto-reconnect and exponential backoff on connection loss

**Bandwidth**

- Compression and codec choices tuned for slow links, with receive/send rates and totals in the performance HUD
- Client-issued bandwidth-saver pause: stop display updates from the session, and hosts that ignore the pause are reported within seconds. Typing or moving the mouse keeps the pause for five more seconds, so waking the display does not fight your input
- Per-host idle preview (Windows preview build): after five idle minutes a busy unattended desktop refreshes about once a minute while a quiet editor stays live; returning to the tab or sending input restores live updates
- ClearCodec is the preferred codec, and H.264/AVC is never advertised or decoded. H.264 smears small text, which is fine for video and bad for code; ClearCodec keeps terminal text crisp while still compressing well on slow links
- Per-host bulk compression (off, RDP4, RDP5, RDP6, RDP6.1) on top of the graphics codec, and per-host color depth (15/16/24/32 bpp) to trade fidelity for bytes; both apply on reconnect

**Interface**

- Quick Connect with `Cmd+K`; connection cards with color accents, tags, and latency indicators
- Status bar along the bottom with connection state and metrics; hide or pin it per host, right from the bar itself
- Bidirectional clipboard sync, with a status indicator
- Dark and light themes, keyboard shortcut overlay (`Cmd+?`), SSH config import (`~/.ssh/config`)

**Rendering and security**

- GPU-rendered frames via wgpu (Metal on macOS, Vulkan on Linux), zero-copy SharedFrame between session and renderer, software Canvas fallback, 0% CPU when idle
- Passwords stored in the OS keychain (macOS Keychain, Linux Secret Service); no plaintext credential storage
- Connection configs as local JSON: no cloud, no telemetry
- Ships as a single `.app`, `.AppImage`, or small NSIS installer

## In progress

- RDP drive redirection (share one local folder to a saved host) with segmented copies: bounded parallel range requests, verified byte for byte
- An rsync-style delta protocol on top of redirects, so only changed blocks move. Deferred until measurements show it pays off
- Auto-update workflow hardening: the signed preview feed and per-platform CI installers are in place; runtime validation currently covers Windows x64 only

## Build from source

Prerequisites:

- [Rust](https://rustup.rs/) (1.77+)
- [Node.js](https://nodejs.org/) (20+)
- [Tauri CLI v2](https://v2.tauri.app/start/prerequisites/)
- macOS: `xcode-select --install`
- Linux: `sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`

```bash
git clone https://github.com/norandom/1-1.git
cd 1-1
npm install
cargo tauri dev        # development, hot reload
cargo tauri build      # production bundle
npx tsc --noEmit       # type check
cargo check
```

### Building the Windows installer locally

```powershell
.\scripts\Invoke-WindowsPreview.ps1 -Installer                                   # x64 per-user NSIS installer
.\scripts\Invoke-WindowsPreview.ps1 -Installer -Target aarch64-pc-windows-msvc  # ARM64, after adding the Rust target and ARM64 MSVC tools
```

The helper keeps frontend assets under `D:\BuildArtifacts` and embeds them through a relative path.

### Releases and the update feed

Updatable releases need `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Local builds can use the persistent private key and DPAPI-protected password under `%LOCALAPPDATA%\1-1-release-signing`. Back that key up, because losing it breaks updates for installed clients, and never commit or publish it. These update signatures are separate from Authenticode and macOS signing and do not guarantee download reputation.

Version tags trigger `.github/workflows/preview-build.yml`: Windows x64 and ARM64 NSIS, macOS Apple Silicon DMG, and Linux x64 AppImage plus `.deb` via Dagger. Stage each signed installer with its `.sig` and `build-info.json` in a folder named `windows-x86_64`, `windows-aarch64`, `darwin-aarch64`, or `linux-x86_64`, then create the feed:

```bash
node scripts/update-manifest.mjs VERSION ASSET_DIRECTORY OUTPUT_JSON
```

Upload the immutable installer assets to the matching version release first, then publish the verified manifest as `latest.json` on the `preview-channel` prerelease. Never replace a published installer under an existing version; check its uploaded digest before touching the feed. Attach the Linux `.deb` to the version release as a download only: the updater replaces the AppImage, and a package install updates through a newer `.deb`.

## Architecture

![1-1 architecture](docs/diagrams/architecture.svg)

### Frame pipeline

![Frame pipeline](docs/diagrams/frame-pipeline.svg)

Both diagrams are generated from editable Mermaid sources in [docs/diagrams/](docs/diagrams/); full details in [ARCHITECTURE.md](ARCHITECTURE.md).

### Tech stack

| Layer | Technology |
|-------|-----------|
| Protocol | [IronRDP](https://github.com/Devolutions/IronRDP) (Rust RDP implementation) |
| Runtime | [Tauri v2](https://v2.tauri.app/) (Rust + WebView) |
| GPU | [wgpu](https://wgpu.rs/) (Metal / Vulkan / DX12) |
| Frontend | React 19 + TypeScript + [shadcn/ui](https://ui.shadcn.com/) |
| Styling | Tailwind CSS v4 |
| TLS | rustls + webpki |
| Auth | CredSSP / NLA via IronRDP |
| Credentials | [keyring](https://crates.io/crates/keyring) (OS keychain) |
| Updates | tauri-plugin-updater, signed preview feed (`latest.json`) |

## Project structure

```
1-1/
├── src/                      # React frontend
│   ├── components/
│   │   ├── connections/      # Connection cards, forms, tree
│   │   ├── session/          # Canvas, toolbar, tabs, HUD
│   │   ├── settings/         # Settings page
│   │   ├── layout/           # Sidebar, titlebar, status bar, pause control
│   │   └── ui/               # shadcn primitives
│   ├── hooks/                # useRdpSession, useClipboard, useTheme, etc.
│   ├── stores/               # Zustand stores
│   ├── lib/                  # Input mapper, frame protocol
│   └── styles/               # Tailwind + theme
├── src-tauri/                # Rust backend
│   └── src/
│       ├── rdp/              # IronRDP session actor, client, input, clipboard,
│       │                     # pause/idle preview, network metrics
│       ├── renderer/         # wgpu GPU renderer, SharedFrame
│       ├── commands/         # Tauri IPC commands (incl. updater)
│       ├── store/            # Persistence (connections, settings, keychain)
│       └── utils/            # TCP probe, latency measurement
├── scripts/                  # Build, installer, CI, and update-feed helpers
└── .github/workflows/        # CI + preview installer builds
```

## Local Windows build storage

Storage reporting is optional. Run it from the repository when useful:

```powershell
.\scripts\Invoke-BuildStorage.ps1
```

It honors `CARGO_TARGET_DIR` and reports logical file sizes. There are no build-blocking storage budgets or mandatory checkpoints. Clean up occasionally when needed; physical usage may differ from the report.

When builds are idle, preview or run package-only cleanup:

```powershell
.\scripts\Invoke-BuildStorage.ps1 -Action CleanProject -WhatIf
.\scripts\Invoke-BuildStorage.ps1 -Action CleanProject
.\scripts\Invoke-BuildStorage.ps1
```

The script uses `cargo clean --package rdp-client` and refuses unsafe target paths, active build tools, and target executables. It does not clean other projects, registry downloads, evidence logs, or user data, and no background cleanup is installed. Avoid a blanket `cargo clean` against the shared target.

App dev and test profiles disable debug symbols and incremental compilation by default. Use a deliberate temporary profile override for debugger investigations, then remove that build variant. Reuse the existing target, toolchain, and feature combinations, and keep only the current and previous runnable test binary outside Cargo's cache.

## License

MIT, inherited from Drift. The fork itself carries no warranty and no support; see the fork notice at the top.

---

<p align="center">
  <sub>Forked from Drift with gratitude. Use at your own risk.</sub>
</p>
