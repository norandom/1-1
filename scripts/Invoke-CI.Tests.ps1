#Requires -Version 7.0
<#
.SYNOPSIS
  Test tier for scripts/Invoke-CI.ps1 — native CI entry points (task 1.1
  preflight tier, task 1.2 rust test tier, task 1.4 container/native command
  contract).

.DESCRIPTION
  Single command (Req 13.4):  pwsh -NoProfile -File scripts/Invoke-CI.Tests.ps1

  The task 1.4 section (CT01-CT11) proves the one-command contract for
  container and native jobs: the .dagger TypeScript module manifest and
  entry points (frontend, rust-checks, linux-build, linux-e2e), the pinned
  versions/config file .config/ci-versions.json (no floating tags), the
  native frontend tier, and the dagger tier that wires the engine runner
  host itself instead of depending on a personal launcher shim. The dagger
  tier is exercised through recording/echo-only shims; no engine or
  container is required by this test tier (live engine proof is recorded
  separately in the task evidence).

  The task 2.2 section (CT12) proves the Linux bundle gate: the linux-build
  Dagger entry executes the repo's current Tauri packaging, exports
  AppImage/deb plus the build log into the scoped artifact directory, and
  MISSING ARTIFACTS FAIL THE TIER (Req 7.2, 7.3) — verified deterministically
  through exit-0 shims that do (or do not) place the expected artifacts; the
  live container bundle run and the corrupted-expectation gate demonstration
  are captured in the task evidence, not here.

  Lightweight, self-contained runner: plain pwsh assertions over exit codes,
  messages and produced files. No Pester or other new dependency frameworks
  (tasks.md effort controls). Disposables are created under
  D:\BuildArtifacts\Temp\opencode\preflight-tests and removed at the end;
  only this test's own resources are cleaned up (Req 12.7).

  Wrapper-rejection fixtures are echo-only: even if the preflight wrongly
  executed them, no WSL or bash process would ever be spawned.

  Rust-tier integration tests compile tiny throwaway fixture crates (under
  the same temp root, never committed) to prove that nextest failures,
  doctest failures and zero-test runs propagate to the tier exit code; the
  application crates in src-tauri/ and native-app/ are not modified.
#>
[CmdletBinding()]
param(
    [string]$BuildArtifactsRoot = 'D:\BuildArtifacts',
    [string]$ProjectName = 'drift-rdp',
    [string]$TempRoot = ''
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Continue'

$RepoRoot   = Split-Path -Parent $PSScriptRoot
$ScriptPath = Join-Path $PSScriptRoot 'Invoke-CI.ps1'
if ([string]::IsNullOrEmpty($TempRoot)) {
    $TempRoot = Join-Path -Path $BuildArtifactsRoot -ChildPath (Join-Path 'Temp' (Join-Path 'opencode' 'preflight-tests'))
}
$PwshExe = (Get-Process -Id $PID).Path

$script:Passed = 0
$script:Failed = 0
$script:FailedNames = [System.Collections.Generic.List[string]]::new()

function Assert-That {
    param(
        [Parameter(Mandatory)][scriptblock]$Condition,
        [Parameter(Mandatory)][string]$Name,
        [string]$Detail = ''
    )
    try {
        $ok = & $Condition
        if ($ok) {
            $script:Passed++
            Write-Output ("  PASS {0}" -f $Name)
        } else {
            $script:Failed++
            $script:FailedNames.Add($Name)
            Write-Output ("  FAIL {0} {1}" -f $Name, $Detail)
        }
    } catch {
        $script:Failed++
        $script:FailedNames.Add($Name)
        Write-Output ("  FAIL {0} :: {1}" -f $Name, $_.Exception.Message)
    }
}

function Assert-Throws {
    param(
        [Parameter(Mandatory)][scriptblock]$Action,
        [Parameter(Mandatory)][string]$Name,
        [string]$Match = ''
    )
    try {
        & $Action | Out-Null
        $script:Failed++
        $script:FailedNames.Add($Name)
        Write-Output ("  FAIL {0} :: did not throw" -f $Name)
    } catch {
        if ($Match -and ($_.Exception.Message -notmatch $Match)) {
            $script:Failed++
            $script:FailedNames.Add($Name)
            Write-Output ("  FAIL {0} :: message '{1}' missing '{2}'" -f $Name, $_.Exception.Message, $Match)
        } else {
            $script:Passed++
            Write-Output ("  PASS {0}" -f $Name)
        }
    }
}

function Invoke-ChildPreflight {
    param(
        [Parameter(Mandatory)][string[]]$Arguments,
        [hashtable]$Environment = @{}
    )
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $PwshExe
    foreach ($argument in $Arguments) { [void]$startInfo.ArgumentList.Add($argument) }
    $startInfo.WorkingDirectory = $RepoRoot
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($key in $Environment.Keys) { $startInfo.Environment[$key] = $Environment[$key] }
    $child = [System.Diagnostics.Process]::Start($startInfo)
    $stdoutTask = $child.StandardOutput.ReadToEndAsync()
    $stderrTask = $child.StandardError.ReadToEndAsync()
    if (-not $child.WaitForExit(300000)) {
        $child.Kill($true)
        throw 'child preflight process timed out after 300s'
    }
    return [pscustomobject]@{ ExitCode = $child.ExitCode; StdOut = $stdoutTask.Result; StdErr = $stderrTask.Result }
}

function New-RustFixtureCrate {
    <# Throwaway fixture crate for rust-tier propagation tests (never committed).
       Kinds: healthy | fail-unit | fail-doctest | zero-tests.
       Each kind gets a UNIQUE package name: this host honors a shared
       CARGO_TARGET_DIR cache, and identical package names across fixtures
       made cargo reuse the first-built test binary (stale-run false
       positives/negatives), so uniqueness is required for honest evidence. #>
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][ValidateSet('healthy', 'fail-unit', 'fail-doctest', 'zero-tests')][string]$Kind
    )
    New-Item -ItemType Directory -Path (Join-Path $Path 'src') -Force | Out-Null
    $packageName = "fixture_$Kind"
    $crateName = $packageName.Replace('-', '_')
    Set-Content -LiteralPath (Join-Path $Path 'Cargo.toml') -Value ("[package]`nname = `"$packageName`"`nversion = `"0.1.0`"`nedition = `"2021`"`n") -Encoding utf8
    $lib = switch ($Kind) {
        'healthy' { @'
/// Adds two numbers.
///
/// ```
/// assert_eq!(fixture::add(2, 3), 5);
/// ```
pub fn add(a: i32, b: i32) -> i32 { a + b }

#[cfg(test)]
mod tests {
    #[test]
    fn t_pass() { assert_eq!(crate::add(1, 2), 3); }
}
'@ }
        'fail-unit' { @'
/// Adds two numbers.
///
/// ```
/// assert_eq!(fixture::add(2, 3), 5);
/// ```
pub fn add(a: i32, b: i32) -> i32 { a + b }

#[cfg(test)]
mod tests {
    #[test]
    fn t_pass() { assert_eq!(crate::add(1, 2), 3); }

    #[test]
    fn t_fixture_fail() { panic!("intentional fixture failure"); }
}
'@ }
        'fail-doctest' { @'
/// Adds two numbers.
///
/// ```
/// assert_eq!(fixture::add(2, 3), 6);
/// ```
pub fn add(a: i32, b: i32) -> i32 { a + b }

#[cfg(test)]
mod tests {
    #[test]
    fn t_pass() { assert_eq!(crate::add(1, 2), 3); }
}
'@ }
        'zero-tests' { @'
pub fn add(a: i32, b: i32) -> i32 { a + b }
'@ }
    }
    # Point the doc examples at this fixture's crate name.
    $lib = $lib.Replace('fixture::add', "$crateName::add")
    Set-Content -LiteralPath (Join-Path $Path 'src\lib.rs') -Value $lib -Encoding utf8
    return (Join-Path $Path 'Cargo.toml')
}

function Read-RustTierReport {
    param([Parameter(Mandatory)][string]$Path)
    try { return (Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json) } catch { return $null }
}

Write-Output '== drift-rdp preflight test tier (scripts/Invoke-CI.Tests.ps1) =='

# --- load harness (implementation under test) ---
$env:DRIFT_CI_IMPORT_ONLY = '1'
try {
    . $ScriptPath
} catch {
    Write-Output ("harness load error: {0}" -f $_.Exception.Message)
} finally {
    Remove-Item Env:DRIFT_CI_IMPORT_ONLY -ErrorAction SilentlyContinue
}

$harnessLoaded = [bool](Get-Command -Name Invoke-Preflight -ErrorAction SilentlyContinue)
Assert-That { $harnessLoaded } 'T00 harness: scripts/Invoke-CI.ps1 dot-sources and exports functions'

try {
    if (-not $harnessLoaded) {
        Write-Output 'HALT: implementation not loadable; remaining tests blocked (RED phase).'
        exit 1
    }

    # --- fixtures (echo-only shims; junction view for reparse resolution) ---
    Write-Output '-- fixtures --'
    New-Item -ItemType Directory -Path $TempRoot -Force | Out-Null
    $WslShimDir = Join-Path $TempRoot 'wsl-shims'
    $BashShimDir = Join-Path $TempRoot 'bash-shims'
    New-Item -ItemType Directory -Path $WslShimDir -Force | Out-Null
    New-Item -ItemType Directory -Path $BashShimDir -Force | Out-Null

    $WslNodeShim = Join-Path $WslShimDir 'node.cmd'
    Set-Content -LiteralPath $WslNodeShim -Value '@echo SHIM-EXECUTED wsl.exe -e node %*' -Encoding ascii

    $BashGitShim = Join-Path $BashShimDir 'git.cmd'
    Set-Content -LiteralPath $BashGitShim -Value '@echo SHIM-EXECUTED bash -c git %*' -Encoding ascii

    $BenignShim = Join-Path $TempRoot 'benign-native-tool.cmd'
    Set-Content -LiteralPath $BenignShim -Value "@echo off`r`n@node.exe --version %*" -Encoding ascii

    $PowerShellBashShim = Join-Path $TempRoot 'delegating-wrapper.ps1'
    Set-Content -LiteralPath $PowerShellBashShim -Value "Write-Output 'delegates to bash -c node'" -Encoding ascii

    $JunctionView = Join-Path $TempRoot 'repo-view'
    if (-not (Test-Path -LiteralPath $JunctionView)) {
        New-Item -ItemType Junction -Path $JunctionView -Target $RepoRoot | Out-Null
    }

    # --- unit: canonical working directory ---
    Write-Output '-- unit: canonical working directory --'
    Assert-That { (Test-CanonicalWorkingDirectory -CurrentDirectory 'D:\Source\drift-rdp' -CanonicalRoot 'D:\Source\drift-rdp').Ok } 'T01 accepts the exact canonical root'
    Assert-That { (Test-CanonicalWorkingDirectory -CurrentDirectory 'd:\source\DRIFT-RDP\src-tauri' -CanonicalRoot 'D:\Source\drift-rdp').Ok } 'T02 accepts a canonical subdirectory (case-insensitive)'
    Assert-That { -not (Test-CanonicalWorkingDirectory -CurrentDirectory 'C:\Users\mariu\Source\drift-rdp' -CanonicalRoot 'D:\Source\drift-rdp').Ok } 'T03 rejects the C: junction view of the repository'
    Assert-That { (Test-CanonicalWorkingDirectory -CurrentDirectory 'C:\Users\mariu\Source\drift-rdp' -CanonicalRoot 'D:\Source\drift-rdp').Reason -match [regex]::Escape('D:\Source\drift-rdp') } 'T04 junction rejection reason names the canonical path'
    Assert-That { -not (Test-CanonicalWorkingDirectory -CurrentDirectory 'C:\Users\mariu\Source\drift-rdp\scripts' -CanonicalRoot 'D:\Source\drift-rdp').Ok } 'T05 rejects a subdirectory of the junction view'
    Assert-That { -not (Test-CanonicalWorkingDirectory -CurrentDirectory 'E:\Source\drift-rdp' -CanonicalRoot 'D:\Source\drift-rdp').Ok } 'T06 rejects a same-named path on another drive'
    Assert-That { -not (Test-CanonicalWorkingDirectory -CurrentDirectory '\\wsl$\Ubuntu\home\dev\drift-rdp' -CanonicalRoot 'D:\Source\drift-rdp').Ok } 'T07 rejects a WSL filesystem view as working directory'
    Assert-That { (Test-CanonicalWorkingDirectory -CurrentDirectory '\\wsl$\Ubuntu\home\dev\drift-rdp' -CanonicalRoot 'D:\Source\drift-rdp').Reason -match 'WSL' } 'T08 WSL cwd rejection reason mentions WSL'

    # --- unit: prohibited wrapper detection (inspect before execution) ---
    Write-Output '-- unit: prohibited wrapper detection --'
    Assert-That { -not (Test-ProhibitedWrapperTarget -CommandPath $PwshExe).Prohibited } 'T09 native executable (pwsh.exe) passes inspection'
    Assert-That { (Test-ProhibitedWrapperTarget -CommandPath $WslNodeShim).Prohibited } 'T10 .cmd shim delegating to wsl.exe is rejected'
    Assert-That { (Test-ProhibitedWrapperTarget -CommandPath $WslNodeShim).Reason -match 'wsl' } 'T11 wsl shim rejection reason mentions wsl'
    Assert-That { (Test-ProhibitedWrapperTarget -CommandPath $PowerShellBashShim).Prohibited } 'T12 .ps1 wrapper delegating to bash is rejected'
    Assert-That { -not (Test-ProhibitedWrapperTarget -CommandPath $BenignShim).Prohibited } 'T13 benign native .cmd shim passes inspection'
    Assert-That { (Test-ProhibitedWrapperTarget -CommandPath '\\wsl.localhost\Ubuntu\usr\bin\node.exe').Prohibited } 'T14 tool resolved on a \\wsl.localhost path is rejected'
    Assert-That { (Test-ProhibitedWrapperTarget -CommandPath 'C:\Program Files\Git\usr\bin\some-tool.cmd').Prohibited } 'T15 tool inside Git\usr\bin (MSYS layer) is rejected'
    Assert-That { (Test-ProhibitedWrapperTarget -CommandPath 'C:\cygwin64\bin\node.exe').Prohibited } 'T16 tool inside a Cygwin root is rejected'
    Assert-That { (Test-ProhibitedWrapperTarget -CommandPath 'D:\tools\bash.exe').Prohibited } 'T17 a path that is itself bash.exe is rejected'

    # --- unit: reparse (junction) target resolution ---
    Write-Output '-- unit: reparse (junction) target resolution --'
    Assert-That { [string]::Equals((Resolve-ReparseTarget -Path $JunctionView), $RepoRoot, [StringComparison]::OrdinalIgnoreCase) } 'T18 junction resolves to its target path'

    # --- unit: scoped isolated outputs ---
    Write-Output '-- unit: scoped isolated outputs --'
    $scopedPath = Get-ScopedOutputPath -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName -Subdir 'preflight'
    Assert-That { [string]::Equals($scopedPath, (Join-Path $BuildArtifactsRoot "$ProjectName\preflight"), [StringComparison]::OrdinalIgnoreCase) } 'T19 default scoped output path is D:\BuildArtifacts\drift-rdp\preflight'
    Assert-Throws { Assert-ScopedOutputPath -Path 'D:\' -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName } 'T20 drive root output path is rejected' 'drive root'
    Assert-Throws { Assert-ScopedOutputPath -Path 'D:\BuildArtifacts' -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName } 'T21 the BuildArtifacts root itself is rejected as an output path' 'BuildArtifacts'
    Assert-Throws { Assert-ScopedOutputPath -Path 'D:\BuildArtifacts\drift-rdp' -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName } 'T22 the project scope root must not be used directly (needs a subdirectory)' 'deeper'
    Assert-Throws { Assert-ScopedOutputPath -Path 'D:\BuildArtifacts\other-project\out' -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName } 'T23 a different project scope is rejected' 'outside'
    Assert-Throws { Assert-ScopedOutputPath -Path 'C:\Temp\drift-out' -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName } 'T24 paths outside D:\BuildArtifacts are rejected' 'outside'
    Assert-That { try { Assert-ScopedOutputPath -Path (Join-Path $scopedPath 'preflight-report.json') -BuildArtifactsRoot $BuildArtifactsRoot -ProjectName $ProjectName; $true } catch { $false } } 'T25 a scoped project output path is accepted'

    # --- integration: real script runs (child pwsh processes at the canonical cwd) ---
    Write-Output '-- integration: positive run --'
    $positive = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath)
    Assert-That { $positive.ExitCode -eq 0 } 'I01a positive preflight run from canonical cwd exits 0' ("exit={0}; stderr={1}" -f $positive.ExitCode, $positive.StdErr)
    Assert-That { $positive.StdOut -match 'pwsh' -and $positive.StdOut -match 'node' -and $positive.StdOut -match 'cargo' } 'I01b reports required tool names'
    $reportPath = Join-Path -Path $BuildArtifactsRoot -ChildPath (Join-Path $ProjectName (Join-Path 'preflight' 'preflight-report.json'))
    Assert-That { Test-Path -LiteralPath $reportPath -PathType Leaf } 'I01c writes report under D:\BuildArtifacts\drift-rdp\preflight'
    $reportJson = $null
    try { $reportJson = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json } catch { }
    Assert-That { $null -ne $reportJson } 'I01d report parses as JSON'
    if ($null -ne $reportJson) {
        foreach ($toolName in 'pwsh', 'git', 'node', 'npm', 'cargo', 'rustc') {
            $entry = @($reportJson.tools) | Where-Object { $_.tool -eq $toolName }
            Assert-That { $null -ne $entry -and $entry.status -eq 'ok' -and -not [string]::IsNullOrEmpty($entry.version) } ("I01e report lists {0} with version and ok status" -f $toolName)
        }
        Assert-That { [string]::Equals($reportJson.canonicalRoot, $RepoRoot, [StringComparison]::OrdinalIgnoreCase) } 'I01f report records the canonical repository root'
    }

    Write-Output '-- integration: junction-view working directory rejection --'
    $junctionCwd = 'C:\Users\mariu\Source\drift-rdp'
    $rejected = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-WorkingDirectory', $junctionCwd)
    Assert-That { $rejected.ExitCode -eq 1 } 'I02a junction-view working directory is rejected (exit 1)'
    $combined = "{0}`n{1}" -f $rejected.StdOut, $rejected.StdErr
    Assert-That { $combined -match [regex]::Escape($junctionCwd) } 'I02b rejection names the offending working directory'
    Assert-That { $combined -match [regex]::Escape($RepoRoot) } 'I02c rejection names the canonical repository path'

    Write-Output '-- integration: WSL filesystem working directory rejection --'
    $wslCwd = '\\wsl$\Ubuntu\home\dev\drift-rdp'
    $wslCwdRun = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-WorkingDirectory', $wslCwd)
    Assert-That { $wslCwdRun.ExitCode -eq 1 } 'I05a WSL filesystem working directory is rejected (exit 1)'
    $wslCwdCombined = "{0}`n{1}" -f $wslCwdRun.StdOut, $wslCwdRun.StdErr
    Assert-That { $wslCwdCombined -match 'WSL' } 'I05b rejection mentions WSL'

    Write-Output '-- integration: prohibited wrapper rejection (PATH-poisoned child) --'
    $wslPoisonedPath = '{0};{1}' -f $WslShimDir, $env:PATH
    $wslRun = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath) -Environment @{ 'PATH' = $wslPoisonedPath }
    $wslCombined = "{0}`n{1}" -f $wslRun.StdOut, $wslRun.StdErr
    Assert-That { $wslRun.ExitCode -eq 1 } 'I03a preflight exits 1 when a required tool delegates to wsl.exe'
    Assert-That { $wslCombined -match [regex]::Escape($WslNodeShim) } 'I03b rejection names the offending shim path'
    Assert-That { $wslCombined -match 'wsl' } 'I03c rejection reason mentions wsl'
    Assert-That { $wslCombined -notmatch 'SHIM-EXECUTED' } 'I03d the prohibited shim was inspected and rejected, never executed'

    $bashPoisonedPath = '{0};{1}' -f $BashShimDir, $env:PATH
    $bashRun = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath) -Environment @{ 'PATH' = $bashPoisonedPath }
    $bashCombined = "{0}`n{1}" -f $bashRun.StdOut, $bashRun.StdErr
    Assert-That { $bashRun.ExitCode -eq 1 } 'I04a preflight exits 1 when a required tool delegates to bash'
    Assert-That { $bashCombined -match [regex]::Escape($BashGitShim) } 'I04b rejection names the offending git shim path'
    Assert-That { $bashCombined -match 'bash' } 'I04c rejection reason mentions bash'
    Assert-That { $bashCombined -notmatch 'SHIM-EXECUTED' } 'I04d the prohibited shim was never executed'

    # --- task 1.2: rust test tier (pinned nextest + cargo test --doc) ---
    Write-Output '-- unit: nextest pin and version gate --'
    Assert-That { $null -ne $DriftNextestMinVersion -and [version]$DriftNextestMinVersion -ge ([version]'0.9.150') } 'R00 harness pin: $DriftNextestMinVersion is at least the 0.9.150 advisory floor'
    Assert-That { (Test-DriftNextestVersion -VersionOutput 'cargo-nextest 0.9.150 (d225b1daf 2026-10-26)').Ok } 'R01 version gate accepts the exact pinned minimum 0.9.150'
    Assert-That { -not (Test-DriftNextestVersion -VersionOutput 'cargo-nextest 0.9.149 (abc 2026-01-01)').Ok } 'R02 version gate rejects 0.9.149 (below the pin)'
    Assert-That { (Test-DriftNextestVersion -VersionOutput 'cargo-nextest 0.9.149 (abc 2026-01-01)').Reason -match [regex]::Escape('cargo install cargo-nextest --locked --version') } 'R02b below-pin rejection names the pinned install command'
    Assert-That { (Test-DriftNextestVersion -VersionOutput 'cargo-nextest 0.9.151 (abc 2026-01-01)').Ok } 'R03 version gate accepts a newer patch (0.9.151)'
    Assert-That { (Test-DriftNextestVersion -VersionOutput 'cargo-nextest 0.10.0 (abc 2026-01-01)').Ok } 'R04 version gate accepts a newer minor (0.10.0)'
    Assert-That { (Test-DriftNextestVersion -VersionOutput 'cargo-nextest 1.0.0 (abc 2026-01-01)').Ok } 'R05 version gate accepts a newer major (1.0.0)'
    Assert-That { -not (Test-DriftNextestVersion -VersionOutput 'nextest not installed').Ok } 'R06 version gate rejects unparseable version output'
    Assert-That { (Get-DriftNextestInstallHint) -match [regex]::Escape("cargo install cargo-nextest --locked --version $DriftNextestMinVersion") } 'R07 install hint contains the exact pinned install command'

    Write-Output '-- unit: .config/nextest.toml (design.md file plan) --'
    $NextestConfigPath = Join-Path $RepoRoot '.config\nextest.toml'
    Assert-That { Test-Path -LiteralPath $NextestConfigPath -PathType Leaf } 'R08 repo-root .config/nextest.toml exists'
    Assert-That { try { $c = Get-Content -LiteralPath $NextestConfigPath -Raw; ($c -match '\[profile\.ci\]') -and ($c -match '\[profile\.ci\.junit\]') -and ($c -match 'slow-timeout') -and ($c -match 'leak-timeout') } catch { $false } } 'R09 ci profile defines junit plus slow-timeout and leak-timeout reporting'
    Assert-That { try { (Get-Content -LiteralPath $NextestConfigPath -Raw) -notmatch '(?m)^\s*retries\s*=' } catch { $false } } 'R10 config sets no automatic retries'
    Assert-That { try { $c = Get-Content -LiteralPath $NextestConfigPath -Raw; ($c -match 'nextest-version') -and ($c -match [regex]::Escape('required = "0.9.150"')) } catch { $false } } 'R11 config pins a machine-readable minimum nextest version (0.9.150)'
    Assert-That { try { $c = Get-Content -LiteralPath $NextestConfigPath -Raw; ($c -match '\[test-groups\]') -and ($c -match 'os-keyring') -and ($c -match 'test-group') } catch { $false } } 'R12 config defines the os-keyring resource group and assigns it via an override'
    Assert-That { $a = New-DriftNextestRunArguments -ConfigPath 'C:\cfg.toml' -ManifestPath 'C:\m\Cargo.toml'; ($a -contains 'run') -and ($a -contains '--profile') -and ($a -contains 'ci') -and ($a -contains '--no-tests') -and ($a -contains 'fail') -and ($a -contains '--config-file') -and ($a -notcontains '--retries') } 'R13 nextest arguments include fail-on-zero and the pinned config, and never --retries'

    Write-Output '-- integration: rust tier fixtures (real cargo, throwaway crates) --'
    $RustTemp = Join-Path $TempRoot 'rust-fixtures'
    New-Item -ItemType Directory -Path $RustTemp -Force | Out-Null
    $HealthyManifest = New-RustFixtureCrate -Path (Join-Path $RustTemp 'healthy') -Kind 'healthy'
    $FailUnitManifest = New-RustFixtureCrate -Path (Join-Path $RustTemp 'fail-unit') -Kind 'fail-unit'
    $FailDocManifest = New-RustFixtureCrate -Path (Join-Path $RustTemp 'fail-doctest') -Kind 'fail-doctest'
    $ZeroManifest = New-RustFixtureCrate -Path (Join-Path $RustTemp 'zero-tests') -Kind 'zero-tests'
    $RustReportPath = Join-Path $BuildArtifactsRoot "$ProjectName\rust\rust-tier-report.json"

    Write-Output '-- integration: IR01 healthy fixture (nextest + doctests pass) --'
    $ir1 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'rust', '-ManifestPath', $HealthyManifest)
    Assert-That { $ir1.ExitCode -eq 0 } 'IR01a rust tier exits 0 on a healthy crate' ("exit={0}; stderr={1}" -f $ir1.ExitCode, $ir1.StdErr)
    Assert-That { Test-Path -LiteralPath $RustReportPath -PathType Leaf } 'IR01b tier writes the rust report under D:\BuildArtifacts\drift-rdp\rust'
    $ir1Report = Read-RustTierReport -Path $RustReportPath
    Assert-That { $null -ne $ir1Report -and $ir1Report.ok } 'IR01c report records ok=true'
    Assert-That { $null -ne $ir1Report -and ((@($ir1Report.tools) | Where-Object { $_.tool -eq 'cargo-nextest' }).version -match '^\d+\.\d+\.\d+$') -and (([version](@($ir1Report.tools) | Where-Object { $_.tool -eq 'cargo-nextest' }).version) -ge ([version]$DriftNextestMinVersion)) } 'IR01d report records a cargo-nextest version satisfying the pin'
    Assert-That { $null -ne $ir1Report -and @($ir1Report.artifacts.junitCopied).Count -ge 1 -and (Test-Path -LiteralPath @($ir1Report.artifacts.junitCopied)[0] -PathType Leaf) } 'IR01e JUnit evidence is copied under the scoped output directory'
    Assert-That { try { (Get-Content -LiteralPath @($ir1Report.artifacts.junitCopied)[0] -Raw) -match '<\?xml' } catch { $false } } 'IR01f copied JUnit file is XML'

    Write-Output '-- integration: IR02 failing unit test propagates --'
    $ir2 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'rust', '-ManifestPath', $FailUnitManifest)
    Assert-That { $ir2.ExitCode -eq 1 } 'IR02a failing unit test propagates: tier exits 1' ("exit={0}" -f $ir2.ExitCode)
    $ir2Report = Read-RustTierReport -Path $RustReportPath
    Assert-That { $null -ne $ir2Report -and -not $ir2Report.ok -and $ir2Report.nextest.exitCode -ne 0 } 'IR02b report records the nonzero nextest exit code'

    Write-Output '-- integration: IR03 failing doctest propagates (nextest itself passes) --'
    $ir3 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'rust', '-ManifestPath', $FailDocManifest)
    Assert-That { $ir3.ExitCode -eq 1 } 'IR03a failing doctest propagates although nextest passed: tier exits 1' ("exit={0}" -f $ir3.ExitCode)
    $ir3Report = Read-RustTierReport -Path $RustReportPath
    Assert-That { $null -ne $ir3Report -and $ir3Report.nextest.exitCode -eq 0 -and $ir3Report.doctest.exitCode -ne 0 } 'IR03b report shows nextest passed but cargo test --doc failed'

    Write-Output '-- integration: IR04 zero tests fails the tier --'
    $ir4 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'rust', '-ManifestPath', $ZeroManifest)
    Assert-That { $ir4.ExitCode -eq 1 } 'IR04a zero executed tests fails the tier (fail-on-zero-tests)' ("exit={0}" -f $ir4.ExitCode)
    $ir4Combined = "{0}`n{1}" -f $ir4.StdOut, $ir4.StdErr
    Assert-That { $ir4Combined -match 'no tests to run' } 'IR04b zero-test failure names the nextest zero-tests error'

    Write-Output '-- integration: IR05 canonical working directory still enforced --'
    $ir5 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'rust', '-WorkingDirectory', 'C:\Users\mariu\Source\drift-rdp')
    Assert-That { $ir5.ExitCode -eq 1 } 'IR05a rust tier rejects the junction-view working directory'
    Assert-That { ("{0}`n{1}" -f $ir5.StdOut, $ir5.StdErr) -match [regex]::Escape($RepoRoot) } 'IR05b rejection names the canonical repository path'

    Write-Output '-- integration: IR06 missing cargo fails cleanly --'
    $ir6 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'rust') -Environment @{ 'PATH' = "$env:SystemRoot\System32" }
    Assert-That { $ir6.ExitCode -eq 1 } 'IR06a rust tier fails cleanly when cargo is missing from PATH'
    Assert-That { ("{0}`n{1}" -f $ir6.StdOut, $ir6.StdErr) -match 'cargo' } 'IR06b missing-tool failure names cargo'

    Write-Output '-- integration: IR07 wsl-delegating cargo-nextest shim rejected before execution --'
    $NextestShimDir = Join-Path $TempRoot 'nextest-shims'
    New-Item -ItemType Directory -Path $NextestShimDir -Force | Out-Null
    $NextestWslShim = Join-Path $NextestShimDir 'cargo-nextest.cmd'
    Set-Content -LiteralPath $NextestWslShim -Value '@echo SHIM-EXECUTED wsl.exe -e cargo-nextest %*' -Encoding ascii
    $ir7 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'rust') -Environment @{ 'PATH' = ('{0};{1}' -f $NextestShimDir, $env:PATH) }
    $ir7Combined = "{0}`n{1}" -f $ir7.StdOut, $ir7.StdErr
    Assert-That { $ir7.ExitCode -eq 1 } 'IR07a rust tier rejects a wsl-delegating cargo-nextest shim'
    Assert-That { $ir7Combined -match [regex]::Escape($NextestWslShim) } 'IR07b rejection names the offending shim path'
    Assert-That { $ir7Combined -notmatch 'SHIM-EXECUTED' } 'IR07c the prohibited shim was inspected and rejected, never executed'

    # --- task 1.4: one command contract for container and native jobs ---
    # Requirements 7.1, 7.2, 8.4, 12.5, 13.3, 13.4. Static + child-process
    # assertions only: this tier must not need the Dagger engine or any
    # container image (live engine evidence is recorded in task evidence).

    Write-Output '-- contract: CT01 Dagger module manifest (.dagger/, design.md file plan) --'
    $ctDaggerJsonPath = Join-Path $RepoRoot '.dagger\dagger.json'
    Assert-That { Test-Path -LiteralPath $ctDaggerJsonPath -PathType Leaf } 'CT01a .dagger/dagger.json exists'
    $ctDaggerJson = $null
    try { $ctDaggerJson = Get-Content -LiteralPath $ctDaggerJsonPath -Raw | ConvertFrom-Json } catch { $ctDaggerJson = $null }
    Assert-That { $null -ne $ctDaggerJson } 'CT01b dagger.json parses as JSON'
    Assert-That { $ctDaggerJson -and $ctDaggerJson.name -eq 'drift' } 'CT01c module name is drift'
    Assert-That { $ctDaggerJson -and $ctDaggerJson.engineVersion -eq 'v0.21.11' } 'CT01d engineVersion pinned to v0.21.11 (task 1.3 proven pin)'
    Assert-That { $ctDaggerJson -and (($ctDaggerJson.sdk -eq 'typescript') -or ($ctDaggerJson.sdk.PSObject -and $ctDaggerJson.sdk.source -eq 'typescript')) } 'CT01e sdk is typescript (Dagger TypeScript SDK, tasks.md tool choices)'
    Assert-That { $ctDaggerJson -and $ctDaggerJson.source -eq '.' } 'CT01f sourceSubpath pinned explicitly so the module lives in .dagger/ (research.md 1.3 finding)'

    Write-Output '-- contract: CT02 pinned versions/config file (.config/ci-versions.json) --'
    $ctPinsPath = Join-Path $RepoRoot '.config\ci-versions.json'
    Assert-That { Test-Path -LiteralPath $ctPinsPath -PathType Leaf } 'CT02a .config/ci-versions.json exists'
    $ctPins = $null
    try { $ctPins = Get-Content -LiteralPath $ctPinsPath -Raw | ConvertFrom-Json } catch { $ctPins = $null }
    Assert-That { $null -ne $ctPins } 'CT02b ci-versions.json parses as JSON'
    Assert-That { $ctPins -and $ctPins.dagger.cliVersion -eq '0.21.11' } 'CT02c dagger CLI pinned to 0.21.11'
    Assert-That { $ctPins -and $ctPins.dagger.engineImage -eq 'registry.dagger.io/engine:v0.21.11' } 'CT02d engine image pinned (task 1.3 spike pin)'
    Assert-That { $ctPins -and $ctPins.node.image -match '^node:22\.[0-9]+\.[0-9]+-bookworm@sha256:[0-9a-f]{64}$' } 'CT02e node image pinned with exact tag and digest (repo CI uses Node 22)'
    Assert-That { $ctPins -and $ctPins.rust.image -match '^rust:1\.[0-9]+\.[0-9]+-bookworm@sha256:[0-9a-f]{64}$' } 'CT02f rust image pinned with exact tag and digest'
    Assert-That { $ctPins -and $ctPins.rust.toolchain -match '^1\.[0-9]+\.[0-9]+$' } 'CT02g rust toolchain version pinned'
    $ctPinsRaw = ''
    if (Test-Path -LiteralPath $ctPinsPath -PathType Leaf) { $ctPinsRaw = Get-Content -LiteralPath $ctPinsPath -Raw }
    Assert-That { $ctPinsRaw -notmatch 'latest' } 'CT02h no floating :latest pins anywhere in ci-versions.json'

    Write-Output '-- contract: CT03 nextest pin consistent across the three pin locations --'
    $ctNextestTomlPath = Join-Path $RepoRoot '.config\nextest.toml'
    $ctNextestToml = ''
    if (Test-Path -LiteralPath $ctNextestTomlPath -PathType Leaf) { $ctNextestToml = Get-Content -LiteralPath $ctNextestTomlPath -Raw }
    Assert-That { $ctPins -and $ctPins.nextest.version -eq '0.9.150' } 'CT03a ci-versions.json pins cargo-nextest 0.9.150'
    Assert-That { $ctNextestToml -match [regex]::Escape('nextest-version = { required = "0.9.150" }') } 'CT03b .config/nextest.toml requires the same pin'
    Assert-That { $null -ne $DriftNextestMinVersion -and $ctPins -and $ctPins.nextest.version -eq $DriftNextestMinVersion } 'CT03c Invoke-CI.ps1 $DriftNextestMinVersion matches ci-versions.json'

    Write-Output '-- contract: CT04 Dagger module entry points (frontend, rust-checks, linux-build, linux-e2e) --'
    $ctModuleSrcPath = Join-Path $RepoRoot '.dagger\src\index.ts'
    $ctModuleSrc = ''
    if (Test-Path -LiteralPath $ctModuleSrcPath -PathType Leaf) { $ctModuleSrc = Get-Content -LiteralPath $ctModuleSrcPath -Raw }
    Assert-That { $ctModuleSrc -ne '' } 'CT04a .dagger/src/index.ts exists (TypeScript module source)'
    Assert-That { $ctModuleSrc -match '@func\(' } 'CT04b module uses Dagger decorators (@func)'
    $ctEntryPatterns = @{
        'frontend'    = 'async frontend\('
        'rust-checks' = 'async rustChecks\('
        'linux-build' = 'async linuxBuild\('
        'linux-e2e'   = 'async linuxE2E\('     # TS method linuxE2E: SDK registers callable linux-e-2-e (digit/letter split); contract name kept in pins.ts
        'contract'    = 'contract\(\):\s*string'
    }
    foreach ($ctEntry in @('frontend', 'rust-checks', 'linux-build', 'linux-e2e', 'contract')) {
        Assert-That { $ctModuleSrc -match $ctEntryPatterns[$ctEntry] } "CT04c module declares entry point '$ctEntry'"
    }
    $ctPinsTsPath = Join-Path $RepoRoot '.dagger\src\pins.ts'
    $ctPinsTs = ''
    if (Test-Path -LiteralPath $ctPinsTsPath -PathType Leaf) { $ctPinsTs = Get-Content -LiteralPath $ctPinsTsPath -Raw }
    Assert-That {
        $ctPins -and $ctPinsTs -ne '' -and
        $ctPinsTs -match [regex]::Escape(($ctPins.node.image -split '@')[0]) -and
        $ctPinsTs -match [regex]::Escape(($ctPins.rust.image -split '@')[0]) -and
        $ctPinsTs -match [regex]::Escape($ctPins.dagger.engineImage)
    } 'CT04d module pins.ts mirrors the node/rust image tags and engine image from ci-versions.json'
    Assert-That {
        $ctPins -and $ctPinsTs -ne '' -and
        $ctPins.node.image -match '@sha256:[0-9a-f]{64}$' -and
        $ctPins.rust.image -match '@sha256:[0-9a-f]{64}$' -and
        $ctPinsTs -match [regex]::Escape(($ctPins.node.image -split '@')[1]) -and
        $ctPinsTs -match [regex]::Escape(($ctPins.rust.image -split '@')[1])
    } 'CT04d1 module pins.ts mirrors the full @sha256 image digests from ci-versions.json (digest drift cross-check)'
    Assert-That { $ctPinsTs -match [regex]::Escape('entry: "linux-e2e"') } 'CT04c1 the contract name linux-e2e is declared in pins.ts (SDK callable is linux-e-2-e)'

    Write-Output '-- contract: CT05 command-list document distinguishes container from native --'
    $ctReadmePath = Join-Path $RepoRoot '.dagger\README.md'
    $ctReadme = ''
    if (Test-Path -LiteralPath $ctReadmePath -PathType Leaf) { $ctReadme = Get-Content -LiteralPath $ctReadmePath -Raw }
    Assert-That { $ctReadme -ne '' } 'CT05a .dagger/README.md exists (CI contract command list)'
    Assert-That { $ctReadme -match 'Linux container' -and $ctReadme -match 'native Windows' -and $ctReadme -match 'native macOS' } 'CT05b contract doc distinguishes Linux container checks from native Windows/macOS validation'
    Assert-That { $ctReadme -match [regex]::Escape('never substitute for native Windows or macOS validation') } 'CT05c contract doc states container results never substitute for native validation'
    Assert-That { $ctPins -and $ctReadme -match [regex]::Escape($ctPins.dagger.engineImage) } 'CT05d contract doc names the pinned engine image'
    Assert-That { $ctReadme -match [regex]::Escape('node:22') -and $ctReadme -match [regex]::Escape('rust:1.') } 'CT05e contract doc names the pinned node and rust images'

    Write-Output '-- contract: CT06 native frontend tier (npm runs natively on Windows) --'
    $ctFrontendCommands = $null
    try { $ctFrontendCommands = Get-DriftFrontendTierCommands } catch { $ctFrontendCommands = $null }
    Assert-That { $null -ne $ctFrontendCommands -and ($ctFrontendCommands -join ' | ') -eq 'npm ci | npm run lint | npx tsc --noEmit | npx vitest run | npm run build' } 'CT06a frontend tier command list: install, lint, typecheck, unit tests, build (Req 7.1)'
    $ctF1 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'frontend') -Environment @{ 'PATH' = "$env:SystemRoot\System32" }
    Assert-That { $ctF1.ExitCode -eq 1 } 'CT06b frontend tier fails cleanly when node/npm are missing from PATH'
    Assert-That { ("{0}`n{1}" -f $ctF1.StdOut, $ctF1.StdErr) -match 'npm' } 'CT06c missing-tool failure names npm'

    Write-Output '-- contract: CT07 dagger tier resolves the real CLI, never the personal launcher --'
    $ctDaggerCli = $null
    try { $ctDaggerCli = Resolve-DriftDaggerCli -PinsPath $ctPinsPath } catch { $ctDaggerCli = $null }
    Assert-That { $null -ne $ctDaggerCli -and $ctDaggerCli -match '\.exe$' } 'CT07a Resolve-DriftDaggerCli finds the real dagger executable (.exe, not a .cmd launcher)'

    Write-Output '-- contract: CT08 dagger tier wires the engine runner host itself (task 1.3 finding) --'
    $ctDaggerShimDir = Join-Path $TempRoot 'dagger-shims'
    New-Item -ItemType Directory -Path $ctDaggerShimDir -Force | Out-Null
    $ctRecorderOut = Join-Path $ctDaggerShimDir 'recorder-out.txt'
    $ctRecorder = Join-Path $ctDaggerShimDir 'dagger.cmd'
    Set-Content -LiteralPath $ctRecorder -Value @(
        '@echo off',
        '> "%DRIFT_DAGGER_RECORDER_OUT%" echo ARGS=%*',
        '>> "%DRIFT_DAGGER_RECORDER_OUT%" echo RUNNER=%_EXPERIMENTAL_DAGGER_RUNNER_HOST%',
        'exit /b 7'
    ) -Encoding ascii
    $ctD1 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'contract', '-DaggerCliPath', $ctRecorder) -Environment @{ 'DRIFT_DAGGER_RECORDER_OUT' = $ctRecorderOut }
    Assert-That { $ctD1.ExitCode -eq 7 } 'CT08a dagger tier propagates the dagger CLI exit code (7 from the recording shim)'
    $ctRec = ''
    if (Test-Path -LiteralPath $ctRecorderOut -PathType Leaf) { $ctRec = Get-Content -LiteralPath $ctRecorderOut -Raw }
    Assert-That { $ctRec -match [regex]::Escape('RUNNER=image+podman://registry.dagger.io/engine:v0.21.11') } 'CT08b tier sets _EXPERIMENTAL_DAGGER_RUNNER_HOST from the pinned engine image (no launcher dependency)'
    Assert-That { $ctRec -match '\bcall\b' -and $ctRec -match '\bcontract\b' -and $ctRec -match '-m' -and $ctRec -notmatch '--source' } 'CT08c tier invokes dagger with: -m <module> call contract (no --source for the contract entry)'
    Assert-That { $ctRec -match [regex]::Escape((Join-Path $RepoRoot '.dagger')) } 'CT08d tier points -m at the repository .dagger module'
    $ctRecorderOut2 = Join-Path $ctDaggerShimDir 'recorder-out-2.txt'
    $ctD3 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'linux-e2e', '-DaggerCliPath', $ctRecorder) -Environment @{ 'DRIFT_DAGGER_RECORDER_OUT' = $ctRecorderOut2 }
    $ctRec2 = ''
    if (Test-Path -LiteralPath $ctRecorderOut2 -PathType Leaf) { $ctRec2 = Get-Content -LiteralPath $ctRecorderOut2 -Raw }
    Assert-That { $ctRec2 -match [regex]::Escape('linux-e-2-e') -and $ctRec2 -match '--source' } 'CT08e tier maps the contract entry linux-e2e to the SDK-registered callable linux-e-2-e and passes --source <repo root>'

    Write-Output '-- contract: CT09 wsl-delegating dagger shim rejected before execution --'
    $ctDaggerWsl = Join-Path $ctDaggerShimDir 'dagger-wsl.cmd'
    Set-Content -LiteralPath $ctDaggerWsl -Value '@echo SHIM-EXECUTED wsl.exe -e dagger %*' -Encoding ascii
    $ctD2 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'contract', '-DaggerCliPath', $ctDaggerWsl) -Environment @{}
    $ctD2Combined = "{0}`n{1}" -f $ctD2.StdOut, $ctD2.StdErr
    Assert-That { $ctD2.ExitCode -eq 1 } 'CT09a dagger tier rejects a wsl-delegating dagger shim'
    Assert-That { $ctD2Combined -match [regex]::Escape($ctDaggerWsl) } 'CT09b rejection names the offending shim path'
    Assert-That { $ctD2Combined -notmatch 'SHIM-EXECUTED' } 'CT09c the prohibited shim was inspected and rejected, never executed'

    Write-Output '-- contract: CT10 single command per tier documented (Req 13.4) --'
    $ctCiSrc = Get-Content -LiteralPath $ScriptPath -Raw
    Assert-That { $ctCiSrc -match [regex]::Escape('-Tier frontend') } 'CT10a Invoke-CI.ps1 documents the frontend tier command'
    Assert-That { $ctCiSrc -match [regex]::Escape('-Tier dagger') } 'CT10b Invoke-CI.ps1 documents the dagger tier command'
    Assert-That { $ctCiSrc -match [regex]::Escape("'frontend', 'rust-checks', 'linux-build', 'linux-e2e', 'contract'") } 'CT10c -DaggerEntry ValidateSet exposes exactly the module entry points plus contract introspection'

    # --- task 2.1: portable checks in Dagger — clean source, JUnit, caches ---
    # Requirements 7.1, 7.3, 13.3 + tasks.md task 2.1: the frontend/rust-checks
    # Dagger entries emit tests/JUnit, keep doctests, cache registry/git/target
    # separately by OS/arch/toolchain/features, never import host
    # target/node_modules, and the tier writes PER-ENTRY reports and exports
    # the JUnit artifacts under scoped outputs.

    Write-Output '-- contract: CT11 task 2.1 portable checks: clean source, JUnit, caches, per-entry reports --'
    $ctIndexTsPath = Join-Path $RepoRoot '.dagger\src\index.ts'
    $ctIndexTs = ''
    if (Test-Path -LiteralPath $ctIndexTsPath -PathType Leaf) { $ctIndexTs = Get-Content -LiteralPath $ctIndexTsPath -Raw }

    # a) lint gate parity: the generated dagger SDK tree must be globally
    #    ignored so clean vs post-dagger lint runs agree (Implementation Notes).
    $ctEslintPath = Join-Path $RepoRoot 'eslint.config.js'
    $ctEslint = ''
    if (Test-Path -LiteralPath $ctEslintPath -PathType Leaf) { $ctEslint = Get-Content -LiteralPath $ctEslintPath -Raw }
    Assert-That { $ctEslint -match [regex]::Escape('.dagger/sdk') } 'CT11a eslint.config.js globally ignores .dagger/sdk (clean vs post-dagger lint runs agree)'

    # b) engine-managed cache volume ids keyed by OS/arch/toolchain/features
    Assert-That { $ctPins -and $ctPinsTs -match [regex]::Escape('drift-npm-registry-linux-x86_64-node-' + $ctPins.node.version) } 'CT11b npm registry cache id is keyed by OS/arch/toolchain (node pin)'
    Assert-That { $ctPins -and $ctPinsTs -match [regex]::Escape('drift-cargo-registry-linux-x86_64-rust-' + $ctPins.rust.toolchain) } 'CT11c cargo registry cache id is keyed by OS/arch/toolchain (rust pin)'
    Assert-That { $ctPins -and $ctPinsTs -match [regex]::Escape('drift-cargo-git-linux-x86_64-rust-' + $ctPins.rust.toolchain) } 'CT11d cargo git cache id is keyed by OS/arch/toolchain (rust pin)'
    Assert-That { $ctPins -and $ctPinsTs -match [regex]::Escape('drift-cargo-target-linux-x86_64-rust-' + $ctPins.rust.toolchain + '-features-default') } 'CT11e cargo target cache id is keyed by OS/arch/toolchain/features'

    # c) module mounts the caches at the canonical tool cache paths and emits junit
    Assert-That {
        $ctIndexTs -match [regex]::Escape('"/root/.npm"') -and
        $ctIndexTs -match [regex]::Escape('"/usr/local/cargo/registry"') -and
        $ctIndexTs -match [regex]::Escape('"/usr/local/cargo/git"') -and
        $ctIndexTs -match [regex]::Escape('"/src/src-tauri/target"')
    } 'CT11f module mounts npm registry + cargo registry/git/target as separate engine-managed cache volumes'
    Assert-That { $ctIndexTs -match [regex]::Escape('/test-results/vitest-junit.xml') -and $ctIndexTs -match [regex]::Escape('/test-results/rust-junit.xml') } 'CT11g frontend vitest and rust nextest write JUnit under /test-results'
    Assert-That { ([regex]::Matches($ctIndexTs, 'Promise<Directory>')).Count -ge 2 } 'CT11h frontend and rustChecks return the JUnit artifact directory (exportable via dagger export --path)'
    Assert-That { $ctIndexTs -match [regex]::Escape('ReturnType.Any') } 'CT11i test steps run with expect any so failing tests still emit their JUnit before the entry fails loudly'
    Assert-That { $ctIndexTs -match [regex]::Escape('"--doc"') -and $ctIndexTs -match [regex]::Escape('"cargo"') -and $ctIndexTs -match [regex]::Escape('"test"') } 'CT11j doctests remain included in rust-checks (cargo test --doc)'
    Assert-That { $ctIndexTs -match 'node_modules' -and $ctIndexTs -match 'src-tauri/target' -and $ctIndexTs -match 'never include host build outputs' } 'CT11k module rejects sources containing host node_modules or src-tauri/target (never import host artifacts)'

    # d) tier behavior through the recording shim: per-entry reports, clean
    #    staged source, artifact export chaining.
    $ctDaggerReportDir = Join-Path $BuildArtifactsRoot "$ProjectName\dagger"
    $ctLegacyReport = Join-Path $ctDaggerReportDir 'dagger-tier-report.json'
    if (Test-Path -LiteralPath $ctLegacyReport) { Remove-Item -LiteralPath $ctLegacyReport -Force }
    $ctStagedRoot = Join-Path $ctDaggerReportDir 'staged-source'
    $ctRecorderOut3 = Join-Path $ctDaggerShimDir 'recorder-out-3.txt'
    $ctD4 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'frontend', '-DaggerCliPath', $ctRecorder) -Environment @{ 'DRIFT_DAGGER_RECORDER_OUT' = $ctRecorderOut3 }
    Assert-That { $ctD4.ExitCode -eq 7 } 'CT11l frontend dagger tier propagates the shim exit code through the staging/export flow'
    $ctRec3 = ''
    if (Test-Path -LiteralPath $ctRecorderOut3 -PathType Leaf) { $ctRec3 = Get-Content -LiteralPath $ctRecorderOut3 -Raw }
    Assert-That { $ctRec3 -match '--source' -and $ctRec3 -match [regex]::Escape('staged-source') } 'CT11m tier passes the staged clean source directory, not the host repository root'
    Assert-That { $ctRec3 -match 'export' -and $ctRec3 -match '--path' -and $ctRec3 -match [regex]::Escape('artifacts\frontend') } 'CT11n frontend entry call chains export --path into the scoped artifacts directory'
    Assert-That {
        (Test-Path -LiteralPath (Join-Path $ctStagedRoot 'package.json') -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $ctStagedRoot 'package-lock.json') -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $ctStagedRoot 'src-tauri\Cargo.toml') -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $ctStagedRoot 'src-tauri\Cargo.lock') -PathType Leaf)
    } 'CT11o staged source carries the lockfiles (npm ci and cargo reuse them)'
    Assert-That {
        -not (Test-Path -LiteralPath (Join-Path $ctStagedRoot 'node_modules')) -and
        -not (Test-Path -LiteralPath (Join-Path $ctStagedRoot 'src-tauri\target')) -and
        -not (Test-Path -LiteralPath (Join-Path $ctStagedRoot '.git')) -and
        -not (Test-Path -LiteralPath (Join-Path $ctStagedRoot '.dagger')) -and
        -not (Test-Path -LiteralPath (Join-Path $ctStagedRoot 'dist'))
    } 'CT11p staged source never contains host node_modules, src-tauri/target, .git, .dagger or dist'
    $ctD4Report = Read-RustTierReport -Path (Join-Path $ctDaggerReportDir 'dagger-tier-report-frontend.json')
    Assert-That { $null -ne $ctD4Report -and $ctD4Report.entry -eq 'frontend' -and $ctD4Report.daggerExitCode -eq 7 } 'CT11q dagger tier writes a per-entry report (dagger-tier-report-frontend.json)'
    Assert-That { -not (Test-Path -LiteralPath $ctLegacyReport) } 'CT11r the legacy last-writer-wins dagger-tier-report.json is gone for good'
    Assert-That { try { [string]$ctD4Report.artifactDir -match [regex]::Escape('artifacts\frontend') } catch { $false } } 'CT11s per-entry report records the scoped artifact directory'
    Assert-That { $ctRec -notmatch 'export' } 'CT11t the contract entry still takes no --source and no export (string contract return)'
    $ctRecorderOut4 = Join-Path $ctDaggerShimDir 'recorder-out-4.txt'
    $ctD5 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'rust-checks', '-DaggerCliPath', $ctRecorder) -Environment @{ 'DRIFT_DAGGER_RECORDER_OUT' = $ctRecorderOut4 }
    Assert-That { $ctD5.ExitCode -eq 7 } 'CT11u rust-checks dagger tier propagates the shim exit code'
    $ctRec4 = ''
    if (Test-Path -LiteralPath $ctRecorderOut4 -PathType Leaf) { $ctRec4 = Get-Content -LiteralPath $ctRecorderOut4 -Raw }
    Assert-That { $ctRec4 -match 'rust-checks' -and $ctRec4 -match 'export' -and $ctRec4 -match [regex]::Escape('artifacts\rust-checks') } 'CT11v rust-checks entry call chains export --path into its own scoped artifacts directory'
    Assert-That { $null -ne (Read-RustTierReport -Path (Join-Path $ctDaggerReportDir 'dagger-tier-report-rust-checks.json')) } 'CT11w dagger tier writes dagger-tier-report-rust-checks.json (per-entry, no clobbering)'

    # --- task 2.2: Linux bundles in Dagger — reuse the current Tauri
    # packaging, export AppImage/deb plus logs into the scoped artifact
    # directory, and FAIL the tier when bundle artifacts are missing.
    # Requirements 7.2, 7.3 + tasks.md task 2.2. Static + child-process
    # assertions only: no engine or container is required here (the live
    # bundle run and the corrupted-expectation demonstration are recorded in
    # the task evidence).

    Write-Output '-- contract: CT12 task 2.2 Linux bundle: AppImage/deb + logs export, missing-artifact gate --'

    # a) the module entry executes the bundle and returns the artifact directory
    #    (task 2.5 moved the build steps into the shared linuxBundleBuildContainer
    #    used by BOTH linuxBuild and linuxE2E — CT12a targets the entry method,
    #    CT12c/d target the shared pipeline that performs the build+verify).
    $ctLbEntryBody = [regex]::Match($ctIndexTs, 'async linuxBuild\([\s\S]*?\r?\n  \}').Value
    $ctLbBody = [regex]::Match($ctIndexTs, 'linuxBundleBuildContainer\([\s\S]*?\r?\n  \}').Value
    Assert-That { $ctLbEntryBody -match 'Promise<Directory>' } 'CT12a linuxBuild returns the bundle artifact directory (exportable via dagger export --path)'
    Assert-That { $ctIndexTs -match [regex]::Escape('npx tauri build --target') -and $ctIndexTs -match [regex]::Escape('x86_64-unknown-linux-gnu') } 'CT12b linuxBuild reuses the current Tauri packaging (npx tauri build --target x86_64-unknown-linux-gnu, the same command .github/workflows/ci.yml runs)'
    Assert-That {
        $ctIndexTs -match [regex]::Escape('"/appimage/*.AppImage"') -and
        $ctIndexTs -match [regex]::Escape('"/deb/*.deb"') -and
        $ctLbBody -match [regex]::Escape('ReturnType.Any') -and
        $ctLbBody -match [regex]::Escape('missing artifacts fail the job')
    } 'CT12c linuxBuild verifies AppImage AND deb after the build and throws when artifacts are missing (Done criterion, Req 7.3)'
    Assert-That { $ctLbBody -match [regex]::Escape('linux-build.log') } 'CT12d linuxBuild captures the build log for export alongside the bundles'

    # b) engine-managed caches + once-provisioned layers in the build container
    $ctLbcBody = [regex]::Match($ctIndexTs, 'linuxBuildContainer\([\s\S]*?\r?\n  \}').Value
    Assert-That {
        $ctLbcBody -match [regex]::Escape('"/root/.npm"') -and
        $ctLbcBody -match [regex]::Escape('"/usr/local/cargo/registry"') -and
        $ctLbcBody -match [regex]::Escape('"/usr/local/cargo/git"') -and
        $ctLbcBody -match [regex]::Escape('"/src/src-tauri/target"')
    } 'CT12e linuxBuildContainer mounts npm registry + cargo registry/git/target as the same engine-managed cache volumes (key scheme unchanged)'
    Assert-That {
        $ctLbcBody -match [regex]::Escape('pins.tauriLinuxAptPackages') -and
        $ctLbcBody -match [regex]::Escape('pins.tauriLinuxBundleAptPackages')
    } 'CT12f linuxBuildContainer provisions the Tauri libs plus bundle extras once as cached layers (before the source mount)'
    Assert-That { $ctLbcBody -match [regex]::Escape('pins.node.image') -and $ctLbcBody -match [regex]::Escape('pins.rust.image') } 'CT12g the build container stays on the pinned rust+node images (node merged in, no unpinned installs)'

    # c) pins/contract honesty: executable gate, expected artifacts, extras parity
    Assert-That {
        $ctPinsTs -match [regex]::Escape('executable CI gate (task 2.2)') -and
        $ctPinsTs -match [regex]::Escape('appimage/*.AppImage') -and
        $ctPinsTs -match [regex]::Escape('deb/*.deb')
    } 'CT12h contract marks linux-build as an executable CI gate with AppImage/deb expected artifacts'
    $ctBundleExtras = @()
    if ($ctPins -and $ctPins.PSObject.Properties['tauriLinuxBundleAptPackages']) { $ctBundleExtras = @($ctPins.tauriLinuxBundleAptPackages) }
    Assert-That {
        $ctBundleExtras.Count -gt 0 -and
        @($ctBundleExtras | Where-Object { $ctPinsTs -notmatch [regex]::Escape(('"' + $_ + '"')) }).Count -eq 0
    } 'CT12i bundle apt extras stay mirrored between ci-versions.json and pins.ts'

    # d) tier behavior through shims (hermetic temp BuildArtifactsRoot so the
    #    real evidence directory is never touched): export chaining, the
    #    missing-artifact failure gate, and its per-entry scope.
    $ctLbRoot = Join-Path $TempRoot 'lb-artifacts-root'
    $ctOkShim = Join-Path $ctDaggerShimDir 'dagger-ok.ps1'
    Set-Content -LiteralPath $ctOkShim -Value @(
        'param()'
        'if ($env:DRIFT_DAGGER_RECORDER_OUT) { "ARGS=$($args -join '' '')" | Set-Content -LiteralPath $env:DRIFT_DAGGER_RECORDER_OUT -Encoding ascii }'
        'if ($env:DRIFT_DAGGER_FAKE_BUNDLES -eq "1") {'
        '  $pathIndex = [array]::IndexOf($args, "--path")'
        '  if ($pathIndex -ge 0 -and ($pathIndex + 1) -lt $args.Count) {'
        '    $exportDir = $args[$pathIndex + 1]'
        '    New-Item -ItemType Directory -Path $exportDir -Force | Out-Null'
        '    Set-Content -LiteralPath (Join-Path $exportDir "Drift_0.1.0_amd64.AppImage") -Value "fake-appimage"'
        '    Set-Content -LiteralPath (Join-Path $exportDir "Drift_0.1.0_amd64.deb") -Value "fake-deb"'
        '    Set-Content -LiteralPath (Join-Path $exportDir "linux-build.log") -Value "fake build log"'
        '  }'
        '}'
        'exit 0'
    ) -Encoding utf8
    $ctLbRecOut = Join-Path $ctDaggerShimDir 'lb-recorder-out.txt'
    $ctLb1 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'linux-build', '-DaggerCliPath', $ctRecorder, '-BuildArtifactsRoot', $ctLbRoot) -Environment @{ 'DRIFT_DAGGER_RECORDER_OUT' = $ctLbRecOut }
    Assert-That { $ctLb1.ExitCode -eq 7 } 'CT12j linux-build dagger tier propagates the shim exit code through the staging/export flow'
    $ctLbRec = ''
    if (Test-Path -LiteralPath $ctLbRecOut -PathType Leaf) { $ctLbRec = Get-Content -LiteralPath $ctLbRecOut -Raw }
    Assert-That { $ctLbRec -match 'linux-build' -and $ctLbRec -match 'export' -and $ctLbRec -match '--path' -and $ctLbRec -match [regex]::Escape('artifacts\linux-build') } 'CT12k linux-build entry call chains export --path into the scoped artifacts directory'
    $ctLb1Report = Read-RustTierReport -Path (Join-Path $ctLbRoot "$ProjectName\dagger\dagger-tier-report-linux-build.json")
    Assert-That { $null -ne $ctLb1Report -and $ctLb1Report.entry -eq 'linux-build' -and $ctLb1Report.daggerExitCode -eq 7 } 'CT12l dagger tier writes the per-entry report dagger-tier-report-linux-build.json'

    # e) the missing-artifact gate: exit-0 call WITHOUT exported bundles must
    #    FAIL the tier (Req 7.3: a green run with absent artifacts is a fake
    #    pass and must not happen).
    $ctLb2 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'linux-build', '-DaggerCliPath', $ctOkShim, '-BuildArtifactsRoot', $ctLbRoot) -Environment @{ 'DRIFT_DAGGER_FAKE_BUNDLES' = '0' }
    $ctLb2Combined = "{0}`n{1}" -f $ctLb2.StdOut, $ctLb2.StdErr
    Assert-That { $ctLb2.ExitCode -eq 1 } 'CT12m exit-0 dagger call with missing bundle artifacts fails the tier (exit 1)'
    Assert-That { $ctLb2Combined -match [regex]::Escape('missing') -and $ctLb2Combined -match 'AppImage' -and $ctLb2Combined -match 'deb' } 'CT12n the missing-artifact failure names the missing AppImage and deb bundles'
    $ctLb2Report = Read-RustTierReport -Path (Join-Path $ctLbRoot "$ProjectName\dagger\dagger-tier-report-linux-build.json")
    Assert-That {
        $null -ne $ctLb2Report -and
        $ctLb2Report.ok -eq $false -and
        @($ctLb2Report.failures).Count -gt 0 -and
        @($ctLb2Report.bundleArtifactsExported).Count -eq 0
    } 'CT12o the failing run reports ok=false with the missing-artifact failure recorded'

    # f) the positive control: exit-0 call WITH exported AppImage/deb + log
    #    passes the tier and the report records the bundles.
    $ctLb3 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'linux-build', '-DaggerCliPath', $ctOkShim, '-BuildArtifactsRoot', $ctLbRoot) -Environment @{ 'DRIFT_DAGGER_FAKE_BUNDLES' = '1' }
    Assert-That { $ctLb3.ExitCode -eq 0 } 'CT12p exit-0 dagger call with exported AppImage/deb + log passes the tier'
    $ctLb3Report = Read-RustTierReport -Path (Join-Path $ctLbRoot "$ProjectName\dagger\dagger-tier-report-linux-build.json")
    Assert-That {
        $null -ne $ctLb3Report -and
        $ctLb3Report.ok -eq $true -and
        @($ctLb3Report.bundleArtifactsExported).Count -eq 2 -and
        "$ctLb3Report.bundleLogExported" -match [regex]::Escape('linux-build.log')
    } 'CT12q the passing run records bundleArtifactsExported (AppImage+deb) and bundleLogExported in the report'

    # g) the gate is scoped to linux-build: a frontend exit-0 call without
    #    junit artifacts keeps the 2.1 behavior (pass; junit is logged only).
    $ctLb4 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'frontend', '-DaggerCliPath', $ctOkShim, '-BuildArtifactsRoot', $ctLbRoot) -Environment @{ 'DRIFT_DAGGER_FAKE_BUNDLES' = '0' }
    Assert-That { $ctLb4.ExitCode -eq 0 } 'CT12r the bundle gate does not change the frontend entry behavior (per-entry gate)'

    # h) README documents the executable bundle gate
    $ctReadmeLb = ''
    if (Test-Path -LiteralPath (Join-Path $RepoRoot '.dagger\README.md') -PathType Leaf) { $ctReadmeLb = Get-Content -LiteralPath (Join-Path $RepoRoot '.dagger\README.md') -Raw }
    Assert-That {
        $ctReadmeLb -match [regex]::Escape('artifacts') -and
        $ctReadmeLb -match [regex]::Escape('linux-build') -and
        $ctReadmeLb -match [regex]::Escape('missing artifacts fail the job') -and
        $ctReadmeLb -match [regex]::Escape('linux-build.log')
    } 'CT12s .dagger/README.md documents the linux-build export convention and the missing-artifact gate'

    # --- task 2.5: Linux cold-launch E2E bootstrap in Dagger — the BUILT app
    # launches headless under Xvfb with isolated app data (Req 12.1, 12.5,
    # 12.7, 12.8), the evidence (status, screenshots incl. the FAILURE
    # screenshot, logs) exports through the tier, and a FAILED cold launch
    # fails the tier with the failure screenshot exported — no fake pass.
    # Static + child-process assertions only: no engine or container is
    # required here (the live cold-launch run and the sabotage failure-path
    # demonstration are recorded in the task evidence).

    Write-Output '-- contract: CT13 task 2.5 linux-e2e cold launch: Xvfb launch, isolated app-data, evidence export, fail-loudly gate --'

    # a) the module entry runs the real cold launch and returns the evidence
    #    directory (exportable), reusing the bundle pipeline as prerequisite.
    $ctE2eBody = [regex]::Match($ctIndexTs, 'async linuxE2E\([\s\S]*?\r?\n  \}').Value
    Assert-That { $ctE2eBody -match 'Promise<Directory>' } 'CT13a linuxE2E returns the evidence directory (exportable via dagger export --path)'
    Assert-That { $ctE2eBody -match [regex]::Escape('cold-launch.sh') -and $ctE2eBody -match [regex]::Escape('linuxBundleBuildContainer') } 'CT13b linuxE2E chains the repo cold-launch harness on top of the verified bundle pipeline (built app, prerequisite gate)'
    Assert-That { $ctE2eBody -match [regex]::Escape('/test-results/e2e/cold-launch-status.json') } 'CT13c linuxE2E enforces the /test-results/e2e/cold-launch-status.json evidence contract'
    Assert-That {
        $ctE2eBody -match [regex]::Escape('ReturnType.Any') -and
        $ctE2eBody -match [regex]::Escape('produced no /test-results/e2e/cold-launch-status.json')
    } 'CT13d linuxE2E throws loudly when the harness produced no status (evidence loss is never a pass)'

    # b) the harness script: Xvfb, isolated XDG app data, failure screenshot
    #    BEFORE failing, graceful shutdown, no-FUSE AppImage path (Req 12.7).
    $ctHarnessPath = Join-Path $RepoRoot 'scripts\linux\cold-launch.sh'
    $ctHarness = ''
    if (Test-Path -LiteralPath $ctHarnessPath -PathType Leaf) { $ctHarness = Get-Content -LiteralPath $ctHarnessPath -Raw }
    Assert-That { $ctHarness -ne '' } 'CT13e the cold-launch harness exists at scripts/linux/cold-launch.sh (staged with the source)'
    Assert-That {
        $ctHarness -match 'Xvfb' -and
        $ctHarness -match [regex]::Escape('--appimage-extract-and-run')
    } 'CT13f the harness launches the built AppImage under Xvfb via --appimage-extract-and-run (no FUSE dependency)'
    Assert-That {
        $ctHarness -match [regex]::Escape('XDG_DATA_HOME') -and
        $ctHarness -match [regex]::Escape('XDG_CONFIG_HOME') -and
        $ctHarness -match [regex]::Escape('HOME=') -and
        $ctHarness -notmatch [regex]::Escape('XDG_DATA_HOME=/root')
    } 'CT13g the harness isolates app data per run (HOME + XDG_DATA_HOME/XDG_CONFIG_HOME/XDG_CACHE_HOME, never host or root state; Req 12.7)'
    Assert-That {
        $ctHarness -match [regex]::Escape('cold-launch-failure-screenshot') -and
        $ctHarness -match [regex]::Escape('capture_failure_screenshot')
    } 'CT13h the harness captures a failure screenshot BEFORE writing the failed status (screenshot-on-failure mandate)'
    Assert-That {
        $ctHarness -match [regex]::Escape('windowclose') -or
        $ctHarness -match [regex]::Escape('WM_DELETE_WINDOW')
    } 'CT13i the harness attempts graceful shutdown (WM_DELETE_WINDOW close) before any signal escalation'

    # c) pins parity: the e2e runtime extras are pinned in both files and the
    #    contract marks linux-e2e as the executable cold-launch gate.
    $ctE2eExtras = @()
    if ($ctPins -and $ctPins.PSObject.Properties['tauriLinuxE2EAptPackages']) { $ctE2eExtras = @($ctPins.tauriLinuxE2EAptPackages) }
    Assert-That {
        $ctE2eExtras.Count -gt 0 -and
        $ctE2eExtras -contains 'xvfb' -and
        @($ctE2eExtras | Where-Object { $ctPinsTs -notmatch [regex]::Escape(('"' + $_ + '"')) }).Count -eq 0
    } 'CT13j the e2e apt extras (incl. xvfb) stay mirrored between ci-versions.json and pins.ts'
    Assert-That {
        $ctPinsTs -match [regex]::Escape('executable cold-launch CI gate (task 2.5)') -and
        $ctPinsTs -match [regex]::Escape('cold-launch-failure-screenshot')
    } 'CT13k the contract marks linux-e2e as the executable cold-launch gate with the failure-screenshot evidence contract'

    # d) tier behavior through shims (hermetic temp BuildArtifactsRoot so the
    #    real evidence directory is never touched): export chaining, the
    #    failed-cold-launch gate, and the missing-status no-fake-pass gate.
    $ctE2eRoot = Join-Path $TempRoot 'e2e-artifacts-root'
    $ctE2eRecOut = Join-Path $ctDaggerShimDir 'e2e-recorder-out.txt'
    $ctE2e1 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'linux-e2e', '-DaggerCliPath', $ctRecorder, '-BuildArtifactsRoot', $ctE2eRoot) -Environment @{ 'DRIFT_DAGGER_RECORDER_OUT' = $ctE2eRecOut }
    Assert-That { $ctE2e1.ExitCode -eq 7 } 'CT13l linux-e2e dagger tier propagates the shim exit code through the staging/export flow'
    $ctE2eRec = ''
    if (Test-Path -LiteralPath $ctE2eRecOut -PathType Leaf) { $ctE2eRec = Get-Content -LiteralPath $ctE2eRecOut -Raw }
    Assert-That { $ctE2eRec -match [regex]::Escape('linux-e-2-e') -and $ctE2eRec -match 'export' -and $ctE2eRec -match '--path' -and $ctE2eRec -match [regex]::Escape('artifacts\linux-e2e') } 'CT13m linux-e2e entry call chains export --path into the scoped artifacts directory (evidence incl. failure screenshots)'

    #    ok-shim with three faces: 0 = exports nothing (no fake pass), 1 =
    #    exports passing cold-launch evidence, 2 = exports FAILED cold-launch
    #    evidence with the failure screenshot.
    $ctColdShim = Join-Path $ctDaggerShimDir 'dagger-cold.ps1'
    Set-Content -LiteralPath $ctColdShim -Value @(
        'param()'
        'if ($env:DRIFT_DAGGER_RECORDER_OUT) { "ARGS=$($args -join '' '')" | Set-Content -LiteralPath $env:DRIFT_DAGGER_RECORDER_OUT -Encoding ascii }'
        '$pathIndex = [array]::IndexOf($args, "--path")'
        'if ($pathIndex -ge 0 -and ($pathIndex + 1) -lt $args.Count) {'
        '  $exportDir = $args[$pathIndex + 1]'
        '  New-Item -ItemType Directory -Path $exportDir -Force | Out-Null'
        '  if ($env:DRIFT_DAGGER_FAKE_COLD_LAUNCH -eq "1") {'
        '    Set-Content -LiteralPath (Join-Path $exportDir "cold-launch-status.json") -Value ''{"ok":true,"windowId":"0x3c00007","exitCode":0,"shutdownPath":"graceful-close","distinctColors":128}'''
        '    Set-Content -LiteralPath (Join-Path $exportDir "cold-launch-screenshot.png") -Value "fake-png"'
        '  }'
        '  if ($env:DRIFT_DAGGER_FAKE_COLD_LAUNCH -eq "2") {'
        '    Set-Content -LiteralPath (Join-Path $exportDir "cold-launch-status.json") -Value ''{"ok":false,"reason":"drift-window-not-found","exitCode":1}'''
        '    Set-Content -LiteralPath (Join-Path $exportDir "cold-launch-failure-screenshot.png") -Value "fake-failure-png"'
        '  }'
        '}'
        'exit 0'
    ) -Encoding utf8
    $ctE2e2 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'linux-e2e', '-DaggerCliPath', $ctColdShim, '-BuildArtifactsRoot', $ctE2eRoot) -Environment @{ 'DRIFT_DAGGER_FAKE_COLD_LAUNCH' = '0' }
    $ctE2e2Combined = "{0}`n{1}" -f $ctE2e2.StdOut, $ctE2e2.StdErr
    Assert-That { $ctE2e2.ExitCode -eq 1 } 'CT13n exit-0 dagger call with NO cold-launch status fails the tier (no fake pass, Req 12.5, 12.8)'
    Assert-That { $ctE2e2Combined -match [regex]::Escape('cold-launch-status.json') } 'CT13o the missing-status failure names the cold-launch status contract'
    $ctE2e3 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'linux-e2e', '-DaggerCliPath', $ctColdShim, '-BuildArtifactsRoot', $ctE2eRoot) -Environment @{ 'DRIFT_DAGGER_FAKE_COLD_LAUNCH' = '2' }
    $ctE2e3Combined = "{0}`n{1}" -f $ctE2e3.StdOut, $ctE2e3.StdErr
    Assert-That { $ctE2e3.ExitCode -eq 1 } 'CT13p a FAILED cold launch (ok=false) fails the tier even though the dagger call exited 0 (fail loudly, Req 12.5)'
    Assert-That {
        $ctE2e3Combined -match [regex]::Escape('cold launch') -and
        $ctE2e3Combined -match [regex]::Escape('drift-window-not-found') -and
        $ctE2e3Combined -match [regex]::Escape('cold-launch-failure-screenshot.png')
    } 'CT13q the failed-cold-launch failure names the harness reason and the exported failure screenshot'
    $ctE2e3Report = Read-RustTierReport -Path (Join-Path $ctE2eRoot "$ProjectName\dagger\dagger-tier-report-linux-e2e.json")
    Assert-That {
        $null -ne $ctE2e3Report -and
        $ctE2e3Report.ok -eq $false -and
        $ctE2e3Report.coldLaunchOk -eq $false -and
        "$ctE2e3Report.coldLaunchFailureScreenshotExported" -match [regex]::Escape('cold-launch-failure-screenshot.png')
    } 'CT13r the failing run reports ok=false with coldLaunchOk=false and the failure screenshot recorded'
    $ctE2e4 = Invoke-ChildPreflight -Arguments @('-NoProfile', '-NonInteractive', '-File', $ScriptPath, '-Tier', 'dagger', '-DaggerEntry', 'linux-e2e', '-DaggerCliPath', $ctColdShim, '-BuildArtifactsRoot', $ctE2eRoot) -Environment @{ 'DRIFT_DAGGER_FAKE_COLD_LAUNCH' = '1' }
    Assert-That { $ctE2e4.ExitCode -eq 0 } 'CT13s exit-0 dagger call with exported ok=true status + screenshot passes the tier'
    $ctE2e4Report = Read-RustTierReport -Path (Join-Path $ctE2eRoot "$ProjectName\dagger\dagger-tier-report-linux-e2e.json")
    Assert-That {
        $null -ne $ctE2e4Report -and
        $ctE2e4Report.ok -eq $true -and
        $ctE2e4Report.coldLaunchOk -eq $true -and
        "$ctE2e4Report.coldLaunchScreenshotExported" -match [regex]::Escape('cold-launch-screenshot.png')
    } 'CT13t the passing run records coldLaunchOk=true and the exported screenshot in the report'

    # e) README documents the executable cold-launch gate and the Linux-only
    #    wording (Linux-container results never substitute native validation).
    $ctReadmeE2e = ''
    if (Test-Path -LiteralPath (Join-Path $RepoRoot '.dagger\README.md') -PathType Leaf) { $ctReadmeE2e = Get-Content -LiteralPath (Join-Path $RepoRoot '.dagger\README.md') -Raw }
    Assert-That {
        $ctReadmeE2e -match [regex]::Escape('linux-e2e') -and
        $ctReadmeE2e -match [regex]::Escape('cold-launch') -and
        $ctReadmeE2e -match [regex]::Escape('cold-launch-failure-screenshot')
    } 'CT13u .dagger/README.md documents the linux-e2e cold-launch gate with the failure-screenshot export'

    # --- task 2.6: thin Actions jobs — the workflow is a thin trigger/
    # artifact layer: portable/Linux checks run through the pinned Dagger
    # bridge on the runner's own Docker engine (dev-host twin:
    # scripts/Invoke-CI.ps1 -Tier dagger -DaggerEntry <entry>), Windows
    # stays a native pwsh/MSVC/WebView2 exception with NO foreign shell,
    # macOS keeps native bundle jobs for BOTH architectures (the task 2.4
    # macOS cold-launch/driver spike is wired as its own native job — CT15),
    # actions are pinned by commit SHA, no Dagger Cloud token or
    # secret is required, every gate is required, and failure evidence is
    # published (Req 7.1-7.5, 8.4, 12.5, 13.3, 13.4). Static contract
    # assertions only: the first live Actions run happens on push — the
    # structural proof is this section.
    Write-Output '-- contract: CT14 task 2.6 thin Actions jobs: Dagger Linux entries, native Windows/macOS exceptions, SHA-pinned actions --'

    $ctWfPath = Join-Path $RepoRoot '.github\workflows\ci.yml'
    $ctWf = ''
    if (Test-Path -LiteralPath $ctWfPath -PathType Leaf) { $ctWf = Get-Content -LiteralPath $ctWfPath -Raw }
    Assert-That { $ctWf -ne '' } 'CT14a the thin workflow exists at .github/workflows/ci.yml'

    # b) triggers retained: push + pull_request still start the workflow.
    Assert-That {
        $ctWf -match '(?m)^on:' -and
        $ctWf -match '(?m)^  push:' -and
        $ctWf -match '(?m)^  pull_request:'
    } 'CT14b push and pull_request triggers are retained (Req 7.1)'

    # c) every action use pinned by full commit SHA (no floating tags).
    #    (Both step forms: the "- uses:" shorthand and "uses:" under a named step.)
    $ctUses = @([regex]::Matches($ctWf, '(?m)^\s*(?:-\s+)?uses:\s*(\S+)') | ForEach-Object { $_.Groups[1].Value })
    Assert-That {
        $ctUses.Count -ge 15 -and
        @($ctUses | Where-Object { $_ -notmatch '@[0-9a-f]{40}$' }).Count -eq 0
    } 'CT14c every action use is pinned by full commit SHA (checkout/setup-node/upload-artifact/Dagger bridge; no floating tags)'

    # d) portable/Linux jobs are thin Dagger bridge calls, one per entry.
    $ctDaggerUses = @($ctUses | Where-Object { $_ -like 'dagger/dagger-for-github@*' })
    Assert-That { $ctDaggerUses.Count -eq 4 } 'CT14d all four portable/Linux jobs (frontend, rust-checks, linux-build, linux-e2e) run through dagger/dagger-for-github (thin Dagger bridge, Req 7.4)'
    Assert-That {
        $ctWf -match ('version:\s*"v?' + [regex]::Escape($ctPins.dagger.cliVersion) + '"') -and
        $ctWf -notmatch 'version:\s*"latest"'
    } 'CT14e the Dagger bridge version input equals the ci-versions.json CLI pin (never latest)'
    Assert-That {
        $ctWf -match [regex]::Escape('call: frontend --source . export --path ci-artifacts/frontend') -and
        $ctWf -match [regex]::Escape('call: rust-checks --source . export --path ci-artifacts/rust-checks') -and
        $ctWf -match [regex]::Escape('call: linux-build --source . export --path ci-artifacts/linux-build')
    } 'CT14f frontend, rust-checks and linux-build entries are called with --source . and export into scoped ci-artifacts dirs (Req 7.1, 7.2)'
    Assert-That { $ctWf -match [regex]::Escape('call: linux-e-2-e --source . export --path ci-artifacts/linux-e2e') } 'CT14g the linux-e2e job calls the SDK-registered linux-e-2-e entry (kebab mapping) with evidence export (Req 12.5)'
    Assert-That {
        $ctWf -notmatch '_EXPERIMENTAL_DAGGER_RUNNER_HOST' -and
        $ctWf -match [regex]::Escape('-Tier dagger -DaggerEntry')
    } 'CT14h Actions Linux runners use their own Docker engine (no image+podman runner host here); the dev-host twin tier command stays documented'

    # i) native Windows exception: pwsh-only tier commands plus installer.
    $ctWinBlock = [regex]::Match($ctWf, '(?ms)^  windows-native:.*?(?=^  [a-z][a-z0-9-]*:|\z)').Value
    Assert-That { $ctWinBlock -ne '' -and $ctWinBlock -match 'runs-on:\s*windows-[0-9]+' } 'CT14i the native Windows job exists on an explicit Windows image (Req 8.4)'
    $ctWinRunCount = @([regex]::Matches($ctWinBlock, '(?m)^\s*run:')).Count
    $ctWinShells = @([regex]::Matches($ctWinBlock, 'shell:\s*(\S+)') | ForEach-Object { $_.Groups[1].Value })
    Assert-That {
        $ctWinRunCount -ge 6 -and
        $ctWinShells.Count -eq $ctWinRunCount -and
        @($ctWinShells | Where-Object { $_ -ne 'pwsh' }).Count -eq 0
    } 'CT14j every Windows run step declares shell: pwsh (native shells only, Req 7.5)'
    Assert-That {
        $ctWinBlock.Length -gt 100 -and
        $ctWinBlock -notmatch 'wsl' -and
        $ctWinBlock -notmatch 'bash'
    } 'CT14k the Windows job contains no wsl and no bash reference anywhere in its definition'
    Assert-That {
        $ctWinBlock -match '(?m)^\s*run:\s*pwsh -NoProfile -File scripts/Invoke-CI\.ps1\s*$' -and
        $ctWinBlock -match [regex]::Escape('-Tier frontend') -and
        $ctWinBlock -match [regex]::Escape('-Tier rust')
    } 'CT14l the Windows job invokes the canonical native tier commands (preflight, frontend, rust) — one command per tier (Req 13.4)'
    Assert-That {
        $ctWinBlock -match ('rustup toolchain install ' + [regex]::Escape($ctPins.rust.toolchain)) -and
        $ctWinBlock -match ('cargo install cargo-nextest --locked --version ' + [regex]::Escape($ctPins.nextest.version))
    } 'CT14m the Windows toolchain and cargo-nextest installs match the ci-versions.json pins'
    Assert-That {
        $ctWinBlock -match 'npx tauri build' -and
        $ctWinBlock -match [regex]::Escape('bundle/nsis/*.exe') -and
        $ctWinBlock -match 'if-no-files-found:\s*error'
    } 'CT14n the Windows job builds and publishes the NSIS installer; a missing installer fails the job (Req 8.4, no fake pass)'
    Assert-That { $ctWinBlock -match [regex]::Escape('D:\BuildArtifacts\drift-rdp') } 'CT14o the Windows job publishes the tiers scoped failure reports under D:\BuildArtifacts\drift-rdp (Req 12.7)'

    # j) native macOS exception: bundle jobs for BOTH architectures.
    $ctMacBlock = [regex]::Match($ctWf, '(?ms)^  macos-bundle:.*?(?=^  [a-z][a-z0-9-]*:|\z)').Value
    Assert-That {
        $ctMacBlock -match [regex]::Escape('os: macos-15') -and
        $ctMacBlock -match [regex]::Escape('target: aarch64-apple-darwin') -and
        $ctMacBlock -match [regex]::Escape('os: macos-15-intel') -and
        $ctMacBlock -match [regex]::Escape('target: x86_64-apple-darwin')
    } 'CT14p the macOS bundle matrix preserves BOTH architectures (aarch64 + x86_64, Req 7.2)'
    Assert-That {
        $ctMacBlock -match 'fail-fast:\s*false' -and
        $ctMacBlock -match 'npx tauri build --target \$\{\{ matrix\.target \}\}'
    } 'CT14q the macOS matrix never cancels the sibling architecture and builds each target natively'
    Assert-That {
        $ctMacBlock -match [regex]::Escape('*.dmg') -and
        $ctMacBlock -match 'if-no-files-found:\s*error'
    } 'CT14r missing macOS bundle artifacts fail the job (Req 7.2, 7.3)'
    Assert-That {
        $ctMacBlock -match ('rustup toolchain install ' + [regex]::Escape($ctPins.rust.toolchain)) -and
        $ctMacBlock -notmatch 'dagger/dagger-for-github'
    } 'CT14s the macOS job is native (no Dagger) and pins the rust toolchain from ci-versions.json'

    # k) runner-side twins of the tier gates: bundle re-check + verdict.
    $ctLbBlock = [regex]::Match($ctWf, '(?ms)^  dagger-linux-build:.*?(?=^  [a-z][a-z0-9-]*:|\z)').Value
    Assert-That {
        $ctLbBlock -match [regex]::Escape('*.AppImage') -and
        $ctLbBlock -match [regex]::Escape('*.deb') -and
        $ctLbBlock -match [regex]::Escape('linux-build.log') -and
        $ctLbBlock -match 'if:\s*always\(\)'
    } 'CT14t linux-build re-verifies the exported AppImage+deb+log on the runner (tier double gate, Req 7.2, 7.3)'
    $ctE2eBlock = [regex]::Match($ctWf, '(?ms)^  dagger-linux-e2e:.*?(?=^  [a-z][a-z0-9-]*:|\z)').Value
    Assert-That {
        $ctE2eBlock -match [regex]::Escape('cold-launch-status.json') -and
        $ctE2eBlock -match [regex]::Escape('.ok == true') -and
        $ctE2eBlock -match 'if:\s*always\(\)'
    } 'CT14u the linux-e2e verdict step fails the job unless the exported cold-launch status is ok=true (fail loudly, Req 12.5)'
    Assert-That {
        $ctE2eBlock -match [regex]::Escape('cold-launch-screenshot.png') -and
        $ctE2eBlock -match [regex]::Escape('first-usable-state screenshot')
    } 'CT14ab the linux-e2e verdict also requires the exported first-usable-state screenshot when ok=true — the runner twin mirrors the dev-host tier gate (Invoke-CI.ps1 ok-without-screenshot check, task 2.7 hardening; no fake pass)'

    # v) gates required, evidence published, no secrets, honest limitations.
    Assert-That { $ctWf -notmatch 'continue-on-error' } 'CT14v no step continues past a failed gate — every gate is required (Req 7.3, 13.3)'
    $ctAlwaysCount = @([regex]::Matches($ctWf, 'if:\s*always\(\)')).Count
    $ctUploadUses = @($ctUses | Where-Object { $_ -like 'actions/upload-artifact@*' })
    Assert-That {
        $ctAlwaysCount -ge 8 -and
        $ctUploadUses.Count -eq 8
    } 'CT14w every job publishes its reports/artifacts via if: always() uploads (failure evidence included; 8 upload steps across the 7 jobs incl. the task 2.4 macOS spike)'
    Assert-That {
        $ctWf -notmatch 'DAGGER_CLOUD_TOKEN' -and
        $ctWf -notmatch 'cloud-token' -and
        $ctWf -notmatch 'secrets\.'
    } 'CT14x no Dagger Cloud token or repository secret is referenced (PR checks need none)'
    Assert-That {
        $ctWf -notmatch 'runs-on:\s*(ubuntu|windows|macos)-latest' -and
        @([regex]::Matches($ctWf, 'runs-on:\s*ubuntu-24\.04')).Count -ge 4
    } 'CT14y runner images are explicit (never -latest labels)'
    Assert-That {
        @([regex]::Matches($ctWf, ('node-version:\s*"?' + [regex]::Escape($ctPins.node.version)))).Count -ge 2
    } 'CT14z Actions node version matches the ci-versions.json node pin'
    Assert-That {
        $ctWf -match [regex]::Escape('task 2.4') -and
        $ctWf -match [regex]::Escape('CI-runner spike') -and
        $ctWf -match [regex]::Escape('never substitute')
    } 'CT14aa the workflow records the task 2.4 macOS driver scope (converted CI-runner spike, user decision 2026-11-11) and never presents Linux-container results as native validation'

    # ─── CT15: task 2.4 — macOS built-app cold launch + WebdriverIO
    # Tauri-service driver proof as a CI-runner spike (user decision
    # 2026-11-11: no macOS dev host exists, so the spike runs on an Actions
    # macOS runner). Design mandate (design.md "Testing Strategy → Delivery
    # gates and brownfield coverage"): drive macOS through the WebdriverIO
    # Tauri service EMBEDDED WebDriver server — never safaridriver, never
    # direct tauri-driver (neither drives WKWebView). The job must fail
    # loudly with exported evidence on any driver failure (Req 12.5, 12.8),
    # isolate its app data from the runner profile (Req 12.7), and keep the
    # spike scoped away from a top-level e2e/ directory (linux-e2e contract).
    Write-Output '-- contract: CT15 task 2.4 macOS cold-launch spike job: embedded Tauri-service driver, isolation, fail-loudly verdict, no e2e/ flip --'

    $ctSpikeBlock = [regex]::Match($ctWf, '(?ms)^  macos-cold-launch-e2e-spike:.*?(?=^  [a-z][a-z0-9-]*:|\z)').Value
    Assert-That {
        $ctSpikeBlock -ne '' -and
        $ctSpikeBlock -match 'runs-on:\s*macos-15'
    } 'CT15a the task 2.4 spike job exists on the explicit macos-15 (aarch64) runner image — native macOS validation, never a container substitute (Req 12.5, 12.8)'
    Assert-That {
        $ctSpikeBlock -match [regex]::Escape('npx tauri build --target aarch64-apple-darwin') -and
        $ctSpikeBlock -match ('rustup toolchain install ' + [regex]::Escape($ctPins.rust.toolchain)) -and
        $ctSpikeBlock -match ('node-version:\s*"?' + [regex]::Escape($ctPins.node.version))
    } 'CT15b the spike builds the UNMODIFIED old-stack app with the proven macos-bundle command and the ci-versions.json rust/node pins'
    Assert-That {
        $ctSpikeBlock -notmatch 'dagger/dagger-for-github' -and
        $ctWf -notmatch 'safaridriver' -and
        $ctWf -notmatch 'crabnebula' -and
        $ctWf -notmatch 'CN_API_KEY'
    } 'CT15c macOS stays a native exception; the driver path is neither safaridriver nor the secret-keyed CrabNebula driver (design.md embedded-server mandate)'
    $ctWdioConfPath = Join-Path $RepoRoot 'scripts\macos\e2e-spike\wdio.conf.js'
    $ctWdioConf = ''
    if (Test-Path -LiteralPath $ctWdioConfPath -PathType Leaf) { $ctWdioConf = Get-Content -LiteralPath $ctWdioConfPath -Raw }
    Assert-That {
        $ctWdioConf -ne '' -and
        $ctWdioConf -match [regex]::Escape("'@wdio/tauri-service'") -and
        $ctWdioConf -match [regex]::Escape("driverProvider: 'embedded'") -and
        $ctWdioConf -match [regex]::Escape('DRIFT_APP_BINARY')
    } 'CT15d the spike wdio config drives the built app through the @wdio/tauri-service embedded WebDriver server (design.md: direct tauri-driver/safaridriver does not drive WKWebView)'
    $ctSpikePkgPath = Join-Path $RepoRoot 'scripts\macos\e2e-spike\package.json'
    $ctSpikePkg = $null
    if (Test-Path -LiteralPath $ctSpikePkgPath -PathType Leaf) {
        try { $ctSpikePkg = Get-Content -LiteralPath $ctSpikePkgPath -Raw | ConvertFrom-Json } catch { $ctSpikePkg = $null }
    }
    Assert-That {
        $null -ne $ctSpikePkg -and
        $null -ne $ctSpikePkg.devDependencies -and
        $null -ne $ctSpikePkg.devDependencies.'@wdio/tauri-service' -and
        $null -ne $ctSpikePkg.devDependencies.webdriverio -and
        $ctSpikePkg.devDependencies.webdriverio -match '^9\.' -and
        @($ctSpikePkg.devDependencies.PSObject.Properties | Where-Object { "$($_.Value)" -notmatch '^\d+\.\d+\.\d+$' }).Count -eq 0
    } 'CT15e spike npm deps are exact-pinned (no ^/~ ranges) on the WebdriverIO 9 line with @wdio/tauri-service (design.md Allowed Dependencies; service peer is webdriverio ^9)'
    $ctSpikeShPath = Join-Path $RepoRoot 'scripts\macos\cold-launch-spike.sh'
    $ctSpikeSh = ''
    if (Test-Path -LiteralPath $ctSpikeShPath -PathType Leaf) { $ctSpikeSh = Get-Content -LiteralPath $ctSpikeShPath -Raw }
    Assert-That {
        $ctSpikeSh -match 'export HOME=' -and
        $ctSpikeSh -match [regex]::Escape('run-home') -and
        $ctSpikeSh -match 'export TMPDIR='
    } 'CT15f the spike harness scopes HOME and TMPDIR into a per-run evidence tree — Tauri app data and WKWebView state are isolated from the runner user profile (Req 12.7)'
    Assert-That {
        $ctSpikeBlock -match [regex]::Escape('cold-launch-driver-status.json') -and
        $ctSpikeBlock -match [regex]::Escape('first-usable-state.png') -and
        $ctSpikeBlock -match 'if:\s*always\(\)'
    } 'CT15g the spike verdict fails the job on a missing/not-ok status or an ok-without-screenshot fake pass, and evidence uploads run if: always() (Req 12.5, 12.8)'
    Assert-That {
        -not (Test-Path -LiteralPath (Join-Path $RepoRoot 'e2e')) -and
        (Test-Path -LiteralPath $ctSpikeShPath -PathType Leaf) -and
        (Test-Path -LiteralPath $ctWdioConfPath -PathType Leaf)
    } 'CT15h the spike lives under scripts/macos/ and creates NO top-level e2e/ directory — the linux-e2e e2e/-absence contract stays untouched until group 12 wires the real suite'
    Assert-That {
        $ctWf -match [regex]::Escape('native macOS') -and
        $ctSpikeBlock -match 'timeout-minutes:'
    } 'CT15i the workflow labels the spike native macOS validation and bounds the job with a timeout (3h cap spirit; runner wait excluded)'

    # ─── CT15 run-1 hardening (live run 43077191197, cancelled at the 60-min
    # job timeout): the CR-stripped harness ran from /tmp so dirname-based
    # REPO_ROOT resolved to / (os-window-probe.txt: "error opening input file
    # '/scripts/macos/window-probe.swift'"), the window probe failed all 60
    # iterations, and the stage-1 failure branch's UNBOUNDED wait on the
    # healthy GUI app hung until GitHub cancelled the job. These assertions
    # pin the fixes: workflow-provided repo root, once-compiled probe,
    # per-stage wall-time caps that write explicit failure statuses, and
    # bounded reaps only.
    Assert-That {
        $ctSpikeBlock -match [regex]::Escape('DRIFT_REPO_ROOT') -and
        $ctSpikeBlock -match [regex]::Escape('github.workspace')
    } 'CT15j the spike step passes DRIFT_REPO_ROOT (github.workspace) so the /tmp-invoked harness cannot mis-resolve the repo root (run-43077191197 root cause)'
    Assert-That {
        $ctSpikeSh -match 'swiftc' -and
        $ctSpikeSh -match [regex]::Escape('window-probe.bin') -and
        $ctSpikeSh -notmatch [regex]::Escape('swift "$REPO_ROOT')
    } 'CT15k the window probe is compiled ONCE with swiftc into window-probe.bin and the poll loop never re-invokes the swift interpreter per iteration'
    Assert-That {
        $ctSpikeSh -match 'STAGE1_CAP_SECS' -and
        $ctSpikeSh -match 'STAGE2_CAP_SECS' -and
        $ctSpikeSh -match 'stage1-timeout' -and
        $ctSpikeSh -match 'stage2-timeout'
    } 'CT15l each stage carries a hard wall-time cap that writes an explicit failure status and exits the harness cleanly — the GitHub job timeout is never the terminator'
    Assert-That {
        $ctSpikeSh -match 'kill_bounded' -and
        @([regex]::Matches($ctSpikeSh, 'wait "')).Count -le 2
    } 'CT15m app teardown goes through a bounded TERM/KILL+reap helper — no unbounded wait on a live GUI process can hang the job again (run-43077191197 hang)'
} finally {
    # Clean up only this test's own resources (Req 12.7): remove the junction
    # reparse point explicitly first so nothing can traverse into the repo.
    $junctionCleanup = Join-Path $TempRoot 'repo-view'
    if (Test-Path -LiteralPath $junctionCleanup) { Remove-Item -LiteralPath $junctionCleanup -Force -ErrorAction SilentlyContinue }
    if (Test-Path -LiteralPath $TempRoot) { Remove-Item -LiteralPath $TempRoot -Recurse -Force -ErrorAction SilentlyContinue }
}

# --- post-cleanup sanity: the repository itself is untouched ---
Assert-That { Test-Path -LiteralPath (Join-Path $RepoRoot 'package.json') -PathType Leaf } 'T99 sanity: repository intact after cleanup (package.json present)'

Write-Output ("== summary: {0} passed, {1} failed ==" -f $script:Passed, $script:Failed)
foreach ($failedName in $script:FailedNames) { Write-Output ("  failed: {0}" -f $failedName) }
if ($script:Failed -gt 0) { exit 1 }
exit 0
