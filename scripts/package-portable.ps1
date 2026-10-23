# Pacchetto zip portabile per una release GitHub: rifà la build (scripts/build-release.ps1) e mette
# picoxterm.exe + picoxterm-rdp.exe in un file PiCoXterm-<versione>-windows-x64.zip dentro release/,
# con il suo .sha256. La versione viene da src-tauri/tauri.conf.json, come fa l'MSIX.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

& "$PSScriptRoot/build-release.ps1"

$version = (Get-Content "$root/src-tauri/tauri.conf.json" -Raw | ConvertFrom-Json).version
$name = "PiCoXterm-$version-windows-x64.zip"

$stage = Join-Path $root 'release/stage'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item "$root/target/release/picoxterm.exe", "$root/target/release/picoxterm-rdp.exe" $stage

$zipPath = Join-Path $root "release/$name"
if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
Compress-Archive -Path "$stage/picoxterm.exe", "$stage/picoxterm-rdp.exe" -DestinationPath $zipPath

$hash = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLower()
"$hash  $name" | Out-File -Encoding ascii "$zipPath.sha256"

Write-Output "Pacchetto: release/$name"
Write-Output "SHA-256:   $hash"
