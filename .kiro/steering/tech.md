# Technology Stack

## Architecture

Tauri v2 desktop app: React 19 + TypeScript webview UI overlay, Rust backend. RDP sessions run as one actor task per connection (`SessionActor`), managed by a `SessionManager` registry. Frames flow IronRDP → `SharedFrame` (double-buffered, condvar-signaled) → GPU render thread, plus IPC frame packets to the webview canvas (visible path). Input flows webview → Tauri IPC → FastPath scancodes.

## Core Technologies

- **Frontend**: TypeScript (strict), React 19, Zustand stores, Tailwind CSS v4, shadcn/ui + Radix, framer-motion, react-router-dom v7
- **Backend**: Rust 2024, MSRV 1.94 (current Cargo.toml), Tokio async runtime
- **Shell**: Tauri v2 (macOS WebKit, Linux WebKitGTK, Windows WebView2)
- **Protocol**: pinned IronRDP component crates aligned with ironrdp-egfx 0.5.0; the facade migration is complete. Cargo.toml/Cargo.lock are authoritative for component versions. The shared DRDYNVC registers graphics and Display Control. Use upstream APIs rather than custom protocol implementations.
- **GPU**: wgpu 24 (Metal / Vulkan / DX12)
- **Credentials**: `keyring` crate → macOS Keychain, Linux Secret Service, Windows Credential Manager

## Key Libraries

- `vitest` — frontend unit tests
- `cargo nextest run` plus `cargo test --doc` — target Rust test tiers; migration tasks preserve current coverage (tokio::test where async)
- Zustand for state; no global mutable singletons in components
- EARS-format acceptance criteria in specs; TDD for delta work

## Development Standards

### Type Safety
TypeScript strict mode; no `any`; shared types mirrored manually between `src/types.ts` and `src-tauri/src/store/*.rs` (camelCase serde on Rust side).

### Code Quality
ESLint 9 + typescript-eslint; `cargo fmt --check` and `cargo clippy -- -D warnings` enforced in CI.

### Testing
Vitest for frontend; `cargo nextest run` plus `cargo test --doc` for backend; Tri-OS built-app E2E is an active-spec target, not an established baseline. Delta work follows red-green-refactor TDD.

## Development Environment

### Required Tools
Rust 1.94+ via rustup; Node/package-manager versions follow repository preflight and manifests. Tauri v2 prerequisites per OS: Windows WebView2 + MSVC; Linux webkit2gtk-4.1 development packages; macOS Xcode CLT.

### Common Commands
```powershell
# pwsh-native (avoid WSL/Git Bash wrappers on Windows per AGENTS.md)
# Dev: npm install; cargo tauri dev
# Build: cargo tauri build
# Frontend tests: npx vitest run
# Rust tests (from src-tauri): cargo nextest run; cargo test --doc
# Typecheck: npx tsc --noEmit; cd src-tauri; cargo check
```

## Key Technical Decisions

- **IronRDP as the only RDP stack** — no custom protocol code; non-AVC codecs with standard bitmap fallback. H.264/AVC transport was removed. CodecPolicy owns advertisement and unsupported-codec behavior; experience preferences must not alter it.
- **Actor-per-session** — no shared mutex contention; `SessionHandle` + mpsc commands; drop handle = kill task.
- **Dual frame paths** — GPU surface renders behind the webview (dormant display path), IPC packets render on webview canvas (visible path); keep both fed until GPU path is promoted.
- **Local-first persistence** — pretty-printed JSON (`connections.json`, `settings.json`); secrets only in OS keychain.

---

_Document standards and patterns, not every dependency_


## Native execution and target CI
Follow AGENTS.md Windows execution policy: native PowerShell and clients, no direct WSL/Git Bash commands. WSL-backed Podman is permitted behind native Dagger/Podman clients. Dagger owns portable/Linux pipeline logic; native Windows/macOS jobs remain explicit exceptions. CI definitions exist, but complete release E2E acceptance is still pending.
Rust tests use pinned cargo-nextest plus cargo test --doc; nextest is tooling, not a runtime crate. Prefer existing dependencies and short compatibility spikes over new infrastructure.

## Build storage
User removed the build guard: no mandatory checkpoints or size limits. Occasional explicit package-only cleanup while idle is sufficient. Optional report/cleanup: scripts/Invoke-BuildStorage.ps1. Honor configured output directories and preserve active output/user data/evidence.

