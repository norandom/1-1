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
| `linux-build`   | `rust:1.98.1-bookworm` + `node:22.23.4-bookworm` | `npm ci` → `npx tauri build --target x86_64-unknown-linux-gnu` (AppImage/deb export) |
| `linux-e2e`     | `rust:1.98.1-bookworm`             | `npm ci` → `npm run test:e2e` (WebKitGTK driver per design.md E2E stack) |
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
| `drift-npm-registry-linux-x86_64-node-22.23.4` | `/root/.npm` | frontend |
| `drift-cargo-registry-linux-x86_64-rust-1.98.1` | `/usr/local/cargo/registry` | rust-checks |
| `drift-cargo-git-linux-x86_64-rust-1.98.1` | `/usr/local/cargo/git` | rust-checks |
| `drift-cargo-target-linux-x86_64-rust-1.98.1-features-default` | `/src/src-tauri/target` | rust-checks |

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
