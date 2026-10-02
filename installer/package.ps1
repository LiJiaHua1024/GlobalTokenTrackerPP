#Requires -Version 5.1
<#
.SYNOPSIS
  Builds release binaries and produces the single-file installer for GlobalTokenTracker++.

.OUTPUT
  dist\GlobalTokenTrackerPP-Setup-<ver>-win-x64.exe — self-extracting per-user installer
  (embeds Flutter Material Design 3 UI + Rust FFI runtime + data as a compressed payload).
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    # Optional override; defaults to the [workspace.package] version in the root Cargo.toml
    # so the installer version always matches the release pipeline's single source of truth.
    [string]$Version
)
$ErrorActionPreference = 'Stop'
$root        = Split-Path $PSScriptRoot
if (-not $Version) {
    $tomlText = Get-Content (Join-Path $root 'Cargo.toml') -Raw
    $match = [regex]::Match($tomlText, '(?s)\[workspace\.package\].*?version\s*=\s*"([^"]+)"')
    if (-not $match.Success) { throw 'cannot resolve version: [workspace.package] version missing in Cargo.toml' }
    $Version = $match.Groups[1].Value
}
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and (Test-Path $cargoBin)) {
    $env:PATH = "$cargoBin;$env:PATH"
}
if (-not (Get-Command flutter -ErrorAction SilentlyContinue) -and (Test-Path 'F:\flutter\bin')) {
    $env:PATH = "F:\flutter\bin;$env:PATH"
}
$target      = Join-Path $root 'target\release'
$flutterUi   = Join-Path $root 'flutter_ui'
$flutterDist = Join-Path $flutterUi 'build\windows\x64\runner\Release'
$payloadDir  = Join-Path $root 'crates\setup\payload'
$payloadZip  = Join-Path $payloadDir 'payload.zip'

if (-not $SkipBuild) {
    Write-Host '==> 1. cargo build --release -p globaltokentracker-ffi' -ForegroundColor Cyan
    Push-Location $root
    cargo build --release -p globaltokentracker-ffi
    if ($LASTEXITCODE -ne 0) { Pop-Location; throw 'cargo build ffi failed' }
    Pop-Location

    Write-Host '==> 2. flutter build windows --release' -ForegroundColor Cyan
    Push-Location $flutterUi
    & flutter build windows --release
    if ($LASTEXITCODE -ne 0) { Pop-Location; throw 'flutter build failed' }
    Pop-Location

    Copy-Item (Join-Path $target 'globaltokentracker_ffi.dll') $flutterDist -Force
}

if (-not (Test-Path (Join-Path $flutterDist 'globaltokentracker_ui.exe'))) {
    throw "missing build output: globaltokentracker_ui.exe in $flutterDist"
}

# Authenticode-sign the distributable binaries BEFORE anything is zipped or
# hashed — the payload zip, the portable zip and the setup exe must all carry
# the signature, and the printed SHA256 digests must match the signed files.
# sign.ps1 skips with a notice when no GTT_SIGN_* credentials are configured.
Write-Host '==> signing distributable binaries' -ForegroundColor Cyan
$signTargets = @(Join-Path $flutterDist 'globaltokentracker_ui.exe')
$ffiDll = Join-Path $flutterDist 'globaltokentracker_ffi.dll'
if (Test-Path $ffiDll) { $signTargets += $ffiDll }
& (Join-Path $PSScriptRoot 'sign.ps1') -File $signTargets

Write-Host '==> staging payload.zip from Flutter Release distribution' -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $payloadDir | Out-Null
if (Test-Path $payloadZip) { Remove-Item $payloadZip -Force }
Compress-Archive -Path "$flutterDist\*" -DestinationPath $payloadZip -CompressionLevel Optimal

Write-Host '==> cargo build --release -p globaltokentracker-setup' -ForegroundColor Cyan
Push-Location $root
(Get-Item (Join-Path $root 'crates\setup\build.rs')).LastWriteTime = Get-Date
cargo build --release -p globaltokentracker-setup
$rc = $LASTEXITCODE
Pop-Location
Remove-Item $payloadZip -Force

if ($rc -ne 0) { throw 'setup build failed' }

$ver = $Version
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null

# 1. Self-extracting GUI Setup Installer
$out = Join-Path $dist "GlobalTokenTrackerPP-Setup-$ver-win-x64.exe"
Copy-Item (Join-Path $target 'globaltokentracker-setup.exe') $out -Force
& (Join-Path $PSScriptRoot 'sign.ps1') -File $out
$mb  = [math]::Round((Get-Item $out).Length / 1MB, 2)
$sha = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()

# 2. Portable Distribution Zip (extract and run directly without installation)
$zipOut = Join-Path $dist "GlobalTokenTrackerPP-$ver-windows-x64.zip"
try {
    if (Test-Path $zipOut) { Remove-Item $zipOut -Force -ErrorAction Stop }
} catch {
    Write-Warning "Could not overwrite $zipOut (file in use). Falling back to GlobalTokenTrackerPP-Portable.zip"
    $zipOut = Join-Path $dist "GlobalTokenTrackerPP-Portable.zip"
    if (Test-Path $zipOut) { Remove-Item $zipOut -Force -ErrorAction SilentlyContinue }
}
Write-Host "==> creating portable distribution zip: $zipOut" -ForegroundColor Cyan
Compress-Archive -Path "$flutterDist\*" -DestinationPath $zipOut -CompressionLevel Optimal
$zipMb  = [math]::Round((Get-Item $zipOut).Length / 1MB, 2)
$zipSha = (Get-FileHash $zipOut -Algorithm SHA256).Hash.ToLower()

Write-Host ''
Write-Host "installer: $out" -ForegroundColor Green
Write-Host "size     : $mb MB"
Write-Host "sha256   : $sha"
Write-Host ''
Write-Host "portable : $zipOut" -ForegroundColor Green
Write-Host "size     : $zipMb MB"
Write-Host "sha256   : $zipSha"
