/**
 * Drift CI module (tasks 1.4 + 2.1): one command contract for container and
 * native jobs (spec `windows-clearcodec-royalts-e2e`, requirements 7.1, 7.2,
 * 8.4, 12.5, 13.3, 13.4; design.md "CI ownership and reuse decisions").
 *
 * Container entries (called via `dagger -m .dagger call <entry> --source <staged source>`
 * or through scripts/Invoke-CI.ps1 -Tier dagger):
 *   frontend     — real pipeline: npm ci, lint, typecheck, vitest (JUnit) and
 *                  build in the pinned node container. Returns the /test-results
 *                  Directory; export it with `... export --path <dir>`.
 *   rust-checks  — real pipeline: fmt --check, clippy -D warnings, cargo
 *                  check, pinned cargo-nextest (ci profile, JUnit) and
 *                  doctests (cargo test --doc) in the pinned rust container
 *                  with the Tauri system libraries. Returns /test-results.
 *   linux-build  — validates the bundle contract (inputs present, images
 *                  pinned) and reports the exact command list; executing the
 *                  bundle is owned by task 2.2, so the returned status is an
 *                  explicit `contract-validated`/deferred — never a faked
 *                  bundle pass.
 *   linux-e2e    — validates E2E prerequisites and fails loudly while the
 *                  e2e/ suite does not exist (spec tasks 2.5+/12.x own the
 *                  suite and driver wiring); it never fakes a pass.
 *   contract     — prints the machine-readable command contract (the twin of
 *                  .dagger/README.md).
 *
 * Task 2.1 source contract — NEVER import host build outputs: the caller
 * stages a clean source (scripts/Invoke-CI.ps1 does; a fresh CI checkout
 * qualifies too) and assertNeverImportHostArtifacts() rejects any source
 * that still contains node_modules, dist or src-tauri/target (host
 * node_modules carry platform-specific binaries; a host-built target/
 * poisons the Linux checks). Staging is explicit so the contract never
 * depends on the CLI's own, version-specific sync filtering.
 *
 * Task 2.1 caching — registry/git/target caches are engine-managed
 * CacheVolumes with ids keyed by OS/arch/toolchain/features (pins.ts
 * cacheVolumeIds); nothing is imported from or exported to the host caches.
 *
 * Task 2.1 gating — test steps run with `expect: ReturnType.Any` so their
 * JUnit is still produced when tests fail; the entry then reads the exit code
 * and THROWS with the JUnit evidence embedded (Req 7.3, 13.3: no fake pass,
 * the dagger call exits non-zero).
 *
 * Native Windows validation (preflight/rust/frontend tiers) runs through
 * scripts/Invoke-CI.ps1 with native tools; Dagger Linux containers are not
 * Windows/macOS build environments and their results never substitute for
 * native Windows or macOS validation.
 */
import { dag, Container, Directory, File, object, func, ReturnType } from "@dagger.io/dagger"
import { pins, contract, cacheVolumeIds } from "./pins"

/** In-container JUnit artifact paths (task 2.1 contract; also mirrored in
 * pins.ts and .dagger/README.md): /test-results is returned by the check
 * entries and exported to the scoped host artifact directory by the tier. */
const VITEST_JUNIT_PATH = "/test-results/vitest-junit.xml"
const RUST_JUNIT_PATH = "/test-results/rust-junit.xml"

/**
 * Enforce the task 2.1 source contract: a source directory must never include
 * host build outputs — node_modules (platform-specific binaries) and a
 * host-built target/ directory poison the Linux checks. Callers stage a clean
 * source (scripts/Invoke-CI.ps1) or pass a fresh checkout.
 */
async function assertNeverImportHostArtifacts(source: Directory): Promise<void> {
  const top = await source.entries()
  const bannedTop = ["node_modules", "dist"]
  const present = bannedTop.filter((name) => top.includes(name))
  try {
    const srcTauri = await source.directory("src-tauri").entries()
    if (srcTauri.includes("target")) {
      present.push("src-tauri/target")
    }
  } catch {
    // A missing src-tauri fails later with a clearer per-file error.
  }
  if (present.length > 0) {
    throw new Error(
      "source directory must never include host build outputs (found: " +
        present.join(", ") +
        "); stage a clean source that excludes node_modules, src-tauri/target and dist " +
        "(scripts/Invoke-CI.ps1 -Tier dagger stages one; task 2.1 source contract)"
    )
  }
}

/** Read a container file, returning "" when it was not produced. */
async function readText(file: File): Promise<string> {
  try {
    return await file.contents()
  } catch {
    return ""
  }
}

/** Build the loud failure for a failed test step, embedding its JUnit evidence. */
function junitFailure(entry: string, step: string, exit: number, junit: string): Error {
  const excerpt = junit.length > 4000 ? junit.slice(0, 4000) + "\n... (truncated)" : junit
  return new Error(
    entry +
      ": " +
      step +
      " failed with exit code " +
      exit +
      " — the check fails loudly, no fake pass (Req 7.3, 13.3). JUnit evidence:\n" +
      excerpt
  )
}

@object()
export class Drift {
  /** Frontend portable checks in the pinned Linux node container (Req 7.1). */
  @func()
  async frontend(source: Directory): Promise<Directory> {
    await assertNeverImportHostArtifacts(source)
    const base = dag
      .container()
      .from(pins.node.image)
      .withMountedCache("/root/.npm", dag.cacheVolume(cacheVolumeIds.npmRegistry))
      .withMountedDirectory("/src", source)
      .withWorkdir("/src")
      .withExec(["mkdir", "-p", "/test-results"])
      .withExec(["npm", "ci"])
      .withExec(["npm", "run", "lint"])
      .withExec(["npx", "tsc", "--noEmit"])
    // Unit tests with JUnit: expect-any so the JUnit is written even when
    // tests fail; the exit code is checked right after (no fake pass).
    const testCtr = base.withExec(
      [
        "npx",
        "vitest",
        "run",
        "--reporter=default",
        "--reporter=junit",
        "--outputFile.junit=" + VITEST_JUNIT_PATH,
      ],
      { expect: ReturnType.Any }
    )
    const testExit = await testCtr.exitCode()
    if (testExit !== 0) {
      const junit = await readText(testCtr.file(VITEST_JUNIT_PATH))
      throw junitFailure("frontend", "npx vitest run", testExit, junit)
    }
    const done = testCtr
      .withExec(["npm", "run", "build"])
      // Honest completion check: vite emits the production bundle to dist/.
      .withExec(["sh", "-c", "test -d dist && node --version"])
    await done.sync()
    return done.directory("/test-results")
  }

  /**
   * The rust-checks base container (task 2.1): the pinned rust image with the
   * Linux Tauri system libraries provisioned ONCE as a cached layer (before
   * the source mount, so the layer survives source changes), rustfmt/clippy
   * and the pinned cargo-nextest installed, cargo registry/git/target caches
   * mounted as separate engine-managed volumes, and the static checks (fmt,
   * clippy, check) already chained. Reused by the rustChecks entry (and later
   * tasks) so the provisioning cost is paid once per image/toolchain pin.
   */
  rustChecksContainer(source: Directory): Container {
    return dag
      .container()
      .from(pins.rust.image)
      .withExec(["apt-get", "update"])
      .withExec([
        "apt-get",
        "install",
        "-y",
        "--no-install-recommends",
        ...pins.tauriLinuxAptPackages,
      ])
      .withExec(["rustup", "component", "add", "rustfmt", "clippy"])
      .withExec([
        "sh",
        "-c",
        // The pinned release tarball contains the binary at its ROOT (no
        // leading directory), so it must NOT be extracted with
        // --strip-components; --version self-verifies the install (a broken
        // install fails HERE, loudly, instead of at first use).
        "curl -LsSf " + pins.nextest.linuxTarball + " | tar -xz -C /usr/local/bin && cargo-nextest --version",
      ])
      .withMountedCache(
        "/usr/local/cargo/registry",
        dag.cacheVolume(cacheVolumeIds.cargoRegistry)
      )
      .withMountedCache("/usr/local/cargo/git", dag.cacheVolume(cacheVolumeIds.cargoGit))
      .withMountedDirectory("/src", source)
      .withMountedCache("/src/src-tauri/target", dag.cacheVolume(cacheVolumeIds.cargoTarget))
      .withWorkdir("/src")
      .withExec(["cargo", "fmt", "--check", "--manifest-path", "src-tauri/Cargo.toml"])
      .withExec(["cargo", "clippy", "--manifest-path", "src-tauri/Cargo.toml", "--", "-D", "warnings"])
      .withExec(["cargo", "check", "--manifest-path", "src-tauri/Cargo.toml"])
  }

  /**
   * Rust portable checks in the pinned Linux rust container with the Tauri
   * system libraries (Req 7.1): fmt, clippy, check, pinned cargo-nextest
   * (ci profile from .config/nextest.toml, JUnit copied to /test-results) and
   * doctests via cargo test --doc (nextest does not run doctests; Req 7.1).
   * Returns the /test-results Directory with the JUnit evidence.
   */
  @func()
  async rustChecks(source: Directory): Promise<Directory> {
    await assertNeverImportHostArtifacts(source)
    // Unit/integration tests with JUnit: expect-any so the JUnit (written by
    // the ci profile to src-tauri/target/nextest/ci/junit.xml) survives a
    // failing run; the exit code is checked right after (no fake pass).
    const nextestCtr = this.rustChecksContainer(source).withExec(
      [
        "cargo-nextest",
        "nextest",
        "run",
        "--config-file",
        ".config/nextest.toml",
        "--profile",
        "ci",
        "--manifest-path",
        "src-tauri/Cargo.toml",
        "--no-tests",
        "fail",
      ],
      { expect: ReturnType.Any }
    )
    const nextestExit = await nextestCtr.exitCode()
    // The copy runs with expect-any so a missing JUnit cannot mask the real
    // failure: the branches below report the nextest exit code, and a PASSING
    // nextest without JUnit is itself a contract violation (ci profile).
    const collected = nextestCtr.withExec(
      [
        "sh",
        "-c",
        "mkdir -p /test-results && cp src-tauri/target/nextest/ci/junit.xml /test-results/rust-junit.xml",
      ],
      { expect: ReturnType.Any }
    )
    await collected.sync()
    const junit = await readText(collected.file(RUST_JUNIT_PATH))
    if (nextestExit !== 0) {
      throw junitFailure("rust-checks", "cargo-nextest nextest run", nextestExit, junit)
    }
    if (junit === "") {
      throw new Error(
        "rust-checks: cargo-nextest exited 0 but produced no JUnit at " +
          RUST_JUNIT_PATH +
          " — the ci profile JUnit export is part of the task 2.1 contract"
      )
    }
    // Doctests remain included as a separate failing-loudly step (Req 7.1).
    const done = collected.withExec([
      "cargo",
      "test",
      "--doc",
      "--manifest-path",
      "src-tauri/Cargo.toml",
    ])
    await done.sync()
    return done.directory("/test-results")
  }

  /**
   * Linux bundle contract (Req 7.2). Task 1.4 validates the contract —
   * inputs present in the mounted source, images pinned — and reports the
   * exact command list; task 2.2 (design.md file plan) owns executing the
   * Tauri bundle and exporting AppImage/deb artifacts. The status is an
   * explicit deferred marker, never a bundle pass.
   */
  @func()
  async linuxBuild(source: Directory): Promise<string> {
    const required = ["package.json", "package-lock.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"]
    const missing: string[] = []
    const topEntries = await source.entries()
    for (const path of required) {
      if (path.includes("/")) {
        const [dir, file] = path.split(/\/(.*)/)
        try {
          const files = await source.directory(dir).entries()
          if (!files.includes(file)) missing.push(path)
        } catch {
          missing.push(path)
        }
      } else if (!topEntries.includes(path)) {
        missing.push(path)
      }
    }
    if (missing.length > 0) {
      throw new Error(
        "linux-build: missing bundle inputs in the mounted source: " +
          missing.join(", ") +
          "; the Linux bundle contract requires them (task 2.2 executes the bundle)"
      )
    }
    const entry = contract.containerEntries.find((e) => e.entry === "linux-build")
    return JSON.stringify(entry)
  }

  /**
   * Linux E2E contract (Req 12.5). The e2e/ suite does not exist yet (spec
   * tasks 2.5+/12.x own the WebdriverIO suite and the WebKitGTK driver
   * wiring), so this entry point validates prerequisites and fails loudly
   * instead of faking a pass. Once the suite lands, this is the single
   * place that runs it in the pinned container.
   *
   * Callable as `dagger call linux-e-2-e`: the dagger v0.21.11 TypeScript
   * SDK kebab-cases method names and splits digit/letter boundaries, so
   * `linuxE2E` registers as `linux-e-2-e` (see daggerCallNames in pins.ts;
   * scripts/Invoke-CI.ps1 maps the contract name `linux-e2e` to it).
   */
  @func()
  async linuxE2E(source: Directory): Promise<string> {
    const entries = await source.entries()
    if (!entries.includes("e2e")) {
      throw new Error(
        "linux-e2e: prerequisite e2e/ suite is not present in the mounted source; " +
          "it is delivered by spec tasks 2.5+/12.x (Req 12.1, 12.5). This entry point " +
          "refuses to fake a pass — Linux-container E2E results also never substitute " +
          "for the native Windows/macOS E2E runs."
      )
    }
    const entry = contract.containerEntries.find((e) => e.entry === "linux-e2e")
    return JSON.stringify(entry)
  }

  /**
   * Machine-readable CI command contract: container entries with pinned
   * images and commands, native tiers with their single commands, and the
   * container/native separation rule. Twin of .dagger/README.md.
   */
  @func()
  contract(): string {
    return JSON.stringify(contract, null, 2)
  }
}
