<# Build a standalone Windows preview. PrepareOnly validates existing frontend assets
   and emits the Tauri override without compiling. Native UI verification is separate. #>
[CmdletBinding()]
param(
    [string]$OutputDirectory = '',
    [switch]$PrepareOnly
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
    @{ build = @{ beforeBuildCommand = ''; frontendDist = $frontendDist } } |
        ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $configPath
    if ($PrepareOnly) { return $configPath }
    & (Join-Path $PSScriptRoot 'Invoke-BuildStorage.ps1') -Action Check
    try {
        & node node_modules/@tauri-apps/cli/tauri.js build --no-bundle --ci --config $configPath -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Windows preview build failed.' }
    } finally {
        & (Join-Path $PSScriptRoot 'Invoke-BuildStorage.ps1') -Action Check
    }
} finally { Pop-Location }
