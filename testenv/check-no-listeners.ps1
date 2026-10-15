<#
.SYNOPSIS
  Verifica SEC-01: PiCoXterm non deve avere porte in ascolto (TCP in Listen, socket UDP aperti).

.DESCRIPTION
  Due modalità.

  Predefinita: fotografa tutti i socket del PC, avvia l'exe, attende, fotografa di nuovo e controlla
  che nessun socket nuovo appartenga all'app o ai suoi processi WebView2.

  -Attach: non avvia nulla. Controlla l'istanza già aperta. Usala dopo aver aperto una sessione SSH
  (anche con jump host o trasferimenti in corso) per verificare che l'uso non apra porte.

  Esce con codice 0 se l'app non ha socket in ascolto, 1 altrimenti.

.EXAMPLE
  .\testenv\check-no-listeners.ps1
  .\testenv\check-no-listeners.ps1 -Attach
#>
param(
    [switch]$Attach,
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\picoxterm.exe'),
    [int]$WaitSeconds = 8
)

function Get-Sockets {
    @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue |
        ForEach-Object { "TCP|$($_.LocalAddress)|$($_.LocalPort)|$($_.OwningProcess)" }) +
    @(Get-NetUDPEndpoint -ErrorAction SilentlyContinue |
        ForEach-Object { "UDP|$($_.LocalAddress)|$($_.LocalPort)|$($_.OwningProcess)" })
}

function Get-AppProcessIds {
    $ids = @(Get-Process -Name picoxterm -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
    # I processi di WebView2 dell'app hanno "picoxterm" nella riga di comando (cartella dati).
    $web = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -match 'picoxterm' } | Select-Object -ExpandProperty ProcessId)
    $ids + $web
}

$started = $null
$before = @()
if ($Attach) {
    if (-not (Get-Process -Name picoxterm -ErrorAction SilentlyContinue)) {
        Write-Host 'PiCoXterm non è in esecuzione: aprilo (e apri una sessione) e rilancia con -Attach.'
        exit 2
    }
} else {
    if (-not (Test-Path $Exe)) {
        Write-Host "Eseguibile non trovato: $Exe"
        exit 2
    }
    $before = Get-Sockets
    $started = Start-Process $Exe -PassThru
    Start-Sleep -Seconds $WaitSeconds
}

$after = Get-Sockets
$appIds = Get-AppProcessIds
$ours = @($after | Where-Object { $appIds -contains [int]($_.Split('|')[3]) })

Write-Host "Processi dell'app controllati: $($appIds.Count)"
if (-not $Attach) {
    $new = @($after | Where-Object { $before -notcontains $_ })
    Write-Host "Socket sul PC: prima $($before.Count), dopo $($after.Count); nuovi: $($new.Count)"
    foreach ($n in $new) {
        $name = (Get-Process -Id ([int]($n.Split('|')[3])) -ErrorAction SilentlyContinue).ProcessName
        Write-Host "  nuovo: $n ($name)"
    }
}

if ($ours.Count -eq 0) {
    Write-Host 'OK: nessun socket in ascolto appartiene a PiCoXterm.'
    $code = 0
} else {
    Write-Host "ATTENZIONE: $($ours.Count) socket in ascolto appartengono all'app:"
    $ours | ForEach-Object { Write-Host "  $_" }
    $code = 1
}

if ($started) {
    Stop-Process -Id $started.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    Get-Process msedgewebview2 -ErrorAction SilentlyContinue |
        Where-Object { $appIds -contains $_.Id } | Stop-Process -Force -ErrorAction SilentlyContinue
}
exit $code
