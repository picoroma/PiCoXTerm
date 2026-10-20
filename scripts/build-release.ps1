# Compila PiCoXterm per l'uso: interfaccia, app e motore RDP, e mette tutto in target/release.
# Se picoxterm.exe è aperto Windows blocca la sostituzione: chiudilo prima.
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

npm --prefix ui run build
if ($LASTEXITCODE -ne 0) { throw 'build dell''interfaccia fallita' }

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'build dell''app fallita' }

# Il motore RDP è un programma a parte (D-33), con il suo Cargo.lock.
cargo build --release --manifest-path rdp-engine/Cargo.toml
if ($LASTEXITCODE -ne 0) { throw 'build del motore RDP fallita' }
Copy-Item rdp-engine/target/release/picoxterm-rdp.exe target/release/picoxterm-rdp.exe -Force

Get-ChildItem target/release/picoxterm.exe, target/release/picoxterm-rdp.exe | Select-Object Name, Length, LastWriteTime
