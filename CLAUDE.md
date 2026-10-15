# CLAUDE.md: PiCoXterm

Istruzioni per Claude Code su questo repository.

## Il progetto

PiCoXterm è un client di accesso remoto "MobaXterm-like" ma essenziale, multipiattaforma (Windows, Linux, macOS).
Protocolli: SSH con SFTP integrato (priorità massima), SFTP, FTP/FTPS, S3, RDP, VNC.

Documenti di riferimento (leggerli prima di iniziare una milestone):
- `docs/SPECIFICHE.md`: specifiche di dettaglio e decisioni registrate (sezione 12)
- `docs/ROADMAP.md`: milestone, attività e criteri di accettazione

## Stack

- Tauri 2, backend Rust (stable), frontend Svelte 5 + TypeScript + Vite
- Terminale: xterm.js. **Non scrivere un emulatore di terminale.**
- SSH/SFTP: russh + russh-sftp · FTP: suppaftp · S3: aws-sdk-s3 · Segreti: keyring · VNC: noVNC

## Regole non negoziabili

1. **Zero porte in ascolto.** Nessun server, nessun proxy WebSocket locale, nessun listener. Solo connessioni in uscita.
2. **Nessun segreto su disco in chiaro, né nei log.** Password, passphrase e secret key passano solo da `SecretProvider`.
3. **Nessuna funzione esclusa** dalla sezione 2.2 delle specifiche senza una nuova decisione registrata nella sezione 12.
4. **I crate `pxt-*` non dipendono da Tauri.**
5. Sessioni e segreti solo tramite i trait `SessionProvider` e `SecretProvider`: niente accessi diretti a file o keyring dalla UI o dai comandi Tauri.
6. Host key sconosciute o cambiate non vengono **mai** accettate in silenzio.
7. Trasferimenti di file sempre in streaming: mai un file intero in memoria.

## Convenzioni

- Rust: `cargo fmt`, `cargo clippy -- -D warnings`; errori con `thiserror` nei crate e `anyhow` solo nell'app; async con tokio.
- Comandi Tauri sottili: la logica sta nei crate `pxt-*`.
- Flussi continui (terminale, avanzamento, VNC) via `tauri::ipc::Channel`, non eventi globali.
- Stringhe UI esternalizzate (i18n it/en) dal primo giorno.
- Test: unit test nei crate; integration test contro `testenv/` (docker-compose).
- Commit piccoli, messaggi in inglese, formato conventional commits (`feat:`, `fix:`, `docs:`...).

## Come lavorare con Francesco

- Procedere **una milestone alla volta** e, dentro la milestone, un passo alla volta: proporre il passo, eseguirlo, fermarsi per conferma.
- Dare un parere critico e onesto: segnalare quando una richiesta va contro le specifiche o è una cattiva idea.
- Quando c'è una scelta da fare, presentare le opzioni con una raccomandazione chiara.
- Dopo ogni decisione rilevante, aggiornare la sezione 12 di `docs/SPECIFICHE.md`.
- A fine milestone, verificare uno per uno i criteri di accettazione di `docs/ROADMAP.md` e riportarne l'esito.

## Stato attuale

- M0 (spike) concluso con esito **go**: vedi `docs/spike-m0.md`. Decisioni D-10..D-13 registrate.
- Milestone corrente: **M1 (SSH a schede e sessioni locali)**. Esiste già: connessione SSH con password, terminale xterm.js, known_hosts con dialogo host key, tracciamento cwd (OSC 7) per bash/tcsh, copia/incolla.
- Gotcha: con `cargo build` diretto serve la feature `custom-protocol` (attiva di default); il backend crittografico di russh è `ring` (D-11); nei comandi shell con heredoc i backslash possono essere alterati, preferire il tool Write per file con escape.
- Prossimo passo: modello dati e provider di sessioni/segreti in `pxt-core`.
