#Requires -Version 5.1
<#
.SYNOPSIS
  Builds release binaries and produces the single-file installer.

.OUTPUT
  dist\CodeLedger-Setup-<ver>.exe — self-extracting per-user installer
  (embeds codeledger-ui.exe + codeledger-cli.exe as a compressed payload).
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
# cargo may not be on PATH when invoked from a bare powershell session.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and (Test-Path $cargoBin)) {
    $env:PATH = "$cargoBin;$env:PATH"
}
$root   = Split-Path $PSScriptRoot
$target = Join-Path $root 'target\release'
$payloadDir = Join-Path $root 'crates\setup\payload'
$payloadZip = Join-Path $payloadDir 'payload.zip'

if (-not $SkipBuild) {
    Write-Host '==> cargo build --release -p codeledger-ui -p codeledger-cli' -ForegroundColor Cyan
    Push-Location $root
    cargo build --release -p codeledger-ui -p codeledger-cli
    if ($LASTEXITCODE -ne 0) { Pop-Location; throw 'cargo build failed' }
    Pop-Location
}

foreach ($exe in 'codeledger-ui.exe', 'codeledger-cli.exe') {
    if (-not (Test-Path (Join-Path $target $exe))) { throw "missing build output: $exe" }
}

Write-Host '==> staging payload.zip' -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $payloadDir | Out-Null
if (Test-Path $payloadZip) { Remove-Item $payloadZip -Force }
Compress-Archive -Path (Join-Path $target 'codeledger-ui.exe'),
                       (Join-Path $target 'codeledger-cli.exe') `
                 -DestinationPath $payloadZip -CompressionLevel Optimal

Write-Host '==> cargo build --release -p codeledger-setup' -ForegroundColor Cyan
Push-Location $root
cargo build --release -p codeledger-setup
$rc = $LASTEXITCODE
Pop-Location
Remove-Item $payloadZip -Force   # never keep the payload around
if ($rc -ne 0) { throw 'setup build failed' }

$ver = (Select-String -Path (Join-Path $root 'Cargo.toml') `
        -Pattern '(?m)^version\s*=\s*"([^"]+)"' |
        Select-Object -First 1).Matches.Groups[1].Value
if (-not $ver) { $ver = '0.1.0' }

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$out = Join-Path $dist "CodeLedger-Setup-$ver-win-x64.exe"
Copy-Item (Join-Path $target 'codeledger-setup.exe') $out -Force

$mb  = [math]::Round((Get-Item $out).Length / 1MB, 2)
$sha = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
Write-Host ''
Write-Host "installer: $out" -ForegroundColor Green
Write-Host "size     : $mb MB"
Write-Host "sha256   : $sha"
