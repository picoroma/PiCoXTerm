# PiCoXterm: report dello spike M0

> 15 ottobre 2026. Riferimento: `ROADMAP.md`, sezione M0.

## Esito complessivo

**Go** sullo stack Tauri 2 + Rust + Svelte 5 + russh + xterm.js, con un rischio aperto sulla memoria (vedi sotto).

Una finestra PiCoXterm su Windows si collega via SSH (password) a un server di test, mostra il terminale xterm.js con lo streaming binario tramite `tauri::ipc::Channel`, e traccia la directory corrente (OSC 7). Verificato a mano da Francesco sulla build `draft2`.

## Verifiche puntuali

| Verifica | Esito | Note |
|---|---|---|
| Connessione SSH + terminale xterm.js via `Channel` | **OK** | Password, PTY, resize, input. Provato su OpenSSH in Docker |
| Host key sconosciuta: dialogo e fingerprint (SEC-03) | **OK** | Accetta e salva in `known_hosts` proprio |
| Host key cambiata: blocco (SEC-04) | Implementato e coperto da unit test; **prova manuale da fare** | Rigenerare le chiavi del container e riconnettersi |
| OSC 7: intercettazione nel flusso | **OK** | Parser con 5 unit test (BEL, ST, percent-encoding, chunk spezzati, sequenze troppo lunghe) |
| OSC 7: iniezione su bash | **OK** | Provato dalla UI sul container di test |
| OSC 7: iniezione su tcsh | Implementata, **non provata** | Serve un server con tcsh |
| Copia su selezione, incolla con tasto destro, avviso multiriga | **OK** | Plugin ufficiale clipboard-manager, permessi minimi |
| Throughput: `cat` di un file da 52 MB | **OK** (UI fluida, output completo) | Tempo di `time` non riportato; da rimisurare con numeri |
| Keyring su Windows (scrivi, leggi, elimina) | **OK** | Crate `keyring` 4.x, feature `v1`, Credential Manager |
| Chiavi `.ppk` | API presente in russh (`PrivateKey::from_ppk`); **non provata con un file reale** | Piano B: conversione guidata con puttygen |
| Pageant | Supporto presente (`AgentClient::connect_pageant`); **non verificabile** | Nessun Pageant in esecuzione su questo PC |
| Agent OpenSSH di Windows (named pipe) | Supporto presente (`connect_named_pipe`); **non verificabile** | Servizio `ssh-agent` disattivato su questo PC |
| Dimensione build release | **OK**: exe singolo da ~4,5 MB | NFR-01 (installer ≤ 20 MB) ampiamente rispettato |
| Memoria a riposo | **Da rivedere** | ~273 MB di memoria privata (app + 8 processi WebView2), senza sessioni aperte. Obiettivo NFR-03: ≤ 250 MB con 5 schede |
| Zero porte in ascolto (SEC-01) | **OK** | `netstat` non mostra porte in LISTEN per il processo (build release) |

Le sonde `agent_probe` e `keyring_probe` sono negli `examples/` dei crate.

## Rischi e piani B

1. **Memoria (NFR-03).** Quasi tutto il consumo è il motore WebView2, non il codice dell'app. Con più schede cresce poco perché il motore è condiviso. Azioni: misurare con 5 schede reali e confrontare con MobaXterm; se resta sopra 250 MB, rivedere l'obiettivo con una decisione registrata in sezione 12 delle specifiche.
2. **`.ppk`, Pageant, agent OpenSSH.** Il supporto c'è nel codice ma non è provato end-to-end. Da verificare in M1 con un file e un agent reali. Piano B per `.ppk`: conversione guidata in formato OpenSSH.
3. **Crittografia di russh.** Il backend di default (`aws-lc-rs`) richiede NASM per compilare su Windows. Usato `ring`: nessuna dipendenza esterna per chi sviluppa, ma da registrare come scelta (vedi D-11).
4. **Terminale.** Lo scrollback è 10.000 righe (default delle specifiche): un output enorme fa uscire i comandi precedenti. Diventerà configurabile in M1.

## Aspetti emersi da ricordare

- Con `cargo build` diretto il frontend non viene incorporato nell'exe: serve la feature `custom-protocol` di Tauri (ora attiva di default in `src-tauri/Cargo.toml`).
- In sviluppo (`tauri dev`) il frontend gira su `127.0.0.1:1420`: è una porta in ascolto **solo in sviluppo**, assente nell'exe di release.
- La password passa oggi dal form direttamente al backend e non viene salvata. Dal salvataggio delle sessioni (M1) passerà solo da `SecretProvider`.

## Criteri di accettazione di M0

| Criterio | Esito |
|---|---|
| Connessione a un server di test da una finestra PiCoXterm su Windows | **OK** |
| Uso corretto di `vim` e `htop` | **Da confermare a mano** (shell e output verificati; le due app non ancora dichiarate provate) |
| Report con esito di ogni verifica e piano B per quelle negative | **OK**, questo documento |
| Go/no-go sullo stack | **Go** |

Restano fuori da M0, da fare in M1: `testenv/` con utenti bash/zsh/ksh/tcsh, FTP TLS e MinIO; la CI su tre piattaforme.
