#Requires -Version 5.1
<#
.SYNOPSIS
  Signs PE files when GTT_SIGN_* env config is present; skips with a notice
  otherwise. Invoked by package.ps1 for ui.exe + cli.exe + the setup exe.

.NOTES
  Cert source (first match wins):
    GTT_SIGN_SHA1           - thumbprint in CurrentUser\My. THE EV TOKEN PATH:
                              SafeNet/eToken/DigiCert KeyLocker clients expose
                              the token's cert into the store; the private key
                              never leaves the HSM (PIN prompt per session).
    GTT_SIGN_PFX            - path to a .pfx (OV cert / local test cert)
    GTT_SIGN_PFX_PASS       - PFX password
    GTT_SIGN_DLIB           - cloud signing dlib (e.g. Azure Trusted Signing's
                              Azure.CodeSigning.Dlib.dll from
                              Microsoft.Trusted.Signing.Client)
    GTT_SIGN_DLIB_METADATA  - metadata .json paired with the dlib
    GTT_SIGN_TSA            - timestamp authority (default DigiCert)

  Nothing cert-related is read from the repo — env vars only, so secrets
  (PFX path/password, token PIN config) never get committed.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string[]]$File,
    [switch]$VerifyOnly
)
$ErrorActionPreference = 'Stop'

function Find-SignTool {
    $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $kits = 'C:\Program Files (x86)\Windows Kits\10\bin'
    $found = Get-ChildItem $kits -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $found) { throw "signtool.exe not found — install Windows SDK" }
    $found
}

$sha1  = $env:GTT_SIGN_SHA1
$pfx   = $env:GTT_SIGN_PFX
$dlib  = $env:GTT_SIGN_DLIB
$tsa   = if ($env:GTT_SIGN_TSA) { $env:GTT_SIGN_TSA } else { 'http://timestamp.digicert.com' }
$signtool = Find-SignTool

if ($VerifyOnly) {
    foreach ($f in $File) {
        & $signtool verify /pa /v $f | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "signature verification failed: $f" }
        Write-Host "    verified: $(Split-Path $f -Leaf)" -ForegroundColor DarkGray
    }
    return
}

if (-not ($sha1 -or $pfx -or $dlib)) {
    Write-Host '==> signing skipped (no GTT_SIGN_SHA1 / GTT_SIGN_PFX / GTT_SIGN_DLIB set)' -ForegroundColor DarkYellow
    return
}

$args = @('sign', '/fd', 'sha256', '/td', 'sha256', '/tr', $tsa, '/v')
if     ($sha1) { $args += @('/sha1', $sha1) }
elseif ($dlib) {
    $args += @('/dlib', $dlib)
    if ($env:GTT_SIGN_DLIB_METADATA) { $args += @('/dmdf', $env:GTT_SIGN_DLIB_METADATA) }
}
else           { $args += @('/f', $pfx); if ($env:GTT_SIGN_PFX_PASS) { $args += @('/p', $env:GTT_SIGN_PFX_PASS) } }
$args += $File

Write-Host "==> signtool sign ($($File -join ', '))" -ForegroundColor Cyan
& $signtool @args
if ($LASTEXITCODE -ne 0) { throw "signtool sign failed (rc=$LASTEXITCODE)" }

foreach ($f in $File) {
    & $signtool verify /pa $f | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "post-sign verification failed: $f" }
}
Write-Host '    signatures verified (/pa)' -ForegroundColor DarkGray
