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
`--source <staged source>` Directory argument resolved by the dagger CLI
(the dagger 0.21.11 TypeScript SDK has no host-directory access inside module
code). Task 2.1 source contract — **never import host build outputs**:
`scripts/Invoke-CI.ps1 -Tier dagger` stages a clean mirror first
(`D:\BuildArtifacts\drift-rdp\dagger\staged-source`) that excludes
`node_modules`, `src-tauri/target`, `dist`, `.git` and `.dagger`, and the
module rejects any source Directory that still contains them
(`assertNeverImportHostArtifacts`). This makes the contract explicit and
independent of the CLI's own sync filtering — which is implicit and
version-specific: measured on 0.21.11, passing the dirty repository root
directly did NOT import `node_modules` (the engine tree hashed identically
to the staged clean source and the call reused the cached result), and an
untracked, not-gitignored `e2e/` sentinel was not synced either. A fresh CI
checkout satisfies the guard trivially.

## Linux container checks (Dagger module, this directory)

Engine pin: `registry.dagger.io/engine:v0.21.11` (Dagger CLI `0.21.11`; task
1.3 spike PROVEN). On the Windows dev host, `scripts/Invoke-CI.ps1 -Tier
dagger` sets `_EXPERIMENTAL_DAGGER_RUNNER_HOST=image+podman://registry.dagger.io/engine:v0.21.11`
itself; GitHub-hosted Linux runners use their Docker engine instead.

| Entry point     | Pinned image                       | Commands (the tier's actual command list) |
|-----------------|------------------------------------|-------------------------------------------|
| `frontend`      | `node:22.23.4-bookworm` (+digest)  | `npm ci` → `npm run lint` → `npx tsc --noEmit` → `npx vitest run --reporter=default --reporter=junit --outputFile.junit=/test-results/vitest-junit.xml` → `npm run build` |
| `rust-checks`   | `rust:1.98.1-bookworm` (+digest)   | apt Tauri libs (once, cached layer) → `rustup component add rustfmt clippy` → install `cargo-nextest 0.9.150` (pinned tarball) → `cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo check` → `cargo-nextest nextest run --config-file .config/nextest.toml --profile ci --no-tests fail` → `cp src-tauri/target/nextest/ci/junit.xml /test-results/rust-junit.xml` → `cargo test --doc` |
| `linux-build`   | `rust:1.98.1-bookworm` + `node:22.23.4-bookworm` | apt Tauri libs + bundle extras (once, cached layer) → merge `/usr/local` from the pinned node image (node+npm, no unpinned installs) → `npm ci` → `npx tauri build --target x86_64-unknown-linux-gnu` (log → `/test-results/linux-build.log`) → verify `bundle/appimage/*.AppImage` + `bundle/deb/*.deb` (missing artifacts throw) |
| `linux-e2e`     | `rust:1.98.1-bookworm` + `node:22.23.4-bookworm` | shared `linux-build` bundle pipeline (missing bundles throw) → apt cold-launch extras `xvfb x11-utils xdotool imagemagick` (once, cached layer) → `sh scripts/linux/cold-launch.sh`: Xvfb display + isolated `HOME`/`XDG_*` app data (Req 12.7) → launch AppImage `--appimage-extract-and-run` (no FUSE) → first-usable-state proof (Drift window mapped, process alive, painted screenshot) → graceful `WM_DELETE_WINDOW` shutdown → status JSON + screenshots + logs under `/test-results/e2e` |
| `contract`      | —                                  | prints this contract as JSON |

### Task 2.1: JUnit artifacts, gating and caches

- `frontend` and `rust-checks` run their test step with
  `expect: ReturnType.Any`, collect the JUnit the reporter still wrote, and
  then **throw with the JUnit embedded** when the exit code is non-zero — a
  failing fixture fails the dagger call (Req 7.3, 13.3: no fake pass) while
  the evidence stays in the tier's `entry-<name>-output.txt`.
- On success both entries return the `/test-results` Directory;
  `scripts/Invoke-CI.ps1 -Tier dagger` chains `export --path
  D:\BuildArtifacts\drift-rdp\dagger\artifacts\<entry>\` so the JUnit
  (`vitest-junit.xml` / `rust-junit.xml`) is a scoped host CI artifact, and
  the tier writes **per-entry reports** `dagger-tier-report-<entry>.json`
  (the legacy shared `dagger-tier-report.json` is gone — no last-writer-wins).
- Doctests remain included: `cargo test --doc` stays the last rust-checks
  step (nextest does not run doctests) and its failures propagate.
- Caches are **engine-managed CacheVolumes** (never imported from or exported
  to the host), keyed by OS/arch/toolchain/features (`src/pins.ts`
  `cacheVolumeIds`):

| Cache volume id | Mounted at | Used by |
|-----------------|------------|---------|
| `drift-npm-registry-linux-x86_64-node-22.23.4` | `/root/.npm` | frontend, linux-build |
| `drift-cargo-registry-linux-x86_64-rust-1.98.1` | `/usr/local/cargo/registry` | rust-checks, linux-build |
| `drift-cargo-git-linux-x86_64-rust-1.98.1` | `/usr/local/cargo/git` | rust-checks, linux-build |
| `drift-cargo-target-linux-x86_64-rust-1.98.1-features-default` | `/src/src-tauri/target` | rust-checks, linux-build |

- The Linux Tauri apt libraries are provisioned **once** as a cached
  container layer, chained *before* the source mount so the layer survives
  source changes; `rustChecksContainer()` is the single reusable builder.

Note on call names: the dagger v0.21.11 TypeScript SDK kebab-cases method
names and splits digit/letter boundaries, so the E2E entry registers as the
callable **`linux-e-2-e`**. The contract name stays `linux-e2e`;
`scripts/Invoke-CI.ps1 -Tier dagger -DaggerEntry linux-e2e` performs the
mapping (see `src/pins.ts`, `daggerCallNames`).

### Execution status and the 2.x split (tasks.md)

- `frontend` and `rust-checks` are **executable CI gates** (task 2.1) — each
  runs its real command list above in the pinned container, emits tests/JUnit
  (exported under `dagger/artifacts/<entry>/`), and fails loudly on any step
  including an intentionally failing fixture (Req 7.1, 7.3, 13.3). Doctests
  remain included in rust-checks.
- `linux-build` is an **executable CI gate** (task 2.2) — see the next
  section: it runs the repo's current Tauri packaging, exports AppImage/deb
  plus the build log under `dagger/artifacts/linux-build/`, and missing
  artifacts fail the job.
- `linux-e2e` is an **executable built-app cold-launch CI gate** (task 2.5)
  — see the task 2.5 section below: the built AppImage launches headless
  under Xvfb with isolated app data, and a failed cold launch fails the
  tier with the failure screenshot exported. The full WebdriverIO
  WebKitGTK-driver suite is later spec work (tasks 12.x) and will extend
  the same entry.

### Task 2.2: Linux bundles — AppImage/deb export and the missing-artifact gate

- `linux-build` **reuses the repository's current Tauri packaging** —
  `npm ci` + `npx tauri build --target x86_64-unknown-linux-gnu` (the same
  command `.github/workflows/ci.yml` runs), driven by
  `src-tauri/tauri.conf.json` (`beforeBuildCommand: npm run build`; bundle
  targets `all` → AppImage/deb on Linux). No packaging configuration is
  forked.
- node+npm come from the **pinned node image** (its `/usr/local` tree merged
  into the pinned rust image — no unpinned package-manager installs), and
  the AppImage bundling extras (`file`, `libfuse2`) are provisioned once as
  the same kind of cached layer as the Tauri libs
  (`tauriLinuxBundleAptPackages` in `.config/ci-versions.json` + `src/pins.ts`).
- The entry returns its `/test-results` Directory; the tier chains
  `export --path D:\BuildArtifacts\drift-rdp\dagger\artifacts\linux-build\`
  so the scoped host CI artifacts are:
  - `bundle/appimage/*.AppImage`, `bundle/deb/*.deb` — the Linux bundles,
  - `linux-build.log` — the full build log,
  - `bundle-paths.txt` + `bundle-manifest.txt` — what the gate verified.
- **Missing artifacts fail the job** (Req 7.2, 7.3) with a double gate: the
  module verifies in-container that `bundle/appimage/*.AppImage` AND
  `bundle/deb/*.deb` exist after an exit-0 build and throws with the found
  paths when either is missing, and the tier re-checks the exported host
  directory (AppImage + deb + `linux-build.log`) and fails the tier with
  exit 1 when anything is absent — a green run with missing artifacts is a
  fake pass and cannot happen. The per-entry report records
  `bundleArtifactsExported` and `bundleLogExported`.
- Cache/output ownership is unchanged from task 2.1: the same four
  engine-managed cache volumes (the bundle build adds no `--features`
  flags, so the `features-default` key scheme still applies), host exports
  are scoped exports only. These are Linux-container bundle artifacts —
  they never substitute for native Windows/macOS build validation.

### Task 2.5: linux-e2e — built-app Linux cold launch under Xvfb

- The entry chains `scripts/linux/cold-launch.sh` (staged with the source)
  on top of the **same verified bundle container** as `linux-build`
  (`linuxBundleBuildContainer`): identical inputs replay the build layers
  from the engine cache, so the cold launch runs the BUILT AppImage —
  `--appimage-extract-and-run`, no FUSE — inside the pinned containers.
- **First usable state, proven**: a 1280x800 Xvfb display, the `Drift`
  window found mapped (xwininfo evidence), the process tree alive, and a
  PAINTED screenshot (distinct-color count; a blank root fails). Shutdown
  is a graceful `WM_DELETE_WINDOW` close with SIGTERM/SIGKILL escalation
  recorded. The GPU path runs on Mesa's llvmpipe software rasterizer — no
  GPU or dbus daemon needed (spike-proven, see research.md task 2.5).
- **Isolated app data (Req 12.7)**: every launch scopes `HOME`,
  `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and
  `XDG_RUNTIME_DIR` into a per-run `/tmp/drift-e2e-home` tree; the app log
  confirms `App data directory: .../drift-e2e-home/share/com.drift.app`
  and `appdata-files.txt` records exactly what the run wrote (the
  container is additionally isolated from the host by construction).
- **Evidence export + fail-loudly (no fake pass)**: the harness always
  writes `/test-results/e2e/` — `cold-launch-status.json` (verdict),
  `cold-launch-screenshot.png`, and on failure
  `cold-launch-failure-screenshot.png` captured BEFORE the failed verdict
  — and the entry returns that Directory. The module THROWS for
  prerequisite/evidence-loss failures (missing bundles, missing harness,
  no status at all), while a FAILED launch (ok=false) returns the evidence
  Directory so the tier can export the failure screenshot and THEN fail
  with exit 1: `scripts/Invoke-CI.ps1 -Tier dagger -DaggerEntry linux-e2e`
  reads the exported status — an exit-0 dagger call never bypasses the
  cold-launch verdict (Req 12.1, 12.5, 12.8).
- The cold-launch runtime extras (`xvfb x11-utils xdotool imagemagick`)
  are pinned in `.config/ci-versions.json` + `src/pins.ts`
  (`tauriLinuxE2EAptPackages`) and provisioned once as the same cached
  layer as the Tauri/bundle packages (shared by linux-build and linux-e2e).
- These are Linux-container results — they never substitute for the native
  Windows/macOS cold-launch validation (Req 12.8 native proofs).

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
