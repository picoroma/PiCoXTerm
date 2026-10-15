# PiCoXterm

Client di accesso remoto leggero, ispirato a MobaXterm ma essenziale e "auditabile per costruzione".
Windows, Linux, macOS · Tauri 2 + Rust · terminale xterm.js.

## Cosa fa oggi (M1 in corso)

**SSH**
- Terminale a schede (xterm.js): copia su selezione, incolla con il tasto destro, avviso per incolla multiriga, ricerca nello scrollback (Ctrl+Maiusc+F), zoom con Ctrl+rotella
- Autenticazione con password, chiave privata (OpenSSH e `.ppk`, con passphrase) o agent (Pageant, agent OpenSSH di Windows, `SSH_AUTH_SOCK`)
- **Jump host** a catena (fino a 5), senza aprire porte locali
- **Host key**: conferma con fingerprint (accetta una volta, accetta e salva, annulla); se la chiave cambia la connessione è bloccata e si sblocca solo rimuovendo la voce salvata
- Tracciamento della directory corrente del terminale (OSC 7, con iniezione per bash e tcsh): base del futuro pannello SFTP
- "Prova connessione" senza aprire la shell, riconnessione nella stessa scheda, Quick connect (`utente@host:porta`)

**Sessioni e impostazioni**
- Albero di cartelle e sessioni con filtro, trascinamento, duplica; password e passphrase solo nel keyring di sistema, mai nel file JSON
- Aspetto per sessione: tema, font, sfondo e colore della scheda (utile per distinguere PROD, TEST e DEV)
- Temi (scuro, chiaro, Solarized, Dracula), font, scrollback e opzioni di copia/incolla configurabili
- Esportazione e importazione delle sessioni in JSON, senza segreti
- Log di audit (JSON Lines, con rotazione) di connessioni e host key, senza credenziali né contenuti di sessione
- Interfaccia in italiano e inglese
- Modalità portabile: un file `portable.flag` accanto all'eseguibile sposta i dati nella cartella `data/` (i segreti restano nel keyring di sistema)

## Cosa è previsto

- **M2**: pannello SFTP che segue la directory del terminale, con coda dei trasferimenti
- **M3**: sessioni SFTP, FTP/FTPS e S3 con file browser a doppio pannello
- **M4**: RDP (client esterno) e VNC
- **M5 e oltre**: import da MobaXterm, installer firmati, provider Devolutions RDM
- **Backlog**: X11 forwarding e, a fasi, un X server (vedi sezione 6.7 delle specifiche); strumenti di diagnosi di rete (ping, verifica porte, porte in ascolto di un server via SSH, sezione 6.8)

Dettagli e criteri di accettazione in [docs/ROADMAP.md](docs/ROADMAP.md).

## Principi di sicurezza

- **Zero porte in ascolto**: solo connessioni in uscita
- Nessun segreto su disco in chiaro, né nei log
- Host key sconosciute o cambiate non vengono mai accettate in silenzio
- Nessun server integrato, nessuna shell locale, nessuna telemetria
- Dipendenze permissive controllate da `cargo deny` (licenze, advisory, sorgenti)

Cosa **non** fa, per scelta: nessun server integrato, nessuna porta in ascolto, nessuna shell locale. X server (a fasi) e alcuni strumenti di rete di diagnosi sono in backlog, dopo la v1.

## Sviluppo

Servono Rust (stable), Node.js e, su Windows, le Build Tools di Visual Studio.

```
cd ui && npm install && npm run build   # frontend, incorporato nell'eseguibile
cd .. && cargo build --release -p picoxterm
```

L'eseguibile è `target/release/picoxterm.exe`.

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd ui && npm run check
```

Le prove end-to-end (autenticazione, jump host, host key) girano contro un server SSH di prova in Docker e non fanno parte della CI. Istruzioni in [testenv/README.md](testenv/README.md).

Per provare l'interfaccia in un browser, senza Tauri, con un backend finto: `npm --prefix ui run dev` (solo sviluppo, non finisce nell'eseguibile).

## Documentazione

- [Specifiche](docs/SPECIFICHE.md): architettura, modello dati, sicurezza e decisioni registrate (sezione 12)
- [Roadmap](docs/ROADMAP.md): milestone e criteri di accettazione
- [Report dello spike M0](docs/spike-m0.md)

## Stato

M0 (spike tecnico) concluso con esito favorevole. Milestone corrente: **M1** (SSH a schede e sessioni locali). Restano da chiudere in M1 la verifica dei criteri di accettazione e alcune rifiniture (per esempio `keyboard-interactive`, non prioritario).

## Licenza

Da definire.
