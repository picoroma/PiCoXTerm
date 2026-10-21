# Impacchetta PiCoXterm come MSIX per lo Store (D-40). Richiede il winapp CLI
# (winget install Microsoft.winappcli --source winget) e una build release già fatta
# o la rifà con scripts/build-release.ps1.
#
# Per la sola prova in locale, genera e installa un certificato di sviluppo con:
#   winapp cert generate --if-exists skip
#   winapp cert install .\devcert.pfx      (come amministratore, una volta sola)
# Lo Store invece firma il pacchetto da solo: quel certificato non serve per la pubblicazione.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

& "$PSScriptRoot/build-release.ps1"

$stage = Join-Path $root 'msix/dist'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item "$root/target/release/picoxterm.exe", "$root/target/release/picoxterm-rdp.exe" $stage
Copy-Item "$root/msix/Package.appxmanifest" $stage
Copy-Item "$root/msix/Assets" $stage -Recurse

$env:WINAPP_CLI_TELEMETRY_OPTOUT = '1'
Push-Location "$root/msix"
try {
    $cert = Join-Path $root 'msix/devcert.pfx'
    winapp pack dist --cert $cert
    if ($LASTEXITCODE -ne 0) { throw 'winapp pack fallito' }
}
finally {
    Pop-Location
}

Get-ChildItem "$root/msix/*.msix" | Select-Object Name, Length, LastWriteTime
