<# Build a standalone Windows preview. PrepareOnly validates existing frontend assets
   and emits the Tauri override without compiling. Native UI verification is separate. #>
[CmdletBinding()]
param(
    [string]$OutputDirectory = '',
    [switch]$PrepareOnly,
    [switch]$Installer,
    [ValidateSet('', 'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')][string]$Target = ''
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path
$tauriDir = Join-Path $repo 'src-tauri'
$version = (Get-Content -LiteralPath (Join-Path $tauriDir 'tauri.conf.json') -Raw | ConvertFrom-Json).version
$releaseRoot = 'D:\BuildArtifacts\drift-rdp\releases'
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $releaseRoot "v$version" }
$output = [IO.Path]::GetFullPath($OutputDirectory).TrimEnd('\', '/')
if (-not $output.StartsWith($releaseRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Preview output must be a child of D:/BuildArtifacts/drift-rdp/releases.'
}
if ([IO.Path]::GetPathRoot($output) -ne [IO.Path]::GetPathRoot($tauriDir)) {
    throw 'Frontend output and src-tauri must share a drive for a relative asset path.'
}
$ancestor = $output
while ($ancestor) {
    if (Test-Path -LiteralPath $ancestor) {
        if ((Get-Item -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'Refusing preview output through a reparse point.'
        }
    }
    $ancestor = Split-Path -Parent $ancestor
}
New-Item -ItemType Directory -Path $output -Force | Out-Null
$frontend = Join-Path $output 'frontend'
if ((Test-Path -LiteralPath $frontend) -and ((Get-Item -LiteralPath $frontend).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'Refusing a redirected frontend directory.'
}
$configPath = Join-Path $output 'tauri-build.json'
Push-Location $repo
try {
    if (-not $PrepareOnly) {
        & node node_modules/typescript/bin/tsc -p tsconfig.app.json --noEmit --tsBuildInfoFile (Join-Path $output 'app.tsbuildinfo')
        if ($LASTEXITCODE -ne 0) { throw 'App TypeScript check failed.' }
        & node node_modules/vite/bin/vite.js build --outDir $frontend
        if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $frontend 'index.html') -PathType Leaf)) {
        throw 'Frontend index.html is missing; refusing an empty preview.'
    }
    # Tauri's untagged FrontendDist tries URL before Directory. A Windows
    # drive prefix (D:/...) parses as a URL and silently disables embedding.
    $frontendDist = [IO.Path]::GetRelativePath($tauriDir, $frontend).Replace('\', '/')
    if ([IO.Path]::IsPathRooted($frontendDist) -or $frontendDist -match '^[a-zA-Z][a-zA-Z0-9+.-]*:') {
        throw 'Frontend distribution must be a relative directory, never a URL.'
    }
    $override = @{ build = @{ beforeBuildCommand = ''; frontendDist = $frontendDist }; bundle = @{ createUpdaterArtifacts = [bool]$Installer } }
    $override |
        ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $configPath
    if ($PrepareOnly) { return $configPath }
    $oldKey = $env:TAURI_SIGNING_PRIVATE_KEY
    $oldPassword = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
    try {
        if ($Installer -and -not $env:TAURI_SIGNING_PRIVATE_KEY) {
            $keyDir = Join-Path $env:LOCALAPPDATA '1-1-release-signing'
            $env:TAURI_SIGNING_PRIVATE_KEY = Join-Path $keyDir 'updater.key'
            if (-not (Test-Path -LiteralPath $env:TAURI_SIGNING_PRIVATE_KEY)) { throw 'Installer requires the persistent updater signing key or TAURI_SIGNING_PRIVATE_KEY.' }
            $secure = (Get-Content -LiteralPath (Join-Path $keyDir 'password.dpapi') -Raw).Trim() | ConvertTo-SecureString
            $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [Net.NetworkCredential]::new('', $secure).Password
        }
        $buildArgs = @('build', '--ci', '--config', $configPath)
        if ($Installer) { $buildArgs += @('--bundles', 'nsis') } else { $buildArgs += '--no-bundle' }
        if ($Target) { $buildArgs += @('--target', $Target) }
        & node node_modules/@tauri-apps/cli/tauri.js @buildArgs -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Windows preview build failed.' }
        if ($Installer) {
            $actualTarget = $Target
            if (-not $actualTarget) { $actualTarget = (& rustc -vV | Select-String '^host: ').ToString().Substring(6).Trim() }
            $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $tauriDir 'target' }
            if ($Target) { $targetDir = Join-Path $targetDir $Target }
            $bundleDir = Join-Path $targetDir 'release/bundle/nsis'
            $arch = if ($actualTarget -eq 'aarch64-pc-windows-msvc') { 'arm64' } else { 'x64' }
            $signed = @(Get-ChildItem -LiteralPath $bundleDir -Filter "*_${version}_${arch}-setup.exe.sig")
            if ($signed.Count -ne 1) { throw 'Expected exactly one signed installer for this version and architecture.' }
            $installerPath = $signed[0].FullName.Substring(0, $signed[0].FullName.Length - 4)
            & node scripts/write-build-info.mjs $actualTarget $installerPath
            if ($LASTEXITCODE -ne 0) { throw 'Installer build metadata failed.' }
        }
    } finally {
        $env:TAURI_SIGNING_PRIVATE_KEY = $oldKey
        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $oldPassword
    }
} finally { Pop-Location }
