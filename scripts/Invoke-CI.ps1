#Requires -Version 7.0
<#
.SYNOPSIS
  Native (pwsh) entry point for drift-rdp CI tier commands.

.DESCRIPTION
  Implements the native tier entry point required by design.md ("CI ownership
  and reuse decisions": scripts/Invoke-CI.ps1 provides native tier entry
  points, also invoked by CI). Task 1.1 ships the preflight tier; later tasks
  (1.2-1.4) extend this script with further tiers — keep functions small and
  composable.

  The preflight (Requirements 7.1, 7.5, 12.7, 13.4):
    - reports required native tool versions and resolved paths;
    - inspects wrapper targets BEFORE execution and rejects WSL / Git Bash
      (MSYS, Cygwin) delegation, naming the offending path;
    - rejects the C:\Users\mariu\Source\drift-rdp junction view as working
      directory in favor of the canonical D:\Source\drift-rdp repository path,
      so cache keys and BuildArtifacts scoping stay stable;
    - scopes every artifact it produces under D:\BuildArtifacts\drift-rdp\...
      (never a drive root) and cleans up only its own resources.

  The preflight honors existing tool caches (npm, cargo); it neither creates
  nor relocates them.

  Single command per tier (Req 13.4):
    preflight:    pwsh -NoProfile -File scripts/Invoke-CI.ps1
    its test tier: pwsh -NoProfile -File scripts/Invoke-CI.Tests.ps1

  Exit codes: 0 = preflight passed; 1 = rejected/failed (reasons on stderr).

  Wrapper inspection covers reparse/symlink target chains, prohibited path
  roots (WSL filesystem views, Git\usr\bin, MSYS, Cygwin), prohibited
  interpreter leaves, and text-delegation patterns inside script shims
  (.cmd/.bat/.ps1/.psm1/.sh/.py). Native .exe binaries cannot contain a
  textual shim and are validated by path rules only.
#>
[CmdletBinding()]
param(
    # Tier to run; later tasks add further tiers.
    [ValidateSet('preflight')]
    [string]$Tier = 'preflight',

    # Override the effective working directory (used by tests; defaults to the
    # real current location so junction views are detected, never bypassed).
    [string]$WorkingDirectory = '',

    [string]$BuildArtifactsRoot = 'D:\BuildArtifacts',
    [string]$ProjectName = 'drift-rdp',
    [switch]$NoReport
)

Set-StrictMode -Version 3.0

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
    $preflightExitCode = Invoke-Preflight -Tier $Tier `
        -WorkingDirectory $WorkingDirectory `
        -BuildArtifactsRoot $BuildArtifactsRoot `
        -ProjectName $ProjectName `
        -NoReport:$NoReport
    exit $preflightExitCode
}
