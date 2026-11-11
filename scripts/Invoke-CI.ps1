#Requires -Version 7.0
<#
.SYNOPSIS
  Native (pwsh) entry point for drift-rdp CI tier commands.

.DESCRIPTION
  Implements the native tier entry point required by design.md ("CI ownership
  and reuse decisions": scripts/Invoke-CI.ps1 provides native tier entry
  points, also invoked by CI). Task 1.1 ships the preflight tier; task 1.2
  adds the rust test tier; later tasks (1.3-1.4) extend this script with
  further tiers — keep functions small and composable.

  The preflight (Requirements 7.1, 7.5, 12.7, 13.4):
    - reports required native tool versions and resolved paths;
    - inspects wrapper targets BEFORE execution and rejects WSL / Git Bash
      (MSYS, Cygwin) delegation, naming the offending path;
    - rejects the C:\Users\mariu\Source\drift-rdp junction view as working
      directory in favor of the canonical D:\Source\drift-rdp repository path,
      so cache keys and BuildArtifacts scoping stay stable;
    - scopes every artifact it produces under D:\BuildArtifacts\drift-rdp\...
      (never a drive root) and cleans up only its own resources.

  The rust test tier (task 1.2; Requirements 7.1, 7.3, 13.3, 13.4):
    - runs the pinned cargo-nextest ($DriftNextestMinVersion, mirrored by
      nextest-version.required in .config/nextest.toml) with the CI profile
      from the repository-root .config/nextest.toml: fail-on-zero-tests
      (--no-tests fail), slow-timeout/leak-timeout reporting, JUnit output,
      and NO automatic retries (no `retries` key anywhere, no --retries flag);
    - serialize only tests that share OS-level resources via the os-keyring
      test group declared in .config/nextest.toml;
    - keeps cargo test --doc as a separate step, because nextest does not run
      doctests, and propagates doctest failures to the tier exit code;
    - copies JUnit evidence and step logs under D:\BuildArtifacts\drift-rdp\rust
      and writes a machine-readable tier report (durations, versions, exit
      codes) so Windows process-launch overhead stays observable rather than
      assumed.

  The preflight honors existing tool caches (npm, cargo); it neither creates
  nor relocates them.

  The frontend test tier (task 1.4; Requirements 7.1, 13.3, 13.4): npm runs
  natively on Windows, so this is the native twin of the Dagger `frontend`
  Linux-container entry point (.dagger/src/index.ts) — same portable command
  list: npm ci, lint, tsc --noEmit, vitest, build; failures propagate to the
  tier exit code and evidence lands under D:\BuildArtifacts\<project>\frontend.

  The dagger tier (task 1.4; Requirements 7.1, 7.2, 8.4, 12.5): invokes the
  Linux-container entries of the .dagger module (frontend, rust-checks,
  linux-build, linux-e2e) or the `contract` introspection. Engine wiring is
  the tier's own job, not a personal launcher's: it loads the pinned versions
  from .config/ci-versions.json, resolves the REAL dagger.exe (never a .cmd
  launcher), makes the native podman client reachable, sets
  _EXPERIMENTAL_DAGGER_RUNNER_HOST=image+podman://<pinned engine image>
  itself (task 1.3 finding), and propagates the dagger exit code. Linux
  containers are not Windows/macOS validation: native tiers stay native.

  Single command per tier (Req 13.4):
    preflight:    pwsh -NoProfile -File scripts/Invoke-CI.ps1
    rust tests:   pwsh -NoProfile -File scripts/Invoke-CI.ps1 -Tier rust
    frontend:     pwsh -NoProfile -File scripts/Invoke-CI.ps1 -Tier frontend
    dagger:       pwsh -NoProfile -File scripts/Invoke-CI.ps1 -Tier dagger -DaggerEntry <frontend|rust-checks|linux-build|linux-e2e|contract>
    their tests:  pwsh -NoProfile -File scripts/Invoke-CI.Tests.ps1
    Full CI command contract (container vs native): .dagger/README.md

  Exit codes: 0 = tier passed; 1 = rejected/failed (reasons on stderr).

  Wrapper inspection covers reparse/symlink target chains, prohibited path
  roots (WSL filesystem views, Git\usr\bin, MSYS, Cygwin), prohibited
  interpreter leaves, and text-delegation patterns inside script shims
  (.cmd/.bat/.ps1/.psm1/.sh/.py). Native .exe binaries cannot contain a
  textual shim and are validated by path rules only.
#>
[CmdletBinding()]
param(
    # Tier to run; later tasks add further tiers.
    [ValidateSet('preflight', 'rust', 'frontend', 'dagger')]
    [string]$Tier = 'preflight',

    # Override the effective working directory (used by tests; defaults to the
    # real current location so junction views are detected, never bypassed).
    [string]$WorkingDirectory = '',

    # Rust tier: cargo manifest under test (defaults to src-tauri\Cargo.toml;
    # tests point it at throwaway fixture crates).
    [string]$ManifestPath = '',

    # Dagger tier (task 1.4): which .dagger module entry point to call. The
    # contract names map 1:1 to the module functions; 'linux-e2e' translates
    # to the SDK-registered callable 'linux-e-2-e' (see Get-DriftDaggerCallName).
    [ValidateSet('frontend', 'rust-checks', 'linux-build', 'linux-e2e', 'contract')]
    [string]$DaggerEntry = 'contract',

    # Dagger tier: explicit path to the real dagger CLI executable (tests
    # point it at recording shims); default resolution never accepts a
    # launcher/wrapper (Resolve-DriftDaggerCli).
    [string]$DaggerCliPath = '',

    [string]$BuildArtifactsRoot = 'D:\BuildArtifacts',
    [string]$ProjectName = 'drift-rdp',
    [switch]$NoReport
)

Set-StrictMode -Version 3.0

# Pinned cargo-nextest minimum version (task 1.2; tasks.md "Crate and tool
# choices": a supported release, at least 0.9.150 per the current upstream
# advisory). cargo-nextest is a CI/developer executable, never an application
# dependency. Keep in sync with nextest-version.required in
# .config/nextest.toml, which nextest also enforces on its own (exit 92).
$DriftNextestMinVersion = '0.9.150'

function ConvertTo-DriftNormalizedPath {
    <# Normalize separators/case shape for path comparisons; does not touch the filesystem. #>
    param([Parameter(Mandatory)][string]$Path)
    $normalized = $Path.Replace('/', '\').TrimEnd('\')
    if ($normalized -match '^([a-zA-Z]):$') { $normalized = "$($Matches[1].ToUpper()):\" }
    if ($normalized -eq '') { $normalized = '\' }
    return $normalized
}

function Resolve-ReparseTarget {
    <# Follow junction/symlink reparse targets (bounded) so alternate views are unmasked. #>
    param([Parameter(Mandatory)][string]$Path)
    $current = $Path
    for ($hop = 0; $hop -lt 16; $hop++) {
        $item = Get-Item -LiteralPath $current -ErrorAction SilentlyContinue
        if ($null -eq $item) { return $current }
        $target = $item.Target
        if (-not $target) { return $current }
        $first = @($target)[0]
        if ([string]::IsNullOrEmpty($first)) { return $current }
        $next = if ([System.IO.Path]::IsPathRooted($first)) { $first }
                else { Join-Path -Path (Split-Path -Parent $current) -ChildPath $first }
        if ([string]::Equals($next, $current, [StringComparison]::OrdinalIgnoreCase)) { return $current }
        $current = $next
    }
    return $current
}

function Get-CanonicalRepoRoot {
    <# Repository root derived from this script's location, with reparse views resolved. #>
    if ([string]::IsNullOrEmpty($PSScriptRoot)) {
        throw 'cannot determine repository root: Invoke-CI.ps1 must be executed or dot-sourced from its file (PSScriptRoot is empty)'
    }
    return (Resolve-ReparseTarget -Path (Split-Path -Parent $PSScriptRoot))
}

function Test-CanonicalWorkingDirectory {
    <# Req 7.5: the canonical repository path must be used as cwd; the C: junction
       view (and any other alternate/WSL view) is rejected so cache keys and
       BuildArtifacts scoping stay stable. Returns @{ Ok; Reason }. #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$CurrentDirectory,
        [Parameter(Mandatory)][string]$CanonicalRoot
    )
    $cwd = ConvertTo-DriftNormalizedPath -Path $CurrentDirectory
    $root = ConvertTo-DriftNormalizedPath -Path $CanonicalRoot
    if ($cwd.StartsWith('\\wsl$\', [StringComparison]::OrdinalIgnoreCase) -or
        $cwd.StartsWith('\\wsl.localhost\', [StringComparison]::OrdinalIgnoreCase)) {
        return [pscustomobject]@{
            Ok = $false
            Reason = "working directory '$CurrentDirectory' is a WSL filesystem view; use the canonical Windows repository path '$CanonicalRoot' with native tooling (Req 7.5)"
        }
    }
    $isEqual = [string]::Equals($cwd, $root, [StringComparison]::OrdinalIgnoreCase)
    $isUnder = $cwd.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)
    if ($isEqual -or $isUnder) {
        return [pscustomobject]@{ Ok = $true; Reason = '' }
    }
    return [pscustomobject]@{
        Ok = $false
        Reason = "working directory '$CurrentDirectory' is not the canonical repository path '$CanonicalRoot'; junction or alternate views (for example C:\Users\mariu\Source\drift-rdp) are rejected so cache keys and BuildArtifacts scoping stay stable (Req 7.5)"
    }
}

function Get-DriftProhibitedDelegationPatterns {
    <# Text patterns that indicate a script shim delegates to a prohibited
       interpreter. PowerShell -match is case-insensitive by default. #>
    return @(
        '\bwsl(\.exe)?\b'
        '\bbash(\.exe)?\b'
        '\\\\wsl\$'
        '\\\\wsl\.localhost'
        '/usr/bin/'
    )
}

function Test-DriftProhibitedPathRoot {
    <# String-only path checks (no filesystem access needed): WSL filesystem
       views and the MSYS/Cygwin layers (including Git for Windows' usr\bin). #>
    param([Parameter(Mandatory)][string]$Path)
    $lowered = $Path.Replace('/', '\').ToLowerInvariant()
    if ($lowered.StartsWith('\\wsl$\') -or $lowered.StartsWith('\\wsl.localhost\')) {
        return @{ Prohibited = $true
                  Reason = "path '$Path' is a WSL filesystem view; WSL may only be reached through native container clients (Req 7.5)" }
    }
    $segments = @($lowered -split '\\')
    for ($i = 0; $i -lt $segments.Count; $i++) {
        $segment = $segments[$i]
        if ($segment -in @('msys', 'msys64', 'cygwin', 'cygwin64')) {
            return @{ Prohibited = $true
                      Reason = "path '$Path' is inside an MSYS/Cygwin layer; repository commands must not delegate to Git Bash/MSYS (Req 7.5)" }
        }
        if ($segment -eq 'bin' -and $i -ge 2 -and $segments[$i - 1] -eq 'usr' -and $segments[$i - 2] -eq 'git') {
            return @{ Prohibited = $true
                      Reason = "path '$Path' is inside the Git for Windows MSYS layer (Git\usr\bin); use the native executable instead (Req 7.5)" }
        }
    }
    return @{ Prohibited = $false; Reason = '' }
}

function Test-DriftProhibitedInterpreterLeaf {
    <# A command path that IS a prohibited interpreter is rejected outright. #>
    param([Parameter(Mandatory)][string]$Path)
    $leaf = (Split-Path -Leaf ($Path.Replace('/', '\'))).ToLowerInvariant()
    if ($leaf -in @('wsl.exe', 'wsl', 'wslg.exe', 'bash.exe', 'bash', 'sh.exe', 'sh')) {
        return "path '$Path' is itself a prohibited interpreter ($leaf); repository commands must run through native executables (Req 7.5)"
    }
    return ''
}

function Test-ProhibitedWrapperTarget {
    <# Inspect a resolved command path BEFORE any execution. Order:
       1. resolve reparse/symlink chain (junction views, alternate roots);
       2. string path checks (WSL views, MSYS/Cygwin layers) on original + resolved;
       3. prohibited interpreter leaf names;
       4. textual delegation patterns inside script shims (bounded size).
       Returns @{ Prohibited; Reason; ResolvedPath }. #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$CommandPath)
    $resolvedPath = Resolve-ReparseTarget -Path $CommandPath
    foreach ($candidate in @($CommandPath, $resolvedPath)) {
        $rootCheck = Test-DriftProhibitedPathRoot -Path $candidate
        if ($rootCheck.Prohibited) {
            return [pscustomobject]@{ Prohibited = $true; Reason = $rootCheck.Reason; ResolvedPath = $resolvedPath }
        }
        $leafReason = Test-DriftProhibitedInterpreterLeaf -Path $candidate
        if ($leafReason) {
            return [pscustomobject]@{ Prohibited = $true; Reason = $leafReason; ResolvedPath = $resolvedPath }
        }
    }
    $extension = [System.IO.Path]::GetExtension($resolvedPath).ToLowerInvariant()
    if ($extension -in @('.cmd', '.bat', '.ps1', '.psm1', '.sh', '.py')) {
        $content = $null
        if (Test-Path -LiteralPath $resolvedPath -PathType Leaf) {
            $file = Get-Item -LiteralPath $resolvedPath
            if ($file.Length -le 1MB) {
                $content = Get-Content -LiteralPath $resolvedPath -Raw -ErrorAction SilentlyContinue
            }
        }
        if ($content) {
            foreach ($pattern in (Get-DriftProhibitedDelegationPatterns)) {
                if ($content -match $pattern) {
                    return [pscustomobject]@{
                        Prohibited = $true
                        Reason = "wrapper '$resolvedPath' delegates to a prohibited interpreter (matched '$pattern'); repository commands must not run through WSL/Git Bash (Req 7.5)"
                        ResolvedPath = $resolvedPath
                    }
                }
            }
        }
    }
    return [pscustomobject]@{ Prohibited = $false; Reason = ''; ResolvedPath = $resolvedPath }
}

function Get-ScopedOutputPath {
    <# Req 12.7: isolated outputs live under D:\BuildArtifacts\<project>\<subdir>. #>
    param(
        [string]$BuildArtifactsRoot = 'D:\BuildArtifacts',
        [string]$ProjectName = 'drift-rdp',
        [string]$Subdir = 'preflight'
    )
    return (Join-Path -Path (Join-Path -Path $BuildArtifactsRoot -ChildPath $ProjectName) -ChildPath $Subdir)
}

function Assert-ScopedOutputPath {
    <# Throws when a path would escape the scoped output root or sit at a
       drive/scope root (outputs must be scoped deeper than the project dir). #>
    param(
        [Parameter(Mandatory)][string]$Path,
        [string]$BuildArtifactsRoot = 'D:\BuildArtifacts',
        [string]$ProjectName = 'drift-rdp'
    )
    $normalizedPath = ConvertTo-DriftNormalizedPath -Path $Path
    $scopeRoot = ConvertTo-DriftNormalizedPath -Path $BuildArtifactsRoot
    $projectScope = ConvertTo-DriftNormalizedPath -Path (Join-Path -Path $BuildArtifactsRoot -ChildPath $ProjectName)
    $requiredPrefix = $projectScope + '\'
    if ($normalizedPath -match '^[a-zA-Z]:\\?$') {
        throw [System.InvalidOperationException]::new("output path '$Path' is a drive root; isolated outputs must be scoped under '$requiredPrefix' (never a drive root, Req 12.7)")
    }
    if ([string]::Equals($normalizedPath, $scopeRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw [System.InvalidOperationException]::new("output path '$Path' equals the BuildArtifacts root '$scopeRoot'; scope it under '$requiredPrefix'")
    }
    if ([string]::Equals($normalizedPath, $projectScope, [StringComparison]::OrdinalIgnoreCase)) {
        throw [System.InvalidOperationException]::new("output path '$Path' equals the project scope root '$projectScope'; scope it deeper ('$projectScope\<tool-or-purpose>\...')")
    }
    if (-not $normalizedPath.StartsWith($requiredPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw [System.InvalidOperationException]::new("output path '$Path' is outside the scoped output directory '$requiredPrefix'; Windows outputs belong under D:/BuildArtifacts/$ProjectName (Req 12.7)")
    }
}

function Get-DriftNextestInstallHint {
    <# Actionable install command for the pinned developer executable. #>
    param([string]$MinimumVersion = $DriftNextestMinVersion)
    return "install the pinned developer executable with: cargo install cargo-nextest --locked --version $MinimumVersion (cargo-nextest is CI/developer tooling, never an application dependency)"
}

function Test-DriftNextestVersion {
    <# Parse `cargo-nextest --version` output and compare it to the pin.
       Returns @{ Ok; Version; Reason }. #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string]$VersionOutput,
        [string]$MinimumVersion = $DriftNextestMinVersion
    )
    if ($VersionOutput -match 'cargo-nextest\s+(\d+)\.(\d+)\.(\d+)') {
        $installed = [version]::new([int]$Matches[1], [int]$Matches[2], [int]$Matches[3])
        $required = [version]$MinimumVersion
        if ($installed -ge $required) {
            return [pscustomobject]@{ Ok = $true; Version = $installed.ToString(); Reason = '' }
        }
        return [pscustomobject]@{
            Ok = $false
            Version = $installed.ToString()
            Reason = "cargo-nextest $installed is older than the pinned minimum $MinimumVersion; $(Get-DriftNextestInstallHint -MinimumVersion $MinimumVersion)"
        }
    }
    return [pscustomobject]@{
        Ok = $false
        Version = ''
        Reason = "could not parse a cargo-nextest version from: $($VersionOutput.Trim()); $(Get-DriftNextestInstallHint -MinimumVersion $MinimumVersion)"
    }
}

function New-DriftNextestRunArguments {
    <# Deterministic nextest argument list (for the cargo-nextest binary,
       cargo's subcommand convention: argv starts with the literal 'nextest'):
       pinned repository config, CI profile, explicit manifest,
       fail-on-zero-tests. Deliberately NO retry flag: retries stay 0 (absent)
       so flaky tests fail the tier (task 1.2). #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$ConfigPath,
        [Parameter(Mandatory)][string]$ManifestPath,
        [string]$Profile = 'ci'
    )
    return @('nextest', 'run', '--config-file', $ConfigPath, '--profile', $Profile, '--manifest-path', $ManifestPath, '--no-tests', 'fail')
}

function Get-DriftCiPins {
    <# Load and validate .config/ci-versions.json (task 1.4): the canonical
       version/config file distinguishing the pinned container images and
       engine from the native toolchain. Throws an actionable error when the
       file is missing or a required pin is absent (never silently floats). #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$PinsPath)
    if (-not (Test-Path -LiteralPath $PinsPath -PathType Leaf)) {
        throw [System.InvalidOperationException]::new("CI version pins not found at '$PinsPath' (task 1.4 contract: .config/ci-versions.json must pin dagger CLI+engine, node and rust images, nextest)")
    }
    try {
        $pins = Get-Content -LiteralPath $PinsPath -Raw | ConvertFrom-Json
    } catch {
        throw [System.InvalidOperationException]::new("CI version pins at '$PinsPath' are not valid JSON: $($_.Exception.Message)")
    }
    $required = @()
    try {
        $required = @(
            @{ Node = 'dagger.cliVersion';      Value = $pins.dagger.cliVersion },
            @{ Node = 'dagger.engineImage';     Value = $pins.dagger.engineImage },
            @{ Node = 'dagger.runnerHostScheme'; Value = $pins.dagger.runnerHostScheme },
            @{ Node = 'node.image';             Value = $pins.node.image },
            @{ Node = 'rust.image';             Value = $pins.rust.image },
            @{ Node = 'rust.toolchain';         Value = $pins.rust.toolchain },
            @{ Node = 'nextest.version';        Value = $pins.nextest.version }
        )
    } catch {
        throw [System.InvalidOperationException]::new("CI version pins at '$PinsPath' are missing a required section (expected dagger, node, rust, nextest): $($_.Exception.Message)")
    }
    foreach ($pin in $required) {
        if ([string]::IsNullOrEmpty([string]$pin.Value)) {
            throw [System.InvalidOperationException]::new("CI version pins at '$PinsPath' are missing the pinned value for '$($pin.Node)'")
        }
    }
    foreach ($imagePin in @($pins.dagger.engineImage, $pins.node.image, $pins.rust.image)) {
        if ($imagePin -match 'latest') {
            throw [System.InvalidOperationException]::new("CI version pins at '$PinsPath' contain a floating ':latest' reference ('$imagePin'); container images must be pinned to exact tags")
        }
    }
    return $pins
}

function Get-DriftFrontendTierCommands {
    <# Ordered frontend tier command list (task 1.4; Req 7.1): install, lint,
       typecheck, unit tests, build. This is the single source mirrored by
       the Dagger `frontend` container entry (.dagger/src/index.ts) — the
       same portable tier commands for container and native jobs. #>
    return @(
        'npm ci'
        'npm run lint'
        'npx tsc --noEmit'
        'npx vitest run'
        'npm run build'
    )
}

function Get-DriftDaggerCallName {
    <# Map a contract entry name to its actual `dagger call` function name.
       The dagger v0.21.11 TypeScript SDK kebab-cases method names AND splits
       digit/letter boundaries, so the linuxE2e method registers as the
       callable 'linux-e-2-e'. The contract name stays 'linux-e2e' (tasks.md,
       .dagger/README.md); this is the only translation point. Mirrors
       daggerCallNames in .dagger/src/pins.ts. #>
    [CmdletBinding()]
    param([Parameter(Mandatory)][ValidateSet('frontend', 'rust-checks', 'linux-build', 'linux-e2e', 'contract')][string]$Entry)
    $map = @{
        'frontend'    = 'frontend'
        'rust-checks' = 'rust-checks'
        'linux-build' = 'linux-build'
        'linux-e2e'   = 'linux-e-2-e'
        'contract'    = 'contract'
    }
    return $map[$Entry]
}

function Resolve-DriftDaggerCli {
    <# Resolve the REAL dagger CLI executable (task 1.4): never a personal
       launcher/wrapper such as a .cmd that re-invokes pwsh. Order: explicit
       path, native dagger.exe on PATH, then the version-pinned install
       convention %LOCALAPPDATA%\Programs\Dagger\<cliVersion>\dagger.exe
       derived from .config/ci-versions.json. Returns $null when not found. #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$PinsPath,
        [string]$ExplicitPath = ''
    )
    if (-not [string]::IsNullOrWhiteSpace($ExplicitPath)) {
        if (Test-Path -LiteralPath $ExplicitPath -PathType Leaf) { return $ExplicitPath }
        return $null
    }
    $onPath = Get-Command -Name 'dagger.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($onPath) { return $onPath.Source }
    try { $pins = Get-DriftCiPins -PinsPath $PinsPath } catch { return $null }
    $version = [string]$pins.dagger.cliVersion
    if ([string]::IsNullOrWhiteSpace($version)) { return $null }
    $conventional = Join-Path $env:LOCALAPPDATA (Join-Path 'Programs\Dagger' (Join-Path $version 'dagger.exe'))
    if (Test-Path -LiteralPath $conventional -PathType Leaf) { return $conventional }
    return $null
}

function Invoke-RustTest {
    <# Rust test tier (task 1.2): pinned cargo-nextest run plus cargo test
       --doc (nextest does not run doctests), with failures propagated to the
       tier exit code (Req 7.1, 7.3, 13.3). Returns an exit code (0 pass,
       1 rejected/failed); human output goes to the console, evidence under
       D:\BuildArtifacts\<project>\rust. #>
    [CmdletBinding()]
    param(
        [string]$Tier = 'rust',
        [string]$WorkingDirectory = '',
        [string]$ManifestPath = '',
        [string]$BuildArtifactsRoot = 'D:\BuildArtifacts',
        [string]$ProjectName = 'drift-rdp',
        [string]$NextestProfile = 'ci',
        [string]$ReportSubdir = 'rust',
        [string]$ReportName = 'rust-tier-report.json',
        [switch]$NoReport
    )
    $ErrorActionPreference = 'Stop'
    $totalStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    if ([string]::IsNullOrWhiteSpace($WorkingDirectory)) { $WorkingDirectory = (Get-Location).ProviderPath }
    $canonicalRoot = Get-CanonicalRepoRoot
    if ([string]::IsNullOrWhiteSpace($ManifestPath)) { $ManifestPath = Join-Path $canonicalRoot 'src-tauri\Cargo.toml' }
    $failures = [System.Collections.Generic.List[string]]::new()

    [Console]::WriteLine("== $ProjectName rust test tier ($Tier) ==")
    [Console]::WriteLine("canonical repository root: $canonicalRoot")
    [Console]::WriteLine("effective working directory: $WorkingDirectory")
    [Console]::WriteLine("manifest under test: $ManifestPath")

    # 1) canonical working directory: reject the C: junction view (Req 7.5)
    $cwdCheck = Test-CanonicalWorkingDirectory -CurrentDirectory $WorkingDirectory -CanonicalRoot $canonicalRoot
    if ($cwdCheck.Ok) {
        [Console]::WriteLine('working directory: OK')
    } else {
        $failures.Add($cwdCheck.Reason)
        [Console]::Error.WriteLine("working directory: REJECTED — $($cwdCheck.Reason)")
    }

    # 2) pinned nextest configuration (design.md file plan: .config/nextest.toml)
    $nextestConfigPath = Join-Path $canonicalRoot '.config\nextest.toml'
    if (Test-Path -LiteralPath $nextestConfigPath -PathType Leaf) {
        [Console]::WriteLine("nextest config: $nextestConfigPath (profile '$NextestProfile')")
    } else {
        $failures.Add("pinned nextest configuration not found at '$nextestConfigPath' (design.md file plan, task 1.2)")
        [Console]::Error.WriteLine("nextest config: MISSING — $nextestConfigPath")
    }

    # 3) manifest under test
    if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
        $failures.Add("cargo manifest not found: $ManifestPath")
        [Console]::Error.WriteLine("manifest: MISSING — $ManifestPath")
    }

    # 4) native tools, inspected BEFORE execution (Req 7.5): cargo + cargo-nextest
    $cargoCommand = Get-Command -Name cargo -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    $nextestCommand = Get-Command -Name cargo-nextest -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    $cargoRow = [pscustomobject]@{ tool = 'cargo'; path = ''; version = ''; status = 'missing' }
    $nextestRow = [pscustomobject]@{ tool = 'cargo-nextest'; path = ''; version = ''; status = 'missing' }
    if (-not $cargoCommand) {
        $failures.Add("required native tool 'cargo' was not found on PATH; the rust tier needs the native Rust toolchain (MSVC target on Windows)")
    } else {
        $cargoRow.path = $cargoCommand.Source
        $wrapper = Test-ProhibitedWrapperTarget -CommandPath $cargoCommand.Source
        if ($wrapper.Prohibited) {
            $failures.Add("required tool 'cargo' resolves to '$($cargoCommand.Source)' — prohibited wrapper rejected: $($wrapper.Reason)")
            $cargoRow.status = 'prohibited'
        } else {
            $cargoRow.status = 'ok'
        }
    }
    if (-not $nextestCommand) {
        $failures.Add("required tool 'cargo-nextest' was not found on PATH; $(Get-DriftNextestInstallHint)")
    } else {
        $nextestRow.path = $nextestCommand.Source
        $wrapper = Test-ProhibitedWrapperTarget -CommandPath $nextestCommand.Source
        if ($wrapper.Prohibited) {
            $failures.Add("required tool 'cargo-nextest' resolves to '$($nextestCommand.Source)' — prohibited wrapper rejected: $($wrapper.Reason)")
            $nextestRow.status = 'prohibited'
        } else {
            $nextestRow.status = 'ok'
        }
    }

    # 5) pinned-version gate (tasks.md: pin a supported release, >= 0.9.150)
    if ($nextestRow.status -eq 'ok') {
        try { $versionOutput = [string](& $nextestCommand.Source --version 2>&1 | Out-String) } catch { $versionOutput = '' }
        $versionCheck = Test-DriftNextestVersion -VersionOutput $versionOutput
        if ($versionCheck.Ok) {
            $nextestRow.version = $versionCheck.Version
        } else {
            $nextestRow.status = 'wrong-version'
            $failures.Add($versionCheck.Reason)
        }
    }
    if ($cargoRow.status -eq 'ok') {
        try { $cargoRow.version = ([string](& $cargoCommand.Source --version 2>&1 | Select-Object -First 1)).Trim() } catch { $cargoRow.version = '' }
    }
    [Console]::WriteLine(($(@($cargoRow, $nextestRow) | Format-Table -AutoSize | Out-String -Width 4096).TrimEnd()))

    # 6) run the tiers (only when every gate above passed)
    $nextestResult = $null
    $doctestResult = $null
    $junitCopied = [System.Collections.Generic.List[string]]::new()
    $reportDir = Get-ScopedOutputPath -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName -Subdir $ReportSubdir

    if ($failures.Count -eq 0) {
        Assert-ScopedOutputPath -Path $reportDir -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName
        New-Item -ItemType Directory -Path $reportDir -Force | Out-Null

        # 6a) cargo nextest run (unit + integration tests; NOT doctests)
        $nextestArgs = New-DriftNextestRunArguments -ConfigPath $nextestConfigPath -ManifestPath $ManifestPath -Profile $NextestProfile
        [Console]::WriteLine("nextest: cargo-nextest $($nextestArgs -join ' ')")
        $stepStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
        $nextestOutputLines = @(& $nextestCommand.Source @nextestArgs 2>&1 | ForEach-Object { "$_" })
        $nextestExit = $LASTEXITCODE
        $stepStopwatch.Stop()
        $nextestOutputLines | Set-Content -LiteralPath (Join-Path $reportDir 'nextest-output.txt') -Encoding utf8
        $nextestSummary = [string](@($nextestOutputLines | Where-Object { $_ -match '^\s*Summary' } | Select-Object -Last 1))
        $nextestResult = [pscustomobject]@{
            exitCode        = $nextestExit
            durationSeconds = [Math]::Round($stepStopwatch.Elapsed.TotalSeconds, 2)
            summary         = $nextestSummary
        }
        if ($nextestExit -eq 0) {
            [Console]::WriteLine("nextest: exit 0 ($($nextestResult.durationSeconds)s) — $nextestSummary")
        } else {
            $failures.Add("cargo nextest run failed with exit code $nextestExit (failures propagate; automatic retries are not configured, task 1.2)")
            if ($nextestExit -eq 4) { $failures.Add('nextest executed zero tests (fail-on-zero-tests via --no-tests fail)') }
            [Console]::Error.WriteLine("nextest: exit $nextestExit — $nextestSummary")
            [Console]::Error.WriteLine(($nextestOutputLines | Select-Object -Last 15) -join [Environment]::NewLine)
        }

        # 6b) doctests: nextest does not run doctests, so keep cargo test --doc
        #     as a separate step whose failures propagate (Req 7.1).
        if ($nextestExit -eq 0) {
            $stepStopwatch.Restart()
            $doctestLines = @(& $cargoCommand.Source test --doc --manifest-path $ManifestPath 2>&1 | ForEach-Object { "$_" })
            $doctestExit = $LASTEXITCODE
            $stepStopwatch.Stop()
            $doctestLines | Set-Content -LiteralPath (Join-Path $reportDir 'doctest-output.txt') -Encoding utf8
            $doctestResultLine = [string](@($doctestLines | Where-Object { $_ -match '^test result:' } | Select-Object -Last 1))
            $doctestResult = [pscustomobject]@{
                exitCode        = $doctestExit
                durationSeconds = [Math]::Round($stepStopwatch.Elapsed.TotalSeconds, 2)
                result          = $doctestResultLine
            }
            if ($doctestExit -eq 0) {
                [Console]::WriteLine("doctests: cargo test --doc exit 0 ($($doctestResult.durationSeconds)s) — $doctestResultLine")
            } else {
                $failures.Add("cargo test --doc failed with exit code $doctestExit (doctest failures propagate, Req 7.1)")
                [Console]::Error.WriteLine("doctests: exit $doctestExit — $doctestResultLine")
                [Console]::Error.WriteLine(($doctestLines | Select-Object -Last 15) -join [Environment]::NewLine)
            }
        } else {
            $doctestResult = [pscustomobject]@{
                exitCode        = $null
                durationSeconds = $null
                result          = 'skipped: nextest step failed'
            }
        }

        # 6c) copy JUnit evidence from <workspace>\target\nextest\<profile> into
        #     the scoped output directory (Req 12.7; nextest writes relative
        #     junit paths under the workspace target directory).
        $junitSourceDir = Join-Path (Split-Path -Parent $ManifestPath) ("target\nextest\$NextestProfile")
        if (Test-Path -LiteralPath $junitSourceDir -PathType Container) {
            foreach ($xmlFile in @(Get-ChildItem -LiteralPath $junitSourceDir -Filter '*.xml' -File -ErrorAction SilentlyContinue)) {
                Copy-Item -LiteralPath $xmlFile.FullName -Destination (Join-Path $reportDir $xmlFile.Name) -Force
                $junitCopied.Add((Join-Path $reportDir $xmlFile.Name))
            }
        }
        if ($junitCopied.Count -gt 0) {
            [Console]::WriteLine("junit evidence: $($junitCopied -join ', ')")
        } else {
            [Console]::WriteLine('junit evidence: none produced by this run (for example, the zero-tests error path)')
        }
    }

    # 7) scoped machine-readable report (Req 12.7): versions, exit codes and
    #    durations stay observable so Windows overhead is measured, not assumed.
    $totalStopwatch.Stop()
    $ok = $failures.Count -eq 0
    if (-not $NoReport) {
        try {
            Assert-ScopedOutputPath -Path $reportDir -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName
            New-Item -ItemType Directory -Path $reportDir -Force | Out-Null
            $report = [pscustomobject]@{
                generatedUtc         = (Get-Date).ToUniversalTime().ToString('o')
                tier                 = $Tier
                ok                   = $ok
                canonicalRoot        = $canonicalRoot
                workingDirectory     = $WorkingDirectory
                buildArtifactsRoot   = $BuildArtifactsRoot
                manifestPath         = $ManifestPath
                nextestProfile       = $NextestProfile
                nextestPin           = $DriftNextestMinVersion
                tools                = @($cargoRow, $nextestRow)
                nextest              = $nextestResult
                doctest              = $doctestResult
                artifacts            = [pscustomobject]@{
                    reportDir  = $reportDir
                    junitCopied = @($junitCopied)
                    nextestLog = (Join-Path $reportDir 'nextest-output.txt')
                    doctestLog = (Join-Path $reportDir 'doctest-output.txt')
                }
                failures             = $failures
                totalDurationSeconds = [Math]::Round($totalStopwatch.Elapsed.TotalSeconds, 2)
            }
            $reportPath = Join-Path -Path $reportDir -ChildPath $ReportName
            $report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $reportPath -Encoding utf8
            [Console]::WriteLine("report: $reportPath")
        } catch {
            $ok = $false
            $failures.Add("scoped report output rejected: $($_.Exception.Message)")
            [Console]::Error.WriteLine("scoped report output rejected: $($_.Exception.Message)")
        }
    }

    if ($ok) {
        [Console]::WriteLine('[PASS] rust test tier passed (nextest + doctests)')
        return 0
    }
    [Console]::Error.WriteLine('[FAIL] rust test tier rejected:')
    foreach ($failure in $failures) { [Console]::Error.WriteLine(" - $failure") }
    return 1
}

function Invoke-FrontendTest {
    <# Frontend test tier (task 1.4; Req 7.1, 13.3, 13.4): the native twin
       of the Dagger `frontend` Linux-container entry — same portable command
       list (Get-DriftFrontendTierCommands), executed with native node/npm on
       Windows. Wrapper targets are inspected BEFORE execution (Req 7.5);
       failures propagate to the tier exit code; evidence lands under
       D:\BuildArtifacts\<project>\frontend. Returns 0 pass / 1 rejected. #>
    [CmdletBinding()]
    param(
        [string]$Tier = 'frontend',
        [string]$WorkingDirectory = '',
        [string]$BuildArtifactsRoot = 'D:\BuildArtifacts',
        [string]$ProjectName = 'drift-rdp',
        [string]$ReportSubdir = 'frontend',
        [string]$ReportName = 'frontend-tier-report.json',
        [switch]$NoReport
    )
    $ErrorActionPreference = 'Stop'
    $totalStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    if ([string]::IsNullOrWhiteSpace($WorkingDirectory)) { $WorkingDirectory = (Get-Location).ProviderPath }
    $canonicalRoot = Get-CanonicalRepoRoot
    $failures = [System.Collections.Generic.List[string]]::new()

    # The step list mirrors Get-DriftFrontendTierCommands one-for-one: tool,
    # arguments and the display string that is asserted by the contract tests.
    $steps = @(
        @{ Tool = 'npm';  Args = @('ci');                             Display = 'npm ci' },
        @{ Tool = 'npm';  Args = @('run', 'lint');                    Display = 'npm run lint' },
        @{ Tool = 'npx';  Args = @('tsc', '--noEmit');                Display = 'npx tsc --noEmit' },
        @{ Tool = 'npx';  Args = @('vitest', 'run');                  Display = 'npx vitest run' },
        @{ Tool = 'npm';  Args = @('run', 'build');                   Display = 'npm run build' }
    )

    [Console]::WriteLine("== $ProjectName frontend test tier ($Tier) ==")
    [Console]::WriteLine("canonical repository root: $canonicalRoot")
    [Console]::WriteLine("effective working directory: $WorkingDirectory")
    [Console]::WriteLine("command list: $((Get-DriftFrontendTierCommands) -join ' -> ')")

    # 1) canonical working directory: reject the C: junction view (Req 7.5)
    $cwdCheck = Test-CanonicalWorkingDirectory -CurrentDirectory $WorkingDirectory -CanonicalRoot $canonicalRoot
    if ($cwdCheck.Ok) {
        [Console]::WriteLine('working directory: OK')
    } else {
        $failures.Add($cwdCheck.Reason)
        [Console]::Error.WriteLine("working directory: REJECTED — $($cwdCheck.Reason)")
    }

    # 2) native tools, inspected BEFORE execution (Req 7.5): node, npm, npx
    $toolRows = [System.Collections.Generic.List[object]]::new()
    $resolvedTools = @{}
    foreach ($tool in @('node', 'npm', 'npx')) {
        $command = Get-Command -Name $tool -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if (-not $command) {
            $failures.Add("required native tool '$tool' was not found on PATH; the frontend tier needs the native Node.js toolchain (npm runs natively on Windows)")
            $toolRows.Add([pscustomobject]@{ tool = $tool; path = ''; version = ''; status = 'missing' })
            continue
        }
        $wrapper = Test-ProhibitedWrapperTarget -CommandPath $command.Source
        if ($wrapper.Prohibited) {
            $failures.Add("required tool '$tool' resolves to '$($command.Source)' — prohibited wrapper rejected: $($wrapper.Reason)")
            $toolRows.Add([pscustomobject]@{ tool = $tool; path = $command.Source; version = ''; status = 'prohibited' })
            continue
        }
        $resolvedTools[$tool] = $command.Source
        $version = ''
        if ($tool -eq 'node') {
            try { $version = ([string](& $command.Source --version 2>&1 | Select-Object -First 1)).Trim() } catch { $version = '' }
        }
        $toolRows.Add([pscustomobject]@{ tool = $tool; path = $command.Source; version = $version; status = 'ok' })
    }
    [Console]::WriteLine(($($toolRows | Format-Table -AutoSize | Out-String -Width 4096).TrimEnd()))

    # 3) pinned version/config files (task 1.4 contract files must exist)
    $pinsPath = Join-Path $canonicalRoot '.config\ci-versions.json'
    if (Test-Path -LiteralPath $pinsPath -PathType Leaf) {
        [Console]::WriteLine("ci pins: $pinsPath")
    } else {
        $failures.Add("CI version pins not found at '$pinsPath' (task 1.4 contract)")
        [Console]::Error.WriteLine("ci pins: MISSING — $pinsPath")
    }

    # 4) run the steps (only when every gate above passed)
    $stepResults = [System.Collections.Generic.List[object]]::new()
    $reportDir = Get-ScopedOutputPath -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName -Subdir $ReportSubdir
    if ($failures.Count -eq 0) {
        Assert-ScopedOutputPath -Path $reportDir -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName
        New-Item -ItemType Directory -Path $reportDir -Force | Out-Null
        $stepIndex = 0
        foreach ($step in $steps) {
            $stepIndex++
            [Console]::WriteLine("step $stepIndex/$($steps.Count): $($step.Display)")
            $stepStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
            Push-Location -LiteralPath $canonicalRoot
            try {
                $outputLines = @(& $resolvedTools[$step.Tool] @($step.Args) 2>&1 | ForEach-Object { "$_" })
                $stepExit = $LASTEXITCODE
            } finally {
                Pop-Location
            }
            $stepStopwatch.Stop()
            $safeName = ($step.Display -replace '[^a-zA-Z0-9]+', '-')
            $outputLines | Set-Content -LiteralPath (Join-Path $reportDir "step$stepIndex-$safeName.txt") -Encoding utf8
            $stepResults.Add([pscustomobject]@{
                step             = $step.Display
                exitCode         = $stepExit
                durationSeconds  = [Math]::Round($stepStopwatch.Elapsed.TotalSeconds, 2)
            })
            if ($stepExit -eq 0) {
                [Console]::WriteLine("step ${stepIndex}: exit 0 ($($stepResults[$stepResults.Count - 1].durationSeconds)s)")
            } else {
                $failures.Add("frontend step '$($step.Display)' failed with exit code $stepExit (failures propagate; Req 7.3, 13.3)")
                [Console]::Error.WriteLine("step ${stepIndex}: exit $stepExit — $($step.Display)")
                [Console]::Error.WriteLine(($outputLines | Select-Object -Last 15) -join [Environment]::NewLine)
                break
            }
        }
    }

    # 5) scoped machine-readable report (Req 12.7)
    $totalStopwatch.Stop()
    $ok = $failures.Count -eq 0
    if (-not $NoReport) {
        try {
            Assert-ScopedOutputPath -Path $reportDir -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName
            New-Item -ItemType Directory -Path $reportDir -Force | Out-Null
            $report = [pscustomobject]@{
                generatedUtc         = (Get-Date).ToUniversalTime().ToString('o')
                tier                 = $Tier
                ok                   = $ok
                canonicalRoot        = $canonicalRoot
                workingDirectory     = $WorkingDirectory
                buildArtifactsRoot   = $BuildArtifactsRoot
                commandList          = @(Get-DriftFrontendTierCommands)
                tools                = $toolRows
                steps                = $stepResults
                failures             = $failures
                totalDurationSeconds = [Math]::Round($totalStopwatch.Elapsed.TotalSeconds, 2)
            }
            $reportPath = Join-Path -Path $reportDir -ChildPath $ReportName
            $report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $reportPath -Encoding utf8
            [Console]::WriteLine("report: $reportPath")
        } catch {
            $ok = $false
            $failures.Add("scoped report output rejected: $($_.Exception.Message)")
            [Console]::Error.WriteLine("scoped report output rejected: $($_.Exception.Message)")
        }
    }

    if ($ok) {
        [Console]::WriteLine('[PASS] frontend test tier passed (install, lint, typecheck, unit tests, build)')
        return 0
    }
    [Console]::Error.WriteLine('[FAIL] frontend test tier rejected:')
    foreach ($failure in $failures) { [Console]::Error.WriteLine(" - $failure") }
    return 1
}

function Invoke-DaggerTier {
    <# Dagger tier (task 1.4; Req 7.1, 7.2, 8.4, 12.5): invokes a
       Linux-container entry point of the .dagger module. Engine wiring is
       done HERE, from the pinned .config/ci-versions.json — never by a
       personal launcher: _EXPERIMENTAL_DAGGER_RUNNER_HOST is set to
       image+podman://<pinned engine image> (task 1.3 finding), the REAL
       dagger.exe is resolved (Resolve-DriftDaggerCli), the native podman
       client is made reachable, and the dagger exit code propagates.
       Linux containers are not native Windows/macOS validation (design.md
       "CI ownership and reuse decisions"). Returns the dagger exit code, or
       1 when rejected before invocation. #>
    [CmdletBinding()]
    param(
        [string]$Tier = 'dagger',
        [Parameter(Mandatory)][ValidateSet('frontend', 'rust-checks', 'linux-build', 'linux-e2e', 'contract')][string]$DaggerEntry,
        [string]$DaggerCliPath = '',
        [string]$WorkingDirectory = '',
        [string]$BuildArtifactsRoot = 'D:\BuildArtifacts',
        [string]$ProjectName = 'drift-rdp',
        [string]$ReportSubdir = 'dagger',
        [string]$ReportName = 'dagger-tier-report.json',
        [switch]$NoReport
    )
    $ErrorActionPreference = 'Stop'
    $totalStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    if ([string]::IsNullOrWhiteSpace($WorkingDirectory)) { $WorkingDirectory = (Get-Location).ProviderPath }
    $canonicalRoot = Get-CanonicalRepoRoot
    $moduleDir = Join-Path $canonicalRoot '.dagger'
    $pinsPath = Join-Path $canonicalRoot '.config\ci-versions.json'
    $failures = [System.Collections.Generic.List[string]]::new()

    [Console]::WriteLine("== $ProjectName dagger tier ($Tier, entry '$DaggerEntry') ==")
    [Console]::WriteLine("canonical repository root: $canonicalRoot")
    [Console]::WriteLine("effective working directory: $WorkingDirectory")

    # 1) canonical working directory + contract files (Req 7.5, task 1.4)
    $cwdCheck = Test-CanonicalWorkingDirectory -CurrentDirectory $WorkingDirectory -CanonicalRoot $canonicalRoot
    if ($cwdCheck.Ok) { [Console]::WriteLine('working directory: OK') } else {
        $failures.Add($cwdCheck.Reason)
        [Console]::Error.WriteLine("working directory: REJECTED — $($cwdCheck.Reason)")
    }
    if (-not (Test-Path -LiteralPath (Join-Path $moduleDir 'dagger.json') -PathType Leaf)) {
        $failures.Add("Dagger module manifest not found at '$moduleDir\dagger.json' (design.md file plan reserves .dagger/)")
    }

    # 2) pinned versions (throws its own actionable error; treated as a failure)
    $pins = $null
    try {
        $pins = Get-DriftCiPins -PinsPath $pinsPath
        [Console]::WriteLine("pins: dagger CLI $($pins.dagger.cliVersion), engine $($pins.dagger.engineImage)")
    } catch {
        $failures.Add($_.Exception.Message)
        [Console]::Error.WriteLine("pins: REJECTED — $($_.Exception.Message)")
    }

    # 3) resolve the REAL dagger CLI (never a launcher) and inspect it first
    $daggerCli = Resolve-DriftDaggerCli -PinsPath $pinsPath -ExplicitPath $DaggerCliPath
    if (-not $daggerCli) {
        $cliVersionHint = if ($pins) { [string]$pins.dagger.cliVersion } else { '<pinned version from .config/ci-versions.json>' }
        $failures.Add("the real dagger CLI executable could not be resolved (explicit path, PATH, or %LOCALAPPDATA%\Programs\Dagger\$cliVersionHint\dagger.exe); a launcher wrapper is not accepted (task 1.4)")
    } else {
        $wrapper = Test-ProhibitedWrapperTarget -CommandPath $daggerCli
        if ($wrapper.Prohibited) {
            $failures.Add("dagger CLI at '$daggerCli' — prohibited wrapper rejected: $($wrapper.Reason)")
        } else {
            [Console]::WriteLine("dagger CLI: $daggerCli")
        }
    }

    # 4) native podman client reachable through its native executable only
    $podmanDir = Join-Path $env:ProgramFiles 'RedHat\Podman'
    $podmanPath = $null
    $podmanOnPath = Get-Command -Name 'podman.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($podmanOnPath) {
        $podmanPath = $podmanOnPath.Source
    } elseif (Test-Path -LiteralPath (Join-Path $podmanDir 'podman.exe') -PathType Leaf) {
        $podmanPath = Join-Path $podmanDir 'podman.exe'
    }
    if (-not $podmanPath) {
        $failures.Add("native podman client executable not found (the pinned image+podman engine requires the native client; WSL-backed engines are reached only through native clients, Req 7.5)")
    } else {
        [Console]::WriteLine("podman client: $podmanPath")
    }

    # 5) invoke: engine env set HERE from the pin (task 1.3 finding), then
    #    propagate the dagger exit code (including success).
    $daggerExit = $null
    $outputLines = @()
    $runnerHost = ''
    if ($pins) { $runnerHost = "$($pins.dagger.runnerHostScheme)://$($pins.dagger.engineImage)" }
    if ($failures.Count -eq 0) {
        $callName = Get-DriftDaggerCallName -Entry $DaggerEntry
        $daggerArgs = @('-m', $moduleDir, 'call', $callName)
        # Entries that consume repository source receive it as a --source
        # Directory argument resolved by the dagger CLI from the canonical
        # repository root (this SDK version has no host-directory access
        # inside module code).
        if ($callName -ne 'contract') { $daggerArgs += @('--source', $canonicalRoot) }
        [Console]::WriteLine("runner host: $runnerHost (set by this tier from the pin, not by a launcher)")
        [Console]::WriteLine("invoking: dagger $($daggerArgs -join ' ')")
        $oldPath = $env:PATH
        $oldRunner = $env:_EXPERIMENTAL_DAGGER_RUNNER_HOST
        $oldDark = $env:FORCE_DARK_MODE
        Push-Location -LiteralPath $canonicalRoot
        try {
            $env:PATH = "$(Split-Path -Parent $podmanPath);$oldPath"
            $env:_EXPERIMENTAL_DAGGER_RUNNER_HOST = $runnerHost
            # Noninteractive agents: avoid blocking terminal-theme detection.
            $env:FORCE_DARK_MODE = '1'
            $outputLines = @(& $daggerCli @daggerArgs 2>&1 | ForEach-Object { "$_" })
            $daggerExit = $LASTEXITCODE
        } finally {
            $env:PATH = $oldPath
            $env:_EXPERIMENTAL_DAGGER_RUNNER_HOST = $oldRunner
            $env:FORCE_DARK_MODE = $oldDark
            Pop-Location
        }
        if ($daggerExit -eq 0) {
            [Console]::WriteLine("dagger: exit 0")
            [Console]::WriteLine(($outputLines | Select-Object -Last 5) -join [Environment]::NewLine)
        } else {
            [Console]::Error.WriteLine("dagger: exit $daggerExit (entry '$DaggerEntry' failed; exit code propagated, Req 7.3, 13.3)")
            [Console]::Error.WriteLine(($outputLines | Select-Object -Last 20) -join [Environment]::NewLine)
        }
    }

    # 6) scoped machine-readable report (Req 12.7)
    $totalStopwatch.Stop()
    $ok = ($failures.Count -eq 0) -and ($daggerExit -eq 0)
    if (-not $NoReport) {
        try {
            $reportDir = Get-ScopedOutputPath -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName -Subdir $ReportSubdir
            Assert-ScopedOutputPath -Path $reportDir -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName
            New-Item -ItemType Directory -Path $reportDir -Force | Out-Null
            $report = [pscustomobject]@{
                generatedUtc         = (Get-Date).ToUniversalTime().ToString('o')
                tier                 = $Tier
                entry                = $DaggerEntry
                callName             = (Get-DriftDaggerCallName -Entry $DaggerEntry)
                ok                   = $ok
                canonicalRoot        = $canonicalRoot
                moduleDir            = $moduleDir
                daggerCli            = $daggerCli
                podmanClient         = $podmanPath
                runnerHost           = $runnerHost
                enginePin            = $(if ($pins) { $pins.dagger.engineImage } else { $null })
                daggerExitCode       = $daggerExit
                failures             = $failures
                totalDurationSeconds = [Math]::Round($totalStopwatch.Elapsed.TotalSeconds, 2)
            }
            $reportPath = Join-Path -Path $reportDir -ChildPath $ReportName
            $report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $reportPath -Encoding utf8
            $outputLines | Set-Content -LiteralPath (Join-Path $reportDir "entry-$((Get-DriftDaggerCallName -Entry $DaggerEntry))-output.txt") -Encoding utf8
            [Console]::WriteLine("report: $reportPath")
        } catch {
            $ok = $false
            $failures.Add("scoped report output rejected: $($_.Exception.Message)")
            [Console]::Error.WriteLine("scoped report output rejected: $($_.Exception.Message)")
        }
    }

    if ($failures.Count -gt 0) {
        [Console]::Error.WriteLine('[FAIL] dagger tier rejected:')
        foreach ($failure in $failures) { [Console]::Error.WriteLine(" - $failure") }
        return 1
    }
    if ($daggerExit -eq 0) { [Console]::WriteLine('[PASS] dagger tier passed') }
    return $daggerExit
}

function Invoke-Preflight {
    <# Preflight tier (task 1.1): canonical cwd, native tool versions/paths with
       wrapper inspection before execution, scoped report output. Returns an
       exit code (0 pass, 1 rejected); human output goes to the console. #>
    [CmdletBinding()]
    param(
        [string]$Tier = 'preflight',
        [string[]]$RequiredTools = @('pwsh', 'git', 'node', 'npm', 'cargo', 'rustc'),
        [string]$WorkingDirectory = '',
        [string]$BuildArtifactsRoot = 'D:\BuildArtifacts',
        [string]$ProjectName = 'drift-rdp',
        [string]$ReportSubdir = 'preflight',
        [string]$ReportName = 'preflight-report.json',
        [switch]$NoReport
    )
    $ErrorActionPreference = 'Stop'

    if ([string]::IsNullOrWhiteSpace($WorkingDirectory)) { $WorkingDirectory = (Get-Location).ProviderPath }
    $canonicalRoot = Get-CanonicalRepoRoot
    $failures = [System.Collections.Generic.List[string]]::new()

    [Console]::WriteLine("== $ProjectName native preflight ($Tier) ==")
    [Console]::WriteLine("canonical repository root: $canonicalRoot")
    [Console]::WriteLine("effective working directory: $WorkingDirectory")

    # 1) canonical working directory: reject the C: junction view (Req 7.5)
    $cwdCheck = Test-CanonicalWorkingDirectory -CurrentDirectory $WorkingDirectory -CanonicalRoot $canonicalRoot
    if ($cwdCheck.Ok) {
        [Console]::WriteLine('working directory: OK')
    } else {
        $failures.Add($cwdCheck.Reason)
        [Console]::Error.WriteLine("working directory: REJECTED — $($cwdCheck.Reason)")
    }

    # 2) required native tools: inspect wrapper target BEFORE execution (Req 7.1/7.5)
    $toolRows = [System.Collections.Generic.List[object]]::new()
    foreach ($tool in $RequiredTools) {
        $command = Get-Command -Name $tool -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if (-not $command) {
            $failures.Add("required native tool '$tool' was not found on PATH")
            $toolRows.Add([pscustomobject]@{ tool = $tool; path = ''; version = ''; status = 'missing' })
            continue
        }
        $wrapper = Test-ProhibitedWrapperTarget -CommandPath $command.Source
        if ($wrapper.Prohibited) {
            $failures.Add("required tool '$tool' resolves to '$($command.Source)' — prohibited wrapper rejected: $($wrapper.Reason)")
            $toolRows.Add([pscustomobject]@{ tool = $tool; path = $command.Source; version = ''; status = 'prohibited' })
            continue
        }
        $version = ''
        try {
            $version = [string](& $command.Source --version 2>&1 | Select-Object -First 1)
            $version = $version.Trim()
        } catch {
            $version = ''
        }
        if ([string]::IsNullOrEmpty($version)) {
            $failures.Add("tool '$tool' at '$($command.Source)' did not report a version")
        }
        $statusValue = if ($version) { 'ok' } else { 'no-version' }
        $toolRows.Add([pscustomobject]@{ tool = $tool; path = $command.Source; version = $version; status = $statusValue })
    }
    $table = $toolRows | Format-Table -AutoSize | Out-String -Width 4096
    [Console]::WriteLine($table.TrimEnd())

    # 3) scoped report output under D:\BuildArtifacts\<project>\... (Req 12.7)
    $ok = $failures.Count -eq 0
    if (-not $NoReport) {
        try {
            $reportDir = Get-ScopedOutputPath -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName -Subdir $ReportSubdir
            Assert-ScopedOutputPath -Path $reportDir -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName
            New-Item -ItemType Directory -Path $reportDir -Force | Out-Null
            $report = [pscustomobject]@{
                generatedUtc       = (Get-Date).ToUniversalTime().ToString('o')
                tier               = $Tier
                ok                 = $ok
                canonicalRoot      = $canonicalRoot
                workingDirectory   = $WorkingDirectory
                buildArtifactsRoot = $BuildArtifactsRoot
                tools              = $toolRows
                failures           = $failures
            }
            $reportPath = Join-Path -Path $reportDir -ChildPath $ReportName
            $report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $reportPath -Encoding utf8
            [Console]::WriteLine("report: $reportPath")
        } catch {
            $ok = $false
            $failures.Add("scoped report output rejected: $($_.Exception.Message)")
            [Console]::Error.WriteLine("scoped report output rejected: $($_.Exception.Message)")
        }
    }

    if ($ok) {
        [Console]::WriteLine('[PASS] native preflight passed')
        return 0
    }
    [Console]::Error.WriteLine('[FAIL] native preflight rejected:')
    foreach ($failure in $failures) { [Console]::Error.WriteLine(" - $failure") }
    return 1
}

# --- entry point (skip when dot-sourced, e.g. by Invoke-CI.Tests.ps1) ---
if ($MyInvocation.InvocationName -ne '.' -and $env:DRIFT_CI_IMPORT_ONLY -ne '1') {
    if ($Tier -eq 'preflight') {
        $preflightExitCode = Invoke-Preflight -Tier $Tier `
            -WorkingDirectory $WorkingDirectory `
            -BuildArtifactsRoot $BuildArtifactsRoot `
            -ProjectName $ProjectName `
            -NoReport:$NoReport
        exit $preflightExitCode
    }
    if ($Tier -eq 'rust') {
        $rustExitCode = Invoke-RustTest -Tier $Tier `
            -WorkingDirectory $WorkingDirectory `
            -ManifestPath $ManifestPath `
            -BuildArtifactsRoot $BuildArtifactsRoot `
            -ProjectName $ProjectName `
            -NoReport:$NoReport
        exit $rustExitCode
    }
    if ($Tier -eq 'frontend') {
        $frontendExitCode = Invoke-FrontendTest -Tier $Tier `
            -WorkingDirectory $WorkingDirectory `
            -BuildArtifactsRoot $BuildArtifactsRoot `
            -ProjectName $ProjectName `
            -NoReport:$NoReport
        exit $frontendExitCode
    }
    if ($Tier -eq 'dagger') {
        $daggerExitCode = Invoke-DaggerTier -Tier $Tier `
            -DaggerEntry $DaggerEntry `
            -DaggerCliPath $DaggerCliPath `
            -WorkingDirectory $WorkingDirectory `
            -BuildArtifactsRoot $BuildArtifactsRoot `
            -ProjectName $ProjectName `
            -NoReport:$NoReport
        exit $daggerExitCode
    }
}
