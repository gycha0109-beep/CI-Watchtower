#Requires -Version 7.0
<#
.SYNOPSIS
  Windows GitHub Actions only: starts the actual packaged CI Watchtower EXE,
  probes its read-only bridge, exits the process and checks restart recovery.
.DESCRIPTION
  Only runs on an ephemeral GitHub-hosted Windows runner. It intentionally uses
  that runner's fresh app-data profile and Windows Credential Manager, NOT the
  user's existing installed WatchTower, SQLite database, or PAT.
  It never prints or exports the bridge token. No database or credential reset.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$SmokeScript
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($env:GITHUB_ACTIONS -ne 'true' -or !$IsWindows) {
    throw 'Packaged EXE lifecycle testing is restricted to Windows GitHub Actions.'
}
$exe = (Resolve-Path -LiteralPath $Executable -ErrorAction Stop).Path
$smoke = (Resolve-Path -LiteralPath $SmokeScript -ErrorAction Stop).Path
$port = 48765
$process = $null
$work = Join-Path $env:RUNNER_TEMP ('watchtower-exe-smoke-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work -Force | Out-Null
$baseline = Join-Path $work 'registry-ids.json'

function Test-Listener {
    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $client.Connect('127.0.0.1', $port)
        return $true
    } catch [System.Net.Sockets.SocketException] {
        return $false
    } finally {
        $client.Dispose()
    }
}

function Start-App {
    $global:LASTEXITCODE = 0
    $script:process = Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe -Parent) -PassThru
    for ($i = 0; $i -lt 80; $i++) {
        if ($script:process.HasExited) {
            throw "Packaged WatchTower EXE exited before serving localhost. ExitCode=$($script:process.ExitCode)"
        }
        if (Test-Listener) { return }
        Start-Sleep -Milliseconds 500
    }
    throw 'Packaged WatchTower EXE did not listen on localhost:48765 within the bounded startup window.'
}

function Stop-App {
    if ($null -ne $script:process) {
        if (!$script:process.HasExited) {
            Stop-Process -Id $script:process.Id -Force -ErrorAction Stop
            if (!$script:process.WaitForExit(15000)) {
                throw 'Packaged WatchTower EXE did not stop in the bounded shutdown window.'
            }
        }
        $script:process.Dispose()
        $script:process = $null
    }
}

function Assert-ProbeReport([string]$ReportPath, [string[]]$Required, [bool]$AllowSkippedRun) {
    if (!(Test-Path -LiteralPath $ReportPath)) {
        throw 'Bridge smoke did not produce its expected report.'
    }
    $report = Get-Content -LiteralPath $ReportPath -Raw | ConvertFrom-Json
    if ($report.failures -ne 0 -or @($report.probes | Where-Object { $_.outcome -eq 'FAIL' }).Count -gt 0) {
        throw 'A live installed-EXE bridge probe failed (see preceding redacted probe lines).'
    }
    foreach ($name in $Required) {
        if (@($report.probes | Where-Object { $_.name -eq $name -and $_.outcome -eq 'PASS' }).Count -ne 1) {
            throw "Missing successful packaged-EXE probe: $name"
        }
    }
    $skipped = @($report.probes | Where-Object { $_.outcome -eq 'SKIP' })
    if (!$AllowSkippedRun -and $skipped.Count -gt 0) {
        throw 'Unexpected skipped probe in packaged EXE lifecycle test.'
    }
    if ($AllowSkippedRun -and $skipped.Count -lt 1) {
        throw 'Synthetic runner must not imply exact GitHub Run acceptance.'
    }
}

if (Test-Listener) {
    throw 'Port 48765 was already occupied on this isolated runner. Refusing to test an unknown listener.'
}

try {
    Start-App
    $firstReport = Join-Path $work 'first-online.json'
    & $smoke -Mode Online -BaselinePath $baseline -WriteBaseline -ReportPath $firstReport
    $firstExit = $LASTEXITCODE
    if ($firstExit -ne 2) {
        throw "Synthetic empty-Registry acceptance must SKIP real Run evidence, not claim full PASS (exit=$firstExit)."
    }
    Assert-ProbeReport $firstReport @(
        'unauthenticated-rejected', 'same-user-os-credential',
        'invalid-bearer-rejected', 'origin-rejected', 'host-rejected',
        'authenticated-health', 'contract-v1-and-no-store',
        'registry-readable', 'registry-shape', 'identity-baseline-written'
    ) $true

    Stop-App
    $offlineReport = Join-Path $work 'stopped.json'
    & $smoke -Mode Offline -ReportPath $offlineReport
    if ($LASTEXITCODE -ne 0) { throw 'Bridge listener remained available after packaged EXE exit.' }
    Assert-ProbeReport $offlineReport @('process-exit-means-unavailable') $false

    Start-App
    $secondReport = Join-Path $work 'second-online.json'
    & $smoke -Mode Online -BaselinePath $baseline -ReportPath $secondReport
    $secondExit = $LASTEXITCODE
    if ($secondExit -ne 2) {
        throw "Restarted packaged EXE must still SKIP missing real Run evidence (exit=$secondExit)."
    }
    Assert-ProbeReport $secondReport @(
        'unauthenticated-rejected', 'same-user-os-credential',
        'authenticated-health', 'contract-v1-and-no-store',
        'registry-readable', 'registry-identities-preserved'
    ) $true
    Write-Host '[PASS] Packaged EXE startup, WinCred authentication, 401/403, read-only registry, stop and restart.'
    Write-Host '[SKIP] Real user Windows database/PAT/PR Run/Autopilot consumer acceptance was not exercised.'
} finally {
    Stop-App
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
