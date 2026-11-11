#Requires -Version 7.0
<#
.SYNOPSIS
  Test tier for scripts/Invoke-CI.ps1 — native CI entry points (task 1.1
  preflight tier, task 1.2 rust test tier).

.DESCRIPTION
  Single command (Req 13.4):  pwsh -NoProfile -File scripts/Invoke-CI.Tests.ps1

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
