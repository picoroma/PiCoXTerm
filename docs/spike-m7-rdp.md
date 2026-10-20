# PiCoXterm: report dello spike RDP integrato (IronRDP)

> 20 ottobre 2026. Riferimento: `SPECIFICHE.md` 9.1 e D-08, `ROADMAP.md` (M7).
> Motivo: l'RDP con client esterno (D-31) apre una finestra fuori dal programma e non è a schede.

## Esito complessivo

**Go** su IronRDP (Devolutions) per la sessione RDP in una scheda. Un programma di prova di circa 200 righe si collega a un server RDP, esegue l'autenticazione **NLA/CredSSP**, riceve e decodifica la grafica e produce un'immagine corretta, usando solo `rustls` con `ring`. Resta un difetto di IronRDP da correggere (una patch di 20 righe, vedi sotto) e restano da costruire input, appunti, ridimensionamento e integrazione nell'interfaccia.

## Verifiche puntuali

| Verifica | Esito | Note |
|---|---|---|
| Licenze | **Superata** | Tutti i crate `ironrdp-*`: MIT OR Apache-2.0; `sspi` (CredSSP/NTLM/Kerberos): MIT OR Apache-2.0. Compatibili con la GPL-3.0 e con `deny.toml` |
| Solo `ring`, niente `aws-lc` né OpenSSL | **Superata** | Senza `ironrdp-tls` (che porta `aws-lc` tramite `tokio-rustls`) e con la funzione `reqwest-rustls-ring` di `ironrdp-tokio`, `cargo tree` non contiene `aws-lc-rs`, `aws-lc-sys`, `openssl`, `native-tls` |
| Passaggio a TLS con il nostro codice | **Superata** | IronRDP non fa il TLS: lo fa il chiamante dopo `connect_begin`. Il suo `ironrdp-tls` accetta **qualunque certificato** senza chiedere nulla: noi usiamo un nostro verificatore con impronta SHA-256 confermata dall'utente (come per l'FTPS, regola 6) |
| Connessione con NLA/CredSSP | **Superata** | Contro xrdp (container di prova): TLS 1.3, `AES_256_GCM_SHA384`, connessi in circa 85 ms, desktop 1280x800 |
| Decodifica grafica | **Superata con una patch** | Vedi sotto |
| Nessuna porta in ascolto | **Superata** | Solo connessione in uscita |
| Windows Server reale (NLA con dominio, RemoteFX/EGFX) | **Da provare** | Il server di prova è Linux (xrdp). Va provato su un server Windows vero |

## Conflitto di dipendenze con `russh` (trovato nel passo 1)

Compilando IronRDP nello stesso workspace dell'app, `cargo` si ferma: `picky` e `sspi` fissano con `=` versioni *release candidate* delle librerie crittografiche (`curve25519-dalek =5.0.0-rc.1`, `ed25519-dalek =3.0.0-rc.1`, `aes-gcm =0.11.0-rc.4`...) e `russh` usa le versioni finali. Non possono stare nello stesso `Cargo.lock`. Soluzione: il motore RDP è un **programma a parte** con il suo `Cargo.lock` (D-33), collegato all'app da pipe.

## Il difetto trovato

Nei bitmap compressi (RLE a 15/16/24 bit e RDP 6.0 a 32 bit) IronRDP 0.17 / `ironrdp-session` 0.11 decodifica le righe con la larghezza `update.width`, che è arrotondata a multiplo di 4, ma poi le copia usando la larghezza del rettangolo di destinazione. Quando le due differiscono l'immagine esce **inclinata a strisce** (vedi la prova prima della correzione).

La correzione è togliere il riempimento da ogni riga prima di copiare: `ironrdp-session-crop-padded-rows.patch` (58 righe di diff). Con la patch la schermata di login di xrdp è perfetta (`xrdp-login-after-patch.png`).

Windows di norma usa RemoteFX/EGFX e potrebbe non incontrare mai il difetto, ma non conviene dipendere dal caso. Strada: **segnalare e proporre la patch a monte** e nel frattempo usare `[patch.crates-io]` con una copia locale del crate.

## Cosa manca per un client completo

1. **Sessione nel backend** (`pxt-rdp`, async): connessione con credenziali dal `SecretProvider`, conferma del certificato (impronta salvata nella sessione), ciclo di lettura, riconnessione, chiusura pulita. **Fatto** (passo 1, D-33): motore in un processo a parte, `RdpSession`, comandi `rdp_connect`/`rdp_input`/`rdp_close`, prove contro xrdp; manca la riconnessione.
2. **Grafica verso l'interfaccia**: rettangoli modificati come byte RGBA su `tauri::ipc::Channel` (niente eventi globali), disegno su canvas.
3. **Input**: mouse, rotella, tastiera con scancode (layout italiano, tasti morti, Ctrl+Alt+Canc, tasto Windows).
4. **Ridimensionamento dinamico** (`ironrdp-displaycontrol`) e scala.
5. **Appunti** (`ironrdp-cliprdr`), e a seguire unità condivise (`ironrdp-rdpdr`) e audio, se servono.
6. **Scheda con "sgancia"**: la scheda può staccarsi in una finestra propria o passare a schermo intero, e tornare nella scheda.
7. **Server con RD Gateway**: IronRDP non parla il protocollo nativo del gateway Microsoft (HTTP/RPC). Le sessioni con gateway restano sul **client di sistema** (D-31, già realizzato); l'opzione "client" diventa "integrato" o "di sistema".
8. **Kerberos**: `sspi` lo supporta ma serve un client di rete per il KDC; si parte con NTLM (domini che lo consentono) e si verifica nel tuo ambiente.

## Rischi

| Rischio | Mitigazione |
|---|---|
| Difetti nella decodifica (come quello trovato) con server diversi | Patch locale e segnalazione a monte; prova su Windows Server, xrdp e Windows 11 |
| Prestazioni dell'IPC con grafica a schermo intero | Inviare solo i rettangoli modificati; misurare prima di ottimizzare |
| Tastiera (layout, tasti morti, combinazioni di sistema) | Scancode dal tasto fisico (`KeyboardEvent.code`), prove con layout italiano |
| Policy aziendali (NLA obbligatoria, Kerberos, gateway) | Fallback al client di sistema, già pronto |
| Crate giovani (versioni 0.x, API che cambiano) | Fissare le versioni, isolare tutto in `pxt-rdp` |

## Proposta di sequenza (M7, fase 2)

1. Backend: connessione e grafica, con test contro il container xrdp (`testenv/`).
2. Scheda con canvas, mouse e tastiera.
3. Ridimensionamento, appunti.
4. Sgancia / schermo intero.
5. Prova su Windows Server e Windows 11 reali; rifinitura; selezione tra client integrato e di sistema.

## File dello spike

`docs/spike-m7-rdp/`: `main.rs` e `Cargo.toml` del programma di prova (non fanno parte del workspace), la patch e l'immagine ottenuta. Per rieseguirlo: `docker compose up -d rdp` in `testenv/`, poi `cargo run -- 127.0.0.1 3390 pxtuser pxtpass-1234` dalla cartella dello spike, applicando la patch a `ironrdp-session` (con `[patch.crates-io]`).
