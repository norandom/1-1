/**
 * Version pins for the drift CI contract (task 1.4).
 *
 * The canonical pin file is .config/ci-versions.json at the repository root;
 * a Dagger module cannot read repository files outside .dagger/ at module
 * load time, so this file mirrors it and scripts/Invoke-CI.Tests.ps1 (CT04d)
 * enforces that the image tags and engine image stay identical in both
 * places. NEVER use floating tags such as `latest`.
 */

export const pins = {
  dagger: {
    cliVersion: "0.21.11",
    engineImage: "registry.dagger.io/engine:v0.21.11",
    // Windows dev host engine discovery (task 1.3 finding):
    // scripts/Invoke-CI.ps1 sets this itself; GitHub Linux runners use Docker.
    runnerHost: "image+podman://registry.dagger.io/engine:v0.21.11",
  },  node: {
    // Repo CI (.github/workflows/ci.yml) runs Node 22.
    image: "node:22.23.4-bookworm@sha256:7702d8aa4d5910871692af4480eb3ba86f23047a6417a44d857d06c95d060d3f",
  },
  rust: {
    // Local host and CI @stable currently resolve to 1.98.1; the active
    // spec's target-state MSRV is 1.89 (src-tauri rust-version bump is later
    // spec work, not this pin).
    image: "rust:1.98.1-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e",
    toolchain: "1.98.1",
  },
  nextest: {
    version: "0.9.150",
    // Pinned prebuilt release tarball (the get.nexte.st URL floats to
    // `latest`, which this repository forbids).
    linuxTarball:
      "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-0.9.150/cargo-nextest-0.9.150-x86_64-unknown-linux-gnu.tar.gz",
  },
  tauriLinuxAptPackages: [
    "libwebkit2gtk-4.1-dev",
    "libappindicator3-dev",
    "librsvg2-dev",
    "patchelf",
    "libdbus-1-dev",
    "libssl-dev",
    "pkg-config",
  ],
} as const

/**
 * Contract entry name -> actual `dagger call` function name.
 *
 * The dagger v0.21.11 TypeScript SDK normalizes method names to kebab-case
 * and splits digit/letter boundaries, so the method `linuxE2e` registers as
 * the callable `linux-e-2-e`. The contract (task text, README,
 * scripts/Invoke-CI.ps1 -DaggerEntry) keeps the single stable name
 * `linux-e2e`; scripts/Invoke-CI.ps1 translates it through this exact
 * mapping before invoking the dagger CLI (verified by tests CT04c/CT08e).
 */
export const daggerCallNames = {
  frontend: "frontend",
  "rust-checks": "rust-checks",
  "linux-build": "linux-build",
  "linux-e2e": "linux-e-2-e",
  contract: "contract",
} as const

/**
 * The one-command CI contract (task 1.4): which tier runs where, with which
 * commands. This is the machine-readable twin of .dagger/README.md; the
 * container/native split is a Requirement 7.4/8.4/12.5 constraint — Dagger
 * Linux containers are not Windows/macOS build environments (design.md
 * "CI ownership and reuse decisions").
 */
export const contract = {
  module: "drift (.dagger/, Dagger TypeScript SDK)",
  engine: pins.dagger.engineImage,
  containerEntries: [
    {
      entry: "frontend",
      image: pins.node.image,
      runsOn: "Linux container (Dagger) — portable checks",
      commands: [
        "npm ci",
        "npm run lint",
        "npx tsc --noEmit",
        "npx vitest run",
        "npm run build",
      ],
      status: "executable (task 2.1 owns JUnit emission and failing-fixture gate parity)",
    },
    {
      entry: "rust-checks",
      image: pins.rust.image,
      runsOn: "Linux container (Dagger) — portable checks",
      commands: [
        "apt-get install " + pins.tauriLinuxAptPackages.join(" "),
        "rustup component add rustfmt clippy",
        "install cargo-nextest " + pins.nextest.version + " from its pinned release tarball",
        "cargo fmt --check --manifest-path src-tauri/Cargo.toml",
        "cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings",
        "cargo check --manifest-path src-tauri/Cargo.toml",
        "cargo-nextest nextest run --config-file .config/nextest.toml --profile ci --manifest-path src-tauri/Cargo.toml --no-tests fail",
        "cargo test --doc --manifest-path src-tauri/Cargo.toml",
      ],
      status: "executable (task 2.1 owns JUnit emission and failing-fixture gate parity)",
    },
    {
      entry: "linux-build",
      image: pins.rust.image + " + " + pins.node.image,
      runsOn: "Linux container (Dagger)",
      commands: ["npm ci", "npx tauri build --target x86_64-unknown-linux-gnu"],
      expectedArtifacts: ["src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/**/*.AppImage", "src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/**/*.deb"],
      status: "contract-validated; bundle execution owned by task 2.2 (design.md file plan) — this entry point must not gate CI until 2.2 lands",
    },
    {
      entry: "linux-e2e",
      image: pins.rust.image,
      runsOn: "Linux container (Dagger) — WebKitGTK driver (design.md E2E stack)",
      commands: ["npm ci", "npm run test:e2e -- --spec e2e/specs/**"],
      status: "blocked until the e2e/ suite lands (spec tasks 2.5+/12.x); the entry point refuses to fake a pass and fails loudly when prerequisites are missing",
    },
  ],
  nativeTiers: [
    {
      tier: "preflight",
      runsOn: "native Windows (pwsh); macOS equivalent via platform-appropriate tools",
      command: "pwsh -NoProfile -File scripts/Invoke-CI.ps1",
    },
    {
      tier: "rust",
      runsOn: "native Windows (MSVC target); macOS equivalent via platform-appropriate tools",
      command: "pwsh -NoProfile -File scripts/Invoke-CI.ps1 -Tier rust",
    },
    {
      tier: "frontend",
      runsOn: "native Windows (npm runs natively); macOS equivalent via platform-appropriate tools",
      command: "pwsh -NoProfile -File scripts/Invoke-CI.ps1 -Tier frontend",
    },
    {
      tier: "dagger",
      runsOn: "native host wrapper that wires the engine env itself, then calls the Linux-container entries",
      command: "pwsh -NoProfile -File scripts/Invoke-CI.ps1 -Tier dagger -DaggerEntry <frontend|rust-checks|linux-build|linux-e2e|contract>",
    },
  ],
  rule: "Dagger Linux containers are not Windows/macOS build environments; results from Linux containers never substitute for native Windows or macOS validation.",
} as const
