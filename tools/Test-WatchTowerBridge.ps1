#Requires -Version 5.1
<#
.SYNOPSIS
  Read-only Windows acceptance probes for the installed CI WatchTower bridge.
.DESCRIPTION
  Retrieves the bridge credential from the current Windows user's Credential Manager.
  Never prints, exports, rotates, or persists credentials; does not write to WatchTower's DB.
  Run Online before/after restarting WatchTower and Offline after fully exiting it.
  SelfTest validates the keyring UTF-16LE decoder without accessing OS credentials.
.EXAMPLE
  .\tools\Test-WatchTowerBridge.ps1 -Mode Online -RepositoryId 2 -RunId 12345 -HeadSha aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa -Attempt 1
.EXAMPLE
  .\tools\Test-WatchTowerBridge.ps1 -Mode Online -BaselinePath "$env:TEMP\watchtower-bridge-baseline.json" -WriteBaseline
.EXAMPLE
  .\tools\Test-WatchTowerBridge.ps1 -Mode Offline
#>
[CmdletBinding()]
param(
    [ValidateSet('Online', 'Offline', 'SelfTest')][string]$Mode = 'Online',
    [ValidateRange(0, 2147483647)][int]$RepositoryId = 0,
    [ValidateRange(0, 9223372036854775807)][long]$RunId = 0,
    [string]$HeadSha,
    [ValidateRange(0, 2147483647)][int]$Attempt = 0,
    [string]$TrackKey,
    [string]$BaselinePath,
    [switch]$WriteBaseline,
    [string]$ReportPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Probes = New-Object 'System.Collections.Generic.List[object]'
$script:Failures = 0

function Add-Probe([string]$Name, [string]$Outcome, [string]$Detail = '') {
    if ($Outcome -eq 'FAIL') { $script:Failures++ }
    $script:Probes.Add([pscustomobject]@{ name = $Name; outcome = $Outcome; detail = $Detail })
    Write-Host ("[{0}] {1}{2}" -f $Outcome, $Name, $(if ($Detail) { " ($Detail)" } else { '' }))
}

function Invoke-Bridge([string]$Path, [string]$Token = '', [string]$HostOverride = '', [string]$Origin = '') {
    $request = [System.Net.HttpWebRequest][System.Net.WebRequest]::Create("http://127.0.0.1:48765$Path")
    $request.Method = 'GET'
    $request.Timeout = 5000
    $request.ReadWriteTimeout = 5000
    $request.AllowAutoRedirect = $false
    $request.Proxy = $null
    if ($HostOverride) { $request.Host = $HostOverride }
    if ($Origin) { $request.Headers.Add('Origin', $Origin) }
    if ($Token) { $request.Headers.Add('Authorization', "Bearer $Token") }
    $response = $null
    try {
        try { $response = $request.GetResponse() }
        catch [System.Net.WebException] {
            if ($null -eq $_.Exception.Response) {
                return [pscustomobject]@{ status = 0; body = $null; cache = ''; code = 'WATCHTOWER_UNAVAILABLE' }
            }
            $response = $_.Exception.Response
        }
        $reader = New-Object System.IO.StreamReader($response.GetResponseStream())
        try { $body = $reader.ReadToEnd() }
        finally { $reader.Dispose() }
        $json = $null
        try { $json = ConvertFrom-Json -InputObject $body -ErrorAction Stop }
        catch { }
        return [pscustomobject]@{
            status = [int]$response.StatusCode
            body = $json
            cache = [string]$response.Headers['Cache-Control']
            code = $(if ($null -ne $json -and $null -ne $json.error) { [string]$json.error.code } else { '' })
        }
    }
    finally { if ($null -ne $response) { $response.Dispose() } }
}

function Assert-Http([string]$Name, $Result, [int]$Status, [string]$Code = '') {
    $ok = ($Result.status -eq $Status) -and (!$Code -or $Result.code -eq $Code)
    $details = "HTTP $($Result.status)"
    if ($Result.code) { $details += " / $($Result.code)" }
    Add-Probe $Name $(if ($ok) { 'PASS' } else { 'FAIL' }) $details
    return $ok
}

function Get-RegistryIdentity($Registry) {
    return [pscustomobject]@{
        projects = @($Registry.projects | ForEach-Object { "$($_.id)|$($_.key)|$($_.name)|$($_.active)" } | Sort-Object)
        repositories = @($Registry.repositories | ForEach-Object { "$($_.id)|$($_.projectId)|$($_.name)|$($_.enabled)" } | Sort-Object)
        tracks = @($Registry.tracks | ForEach-Object { "$($_.id)|$($_.projectId)|$($_.canonicalTrackKey)|$($_.name)|$($_.active)" } | Sort-Object)
    }
}

function Initialize-CredentialReader {
    if ($null -ne ('WatchTowerCredentialReader' -as [type])) { return }
        # keyring 3 Windows-native: target_name = account + "." + service
        # service=ci-watchtower, account=development-autopilot-bridge-v1
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
[StructLayout(LayoutKind.Sequential)]
public struct WatchTowerNativeCredential {
    public UInt32 Flags;
    public UInt32 Type;
    public IntPtr TargetName;
    public IntPtr Comment;
    public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
    public UInt32 CredentialBlobSize;
    public IntPtr CredentialBlob;
    public UInt32 Persist;
    public UInt32 AttributeCount;
    public IntPtr Attributes;
    public IntPtr TargetAlias;
    public IntPtr UserName;
}
public static class WatchTowerCredentialReader {
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true, EntryPoint = "CredReadW")]
    static extern bool CredRead(string target, UInt32 type, UInt32 flags, out IntPtr credential);
    [DllImport("advapi32.dll", EntryPoint = "CredFree")]
    static extern void CredFree(IntPtr credential);
    public static string DecodePassword(byte[] bytes) {
        if (bytes == null || bytes.Length != 128)
            throw new InvalidOperationException("Expected a 128-byte UTF-16LE bridge credential");
        var password = System.Text.Encoding.Unicode.GetString(bytes);
        if (password.Length != 64 || !System.Text.RegularExpressions.Regex.IsMatch(password, @"\A[0-9a-fA-F]{64}\z"))
            throw new InvalidOperationException("Invalid bridge credential encoding or format");
        return password;
    }
    public static string ReadPassword(string target) {
        IntPtr pointer;
        if (!CredRead(target, 1, 0, out pointer))
            throw new InvalidOperationException("Credential Manager lookup failed: Windows error " + Marshal.GetLastWin32Error());
        try {
            var credential = (WatchTowerNativeCredential)Marshal.PtrToStructure(pointer, typeof(WatchTowerNativeCredential));
            // keyring::Entry::set_password stores the 64-character token as UTF-16LE.
            if (credential.CredentialBlobSize != 128) throw new InvalidOperationException("Unexpected bridge credential length");
            var bytes = new byte[credential.CredentialBlobSize];
            Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
            try { return DecodePassword(bytes); }
            finally { Array.Clear(bytes, 0, bytes.Length); }
        } finally { CredFree(pointer); }
    }
}
'@ -ErrorAction Stop
}

if ($WriteBaseline -and (!$BaselinePath -or $Mode -ne 'Online')) {
    throw 'WriteBaseline requires -Mode Online and -BaselinePath.'
}
if ($RunId -gt 0 -and (!$HeadSha -or $Attempt -lt 1 -or $RepositoryId -lt 1)) {
    throw 'For exact Run acceptance supply -RepositoryId, -RunId, -HeadSha and -Attempt together.'
}
if ($HeadSha -and $HeadSha -notmatch '^[0-9a-fA-F]{40}$') { throw 'HeadSha must be a 40-character hexadecimal SHA.' }
if ($TrackKey -and $TrackKey -notmatch '^[a-z0-9-]{1,100}$') { throw 'TrackKey must be a canonical key.' }

try {
    if ($Mode -eq 'SelfTest') {
        # CI-only local encoding check. No Credential Manager or HTTP access.
        Initialize-CredentialReader
        $sample = '0123456789abcdef' * 4
        $encoded = [System.Text.Encoding]::Unicode.GetBytes($sample)
        try {
            $decoded = [WatchTowerCredentialReader]::DecodePassword($encoded)
            Add-Probe 'keyring-utf16le-token-decoding' $(if ($decoded -ceq $sample) { 'PASS' } else { 'FAIL' })
            $rejected = $false
            try { [WatchTowerCredentialReader]::DecodePassword([System.Text.Encoding]::UTF8.GetBytes($sample)) | Out-Null }
            catch [System.InvalidOperationException] { $rejected = $true }
            Add-Probe 'utf8-token-blob-rejected' $(if ($rejected) { 'PASS' } else { 'FAIL' })
            $rejected = $false
            try { [WatchTowerCredentialReader]::DecodePassword([System.Text.Encoding]::Unicode.GetBytes(('g' * 64))) | Out-Null }
            catch [System.InvalidOperationException] { $rejected = $true }
            Add-Probe 'non-hex-token-rejected' $(if ($rejected) { 'PASS' } else { 'FAIL' })
        }
        finally { [System.Array]::Clear($encoded, 0, $encoded.Length) }
    }
    elseif ($Mode -eq 'Offline') {
        $offline = Invoke-Bridge -Path '/v1/health'
        Add-Probe 'process-exit-means-unavailable' $(if ($offline.status -eq 0) { 'PASS' } else { 'FAIL' }) "HTTP $($offline.status)"
    }
    else {
        $noAuth = Invoke-Bridge -Path '/v1/health'
        $reachable = Assert-Http 'unauthenticated-rejected' $noAuth 401 'UNAUTHORIZED'
        if (!$reachable -and $noAuth.status -eq 0) { throw 'Bridge is not listening; launch WatchTower under the same Windows account.' }

        Initialize-CredentialReader
        $token = [WatchTowerCredentialReader]::ReadPassword('development-autopilot-bridge-v1.ci-watchtower')
        if ($token -notmatch '^[0-9a-fA-F]{64}$') { throw 'Bridge credential has an invalid format.' }
        Add-Probe 'same-user-os-credential' 'PASS' 'separate bridge credential read; secret redacted'

        $wrong = Invoke-Bridge -Path '/v1/health' -Token ('0' * 64)
        # If the actual credential happens to be all zeros, use a different wrong token.
        if ($token -eq ('0' * 64)) { $wrong = Invoke-Bridge -Path '/v1/health' -Token ('1' * 64) }
        Assert-Http 'invalid-bearer-rejected' $wrong 401 'UNAUTHORIZED' | Out-Null
        Assert-Http 'origin-rejected' (Invoke-Bridge -Path '/v1/health' -Token $token -Origin 'https://other.invalid') 403 'LOCAL_CLIENT_ONLY' | Out-Null
        Assert-Http 'host-rejected' (Invoke-Bridge -Path '/v1/health' -Token $token -HostOverride 'localhost:48765') 403 'LOCAL_CLIENT_ONLY' | Out-Null

        $health = Invoke-Bridge -Path '/v1/health' -Token $token
        $healthy = Assert-Http 'authenticated-health' $health 200
        if (!$healthy) { throw 'Authenticated bridge health failed.' }
        Add-Probe 'contract-v1-and-no-store' $(if ($health.body.contractVersion -eq '1.0' -and $health.body.data.version -eq '1.0' -and $health.cache -match 'no-store') { 'PASS' } else { 'FAIL' })
        Assert-Http 'repository-scope-required' (Invoke-Bridge -Path '/v1/runs' -Token $token) 400 'REPOSITORY_SCOPE_REQUIRED' | Out-Null
        Assert-Http 'invalid-scope-rejected' (Invoke-Bridge -Path '/v1/registry?project_id=-1' -Token $token) 400 'INVALID_SCOPE' | Out-Null

        $registry = Invoke-Bridge -Path '/v1/registry' -Token $token
        if (!(Assert-Http 'registry-readable' $registry 200)) { throw 'Registry read failed.' }
        $identities = Get-RegistryIdentity $registry.body.data
        Add-Probe 'registry-shape' $(if ($null -ne $registry.body.data.projects -and $null -ne $registry.body.data.repositories -and $null -ne $registry.body.data.tracks) { 'PASS' } else { 'FAIL' })
        if ($BaselinePath) {
            if ($WriteBaseline) {
                $identities | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $BaselinePath -Encoding UTF8
                Add-Probe 'identity-baseline-written' 'PASS' 'IDs/keys only; no credential, aliases, or run metadata'
            }
            elseif (Test-Path -LiteralPath $BaselinePath) {
                $old = Get-Content -LiteralPath $BaselinePath -Raw | ConvertFrom-Json
                $currentJson = ConvertTo-Json -InputObject $identities -Compress -Depth 5
                $oldJson = ConvertTo-Json -InputObject $old -Compress -Depth 5
                Add-Probe 'registry-identities-preserved' $(if ($currentJson -ceq $oldJson) { 'PASS' } else { 'FAIL' }) 'compare same installed user before/after restart'
            }
            else { Add-Probe 'registry-baseline-not-found' 'FAIL' 'Use -WriteBaseline before restart.' }
        }

        $repo = $RepositoryId
        if ($repo -eq 0) {
            $selected = @($registry.body.data.repositories | Where-Object { $_.enabled } | Select-Object -First 1)
            if ($selected.Count -gt 0) { $repo = [int]$selected[0].id }
        }
        if ($repo -le 0) {
            Add-Probe 'registered-repository-run' 'SKIP' 'Provide a registered RepositoryId to complete the live gate.'
        }
        else {
            Add-Probe 'repository-in-registry' $(if (@($registry.body.data.repositories | Where-Object { $_.id -eq $repo }).Count -gt 0) { 'PASS' } else { 'FAIL' })
            $list = Invoke-Bridge -Path "/v1/runs?repository_id=$repo&limit=30" -Token $token
            if (!(Assert-Http 'repository-runs-readable' $list 200)) { throw 'Repository run list failed.' }
            Add-Probe 'bounded-sample-not-complete-coverage' $(if ($list.body.data.completeCoverage -ceq $false) { 'PASS' } else { 'FAIL' })
            Add-Probe 'run-list-never-authorizes-advance' $(if (@($list.body.data.runs | Where-Object { $_.safeToAdvance -ne $false }).Count -eq 0) { 'PASS' } else { 'FAIL' })
            $selectedRun = $null
            if ($RunId -gt 0) {
                $selectedRun = [pscustomobject]@{ runId = $RunId; headSha = $HeadSha; runAttempt = $Attempt }
            }
            else {
                $selectedRun = @($list.body.data.runs | Where-Object { $_.runAttempt -eq $_.latestKnownAttempt } | Select-Object -First 1)
                if ($selectedRun.Count -gt 0) { $selectedRun = $selectedRun[0] } else { $selectedRun = $null }
            }
            if ($null -eq $selectedRun) {
                Add-Probe 'exact-run-with-attempt' 'SKIP' 'No observed run with a current attempt; real Run evidence still required.'
            }
            else {
                $rid = [long]$selectedRun.runId
                $sha = [string]$selectedRun.headSha
                $attemptNumber = [int]$selectedRun.runAttempt
                $singlePath = "/v1/runs/$rid" + "?repository_id=$repo&head_sha=$sha&attempt=$attemptNumber"
                $single = Invoke-Bridge -Path $singlePath -Token $token
                if (Assert-Http 'exact-run-with-attempt' $single 200) {
                    $run = $single.body.data.run
                    Add-Probe 'run-evidence-not-authorization' $(if ($run.runId -eq $rid -and $run.headSha -eq $sha -and $run.runAttempt -eq $attemptNumber -and $run.targetVerified -eq $true -and $run.safeToAdvance -eq $false) { 'PASS' } else { 'FAIL' })
                    if ($TrackKey) {
                        Add-Probe 'expected-track-matches' $(if ($run.workTrackKey -ceq $TrackKey) { 'PASS' } else { 'FAIL' }) 'manual attribution and responsibility remain independent'
                    }
                    $alternateSha = if ($sha -eq ('a' * 40)) { 'b' * 40 } else { 'a' * 40 }
                    Assert-Http 'wrong-head-is-conflict' (Invoke-Bridge -Path ("/v1/runs/$rid" + "?repository_id=$repo&head_sha=$alternateSha&attempt=$attemptNumber") -Token $token) 409 'HEAD_SHA_MISMATCH' | Out-Null
                    $alternateAttempt = if ($attemptNumber -eq 1) { 2 } else { 1 }
                    Assert-Http 'wrong-attempt-is-conflict' (Invoke-Bridge -Path ("/v1/runs/$rid" + "?repository_id=$repo&head_sha=$sha&attempt=$alternateAttempt") -Token $token) 409 'RUN_ATTEMPT_MISMATCH' | Out-Null
                }
                Assert-Http 'unknown-run-is-not-success' (Invoke-Bridge -Path "/v1/runs/9223372036854775806?repository_id=$repo" -Token $token) 404 'RUN_NOT_FOUND' | Out-Null
            }
        }
    }
}
catch {
    Add-Probe 'script-or-environment-error' 'FAIL' $_.Exception.Message
}
finally {
    # The report contains only probe names, results and non-secret diagnostics.
    if ($ReportPath) {
        [pscustomobject]@{
            mode = $Mode
            at = [DateTimeOffset]::UtcNow.ToString('o')
            failures = $script:Failures
            probes = @($script:Probes.ToArray())
        } | ConvertTo-Json -Depth 7 | Set-Content -LiteralPath $ReportPath -Encoding UTF8
    }
}
if ($script:Failures -gt 0) { exit 1 }
if (@($script:Probes | Where-Object { $_.outcome -eq 'SKIP' }).Count -gt 0) { exit 2 }
exit 0
