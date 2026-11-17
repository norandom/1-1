$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$fixture = 'D:\BuildArtifacts\drift-rdp\releases\packaging-tests\' + [guid]::NewGuid().ToString('N')
New-Item -ItemType Directory -Path (Join-Path $fixture 'frontend') -Force | Out-Null
'<!doctype html><html><body>fixture</body></html>' | Set-Content -LiteralPath (Join-Path $fixture 'frontend/index.html')
$configPath = & (Join-Path $PSScriptRoot 'Invoke-WindowsPreview.ps1') -PrepareOnly -OutputDirectory $fixture
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
# This recreates the alpha.1 bug: the drive prefix is also an absolute URI scheme.
if (-not ([uri]'D:/BuildArtifacts/frontend').IsAbsoluteUri) { throw 'Absolute-path regression fixture is invalid.' }
if ([IO.Path]::IsPathRooted($config.build.frontendDist) -or $config.build.frontendDist -match '^[a-zA-Z][a-zA-Z0-9+.-]*:') {
    throw 'REGRESSION: frontendDist is an absolute path/URL instead of embedded relative assets.'
}
$resolved = [IO.Path]::GetFullPath((Join-Path (Join-Path $repo 'src-tauri') $config.build.frontendDist))
if ($resolved -ne (Join-Path $fixture 'frontend')) { throw 'Relative frontendDist resolves to the wrong directory.' }
if ($config.build.beforeBuildCommand -ne '') { throw 'Preview must not invoke the default repository dist build.' }
foreach ($bad in @('D:\', 'D:\BuildArtifacts', 'D:\BuildArtifacts\drift-rdp\releases', 'D:\BuildArtifacts\drift-rdp\releases\', 'C:\outside-preview', 'https://example.test/frontend', (Join-Path $fixture 'missing'))) {
    $rejected = $false
    try { & (Join-Path $PSScriptRoot 'Invoke-WindowsPreview.ps1') -PrepareOnly -OutputDirectory $bad | Out-Null }
    catch { $rejected = $true }
    if (-not $rejected) { throw "Unsafe or missing output was accepted: $bad" }
}
'PASS: relative embedding path, correct resolution, disabled default hook, root/outside/missing-index rejection.'
