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
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and (Test-Path $cargoBin)) {
    $env:PATH = "$cargoBin;$env:PATH"
}
if (-not (Get-Command flutter -ErrorAction SilentlyContinue) -and (Test-Path 'F:\flutter\bin')) {
    $env:PATH = "F:\flutter\bin;$env:PATH"
}
$root        = Split-Path $PSScriptRoot
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

$ver = '1.0.0'
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$out = Join-Path $dist "GlobalTokenTrackerPP-Setup-$ver-win-x64.exe"
Copy-Item (Join-Path $target 'globaltokentracker-setup.exe') $out -Force

$mb  = [math]::Round((Get-Item $out).Length / 1MB, 2)
$sha = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
Write-Host ''
Write-Host "installer: $out" -ForegroundColor Green
Write-Host "size     : $mb MB"
Write-Host "sha256   : $sha"
