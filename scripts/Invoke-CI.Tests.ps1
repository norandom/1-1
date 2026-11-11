#Requires -Version 7.0
<#
.SYNOPSIS
  Test tier for scripts/Invoke-CI.ps1 — the native preflight entry point (task 1.1).

.DESCRIPTION
  Single command (Req 13.4):  pwsh -NoProfile -File scripts/Invoke-CI.Tests.ps1

  Lightweight, self-contained runner: plain pwsh assertions over exit codes,
  messages and produced files. No Pester or other new dependency frameworks
  (tasks.md effort controls). Disposables are created under
  D:\BuildArtifacts\Temp\opencode\preflight-tests and removed at the end;
  only this test's own resources are cleaned up (Req 12.7).

  Wrapper-rejection fixtures are echo-only: even if the preflight wrongly
  executed them, no WSL or bash process would ever be spawned.
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
