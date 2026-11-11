/**
 * Drift CI module (task 1.4): one command contract for container and native
 * jobs (spec `windows-clearcodec-royalts-e2e`, requirements 7.1, 7.2, 8.4,
 * 12.5, 13.3, 13.4; design.md "CI ownership and reuse decisions").
 *
 * Container entries (called via `dagger -m .dagger call <entry> --source <repo root>`
 * or through scripts/Invoke-CI.ps1 -Tier dagger):
 *   frontend     — real pipeline: npm ci, lint, typecheck, vitest, build in
 *                  the pinned node container.
 *   rust-checks  — real pipeline: fmt --check, clippy -D warnings, cargo
 *                  check, pinned cargo-nextest (ci profile) and doctests in
 *                  the pinned rust container with the Tauri system libraries.
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
 * Repository source is passed as a `source` Directory argument and resolved
 * by the CALLER (this dagger v0.21.11 SDK has no host-directory access from
 * module code), so callers control what is synced into the engine. Note:
 * the dagger 0.21 CLI cannot exclude host paths for Directory arguments —
 * scripts/Invoke-CI.ps1 passes the canonical repository root; task 2.1
 * revisits source loading when a newer engine/SDK allows host excludes.
 *
 * Native Windows validation (preflight/rust/frontend tiers) runs through
 * scripts/Invoke-CI.ps1 with native tools; Dagger Linux containers are not
 * Windows/macOS build environments and their results never substitute for
 * native Windows or macOS validation.
 */
import { dag, Container, Directory, object, func } from "@dagger.io/dagger"
import { pins, contract } from "./pins"

/** Mount the caller-provided repository source at /src in a container. */
function mountRepo(ctr: Container, source: Directory): Container {
  return ctr.withMountedDirectory("/src", source).withWorkdir("/src")
}

@object()
export class Drift {
  /** Frontend portable checks in the pinned Linux node container (Req 7.1). */
  @func()
  async frontend(source: Directory): Promise<string> {
    const ctr = mountRepo(
      dag.container().from(pins.node.image),
      source
    )
      .withExec(["npm", "ci"])
      .withExec(["npm", "run", "lint"])
      .withExec(["npx", "tsc", "--noEmit"])
      .withExec(["npx", "vitest", "run"])
      .withExec(["npm", "run", "build"])
      // Honest completion check: vite emits the production bundle to dist/.
      .withExec(["sh", "-c", "test -d dist && node --version"])
    await ctr.sync()
    return (
      "frontend: PASS in " +
      pins.node.image +
      " (npm ci, lint, tsc --noEmit, vitest run, build; node " +
      (await ctr.stdout()).trim() +
      ")"
    )
  }

  /**
   * Rust portable checks in the pinned Linux rust container with the Tauri
   * system libraries (Req 7.1): fmt, clippy, check, pinned cargo-nextest
   * (ci profile from .config/nextest.toml) and doctests.
   */
  @func()
  async rustChecks(source: Directory): Promise<string> {
    const ctr = this.rustChecksContainer(source)
    await ctr.sync()
    return (
      "rust-checks: PASS in " +
      pins.rust.image +
      " (fmt --check, clippy -D warnings, cargo check, cargo-nextest " +
      pins.nextest.version +
      " [ci profile], cargo test --doc)"
    )
  }

  /** The rust-checks pipeline, reusable by later tasks (2.1 parity gates). */
  rustChecksContainer(source: Directory): Container {
    return mountRepo(dag.container().from(pins.rust.image), source)
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
        "curl -LsSf " + pins.nextest.linuxTarball + " | tar -xz --strip-components=1 -C /usr/local/bin",
      ])
      .withExec(["cargo", "fmt", "--check", "--manifest-path", "src-tauri/Cargo.toml"])
      .withExec(["cargo", "clippy", "--manifest-path", "src-tauri/Cargo.toml", "--", "-D", "warnings"])
      .withExec(["cargo", "check", "--manifest-path", "src-tauri/Cargo.toml"])
      .withExec([
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
      ])
      .withExec(["cargo", "test", "--doc", "--manifest-path", "src-tauri/Cargo.toml"])
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
