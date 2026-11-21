<#
.SYNOPSIS
  CPU per second of a running 1-1 on Windows: main thread, the app process,
  and its WebView2 processes (renderer, GPU, browser, others). No admin needed.

.DESCRIPTION
  Samples processor time once per second for -Seconds seconds and prints one
  row per second plus averages and maxima. Percentages are of one core, as in
  docs/profiling-linux.md. The main thread is the app's oldest thread (the one
  that runs the window and the WebView2 controller).

  For call stacks and GPU time use Windows Performance Recorder instead
  (docs/profiling-windows.md).

.EXAMPLE
  pwsh -NoProfile -File scripts/windows/Measure-AppCpu.ps1 -Seconds 30 -Name idle -OutDirectory D:/BuildArtifacts/drift-rdp/profiles
#>
[CmdletBinding()]
param(
    [int]$Seconds = 30,
    [string]$Name = 'run',
    [string]$ProcessName = '',
    [string]$OutDirectory = ''
)
$ErrorActionPreference = 'Stop'

function Find-App {
    $names = if ($ProcessName) { @($ProcessName) } else { @('1-1', 'rdp-client') }
    $found = @(Get-Process -Name $names -ErrorAction SilentlyContinue)
    if ($found.Count -ne 1) { throw "Expected one running 1-1 process ($($names -join ', ')), found $($found.Count)." }
    $found[0]
}

# WebView2 processes are the app's descendants: the browser process is its
# child, renderer and GPU processes are children of the browser process.
function Get-WebViewProcesses([int]$rootId) {
    $all = Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'"
    $ids = [Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($rootId)
    do {
        $added = $false
        foreach ($p in $all) {
            if ($ids.Contains([int]$p.ParentProcessId) -and $ids.Add([int]$p.ProcessId)) { $added = $true }
        }
    } while ($added)
    foreach ($p in $all) {
        if (-not $ids.Contains([int]$p.ProcessId)) { continue }
        $kind = if ($p.CommandLine -match '--type=([a-z-]+)') {
            switch ($Matches[1]) { 'renderer' { 'renderer' } 'gpu-process' { 'gpu' } default { 'other' } }
        } else { 'browser' }
        [pscustomobject]@{ Id = [int]$p.ProcessId; Kind = $kind }
    }
}

function Get-Times($app, $webviews) {
    $app.Refresh()
    $main = $app.Threads | Sort-Object { try { $_.StartTime } catch { [datetime]::MaxValue } } | Select-Object -First 1
    $t = @{ main = $main.TotalProcessorTime.TotalMilliseconds; app = $app.TotalProcessorTime.TotalMilliseconds
            renderer = 0.0; gpu = 0.0; browser = 0.0; other = 0.0 }
    foreach ($w in $webviews) {
        $p = Get-Process -Id $w.Id -ErrorAction SilentlyContinue
        if ($p) { $t[$w.Kind] += $p.TotalProcessorTime.TotalMilliseconds }
    }
    $t
}

$app = Find-App
$webviews = @(Get-WebViewProcesses $app.Id)
Write-Host ("1-1 pid {0}; WebView2: {1}" -f $app.Id, (($webviews | Group-Object Kind | ForEach-Object { "$($_.Count) $($_.Name)" }) -join ', '))
Write-Host "Recording '$Name' for $Seconds s ..."

$keys = 'main', 'app', 'renderer', 'gpu', 'browser', 'other'
$rows = [Collections.Generic.List[object]]::new()
$clock = [Diagnostics.Stopwatch]::StartNew()
$prev = Get-Times $app $webviews
$prevMs = 0.0
for ($i = 1; $i -le $Seconds; $i++) {
    $next = $i * 1000
    $wait = $next - $clock.ElapsedMilliseconds
    if ($wait -gt 0) { [Threading.Thread]::Sleep([int]$wait) }
    $now = Get-Times $app $webviews
    $ms = $clock.Elapsed.TotalMilliseconds
    $span = $ms - $prevMs
    $row = [ordered]@{ second = $i }
    foreach ($k in $keys) { $row[$k] = [math]::Round(100 * ($now[$k] - $prev[$k]) / $span, 1) }
    $row['webview'] = [math]::Round($row.renderer + $row.gpu + $row.browser + $row.other, 1)
    $row['whole'] = [math]::Round($row.app + $row.webview, 1)
    $rows.Add([pscustomobject]$row)
    $prev = $now; $prevMs = $ms
}

$rows | Format-Table -AutoSize | Out-String | Write-Host
$columns = 'main', 'app', 'renderer', 'gpu', 'webview', 'whole'
$summary = foreach ($c in $columns) {
    $m = $rows | Measure-Object -Property $c -Average -Maximum
    [pscustomobject]@{ measure = $c; average = [math]::Round($m.Average, 1); max = [math]::Round($m.Maximum, 1) }
}
Write-Host "Summary for '$Name' (% of one core):"
$summary | Format-Table -AutoSize | Out-String | Write-Host

if ($OutDirectory) {
    $dir = [IO.Path]::GetFullPath($OutDirectory)
    if ([IO.Path]::GetPathRoot($dir) -eq $dir) { throw 'Refusing a drive root as the output directory.' }
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $rows | Export-Csv -LiteralPath (Join-Path $dir "$stamp-$Name.csv")
    Write-Host "Saved $(Join-Path $dir "$stamp-$Name.csv")"
}
