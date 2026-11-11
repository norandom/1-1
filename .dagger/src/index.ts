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
 *   linux-build  — real pipeline (task 2.2): reuses the repository's current
 *                  Tauri packaging (npm ci + npx tauri build --target
 *                  x86_64-unknown-linux-gnu, the same command the GitHub
 *                  workflow runs, driven by src-tauri/tauri.conf.json) in
 *                  the pinned rust+node containers. Captures the build log,
 *                  VERIFIES the AppImage and deb exist (missing artifacts
 *                  throw — Req 7.2, 7.3), and returns /test-results with
 *                  the bundles + logs for the tier's scoped host export.
 *   linux-e2e    — executable cold-launch CI gate (task 2.5): reuses the
 *                  linux-build bundle pipeline as its prerequisite, then
 *                  launches the BUILT AppImage headless under Xvfb with
 *                  isolated XDG app data via scripts/linux/cold-launch.sh.
 *                  Returns /test-results/e2e with the status JSON,
 *                  screenshots (incl. the failure screenshot) and logs;
 *                  the tier fails when ok != true — no fake pass.
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

/** In-container bundle layout for the linux-build entry (task 2.2; mirrored
 * in pins.ts): the build log, the bundle-paths evidence and the staged
 * AppImage/deb artifacts live under /test-results, which the entry returns
 * for the tier's scoped host export. */
const LINUX_BUILD_LOG = "/test-results/linux-build.log"
const LINUX_BUNDLE_PATHS = "/test-results/bundle-paths.txt"
const LINUX_BUNDLE_TARGET = "x86_64-unknown-linux-gnu"
const LINUX_BUNDLE_DIR = "src-tauri/target/" + LINUX_BUNDLE_TARGET + "/release/bundle"
const LINUX_APPIMAGE_GLOB = LINUX_BUNDLE_DIR + "/appimage/*.AppImage"
const LINUX_DEB_GLOB = LINUX_BUNDLE_DIR + "/deb/*.deb"

/** In-container evidence layout for the linux-e2e cold-launch entry (task
 * 2.5; mirrored in pins.ts and scripts/linux/cold-launch.sh): the harness
 * writes its status, screenshots and logs under /test-results/e2e, and the
 * entry returns that subdirectory so the tier's scoped export stays lean
 * (the heavy bundles stay in the linux-build artifacts). */
const LINUX_E2E_DIR = "/test-results/e2e"
const LINUX_E2E_STATUS = LINUX_E2E_DIR + "/cold-launch-status.json"

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

/** Build the loud failure for a failed bundle step, embedding its build-log evidence. */
function bundleFailure(entry: string, step: string, exit: number, log: string): Error {
  const excerpt = log.length > 4000 ? log.slice(0, 4000) + "\n... (truncated)" : log
  return new Error(
    entry +
      ": " +
      step +
      " failed with exit code " +
      exit +
      " — the bundle gate fails loudly, no fake pass (Req 7.2, 7.3). Build log evidence:\n" +
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
   * The linux-build base container (task 2.2): the pinned rust image with
   * the pinned node image's /usr/local tree merged on top (the canonical
   * way to run npm inside the rust container while staying on exactly the
   * two pinned images — no unpinned package-manager installs), the Tauri
   * system libraries plus the bundle extras provisioned ONCE as cached
   * layers (before the source mount, so the layers survive source changes),
   * and the SAME engine-managed cache volumes as rust-checks: the bundle
   * build's cargo feature set is unchanged (no --features flags), so the
   * pins.ts cacheVolumeIds key scheme (OS/arch/toolchain/features) applies
   * unchanged. The merged toolchain is self-verified loudly before any
   * build step runs.
   */
  linuxBuildContainer(source: Directory): Container {
    const nodeTools = dag.container().from(pins.node.image).directory("/usr/local")
    return dag
      .container()
      .from(pins.rust.image)
      .withDirectory("/usr/local", nodeTools)
      .withExec(["apt-get", "update"])
      .withExec([
        "apt-get",
        "install",
        "-y",
        "--no-install-recommends",
        ...pins.tauriLinuxAptPackages,
        ...pins.tauriLinuxBundleAptPackages,
        // Task 2.5: the cold-launch runtime extras ride the SAME cached
        // layer (before the source mount) so linux-build and linux-e2e
        // share one provisioning; linux-build simply does not use them.
        ...pins.tauriLinuxE2EAptPackages,
      ])
      .withExec(["node", "--version"])
      .withExec(["npm", "--version"])
      .withMountedCache("/root/.npm", dag.cacheVolume(cacheVolumeIds.npmRegistry))
      .withMountedCache(
        "/usr/local/cargo/registry",
        dag.cacheVolume(cacheVolumeIds.cargoRegistry)
      )
      .withMountedCache("/usr/local/cargo/git", dag.cacheVolume(cacheVolumeIds.cargoGit))
      .withMountedDirectory("/src", source)
      .withMountedCache("/src/src-tauri/target", dag.cacheVolume(cacheVolumeIds.cargoTarget))
      .withWorkdir("/src")
      .withExec(["mkdir", "-p", "/test-results"])
      .withExec(["npm", "ci"])
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
   * Linux bundle build (Req 7.2; task 2.2) — shared pipeline for linuxBuild
   * and linuxE2E (task 2.5). Reuses the repository's current Tauri
   * packaging — npm ci plus `npx tauri build --target
   * x86_64-unknown-linux-gnu` (the same command .github/workflows/ci.yml
   * runs), driven by src-tauri/tauri.conf.json (beforeBuildCommand
   * `npm run build`; bundle targets "all" → AppImage/deb on Linux) — inside
   * the pinned containers. The build log is captured to
   * /test-results/linux-build.log; after a successful build the produced
   * AppImage and deb are VERIFIED (missing artifacts throw — a green build
   * without bundles is a fake pass and missing artifacts fail the job, Req
   * 7.2, 7.3), and the verified bundles + logs are staged under
   * /test-results. `linuxBuild` returns that Directory for the tier's
   * scoped host export; `linuxE2E` chains the cold-launch harness on top
   * of the SAME container so identical inputs replay the build layers from
   * the engine cache (the 2.5 cold launch never rebuilds what 2.2 already
   * cached). Linux-container bundle results never substitute for native
   * Windows or macOS artifacts.
   */
  private async linuxBundleBuildContainer(source: Directory): Promise<Container> {
    await assertNeverImportHostArtifacts(source)
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

    // 1) Build with the current Tauri packaging. The step runs with
    //    expect-any and logs to /test-results/linux-build.log so the full
    //    build evidence survives both success and failure; the exit code is
    //    checked right after and FAILS the call loudly (no fake pass).
    const buildCtr = this.linuxBuildContainer(source).withExec(
      [
        "sh",
        "-c",
        "npx tauri build --target " +
          LINUX_BUNDLE_TARGET +
          " > " +
          LINUX_BUILD_LOG +
          " 2>&1; ec=$?; cat " +
          LINUX_BUILD_LOG +
          "; exit $ec",
      ],
      { expect: ReturnType.Any }
    )
    const buildExit = await buildCtr.exitCode()
    const buildLog = await readText(buildCtr.file(LINUX_BUILD_LOG))
    if (buildExit !== 0) {
      throw bundleFailure(
        "linux-build",
        "npx tauri build --target " + LINUX_BUNDLE_TARGET,
        buildExit,
        buildLog
      )
    }
    if (buildLog === "") {
      throw new Error(
        "linux-build: build exited 0 but produced no log at " +
          LINUX_BUILD_LOG +
          " — exporting the build log is part of the task 2.2 contract"
      )
    }

    // 2) Verify the bundle contract: AppImage AND deb must exist after a
    //    green build. Missing artifacts fail the job (task 2.2 Done
    //    criterion, Req 7.2, 7.3) — never a fake pass. bundle-paths.txt
    //    records what was actually found as the failure evidence.
    const verifyCtr = buildCtr.withExec(
      [
        "sh",
        "-c",
        'appimage=$(ls ' +
          LINUX_APPIMAGE_GLOB +
          " 2>/dev/null || true); " +
          "deb=$(ls " +
          LINUX_DEB_GLOB +
          " 2>/dev/null || true); " +
          'printf "APPIMAGE=%s\\nDEB=%s\\n" "$appimage" "$deb" > ' +
          LINUX_BUNDLE_PATHS +
          "; cat " +
          LINUX_BUNDLE_PATHS +
          '; test -n "$appimage" -a -n "$deb"',
      ],
      { expect: ReturnType.Any }
    )
    await verifyCtr.sync()
    const verifyExit = await verifyCtr.exitCode()
    const bundlePaths = await readText(verifyCtr.file(LINUX_BUNDLE_PATHS))
    if (verifyExit !== 0) {
      throw new Error(
        "linux-build: bundle artifacts missing after an exit-0 build — missing artifacts fail the job (Req 7.2, 7.3). Expected AppImage at " +
          LINUX_APPIMAGE_GLOB +
          " and deb at " +
          LINUX_DEB_GLOB +
          "; found: " +
          (bundlePaths === "" ? "<nothing>" : bundlePaths.replace(/\n/g, "; "))
      )
    }

    // 3) Stage the verified bundles + logs for the tier's scoped host export.
    const done = verifyCtr.withExec([
      "sh",
      "-c",
      "set -e; mkdir -p /test-results/bundle/appimage /test-results/bundle/deb; " +
        "cp " +
        LINUX_APPIMAGE_GLOB +
        " /test-results/bundle/appimage/; " +
        "cp " +
        LINUX_DEB_GLOB +
        " /test-results/bundle/deb/; " +
        "ls -lR /test-results/bundle > /test-results/bundle-manifest.txt; cat /test-results/bundle-manifest.txt",
    ])
    await done.sync()
    return done
  }

  /** Linux bundle export (Req 7.2; task 2.2): the shared bundle pipeline's
   * /test-results with the verified AppImage/deb + logs, for the tier's
   * scoped host export (`dagger ... export --path`). */
  @func()
  async linuxBuild(source: Directory): Promise<Directory> {
    const done = await this.linuxBundleBuildContainer(source)
    return done.directory("/test-results")
  }

  /**
   * Linux E2E — executable built-app cold-launch CI gate (Req 12.1, 12.5,
   * 12.7, 12.8; task 2.5). Chains the cold-launch harness
   * (scripts/linux/cold-launch.sh, staged with the source) on top of the
   * SAME verified bundle container as linux-build: the built AppImage is
   * launched headless under Xvfb with per-run isolated XDG app data, must
   * reach first usable state (Drift window mapped, process alive, PAINTED
   * screenshot) and shut down gracefully. The harness writes its verdict
   * and evidence to /test-results/e2e (status JSON, screenshot, FAILURE
   * screenshot on failure, logs), and this entry returns that Directory.
   *
   * Fail-loudly contract (no fake pass): prerequisite failures (missing
   * bundle inputs, failing build, missing bundles, missing harness, or a
   * harness that produced NO status at all) THROW — a dagger call that
   * cannot produce evidence must not exit 0. A FAILED cold launch
   * (ok=false), in contrast, does NOT throw here: the evidence Directory is
   * returned precisely so the tier can `export --path` the failure
   * screenshot into the scoped artifacts directory and THEN fail the job
   * with exit 1 (scripts/Invoke-CI.ps1 reads cold-launch-status.json);
   * throwing would lose the failure evidence because a failed dagger call
   * cannot chain an export.
   *
   * The full WebdriverIO suite on the WebKitGTK driver is later spec work
   * (tasks 12.x) and will extend this entry; until then this is the
   * minimal cold-launch proof. Linux-container E2E results never
   * substitute for the native Windows/macOS E2E runs.
   *
   * Callable as `dagger call linux-e-2-e`: the dagger v0.21.11 TypeScript
   * SDK kebab-cases method names and splits digit/letter boundaries, so
   * `linuxE2E` registers as `linux-e-2-e` (see daggerCallNames in pins.ts;
   * scripts/Invoke-CI.ps1 maps the contract name `linux-e2e` to it).
   */
  @func()
  async linuxE2E(source: Directory): Promise<Directory> {
    const built = await this.linuxBundleBuildContainer(source)
    // tr strips any stray CR so a Windows checkout of the harness cannot
    // break the in-container shell (defense in depth; the repo keeps LF).
    const runCtr = built.withExec(
      [
        "sh",
        "-c",
        "tr -d '\\r' < scripts/linux/cold-launch.sh > /tmp/cold-launch.sh && sh /tmp/cold-launch.sh",
      ],
      { expect: ReturnType.Any }
    )
    await runCtr.sync()
    const status = await readText(runCtr.file(LINUX_E2E_STATUS))
    if (status === "") {
      throw new Error(
        "linux-e2e: the cold-launch harness produced no /test-results/e2e/cold-launch-status.json — " +
          "the run failed before any evidence could be collected (missing harness in the staged " +
          "source or a crashed harness; the step log is in the tier output). This entry point " +
          "refuses to fake a pass (Req 12.5, 12.8)"
      )
    }
    // ok=false is handled by the tier AFTER the evidence export (see the
    // docblock): return the evidence directory either way.
    return runCtr.directory(LINUX_E2E_DIR)
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
