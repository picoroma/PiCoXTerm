# Mette dei file di esempio nel server FTP/FTPS di prova (servizio `ftp`, porta 2121), così c'è qualcosa da
# vedere e da scaricare. Si può rilanciare quando si vuole: rifà gli stessi file. Solo sviluppo.
#   powershell -ExecutionPolicy Bypass -File testenv/seed-ftp.ps1
$ErrorActionPreference = 'Stop'
$c = 'pxt-test-ftp'
if (-not (docker ps --format '{{.Names}}' | Select-String -SimpleMatch $c)) {
    Write-Error "Il container $c non è in esecuzione: avvialo con `docker compose up -d ftp` (cartella testenv)."
}
$script = @'
set -e
cd /home/test
mkdir -p Documenti Immagini "Cartella con spazi" Archivio/2025 Archivio/2026
echo "Benvenuto sul server FTP/FTPS di prova." > LEGGIMI.txt
echo "Relazione di prova" > Documenti/relazione.txt
printf 'nome;valore\nuno;1\ndue;2\ntre;3\n' > Documenti/dati.csv
echo "contenuto con accenti: è à ù ò" > "Cartella con spazi/file con spazi.txt"
head -c 300000 /dev/urandom > Immagini/foto.bin
head -c 5000000 /dev/urandom > Archivio/2025/grande-5MB.bin
echo "log 2026" > Archivio/2026/log.txt
chown -R ftpuser:ftpgroup /home/test
echo "File di esempio creati in /home/test"
'@
$script | docker exec -i $c sh
Write-Host ''
Write-Host 'Pronto. Collegati con:'
Write-Host '  FTP in chiaro        127.0.0.1  porta 2121  utente test  password test'
Write-Host '  FTPS esplicito (TLS) 127.0.0.1  porta 2121  utente test  password test (certificato autofirmato)'
