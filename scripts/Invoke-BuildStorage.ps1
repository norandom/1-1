[CmdletBinding(SupportsShouldProcess, ConfirmImpact = 'Low')]
param(
    [ValidateSet('Check', 'CleanProject')][string]$Action = 'Check'
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $repo 'src-tauri/Cargo.toml'
if (-not $env:CARGO_TARGET_DIR) { throw 'CARGO_TARGET_DIR must explicitly name the build cache.' }
$target = (Resolve-Path -LiteralPath $env:CARGO_TARGET_DIR).Path.TrimEnd('\', '/')
$allowed = (Resolve-Path -LiteralPath 'D:/BuildArtifacts').Path.TrimEnd('\', '/') + '\'
if (-not $target.StartsWith($allowed, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing a drive root, BuildArtifacts root, or target outside D:/BuildArtifacts.'
}
# Reject a redirected target rather than accidentally cleaning another location.
$ancestor = Get-Item -LiteralPath $target
while ($ancestor -and $ancestor.FullName.StartsWith($allowed, [StringComparison]::OrdinalIgnoreCase)) {
    if ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing a target reached through a reparse point.' }
    $ancestor = $ancestor.Parent
}
$busy = @(Get-Process -Name cargo,rustc,cargo-nextest,link,lld-link -ErrorAction SilentlyContinue)
if ($Action -eq 'CleanProject' -and $busy.Count) { throw 'A Cargo/compiler/linker job is active. Wait before deleting build artifacts.' }
$runningOutput = @(Get-Process | Where-Object {
    try { $_.Path -and $_.Path.StartsWith($target + '\', [StringComparison]::OrdinalIgnoreCase) } catch { $false }
})
if ($Action -eq 'CleanProject' -and $runningOutput.Count) { throw 'An executable is running from this target directory; refusing cleanup.' }
$reportDir = 'D:/BuildArtifacts/drift-rdp/storage'
New-Item -ItemType Directory -Path $reportDir -Force | Out-Null
Push-Location $repo
try {
    # Cargo identifies package artifacts; never guess ownership from app_lib filenames.
    $preview = @(& cargo clean --manifest-path $manifest --target-dir $target --package rdp-client --dry-run --verbose --color never 2>&1)
    if ($LASTEXITCODE -ne 0) { throw 'Cargo cleanup preview failed.' }
    $preview | Set-Content -LiteralPath (Join-Path $reportDir 'project-clean-preview.log')
    $paths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $preview) {
        $line = "$entry".Trim()
        if ($line.StartsWith($target + '\', [StringComparison]::OrdinalIgnoreCase)) { [void]$paths.Add($line) }
    }
    [long]$projectBytes = 0
    foreach ($path in $paths) { if ([IO.File]::Exists($path)) { $projectBytes += ([IO.FileInfo]::new($path)).Length } }
    if (-not ($preview -match 'Summary|Removed')) { throw 'Unrecognized Cargo preview; refusing to infer an empty cache.' }
    [long]$sharedBytes = 0
    foreach ($path in [IO.Directory]::EnumerateFiles($target, '*', [IO.SearchOption]::AllDirectories)) {
        $sharedBytes += ([IO.FileInfo]::new($path)).Length
    }
    $report = [ordered]@{ timestamp = [DateTime]::UtcNow.ToString('o'); target = $target; package = 'rdp-client'; projectGiB = [math]::Round($projectBytes / 1GB, 2); sharedLogicalGiB = [math]::Round($sharedBytes / 1GB, 2); note = 'Optional report only; no build limits. Logical sizes can differ from physical usage.' }
    $report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $reportDir 'latest.json')
    $report | ConvertTo-Json
    if ($Action -eq 'CleanProject') {
        if ($PSCmdlet.ShouldProcess("rdp-client artifacts in $target", 'cargo clean --package rdp-client')) {
            & cargo clean --manifest-path $manifest --target-dir $target --package rdp-client --color never
            if ($LASTEXITCODE -ne 0) { throw 'Package cleanup failed.' }
        }
    }
} finally { Pop-Location }
