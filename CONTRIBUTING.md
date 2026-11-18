# Contributing

1-1 is a personal fork of [Drift](https://github.com/CREVIOS/drift-rdp). It carries no warranty and no support, and I am not taking pull requests here. If you want to contribute to the upstream project, fork [drift-rdp](https://github.com/CREVIOS/drift-rdp) and open your PR there.

If you hack on your own copy of this fork, the setup below still applies.

## Development setup

```bash
git clone https://github.com/norandom/1-1.git
cd 1-1
npm install
cargo tauri dev
```

### Prerequisites

- Rust 1.77+
- Node.js 20+
- Tauri CLI v2 (`cargo install tauri-cli`)
- macOS: `xcode-select --install`
- Linux: `sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`

### Windows installer and the WebView2 bootstrapper

The NSIS installer pins `bundle.windows.webviewInstallMode.type` to `downloadBootstrapper` in `src-tauri/tauri.conf.json`: the installer downloads Microsoft's WebView2 Evergreen bootstrapper at install time, which keeps the shipped artifact small. On a machine without internet the bootstrapper cannot download the runtime, so pre-install the [WebView2 Evergreen Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) before running the installer, or build a fully self-contained artifact locally by switching the mode to `offlineInstaller`. Never commit that switch; the pinned default is a CI contract, and `scripts/Invoke-CI.Tests.ps1` (CT17) plus the `windows-native` workflow job fail if the configured mode regresses.

## Local workflow

1. Branch descriptively: `feat/clipboard-sync`, `fix/scroll-direction`, `docs/architecture`
2. Keep changes focused and small
3. Test locally:

   ```bash
   cargo check            # Rust compilation
   cargo test             # Rust tests
   npx tsc --noEmit       # TypeScript types
   npx vitest run         # Frontend tests
   cargo fmt --check      # Rust formatting
   cargo clippy           # Rust lints
   ```

4. Build the Windows preview installer: `.\scripts\Invoke-WindowsPreview.ps1 -Installer`

## Code style

### Rust

- `cargo fmt` for formatting
- `cargo clippy -- -D warnings` for lints
- Prefer `Result<T, E>` over `unwrap()`/`expect()` in production paths
- Use `log::info!`/`log::warn!`/`log::error!` for observability
- Document public APIs with `///` doc comments

### TypeScript

- Strict mode (`noEmit` check must pass)
- Use shadcn/ui components from `@/components/ui/`
- Use `cn()` from `@/lib/utils` for conditional classes
- Hooks go in `src/hooks/`, stores in `src/stores/`

### Commits

- Use [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `perf:`, `docs:`, `refactor:`, `test:`
- Keep messages concise

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for system design and data flow.

Key principles:

- Actor per session: each RDP connection runs in its own tokio task
- SharedFrame: zero-copy double buffer between session actor and GPU renderer
- Fallback paths: GPU rendering falls back to webview Canvas if wgpu is unavailable
- No panics in production: use `Result<>` for anything that can fail
