# Drift CI command contract (task 1.4)

One command contract for **container** (Linux, via Dagger) and **native**
(Windows/macOS) CI jobs — spec `windows-clearcodec-royalts-e2e`, requirements
7.1, 7.2, 8.4, 12.5, 13.3, 13.4; design.md "CI ownership and reuse decisions"
and File Structure Plan (`.dagger/`).

**The rule:** GitHub Actions remains the trigger/artifact layer. Linux jobs
call this Dagger module; native Windows (MSVC/WebView2) and macOS bundle/UI
jobs run the **same tier commands** through platform-appropriate native tools
(pwsh on Windows). Dagger Linux containers are not Windows/macOS build
environments, and **results from Dagger Linux containers never substitute for native Windows or macOS validation**; container and native jobs are distinct
and labeled. Windows host commands use native pwsh — never WSL or Git Bash —
and the WSL-backed Podman engine is reached only through native clients
(Req 7.5).

Machine-readable twin: `dagger -m .dagger call contract` (source:
`src/pins.ts`). Entries that consume repository source receive it as a
`--source <repository root>` Directory argument resolved by the dagger CLI
(the dagger 0.21.11 TypeScript SDK has no host-directory access inside module
code), e.g. `dagger -m .dagger call frontend --source .` — run from the
repository root, or through `scripts/Invoke-CI.ps1 -Tier dagger` which passes
the canonical root itself. Known cost: the 0.21 CLI cannot exclude host paths
for Directory arguments, so the first sync includes host `node_modules`
(~254 MB today); engine snapshots are content-addressed and incremental, and
task 2.1 revisits source loading with a newer engine/SDK that allows host
excludes.

## Linux container checks (Dagger module, this directory)

Engine pin: `registry.dagger.io/engine:v0.21.11` (Dagger CLI `0.21.11`; task
1.3 spike PROVEN). On the Windows dev host, `scripts/Invoke-CI.ps1 -Tier
dagger` sets `_EXPERIMENTAL_DAGGER_RUNNER_HOST=image+podman://registry.dagger.io/engine:v0.21.11`
itself; GitHub-hosted Linux runners use their Docker engine instead.

| Entry point     | Pinned image                       | Commands (the tier's actual command list) |
|-----------------|------------------------------------|-------------------------------------------|
| `frontend`      | `node:22.23.4-bookworm` (+digest)  | `npm ci` → `npm run lint` → `npx tsc --noEmit` → `npx vitest run` → `npm run build` |
| `rust-checks`   | `rust:1.98.1-bookworm` (+digest)   | apt Tauri libs → `rustup component add rustfmt clippy` → install `cargo-nextest 0.9.150` (pinned tarball) → `cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo check` → `cargo-nextest nextest run --config-file .config/nextest.toml --profile ci --no-tests fail` → `cargo test --doc` |
| `linux-build`   | `rust:1.98.1-bookworm` + `node:22.23.4-bookworm` | `npm ci` → `npx tauri build --target x86_64-unknown-linux-gnu` (AppImage/deb export) |
| `linux-e2e`     | `rust:1.98.1-bookworm`             | `npm ci` → `npm run test:e2e` (WebKitGTK driver per design.md E2E stack) |
| `contract`      | —                                  | prints this contract as JSON |

Note on call names: the dagger v0.21.11 TypeScript SDK kebab-cases method
names and splits digit/letter boundaries, so the E2E entry registers as the
callable **`linux-e-2-e`**. The contract name stays `linux-e2e`;
`scripts/Invoke-CI.ps1 -Tier dagger -DaggerEntry linux-e2e` performs the
mapping (see `src/pins.ts`, `daggerCallNames`).

### Execution status and the 2.x split (tasks.md)

- `frontend` and `rust-checks` are **executable now** — each runs its real
  command list above in the pinned container and fails loudly on any step
  (Req 7.1, 13.3). Task 2.1 owns making them CI gates with JUnit emission,
  the intentionally-failing-fixture gate, and cache/artifact parity.
- `linux-build` **validates its contract** (inputs present, images pinned)
  and returns an explicit `contract-validated` / deferred status — task 2.2
  owns executing the Tauri bundle and exporting AppImage/deb artifacts. It
  must not gate CI until 2.2 lands.
- `linux-e2e` **fails loudly while the `e2e/` suite does not exist** (spec
  tasks 2.5+/12.x own the WebdriverIO suite and driver wiring) — it refuses
  to fake a pass.

## Native Windows validation (pwsh, scripts/Invoke-CI.ps1)

npm runs natively on Windows, so the frontend tier is in-contract natively;
Rust tiers use the native MSVC toolchain. Single command per tier (Req 13.4),
run from the canonical repository root `D:\Source\drift-rdp`:

| Tier       | Single command | Commands |
|------------|----------------|----------|
| preflight  | `pwsh -NoProfile -File scripts/Invoke-CI.ps1` | native tool/version + wrapper inspection (tasks 1.1/1.2) |
| rust       | `pwsh -NoProfile -File scripts/Invoke-CI.ps1 -Tier rust` | pinned `cargo nextest run` (ci profile, `.config/nextest.toml`) + `cargo test --doc` |
| frontend   | `pwsh -NoProfile -File scripts/Invoke-CI.ps1 -Tier frontend` | `npm ci` → `npm run lint` → `npx tsc --noEmit` → `npx vitest run` → `npm run build` |
| dagger     | `pwsh -NoProfile -File scripts/Invoke-CI.ps1 -Tier dagger -DaggerEntry <frontend\|rust-checks\|linux-build\|linux-e2e\|contract>` | wires the engine runner host from the pin, then invokes the Linux-container entry above |

Windows installer artifacts and native Windows/macOS E2E (Req 8.4, 12.5) are
produced by the native jobs themselves — never by these containers.

## Native macOS validation

The same tier commands run natively on macOS runners via
platform-appropriate tools (zsh/bash + native node/npm/cargo): `npm ci`, lint,
`tsc --noEmit`, `vitest run`, `npm run build`, `cargo fmt --check`,
`cargo clippy -- -D warnings`, `cargo check`, pinned `cargo nextest run` +
`cargo test --doc`, bundle + E2E with the WebdriverIO Tauri service. Actions
wiring for those jobs is task 2.x work; macOS results likewise never come
from a Linux container.

## Version pins (never floating)

Canonical file: `.config/ci-versions.json` (mirrored into `src/pins.ts`
because a Dagger module cannot read repository files outside `.dagger/` at
module load; consistency enforced by `scripts/Invoke-CI.Tests.ps1`).

| What | Pin |
|------|-----|
| Dagger CLI | `0.21.11` |
| Dagger engine | `registry.dagger.io/engine:v0.21.11` |
| Node image | `node:22.23.4-bookworm@sha256:7702d8aa4d5910871692af4480eb3ba86f23047a6417a44d857d06c95d060d3f` (repo CI uses Node 22) |
| Rust image / toolchain | `rust:1.98.1-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e` / `1.98.1` (target-state MSRV 1.89 is later spec work) |
| cargo-nextest | `0.9.150` (same pin as `.config/nextest.toml` and `scripts/Invoke-CI.ps1`) |
