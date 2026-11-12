<#
.SYNOPSIS
  Task 3.5 baseline runner: old-stack real-server TLS/NLA smoke and the
  design workload-matrix benchmark (explicit invocation only).

.DESCRIPTION
  Builds the lib test binary and invokes the two #[ignore]d harnesses in
  src-tauri/src/rdp/baseline.rs directly:

    -Mode Smoke    real-server TLS/NLA smoke against the staged LAN hosts
                   (numbered triples RDP_SMOKE_HOSTn/USERn/PASSn, read
                   data-driven — currently three: host1 Windows 11 native
                   RDP, host2 FreeRDP/Linux, host3 IronRDP/Linux).
                   Credentials are loaded into the process
                   environment from an env file OUTSIDE the repository
                   (default D:\BuildArtifacts\drift-rdp\secrets\rdp-smoke.env)
                   and are never printed, echoed or written to any output.
                   Evidence (redacted by construction) lands in the out dir.

    -Mode Workload design.md L481 workload matrix benchmark. The runner
                   samples the test process CPU / working set every second
                   while it runs (design: report CPU and process memory;
                   first-vs-last 10 s averages derive from these samples).

  Both modes record a hardware/build snapshot (CPU, RAM, GPU, OS, commit)
  and perform a redaction self-check: if any secret VALUE from the env file
  appears in any produced artifact, the run FAILS loudly (the value itself
  is never printed).

.EXAMPLE
  pwsh -NoProfile -File scripts/Invoke-Baseline.ps1 -Mode Smoke
  pwsh -NoProfile -File scripts/Invoke-Baseline.ps1 -Mode Workload
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Smoke', 'Workload')]
    [string]$Mode,

    [string]$SecretsPath = 'D:\BuildArtifacts\drift-rdp\secrets\rdp-smoke.env',

    [string]$OutDir = 'D:\BuildArtifacts\drift-rdp\perf-baseline',

    [string]$RepoRoot = '',

    # Run the benchmark binary compiled in release mode (the profile the
    # old-vs-new design comparison must use; the default dev build is only
    # a harness self-check).
    [switch]$Release
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepoRoot)) { $RepoRoot = (Get-Location).ProviderPath }

function Import-SecretsEnvironment {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "secrets env file not found at '$Path' (expected RDP_SMOKE_HOST1/USER1/PASS1 + HOST2/USER2/PASS2 + HOST3/USER3/PASS3)"
    }
    $values = @{}
    foreach ($line in Get-Content -LiteralPath $Path) {
        $trimmed = $line.Trim()
        if ($trimmed.Length -eq 0 -or $trimmed.StartsWith('#')) { continue }
        $key, $value = $trimmed -split '=', 2
        if ([string]::IsNullOrWhiteSpace($key)) { continue }
        $values[$key.Trim()] = $value
        [Environment]::SetEnvironmentVariable($key.Trim(), $value, 'Process')
    }
    foreach ($required in 'RDP_SMOKE_HOST1', 'RDP_SMOKE_USER1', 'RDP_SMOKE_PASS1', 'RDP_SMOKE_HOST2', 'RDP_SMOKE_USER2', 'RDP_SMOKE_PASS2', 'RDP_SMOKE_HOST3', 'RDP_SMOKE_USER3', 'RDP_SMOKE_PASS3') {
        if (-not $values.ContainsKey($required) -or [string]::IsNullOrEmpty($values[$required])) {
            throw "secrets env file '$Path' is missing a value for $required"
        }
    }
    return $values
}

function Write-HardwareSnapshot {
    param([string]$Directory, [string]$BuildId)
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
    $gpu = Get-CimInstance Win32_VideoController | Select-Object -First 1
    $cs = Get-CimInstance Win32_ComputerSystem
    $os = Get-CimInstance Win32_OperatingSystem
    $snapshot = [pscustomobject]@{
        capturedAtUtc   = (Get-Date).ToUniversalTime().ToString('o')
        buildId         = $BuildId
        os              = "$($os.Caption) ($($os.Version), build $($os.BuildNumber))"
        cpu             = $cpu.Name.Trim()
        cpuCores        = $cpu.NumberOfCores
        cpuLogical      = $cpu.NumberOfLogicalProcessors
        ramTotalBytes   = [uint64]$cs.TotalPhysicalMemory
        gpu             = $gpu.Name
        rustc           = ((& cargo --version 2>&1 | Select-Object -First 1) -as [string]).Trim()
        workloadTooling = 'scripts/Invoke-Baseline.ps1 + src-tauri/src/rdp/baseline.rs'
    }
    $snapshot | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Directory 'hardware.json') -Encoding utf8
    [Console]::WriteLine("hardware: $($snapshot.cpu) | $($snapshot.cpuLogical) logical | $([math]::Round($snapshot.ramTotalBytes / 1GB, 1)) GiB RAM | GPU $($snapshot.gpu)")
    [Console]::WriteLine("os: $($snapshot.os)")
    [Console]::WriteLine("build: $BuildId")
}

function Get-EncodedSecretForms {
    <# The recoverable encodings sspi dumps credential buffers in
       (debug-round finding): UTF-16LE hex, byte-level (UTF-8/ASCII) hex
       and the ", "-joined decimal UTF-16LE byte list — computed for the
       value AND its ASCII case variants (NTLM uppercases the domain
       inside its messages, so the encoded bytes of the uppercased form
       differ from the staged one). Only forms of at least 8 characters
       are returned (shorter encoded fragments are noise-prone).
       Derived from USER/PASS values/components only — HOST values stay
       exempt (IP[:port] literals are permitted evidence). #>
    param([string]$Value)
    $forms = @()
    if ([string]::IsNullOrEmpty($Value)) { return $forms }
    $variants = @($Value)
    if ($Value.ToCharArray() | Where-Object { [int]$_ -gt 127 }) {
        # non-ASCII value: only the as-staged variant (case folding
        # non-ASCII encodings would be pure noise)
    } else {
        $variants += $Value.ToUpperInvariant()
        $variants += $Value.ToLowerInvariant()
    }
    foreach ($variant in ($variants | Select-Object -Unique)) {
        $u16 = [System.Text.Encoding]::Unicode.GetBytes($variant)
        $hex16 = -join ($u16 | ForEach-Object { $_.ToString('x2') })
        $hex8 = -join ([System.Text.Encoding]::UTF8.GetBytes($variant) | ForEach-Object { $_.ToString('x2') })
        $dec = ($u16 | ForEach-Object { "$_" }) -join ', '
        foreach ($f in @($hex16, $hex8, $dec)) {
            if ($f.Length -ge 8) { $forms += $f }
        }
    }
    return $forms
}

function Get-SecretTokens {
    <# Every token that must never appear in artifacts, derived from the
       staged env values: the full value of each USER/PASS secret plus every
       separator-split component (>= 4 chars) of USER and HOST values —
       sspi logs logon names split AND the NTLM domain UPPERCASED, so the
       caller matches case-insensitively. Values/components shorter than 4
       characters are matched with word boundaries (bare substring matching
       on a 2-char username is pure noise). HOST values that are bare
       IP[:port] literals stay allowed (hosts+ports are permitted evidence
       per design L453); hostname components are not allowed.
       Returns token rows: pattern (case-insensitive regex) + label. #>
    param([hashtable]$SecretValues)
    # NOTE: no scriptblock helpers here — pwsh scriptblocks get their own
    # scope and would silently fail to accumulate into this list (that bug
    # made the round-1 runner check a false-pass; found by the external sweep).
    $rows = [System.Collections.Generic.List[object]]::new()
    foreach ($key in $SecretValues.Keys) {
        $value = $SecretValues[$key]
        if ([string]::IsNullOrEmpty($value)) { continue }
        $isUserOrPass = $key -like 'RDP_SMOKE_PASS*' -or $key -like 'RDP_SMOKE_USER*'
        $isHost = $key -like 'RDP_SMOKE_HOST*'
        if ($isUserOrPass) {
            if ($value.Length -ge 4) {
                $rows.Add([pscustomobject]@{ pattern = [regex]::Escape($value); label = $key })
            } else {
                $rows.Add([pscustomobject]@{ pattern = "(?<![A-Za-z0-9])$([regex]::Escape($value))(?![A-Za-z0-9])"; label = $key })
            }
            foreach ($enc in Get-EncodedSecretForms -Value $value) {
                $rows.Add([pscustomobject]@{ pattern = [regex]::Escape($enc); label = "$key(encoded)" })
            }
        }
        if ($isUserOrPass -or $isHost) {
            foreach ($part in ($value -split '[\\@:]')) {
                $trimmed = $part.Trim()
                # skip ports / other all-digit components (allowed evidence)
                # and IP literals; hostname components are NOT allowed
                if ($trimmed.Length -ge 4 -and $trimmed -notmatch '^\d+$' -and $trimmed -notmatch '^\d{1,3}(\.\d{1,3}){3}$') {
                    $rows.Add([pscustomobject]@{ pattern = [regex]::Escape($trimmed); label = "$key(component)" })
                    if ($isUserOrPass) {
                        foreach ($enc in Get-EncodedSecretForms -Value $trimmed) {
                            $rows.Add([pscustomobject]@{ pattern = [regex]::Escape($enc); label = "$key(component-encoded)" })
                        }
                    }
                }
            }
        }
    }
    return $rows
}

function Assert-NoSecretsInArtifacts {
    param([string]$Directory, [hashtable]$SecretValues)
    if ($SecretValues.Count -eq 0) { return }
    # Defense in depth: scrub secret tokens from every artifact first (the
    # libtest harness and panic paths bypass the in-process logger's own
    # scrubbing, e.g. assertion output). Matching is CASE-INSENSITIVE and
    # covers full values plus separator components (hostnames included).
    $tokens = Get-SecretTokens -SecretValues $SecretValues
    foreach ($file in Get-ChildItem -LiteralPath $Directory -File) {
        $content = [string](Get-Content -LiteralPath $file.FullName -Raw -ErrorAction SilentlyContinue)
        if ($null -eq $content) { continue }
        $scrubbed = $content
        foreach ($tok in $tokens) {
            $scrubbed = $scrubbed -replace $tok.pattern, '<redacted>'
        }
        if ($scrubbed -ne $content) {
            Set-Content -LiteralPath $file.FullName -Value $scrubbed -NoNewline -Encoding utf8
            [Console]::WriteLine("scrubbed secret tokens from $($file.Name)")
        }
    }
    $offenders = @()
    foreach ($file in Get-ChildItem -LiteralPath $Directory -File) {
        $content = [string](Get-Content -LiteralPath $file.FullName -Raw -ErrorAction SilentlyContinue)
        foreach ($tok in $tokens) {
            if ($content -match $tok.pattern) {
                $offenders += "$($file.Name):$($tok.label)"
            }
        }
    }
    if ($offenders.Count -gt 0) {
        # names of offending files/keys only — values stay unprinted
        throw "REDACTION FAILURE: secret tokens found in artifacts ($($offenders -join ', ')). Artifacts are invalid."
    }
    # STRUCTURE check (secret-agnostic): credential-buffer record shapes
    # must not appear at all — the harness's record-level redaction removes
    # them upstream. This also catches UNSTAGED credential material (e.g. a
    # mistyped password the env file never staged), which value-derived
    # tokens can never match (debug-round root cause 1).
    $structureShapes = @(
        'AuthIdentityBuffers',
        'SecurityBufferRef',
        'write_password_credentials',
        # NOTE: `(?<=\s)sspi` — a literal `\sspi` pattern would parse as
        # `\s` + "spi" and never match " sspi" (bug found by the round-1
        # final sweep; the lookbehind sidesteps the ambiguity)
        '(?<=\s)sspi\S*\]\s*(encrypt|decrypt)_message',
        '(?<=\s)sspi\S*\].*return=Ok\(\[\d'
    )
    $structureOffenders = @()
    foreach ($file in Get-ChildItem -LiteralPath $Directory -File) {
        $content = [string](Get-Content -LiteralPath $file.FullName -Raw -ErrorAction SilentlyContinue)
        foreach ($shape in $structureShapes) {
            if ($content -match $shape) { $structureOffenders += "$($file.Name):$shape" }
        }
    }
    if ($structureOffenders.Count -gt 0) {
        throw "REDACTION FAILURE: credential-buffer record shapes found in artifacts ($($structureOffenders -join ', ')). Artifacts are invalid."
    }
    [Console]::WriteLine('redaction self-check: OK (no secret tokens — values, components incl. hostnames, encoded forms, any casing — and no credential-buffer record shapes in artifacts)')
}

function Get-TestBinary {
    param([string]$ManifestPath, [switch]$Release)
    $cargoArgs = @('test', '--lib', '--no-run', '--manifest-path', $ManifestPath)
    if ($Release) { $cargoArgs += '--release' }
    [Console]::WriteLine("building lib test binary: cargo $($cargoArgs -join ' ')")
    $buildOutput = @(& cargo @cargoArgs 2>&1 | ForEach-Object { "$_" })
    $buildOutput | Write-Host
    if ($LASTEXITCODE -ne 0) { throw "cargo test --lib --no-run failed with exit code $LASTEXITCODE" }
    # The target directory may be redirected (e.g. CARGO_TARGET_DIR under
    # D:\BuildArtifacts), so trust cargo's own reported executable path.
    $binary = $buildOutput | Where-Object { $_ -match 'Executable unittests src\\lib\.rs \((.+?)\)' } |
        ForEach-Object { $Matches[1] } | Select-Object -Last 1
    if (-not $binary -or -not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "could not resolve the unittest binary path from cargo output"
    }
    [Console]::WriteLine("test binary: $binary")
    return $binary
}

# --- main ----------------------------------------------------------------
$manifest = Join-Path $RepoRoot 'src-tauri\Cargo.toml'
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

$gitHead = (git -C $RepoRoot rev-parse HEAD 2>$null | Select-Object -First 1)
if (-not $gitHead) { $gitHead = 'unknown' }
$dirty = @((git -C $RepoRoot status --porcelain 2>$null)).Count
$buildId = "$gitHead (dirty=$dirty)"

# Every invocation writes to its OWN timestamped run directory under the
# output root, so re-runs never overwrite retained/cited evidence
# (reviewer finding, remediation round 1).
$runId = Get-Date -Format 'yyyyMMdd-HHmmss'
$runDir = Join-Path $OutDir "$($Mode.ToLower())-$runId"
New-Item -ItemType Directory -Path $runDir -Force | Out-Null
$env:DRIFT_BASELINE_OUT_DIR = $runDir
[Console]::WriteLine("run directory: $runDir")

$secrets = @{}
if ($Mode -eq 'Smoke') {
    $secrets = Import-SecretsEnvironment -Path $SecretsPath
    [Console]::WriteLine("secrets loaded into process env from $SecretsPath (values never printed)")
}

Write-HardwareSnapshot -Directory $runDir -BuildId $buildId
$testBinary = Get-TestBinary -ManifestPath $manifest -Release:$Release

if ($Mode -eq 'Smoke') {
    $console = Join-Path $runDir 'smoke-run-console.txt'
    [Console]::WriteLine("== smoke run (explicit, foreground) -> $console")
    & $testBinary rdp::baseline::real_server_tls_nla_smoke --exact --ignored --nocapture --test-threads 1 2>&1 |
        ForEach-Object { "$_" } | Tee-Object -FilePath $console
    $exit = $LASTEXITCODE
    Assert-NoSecretsInArtifacts -Directory $runDir -SecretValues $secrets
    if ($exit -ne 0) { throw "smoke harness failed with exit code $exit — evidence incomplete, upgrade gate blocked (run dir: $runDir)" }
    [Console]::WriteLine("SMOKE: PASS — report: $(Join-Path $runDir 'smoke-report.json')")
} else {
    $env:DRIFT_BENCH_BUILD_ID = $buildId
    $console = Join-Path $runDir 'bench-run-console.txt'
    $samples = Join-Path $runDir 'process-samples.csv'
    [Console]::WriteLine("== workload-matrix benchmark run (explicit, foreground) -> $console")
    $stdout = Join-Path $runDir 'bench-stdout.txt'
    $stderr = Join-Path $runDir 'bench-stderr.txt'
    $proc = Start-Process -FilePath $testBinary -ArgumentList @('rdp::baseline::workload_matrix_baseline', '--exact', '--ignored', '--nocapture') `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru -NoNewWindow
    $inv = [System.Globalization.CultureInfo]::InvariantCulture
    $sampleRow = {
        param($p)
        '{0:o},{1},{2},{3}' -f (Get-Date).ToUniversalTime(),
            ([double]$p.CPU).ToString('F3', $inv),
            ([double]($p.WorkingSet64 / 1MB)).ToString('F1', $inv),
            ([double]($p.PrivateMemorySize64 / 1MB)).ToString('F1', $inv)
    }
    'timestamp_utc,cpu_seconds,working_set_mb,private_mb' | Set-Content -LiteralPath $samples -Encoding utf8
    while (-not $proc.HasExited) {
        try {
            $p = Get-Process -Id $proc.Id -ErrorAction Stop
            Add-Content -LiteralPath $samples -Value (& $sampleRow $p) -Encoding utf8
        } catch { <# process exited between check and sample — final state below #> }
        Start-Sleep -Milliseconds 1000
    }
    # final sample + merged console transcript
    try {
        $p = Get-Process -Id $proc.Id -ErrorAction Stop
        Add-Content -LiteralPath $samples -Value (& $sampleRow $p) -Encoding utf8
    } catch { }
    Get-Content -LiteralPath $stdout, $stderr -ErrorAction SilentlyContinue | Set-Content -LiteralPath $console -Encoding utf8
    if ($proc.ExitCode -ne 0) { throw "workload benchmark failed with exit code $($proc.ExitCode) (run dir: $runDir)" }
    [Console]::WriteLine("WORKLOAD: PASS — report: $(Join-Path $runDir 'bench-report.json') | process samples: $samples")
}
