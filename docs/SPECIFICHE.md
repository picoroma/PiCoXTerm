# PiCoXterm: specifiche di dettaglio

> Versione 0.1 (bozza), 15 ottobre 2026
> Autore: Francesco Cocco
> Stato: da validare con lo spike M0 (vedi `ROADMAP.md`)

---

## 1. Visione

PiCoXterm è un client di accesso remoto **"MobaXterm-like" ma essenziale**, multipiattaforma (Windows, Linux, macOS), pensato per l'uso quotidiano di un team AMS SAP Basis.

Riprende da MobaXterm **solo** ciò che serve davvero:
- l'albero delle sessioni salvate;
- il terminale SSH a schede;
- **il pannello SFTP che segue la directory corrente del terminale** (la funzione chiave);
- l'apertura rapida di sessioni RDP, VNC, FTP e S3 dallo stesso albero.

Tutto il resto (server integrati, ambiente Unix locale) è **escluso per progetto**, perché è proprio ciò che rende MobaXterm difficile da approvare in ambito aziendale. Per l'X server e per alcuni strumenti di rete l'esclusione è **sospesa e rinviata** (decisioni D-16 e D-17): se ne riparla dopo la v1, con le cautele descritte in 6.7.

### 1.1 Principio guida: "auditabile per costruzione"

Un revisore di sicurezza deve poter capire in pochi minuti cosa fa l'eseguibile:
- **zero porte in ascolto**: solo connessioni in uscita, avviate dall'utente;
- nessuna shell locale, nessun binario Unix incluso, nessun sistema di plugin;
- nessun segreto salvato in chiaro su disco;
- dipendenze open source note e con licenze permissive;
- eseguibile firmato.

---

## 2. Perimetro

### 2.1 Tipi di sessione

| Tipo | Terminale | File browser | Priorità | Milestone |
|---|---|---|---|---|
| **SSH** (con SFTP integrato) | Sì | Sì, sincronizzato con la `cwd` | **Massima** | M1–M2 |
| **SFTP** (senza terminale) | No | Sì | Alta | M3 |
| **FTP / FTPS** | No | Sì | Media | M3 |
| **S3** (AWS e compatibili) | No | Sì | Media | M3 |
| **RDP** | No | No | Media | M4 (client esterno), M7 (integrato) |
| **VNC** | No | No | Bassa | M4 |

### 2.2 Esclusioni esplicite (non-feature)

Queste funzioni **non verranno implementate**, salvo una decisione esplicita registrata nella sezione 12:

| Funzione MobaXterm | Motivo dell'esclusione |
|---|---|
| Server integrati (FTP, TFTP, HTTP, SSH, Telnet, VNC, NFS, iperf) | Trasformano il PC in un server: principale causa di blocco |
| Port scanner, network scanner, ping/traceroute grafici | Percepiti come strumenti offensivi. **Da rivalutare dopo la v1** (D-17): si valuteranno solo gli strumenti di diagnosi davvero utili |
| Gestore di tunnel SSH (MobaSSHTunnel) | Rischio di esfiltrazione; vedi 6.6 per il caso singolo |
| Shell locale, Cygwin, Busybox, plugin | Binari non controllati dall'EDR |
| X server integrato | Apre porte in ascolto. **Rinviato, non scartato** (D-16): prima l'X11 forwarding verso un X server esterno, vedi 6.7 |
| Telnet, rlogin, Mosh, seriale | Protocolli in chiaro o fuori perimetro |
| Editor di testo integrato | **Ammesso, in forma ridotta** (D-19, vedi 7.6): solo per file di testo di dimensione limitata. Resta disponibile anche l'editor di sistema |
| Macro e registrazione di tasti | Fuori perimetro v1 |
| Vault di password condiviso fatto in casa | La condivisione arriva tramite RDM (M6) |

---

## 3. Architettura

### 3.1 Stack tecnologico

| Livello | Scelta | Motivazione |
|---|---|---|
| Shell applicativa | **Tauri 2** | WebView nativa del sistema; eseguibile da ~10–15 MB; multipiattaforma |
| Backend | **Rust** (stable, edition 2021) | Sicurezza di memoria; tutti i protocolli in-process |
| Frontend | **Svelte 5 + TypeScript + Vite** | Leggero, poco boilerplate. React è un'alternativa accettabile, ma va decisa in M0 e poi non si cambia |
| Terminale | **xterm.js** (+ addon `fit`, `search`, `web-links`, `unicode11`, `webgl`) | Emulatore maturo (lo stesso di VS Code): **non si scrive un emulatore** |
| SSH / SFTP | **russh** + **russh-sftp** | Rust puro, asincrono (tokio), mantenuto attivamente |
| Chiavi | Componente chiavi di russh / crate `ssh-key` | OpenSSH; **supporto .ppk da verificare in M0** |
| FTP / FTPS | **suppaftp** (async, TLS via rustls) | Supporta TLS esplicito e implicito |
| S3 | **aws-sdk-s3** + `aws-config` | SDK ufficiale; profili, SSO, endpoint personalizzati |
| VNC | **noVNC** nel frontend, trasporto via IPC Tauri | Nessun proxy WebSocket locale (vedi 9.2) |
| RDP | v1: client esterno; v2: **IronRDP** | Vedi 9.1 |
| Segreti | crate **keyring** | Credential Manager (Win), Keychain (macOS), Secret Service (Linux) |
| Font | DejaVu Sans Mono incluso; scelta libera dell'utente | Licenza libera |

### 3.2 Struttura del repository

```
PiCoXTerm/
├─ CLAUDE.md                 # Istruzioni per Claude Code
├─ README.md
├─ docs/
│  ├─ SPECIFICHE.md          # Questo documento
│  └─ ROADMAP.md
├─ crates/
│  ├─ pxt-core/              # Modello dati, provider di sessioni e segreti, policy, errori
│  ├─ pxt-ssh/               # Connessione SSH, canale shell, known_hosts, agent, jump host
│  ├─ pxt-fs/                # Trait RemoteFs + backend sftp/ftp/s3/local, coda trasferimenti
│  ├─ pxt-rdp/               # Avvio client RDP esterno (poi IronRDP)
│  └─ pxt-vnc/               # Trasporto TCP per noVNC
├─ src-tauri/                # App Tauri: comandi IPC, stato, finestre
├─ ui/                       # Frontend Svelte
└─ testenv/                  # docker-compose con server di test (SSH, FTP, MinIO, VNC)
```

Regola: **i crate `pxt-*` non dipendono da Tauri.** Si possono testare da soli e, in futuro, riusare in una CLI.

### 3.3 Astrazioni centrali

```rust
/// Fonte dell'albero delle sessioni (locale in v1, RDM in M6).
trait SessionProvider {
    fn id(&self) -> &str;                       // "local", "rdm"
    fn is_read_only(&self) -> bool;
    async fn list(&self) -> Result<Vec<SessionNode>>;   // cartelle + sessioni
    async fn save(&self, node: SessionNode) -> Result<()>;
    async fn delete(&self, id: Uuid) -> Result<()>;
}

/// Fonte delle credenziali (keyring in v1, RDM in M6).
trait SecretProvider {
    async fn get(&self, r: &CredentialRef) -> Result<Option<Secret>>;
    async fn set(&self, r: &CredentialRef, s: Secret) -> Result<()>;
    async fn delete(&self, r: &CredentialRef) -> Result<()>;
}

/// Filesystem remoto unico per tutti i file browser.
trait RemoteFs {
    async fn list(&self, path: &str) -> Result<Vec<Entry>>;
    async fn stat(&self, path: &str) -> Result<Entry>;
    async fn read(&self, path: &str) -> Result<ByteStream>;     // streaming, mai tutto in RAM
    async fn write(&self, path: &str, src: ByteStream, size: Option<u64>) -> Result<()>;
    async fn mkdir(&self, path: &str) -> Result<()>;
    async fn remove(&self, path: &str, recursive: bool) -> Result<()>;
    async fn rename(&self, from: &str, to: &str) -> Result<()>;
    async fn set_permissions(&self, path: &str, mode: u32) -> Result<()>; // non supportato da S3
    fn capabilities(&self) -> FsCapabilities;  // permessi, symlink, rename atomico, ecc.
}
```

`SecretProvider` e `SessionProvider` vanno separati: le sessioni sono dati non segreti, condivisibili e versionabili; i segreti non escono mai dal vault.

### 3.4 Comunicazione backend ↔ frontend

- Comandi richiesta/risposta: `#[tauri::command]`.
- Flussi continui (output del terminale, avanzamento dei trasferimenti, frame VNC): **`tauri::ipc::Channel`**, non gli eventi globali, per prestazioni e isolamento per scheda.
- Input da tastiera: comando `term_write(session_id, bytes)` con batching lato frontend.
- Ogni sessione aperta ha un `ConnectionId`. Chiudere la scheda chiude la connessione e libera le risorse.

---

## 4. Modello dati

### 4.1 File delle sessioni

Formato **JSON**, leggibile, con versione dello schema. Posizione:
- **modalità installata**: `%APPDATA%\PiCoXterm\sessions.json` (Win), `~/Library/Application Support/PiCoXterm/` (macOS), `~/.config/picoxterm/` (Linux);
- **modalità portabile**: se accanto all'eseguibile c'è un file `portable.flag`, tutti i dati stanno nella cartella `data/` accanto all'eseguibile (ma i segreti restano nel keyring di sistema).

```jsonc
{
  "schemaVersion": 1,
  "nodes": [
    { "id": "f1…", "type": "folder", "name": "Clienti", "parentId": null },
    { "id": "f2…", "type": "folder", "name": "Bulgari", "parentId": "f1…" },
    {
      "id": "s1…",
      "type": "session",
      "parentId": "f2…",
      "name": "BGP PRD - app01",
      "protocol": "ssh",
      "host": "10.0.1.15",
      "port": 22,
      "username": "bgpadm",
      "credentialRef": { "provider": "local", "id": "c9…" },   // mai la password qui
      "auth": { "method": "password" },          // password | key | agent | keyboard-interactive
      "jumpHosts": ["s7…"],                       // riferimenti ad altre sessioni SSH
      "ssh": {
        "followCwd": "auto",                     // auto | osc7-only | off
        "keepAliveSecs": 30,
        "x11Forwarding": false,
        "initialCommand": null,
        "terminalType": "xterm-256color",
        "encoding": "utf-8"
      },
      "sftp": { "startPath": null, "showHidden": false },
      "ui": { "color": "#c0392b", "icon": "linux", "tags": ["PRD", "SAP"] },
      "notes": "",
      "createdAt": "2026-10-15T08:00:00Z",
      "updatedAt": "2026-10-15T08:00:00Z"
    }
  ]
}
```

Regole:
- **gli ID sono UUID v4**; i riferimenti (jump host, credenziali) avvengono per ID, così la mappatura verso RDM sarà quasi uno a uno;
- scrittura **atomica** (file temporaneo + rename) con backup rotativo (ultimi 5);
- migrazioni dello schema gestite in `pxt-core`, con test.

### 4.2 Campi specifici per protocollo

| Protocollo | Campi aggiuntivi |
|---|---|
| `sftp` | `startPath`, `jumpHosts`, `auth` |
| `ftp` | `tls`: `explicit` (default) / `implicit` / `none`; `passive` (default `true`); `certPin` |
| `s3` | `profile` (profilo AWS) oppure credenziali proprie; `region`; `endpoint` (MinIO/compatibili); `bucket` opzionale; `pathStyle` |
| `rdp` | `domain`, `client` (`auto`/`mstsc`/`windows-app`/`freerdp`), `fullscreen`, `multimon`, `resolution`, `redirectClipboard`, `redirectDrives` (default `false`), `gateway` |
| `vnc` | `password` via `credentialRef`, `viewOnly`, `quality`, `sshTunnelVia` (opzionale, vedi 9.2) |

### 4.3 Altri file

| File | Contenuto |
|---|---|
| `settings.json` | Preferenze utente: tema, font, comportamento copia/incolla, ecc. |
| `known_hosts` | Formato OpenSSH standard, file **proprio** di PiCoXterm. Possibilità di importare da `~/.ssh/known_hosts` |
| `ftp_certs.json` | Pin dei certificati FTPS accettati |
| `policy.json` | Policy amministrativa (sezione 5.4) |
| `logs/` | Log applicativo e audit delle connessioni |

---

## 5. Sicurezza

### 5.1 Requisiti obbligatori

| ID | Requisito |
|---|---|
| SEC-01 | Nessun socket in ascolto, mai. Verificabile con `netstat`/`ss` durante l'uso |
| SEC-02 | Password, passphrase e secret key **solo** tramite `SecretProvider` (keyring di sistema). Mai su disco in chiaro, mai nei log |
| SEC-03 | Host key SSH sconosciuta: dialogo con fingerprint SHA256 e scelta esplicita (accetta una volta / accetta e salva / annulla) |
| SEC-04 | Host key **cambiata**: blocco della connessione con avviso evidente. Si sblocca solo rimuovendo la voce manualmente |
| SEC-05 | FTPS come default; FTP in chiaro solo con avviso visibile nella scheda e disattivabile da policy |
| SEC-06 | Certificati TLS non validi: dialogo con fingerprint e pin (stessa logica di SEC-03) |
| SEC-07 | Port forwarding (locale, remoto, dinamico) **assente** in v1 |
| SEC-08 | Agent forwarding SSH **disattivato** di default, attivabile per singola sessione con avviso |
| SEC-09 | Nessuna telemetria, nessuna connessione verso server di terze parti; aggiornamento automatico assente o disattivabile da policy |
| SEC-10 | Frontend con CSP restrittiva, capability Tauri al minimo indispensabile, nessun caricamento di contenuti remoti nella WebView |
| SEC-11 | Le password memorizzate in RAM vengono azzerate dopo l'uso (crate `zeroize`) |
| SEC-12 | `cargo audit` e `cargo deny` (licenze e advisory) nella CI |

### 5.2 Log di audit

Una riga per evento in `logs/audit.log` (JSON Lines): apertura e chiusura della connessione, host, utente, protocollo, esito, fingerprint della host key accettata. **Mai** contenuti di sessione o credenziali. Rotazione per dimensione.

### 5.3 Registrazione della sessione (opzionale)

La registrazione dell'output del terminale su file è **disattivata di default**, attivabile per singola sessione. Avvertenza in UI: può contenere dati sensibili.

### 5.4 Policy amministrativa

File `policy.json` (opzionale) letto all'avvio da una posizione di sistema (`%ProgramData%\PiCoXterm\` su Windows, `/etc/picoxterm/` su Linux), con precedenza sulle impostazioni utente:

```jsonc
{
  "allowPlainFtp": false,
  "allowSavePasswords": true,
  "allowAgentForwarding": false,
  "allowX11Forwarding": false,
  "allowSessionLogging": true,
  "allowedProtocols": ["ssh", "sftp", "ftp", "s3", "rdp", "vnc"]
}
```

È il documento da mostrare all'IT: ogni funzione sensibile ha un interruttore centrale.

---

## 6. SSH (funzione principale)

### 6.1 Autenticazione

| Metodo | v1 | Note |
|---|---|---|
| Password | Sì | Salvabile nel keyring (se la policy lo consente) |
| Keyboard-interactive | Sì | Anche per OTP/MFA: dialogo con i prompt del server |
| Chiave privata OpenSSH (ed25519, ecdsa, rsa) | Sì | Passphrase richiesta o dal keyring |
| Chiave .ppk (PuTTY) | **Da verificare in M0** | Se non supportata: conversione guidata in OpenSSH |
| Agent: Pageant (Windows) | **Da verificare in M0** | |
| Agent: OpenSSH per Windows (named pipe) | **Da verificare in M0** | |
| Agent: `SSH_AUTH_SOCK` (Linux/macOS) | Sì | |
| Certificati SSH | Backlog | |

### 6.2 Connessione

- Jump host (equivalente di ProxyJump) a catena, riferiti ad altre sessioni salvate.
- Keepalive configurabile; rilevamento della disconnessione con banner nella scheda e pulsante **"Riconnetti"** (stessa scheda, scrollback conservato).
- Timeout di connessione configurabile (default 15 s).
- Algoritmi: i default di russh. Possibilità per sessione di abilitare algoritmi legacy (per esempio `ssh-rsa` con SHA-1, presente su vecchi AIX/HP-UX nei landscape SAP) **con avviso**.

### 6.3 Terminale

| Funzione | v1 |
|---|---|
| Schede multiple, rinomina, colore da sessione | Sì |
| Duplica scheda (nuova connessione stessa sessione) | Sì |
| Resize corretto (`window-change`) | Sì |
| 256 colori / truecolor, UTF-8, larghezze Unicode | Sì |
| Copia su selezione e incolla con tasto destro (stile Moba/PuTTY) | Sì, configurabile |
| Avviso incolla multiriga | Sì |
| Ricerca nello scrollback (Ctrl+Shift+F) | Sì |
| Scrollback configurabile (default 10.000 righe) | Sì |
| Zoom font (Ctrl+rotella) | Sì |
| Link cliccabili | Sì |
| Temi (scuro, chiaro, più 2–3 predefiniti) | Sì |
| Split orizzontale/verticale | M5 |
| MultiExec (stesso input su più schede) | Backlog, con conferma esplicita |
| X11 forwarding verso un X server esterno | Backlog (fase 1 di 6.7), disattivato da policy |

### 6.4 SFTP integrato: il pannello che segue la directory

Layout: albero delle sessioni a sinistra; sotto (o a fianco) il **pannello SFTP** della scheda attiva, come in MobaXterm.

**Canale:** l'SFTP usa un **sottosistema sulla stessa connessione SSH** (nessuna seconda autenticazione).

**Tracciamento della `cwd`, in ordine di preferenza:**

1. **OSC 7**: se la shell remota emette `ESC ] 7 ; file://host/path BEL`, il backend intercetta la sequenza nel flusso dati e aggiorna il pannello. Nessuna modifica lato server.
2. **Iniezione (`followCwd: auto`)**: dopo il login, PiCoXterm invia **una sola riga** che aggiunge l'emissione di OSC 7 al prompt, in base alla shell rilevata:
   - **bash**: `PROMPT_COMMAND` con `printf '\033]7;file://%s%s\007'`;
   - **zsh**: funzione `precmd`;
   - **ksh / ksh93** (tipico di AIX): sequenza incorporata in `PS1`;
   - **csh / tcsh** (tipico degli utenti `<sid>adm` SAP): `alias precmd`.
   La riga è preceduta da uno spazio (non finisce nella history con `HISTCONTROL=ignorespace`) e seguita da `clear`.
3. **Fallback**: pulsante **"Sincronizza con il terminale"** che legge la directory tramite un canale exec separato, quando possibile.

**Limite noto (da documentare in UI):** dopo `sudo su - <sid>adm` il tracciamento si perde, perché la nuova shell non ha l'iniezione. È lo stesso comportamento di MobaXterm. Mitigazione v1: pulsante **"Re-inietta tracciamento"** nella scheda, che reinvia la riga per la shell corrente. Inoltre il pannello SFTP lavora con l'utente di login, quindi i file di `<sid>adm` potrebbero non essere scrivibili (vedi D-24: non è prevista una vista con un altro utente).

**Opzioni:** "Segui terminale" attivabile/disattivabile con un'icona (lucchetto); con il pannello sbloccato si naviga liberamente.

### 6.5 Rilevamento della shell

Al login si legge `$SHELL` tramite un canale exec (`echo $SHELL`) in parallelo all'apertura della shell interattiva. Se il canale exec non è disponibile, si usa l'heuristica sul prompt oppure si disattiva l'iniezione.

### 6.6 Port forwarding

Assente in v1 (SEC-07). Se in futuro servisse (per esempio un tunnel singolo verso SAP GUI o HANA Studio), sarà:
- solo **locale verso 127.0.0.1**, definito nella sessione, mai dinamico;
- disattivato di default e controllato da `policy.json`;
- registrato nel log di audit.

Nota: un port forward locale apre una porta in ascolto su loopback. Questo va dichiarato esplicitamente come eccezione a SEC-01.

---

### 6.7 X11 e X server (backlog, non prioritario)

Le applicazioni grafiche remote (per esempio SAPinst, `hdbsetup`, gli installer Oracle) richiedono X11. L'esigenza è reale e viene affrontata **a fasi**, dalla meno rischiosa alla più impegnativa:

| Fase | Contenuto | Porte in ascolto | Note |
|---|---|---|---|
| 1 | **X11 forwarding verso un X server esterno** (VcXsrv, X410, XQuartz…). PiCoXterm inoltra il canale X11 della sessione SSH e si collega **in uscita** al `DISPLAY` locale | Nessuna aperta da PiCoXterm (l'eventuale ascolto è del server X esterno) | Per sessione, disattivato di default, con avviso; interruttore `allowX11Forwarding` in `policy.json`; evento nel log di audit |
| 2 | **Avvio guidato** di un X server già installato: rilevamento e avvio con parametri sicuri (accesso solo locale) | Del server X esterno | Nessun binario incluso nel pacchetto |
| 3 | **X server integrato** | Sì: va dichiarata un'eccezione esplicita a SEC-01, come per il port forwarding (6.6) | Da decidere solo se 1 e 2 non bastano. Preferire socket Unix o named pipe alla porta TCP; costo di realizzazione molto alto; disattivato di default e controllato da policy |

**Come lo risolve MobaXterm** (dalla sua documentazione ufficiale, analizzata il 15 ottobre 2026):

| Aspetto | MobaXterm |
|---|---|
| Server X | **Integrato nell'eseguibile**, basato su Xorg compilato per Windows; supporta OpenGL, Composite, Randr e XDMCP |
| Forwarding | Casella **"X11-Forwarding"** nella sessione SSH: reindirizzamento automatico del `DISPLAY` nel canale SSH. Il traffico è cifrato, comprimibile e non richiede altre porte o regole firewall. Funziona anche attraverso un jump host |
| `DISPLAY` | Impostato in automatico; sul server compare come `localhost:10.0` (display virtuale creato da sshd) |
| Avvio | Opzione "avvia l'X server all'apertura", scorciatoia Ctrl+Maiusc+X, icona di stato; `-noX` lo disattiva |
| Impostazioni X11 | Motore di rendering, tastiera, monitor, *display offset*, appunti (disattivato / attivo / senza copia-su-selezione), accelerazione OpenGL, modalità finestra (multiwindow, finestra singola, schermo intero, con window manager integrato Fvwm o Dwm) |
| Sicurezza | "X11 remote access": **Restricted** (controllo accessi, host abilitati con `xhost`) oppure **Full** (nessun controllo: le applicazioni remote possono ascoltare e simulare tasti). Le connessioni inoltrate via SSH passano senza avvisi |
| **Punto critico** | Il server X **ascolta su tutte le interfacce di rete** (127.0.0.1 e ogni scheda). È proprio il comportamento che gli strumenti di sicurezza aziendali segnalano |

Cosa riprendere: la casella per sessione, il `DISPLAY` automatico, lo stato visibile del server X, gli appunti condivisi, le modalità di finestra, il funzionamento attraverso i jump host. Cosa cambiare: l'ascolto va limitato a **loopback o a un canale locale** (socket Unix / named pipe) con autenticazione obbligatoria, mai su tutte le interfacce.

**Architettura consigliata per la fase 3:** l'X server non entra nell'eseguibile principale. Diventa un **componente opzionale separato** (proprio binario, firmato, con propria approvazione IT), avviato e controllato da PiCoXterm. L'app principale resta così senza porte in ascolto e verificabile, e chi non usa X11 non porta il rischio. Le fasi 1 e 2 danno già la stessa comodità d'uso con un X server esterno.

Requisiti comuni: autenticazione X11 con cookie `MIT-MAGIC-COOKIE-1` generato per sessione, mai accesso non autenticato; nessuna esposizione oltre il loopback; tutto registrato nell'audit (5.2). Resta valido SEC-08: nessuna funzione "pericolosa" attiva senza una scelta esplicita dell'utente.

### 6.8 Strumenti di rete di diagnosi (backlog, non prioritario)

Richiesta dell'utente (15 ottobre 2026, D-17): ping e verifica dell'apertura delle porte. Perimetro proposto, con strumenti di **diagnosi** e non di attacco:

| Strumento | Come funziona | Porte in ascolto | Note |
|---|---|---|---|
| **Ping** | ICMP (su Windows con l'API di sistema, senza amministratore) con ripiego su prova di connessione TCP | Nessuna | Esito con tempi minimo, medio e massimo |
| **Verifica di una porta o di un intervallo** | Tentativo di connessione TCP in uscita dal PC verso host e porta (o intervallo, per esempio 3200-3299) | Nessuna | Raggiungibile anche **dal punto di vista di un server**: la prova parte dal jump host o dal server tramite `direct-tcpip`, utile per verificare i firewall tra due server |
| **Porte in ascolto su un server** | Esegue `ss -tlnp` (ripiego `netstat`) sul server tramite una sessione SSH già aperta e mostra una tabella (porta, indirizzo, processo) | Nessuna | È una *lettura* della configurazione del server, non una scansione |
| Scansione di tutte le 65.535 porte di un host | Non prevista: per "tutte le porte in ascolto" si usa la lista sul server (riga precedente), scelta dell'utente il 15 ottobre 2026 | Nessuna | Resta esclusa (2.2); si può riaprire con una nuova decisione |

Regole comuni: tutto in uscita (SEC-01), interruttore `allowNetworkTools` in `policy.json` (default consentito solo per ping e porta singola), ogni uso registrato nel log di audit (5.2), limiti di velocità e di numero di porte per richiesta.

---

## 7. File browser unico

### 7.1 Componente

Un unico componente UI, usato in due modalità:
- **pannello laterale** sotto una scheda SSH (singolo pannello remoto);
- **scheda a doppio pannello** (locale ↔ remoto) per sessioni SFTP, FTP e S3.

### 7.2 Funzioni

| Funzione | SFTP | FTP | S3 |
|---|---|---|---|
| Navigazione, barra del percorso modificabile, preferiti | Sì | Sì | Sì |
| Ordinamento, file nascosti, filtro rapido | Sì | Sì | Sì |
| Upload/download con drag & drop (anche dall'Esplora risorse) | Sì | Sì | Sì |
| Cartelle ricorsive | Sì | Sì | Sì (prefissi) |
| Crea cartella, rinomina, elimina (con conferma) | Sì | Sì | Rinomina = copia + elimina, con avviso |
| Permessi (chmod) | Sì | Se il server lo supporta | No |
| Proprietario/gruppo visibili | Sì | Se disponibili | No |
| Symlink | Sì | No | No |
| Modifica con l'**editor integrato** (clic destro, Modifica) | Sì | Sì | Sì |
| Apri con l'editor di sistema e ricarica al salvataggio | Sì | Sì | Sì |

### 7.3 Coda dei trasferimenti

- Coda unica globale, visibile in un pannello a scomparsa.
- Concorrenza configurabile (default 3 per connessione).
- Avanzamento, velocità, tempo stimato; pausa, annulla, riprova.
- Conflitti: chiedi / sovrascrivi / salta / rinomina / sovrascrivi se più recente, con "applica a tutti".
- Ripresa dei trasferimenti interrotti (SFTP/FTP tramite offset; S3 multipart per file > 100 MB).
- **Streaming**: nessun file caricato interamente in memoria.
- Scrittura in un file temporaneo con rename finale quando il backend lo consente.

### 7.4 FTP / FTPS

- TLS esplicito (AUTH TLS) di default; implicito (porta 990) opzionale.
- Modalità passiva di default; attiva solo su richiesta (richiede una connessione in ingresso, quindi **va valutata rispetto a SEC-01**: proposta di non supportarla in v1).
- Parsing dei listing con `MLSD` quando disponibile, fallback su `LIST` (formati Unix e Windows/IIS).

### 7.5 S3

- Autenticazione: profili AWS da `~/.aws` (inclusi SSO e `credential_process`), oppure access key per sessione nel keyring.
- Senza bucket indicato: elenco dei bucket come cartelle di primo livello.
- Endpoint personalizzato e path-style per MinIO e storage compatibili.
- Visualizzazione della classe di storage; avviso su oggetti Glacier non scaricabili direttamente.
- Upload multipart, con pulizia degli upload incompleti in caso di annullamento.

### 7.6 Modifica dei file remoti

**Editor integrato (D-19).** Clic destro su un file di testo del pannello, voce **Modifica**: il file si apre in una scheda dell'editor di PiCoXterm, come in MobaXterm. Vantaggio di sicurezza rispetto all'editor di sistema: il contenuto resta **in memoria** e non viene scritto in una cartella temporanea sul disco (utile per profili SAP, file di configurazione con parametri sensibili). Caratteristiche:

- evidenziazione della sintassi per i formati comuni (shell, YAML, JSON, XML, INI/conf, Python, SQL, log), numeri di riga, cerca e sostituisci, annulla e ripeti, a capo automatico a scelta;
- salvataggio con Ctrl+S: prima di scrivere si controlla che il file remoto non sia cambiato dall'apertura (data e dimensione); se è cambiato si chiede cosa fare (sovrascrivi, scarica la versione nuova, annulla);
- conservazione di codifica (UTF-8 con ripiego su altre, rilevata) e fine riga (LF o CRLF), permessi e proprietario del file;
- indicatore di modifiche non salvate e conferma alla chiusura;
- **limite di dimensione** (di partenza 5 MiB) e rifiuto dei file binari: oltre il limite si propone il download oppure l'editor di sistema;
- eccezione **motivata e limitata** alla regola "mai un file intero in memoria" (CLAUDE.md, regola 7): vale solo per l'editor, solo sotto il limite, mai per i trasferimenti;
- la modifica come altro utente (sudo) non è prevista: vedi D-24.

**Editor di sistema.** "Apri" scarica il file in una cartella temporanea dedicata, lo apre con l'editor associato del sistema e osserva il file: al salvataggio chiede (o esegue, se configurato) il caricamento. Alla chiusura della sessione i file temporanei vengono eliminati.

---

### 7.7 Pannello SFTP nella sidebar (M2): cosa riprende da MobaXterm e cosa migliora

Fonte: documentazione ufficiale di MobaXterm e guide d'uso (15 ottobre 2026). MobaXterm mostra un browser nella barra laterale a sinistra, con trascinamento da e verso Esplora risorse, doppio clic per aprire e modificare un file (le modifiche vengono salvate sul server in automatico), un'opzione sperimentale "Follow SSH path" che tiene il browser nella directory del terminale, e un tipo di browser selezionabile (SFTP o SCP).

| Aspetto | MobaXterm | PiCoXterm |
|---|---|---|
| Posizione | Sidebar a sinistra | Sidebar a sinistra, **sotto l'albero**, con divisori trascinabili (larghezza e altezza ricordate) e un pulsante per **allargare** la sidebar quando serve più spazio |
| Seguire il terminale | Opzione sperimentale | Lucchetto sempre visibile: agganciato = segue la `cwd`; sbloccato = navigazione libera. La navigazione manuale non viene annullata; si torna a seguire solo al cambio di directory del terminale o riagganciando |
| Navigazione | Elenco | Campo del percorso modificabile, cronologia **indietro/avanti**, cartella superiore, aggiorna |
| Elenco | Colonne base | Colonne ordinabili (nome, dimensione, data), cartelle sempre prime, **filtro rapido**, file nascosti a richiesta, permessi e proprietario nella descrizione al passaggio del mouse; i link simbolici verso cartelle si aprono come cartelle |
| Errori | Non documentati | Messaggio chiaro (percorso inesistente, permesso negato) con "Riprova"; l'elenco precedente resta visibile |
| Modifica di un file remoto | Salvataggio automatico sul server | Prevista (M2, passo successivo): conferma prima di caricare, configurabile; controllo che il file remoto non sia cambiato nel frattempo |
| Trasferimenti | Non documentati nel dettaglio | Coda con avanzamento, annulla e riprova, gestione dei conflitti, ripresa; sempre in streaming |
| Connessione | Browser separato dal terminale | **Stessa connessione SSH** del terminale (sottosistema `sftp`): nessuna seconda autenticazione; funziona anche attraverso i jump host |

Passi di M2 completati finora: livello `pxt-fs` (elenco, informazioni, percorso di partenza, flussi di lettura e scrittura), pannello di navigazione con aggancio al terminale (rilevamento automatico della shell: bash, zsh, tcsh) e **trasferimenti di file e cartelle** (sezione 7.3): piano preliminare, un'unica scelta per i conflitti (salta, sovrascrivi, sovrascrivi se più recenti, mantieni entrambi), copia a blocchi su file temporaneo con rename finale, data e permessi conservati, avanzamento con velocità e tempo residuo, annullamento senza file parziali, errori su singoli file che non fermano il lotto, trascinamento da Esplora risorse. **Operazioni di modifica** dal tasto destro del pannello: Elimina (conferma con focus sul pulsante sicuro; cartelle con tutto il contenuto; i link simbolici non vengono mai seguiti; la radice è rifiutata), Proprietà (percorso, tipo, dimensione anche di un albero, date, proprietario e gruppo, permessi rwx con casella e ottale; "applica al contenuto" come `chmod -R` ma con i bit speciali solo sull'elemento scelto), Rinomina (F2), Nuova cartella, Copia percorso. Restano: editor integrato (D-19), trasferimenti paralleli e ripresa, apertura nell'editor di sistema, azione "cd qui" verso il terminale. Il trascinamento **dal** pannello verso Esplora risorse non è possibile con la WebView (si usa "Scarica").

---

## 8. Albero delle sessioni e interfaccia

### 8.1 Layout della finestra

```
┌──────────────────────────────────────────────────────────────────┐
│ Barra: [+ Nuova] [Quick connect: user@host:port] [⚙]             │
├───────────────┬──────────────────────────────────────────────────┤
│ Sessioni      │ [scheda 1] [scheda 2] [scheda 3]                 │
│ 🔎 filtro     │                                                  │
│ ▸ Clienti     │            Terminale (xterm.js)                  │
│   ▸ Bulgari   │                                                  │
│     ● PRD app │                                                  │
│ ▸ Personali   │                                                  │
├───────────────┤                                                  │
│ SFTP (cwd)    │                                                  │
│ /usr/sap/BGP  │                                                  │
│ 📁 SYS        │                                                  │
│ 📄 profile    │                                                  │
├───────────────┴──────────────────────────────────────────────────┤
│ Trasferimenti: 2 in corso · 1 in coda                    [▲]     │
└──────────────────────────────────────────────────────────────────┘
```

### 8.2 Albero

- Cartelle annidate, drag & drop per spostare, ordinamento manuale o alfabetico.
- Filtro rapido per nome, host, tag.
- Doppio clic: apre la sessione. Menu contestuale: apri, apri in nuova finestra, modifica, duplica, copia host, elimina.
- Icona per protocollo, colore della sessione, badge per gli ambienti (PRD/QAS/DEV) tramite tag.
- **Quick connect**: `user@host:port` apre una sessione SSH non salvata, con la possibilità di salvarla dopo.
- Più provider nello stesso albero (in M6: nodo radice "Locale" e nodo radice "RDM").

### 8.3 Editor della sessione

Dialogo con schede: **Generale** (nome, host, porta, utente, cartella) · **Autenticazione** · **Avanzate** (protocollo specifico) · **Aspetto** · **Note**.
Il pulsante "Test connessione" verifica raggiungibilità e autenticazione senza aprire la scheda.

### 8.4 Import ed export

| Funzione | Milestone |
|---|---|
| Export/import del file sessioni JSON (senza segreti) | M1 |
| **Import da MobaXterm** (`.mxtsessions` / `MobaXterm.ini`) | M5 |
| Import da PuTTY (registro di Windows) | Backlog |
| Import da `~/.ssh/config` | Backlog |

### 8.5 Scorciatoie da tastiera (default)

| Azione | Tasti |
|---|---|
| Nuova sessione | Ctrl+Shift+N |
| Quick connect | Ctrl+Shift+Q |
| Chiudi scheda | Ctrl+Shift+W |
| Scheda successiva/precedente | Ctrl+Tab / Ctrl+Shift+Tab |
| Cerca nel terminale | Ctrl+Shift+F |
| Copia / Incolla | Ctrl+Shift+C / Ctrl+Shift+V |
| Mostra/nascondi pannello laterale | Ctrl+Shift+B |

Le scorciatoie con Ctrl+Shift evitano conflitti con i programmi nel terminale (vim, mc, ecc.).

---

## 9. RDP e VNC

### 9.1 RDP

**Fase 1 (M4): client esterno.** PiCoXterm genera un file `.rdp` temporaneo (Windows) o una riga di comando (FreeRDP su Linux/macOS) e avvia il client del sistema:
- Windows: `mstsc.exe` (oppure Windows App, se configurato);
- macOS: Windows App (Microsoft), con file `.rdp`;
- Linux: `xfreerdp` / `wlfreerdp` se presenti.

Credenziali: su Windows vengono passate tramite una voce temporanea `TERMSRV/<host>` nel Credential Manager, rimossa dopo l'avvio. **Da verificare in M4** il comportamento con NLA e con le policy che vietano il salvataggio delle credenziali RDP. Su macOS/Linux, se il passaggio sicuro non è possibile, il client chiede la password all'utente.

**Fase 2 (M7): integrato con IronRDP** (libreria RDP in Rust di Devolutions) in una scheda, con rendering su canvas tramite IPC. Richiede uno spike dedicato: prestazioni, clipboard, ridimensionamento, NLA/CredSSP.

### 9.2 VNC

**noVNC** nel frontend. noVNC accetta, al posto di un URL WebSocket, un oggetto "canale" con interfaccia simile a WebSocket. PiCoXterm fornisce un **adattatore** che inoltra i byte RFB via IPC Tauri a una connessione TCP aperta dal backend Rust.

In questo modo **non serve un proxy WebSocket locale** (che violerebbe SEC-01). La compatibilità dell'adattatore con la versione di noVNC scelta va verificata nello spike di M4.

Opzione `sshTunnelVia`: il backend apre un canale `direct-tcpip` su una connessione SSH esistente verso la porta VNC. Non apre porte locali, quindi resta compatibile con SEC-01.

---

## 10. Requisiti non funzionali

| ID | Requisito | Obiettivo |
|---|---|---|
| NFR-01 | Dimensione installer Windows | ≤ 20 MB |
| NFR-02 | Avvio a freddo fino all'albero delle sessioni | ≤ 1,5 s su un laptop aziendale tipico |
| NFR-03 | Memoria con 5 schede SSH aperte | ≤ 250 MB complessivi (WebView inclusa) |
| NFR-04 | Throughput del terminale | `cat` di un file da 50 MB senza blocco della UI |
| NFR-05 | Throughput SFTP su LAN | ≥ 70% di quello di `sftp` di OpenSSH sullo stesso link |
| NFR-06 | Piattaforme | Windows 10 22H2+ / 11 (x64, arm64); macOS 13+; Ubuntu 22.04+ e derivate |
| NFR-07 | Distribuzione | Windows: MSI + ZIP portabile, firmati; macOS: DMG firmato e notarizzato; Linux: AppImage + .deb |
| NFR-08 | WebView2 su Windows | Usare quella di sistema; segnalare se manca. Valutare il runtime "fixed version" solo per la versione portabile |
| NFR-09 | Lingua | UI in inglese e italiano (i18n dal primo giorno, stringhe fuori dal codice) |
| NFR-10 | Accessibilità | Navigazione completa da tastiera dell'albero e del file browser |

---

## 11. Qualità e test

- **Unit test** nei crate `pxt-*` (modello dati, migrazioni, parsing OSC 7, parsing dei listing FTP).
- **Integration test** contro l'ambiente `testenv/` (docker-compose):
  - OpenSSH server con utenti bash, zsh, ksh, tcsh, password e chiave;
  - server FTP con TLS (per esempio pure-ftpd o vsftpd);
  - **MinIO** per S3;
  - server VNC.
- **Test manuali** su sistemi reali: SLES/RHEL (SAP), AIX con ksh se disponibile, utente `<sid>adm` con csh.
- **CI** (GitHub Actions): build su Windows, macOS e Linux; `cargo clippy -D warnings`, `cargo fmt --check`, `cargo audit`, `cargo deny`, test del frontend.
- Ogni milestone si chiude solo con i criteri di accettazione di `ROADMAP.md` verificati.

---

## 12. Decisioni registrate

| # | Data | Decisione | Motivazione |
|---|---|---|---|
| D-01 | 2026-10-15 | Nome: **PiCoXterm** | |
| D-02 | 2026-10-15 | Multipiattaforma da subito: **Tauri 2 + Rust** | Richiesta di Win/Linux/macOS; eseguibile leggero |
| D-03 | 2026-10-15 | Terminale: **xterm.js**, nessun emulatore scritto da zero | Rischio e tempi |
| D-04 | 2026-10-15 | Escluse tutte le funzioni server, di rete e l'ambiente Unix locale | Approvabilità in ambito aziendale |
| D-05 | 2026-10-15 | Sessioni **locali prima**, RDM dopo (M6), ma con `SessionProvider`/`SecretProvider` dal giorno 1 | Validare prima la UX; evitare rifacimenti |
| D-06 | 2026-10-15 | Segreti solo nel keyring del sistema; **nessun vault condiviso fatto in casa** | La condivisione del team passa da RDM |
| D-07 | 2026-10-15 | Integrazione RDM tramite **modulo PowerShell ufficiale** (poi CLI), **mai lettura diretta del DB MSSQL** | Schema non documentato, cifratura, permessi e audit di RDM |
| D-08 | 2026-10-15 | RDP: client esterno in v1, IronRDP in v2 | Non bloccare l'MVP sulla parte più difficile |
| D-09 | 2026-10-15 | FTPS default; FTP in chiaro con avviso e disattivabile da policy | Sicurezza |
| D-10 | 2026-10-15 | Framework frontend: **Svelte 5 + TypeScript + Vite** (Vite semplice, senza SvelteKit); package manager npm | Leggerezza, poco boilerplate; SvelteKit non serve in una SPA Tauri |
| D-11 | 2026-10-15 | Backend crittografico di russh: **`ring`** al posto di `aws-lc-rs` | `aws-lc-rs` richiede NASM per compilare su Windows; `ring` non ha dipendenze esterne |
| D-12 | 2026-10-15 | Appunti tramite il plugin ufficiale **tauri-plugin-clipboard-manager**, con i soli permessi `read-text` e `write-text` | `navigator.clipboard` in WebView2 può chiedere permessi a ogni incolla |
| D-14 | 2026-10-15 | Jump host: catena piatta (max 5) di sessioni SSH salvate, via `direct-tcpip` sulla connessione del salto precedente; le credenziali di ogni jump host devono essere già salvate (keyring), o usare agent / chiave senza passphrase. I `jumpHosts` di un jump host non vengono seguiti | Nessuna porta locale in ascolto (SEC-01); evita dialoghi di password per salti intermedi in v1 |
| D-15 | 2026-10-15 | Host key: tre scelte per una chiave nuova (accetta una volta / accetta e salva / annulla); chiave cambiata = blocco senza alcuna domanda e dialogo di avviso con le due fingerprint. Si sblocca solo rimuovendo la voce salvata dalle impostazioni (due passaggi, registrato nell'audit) | SEC-03, SEC-04 |
| D-16 | 2026-10-15 | **X server**: l'esclusione (2.2) diventa un **rinvio**. Si farà a fasi dopo la v1 (6.7): 1) X11 forwarding verso un X server esterno; 2) avvio guidato di un X server installato; 3) X server integrato solo se necessario, con eccezione esplicita a SEC-01 | Richiesta dell'utente (comodo per le applicazioni grafiche SAP); la fase 1 non apre porte, le successive richiedono cautele |
| D-17 | 2026-10-15 | **Strumenti di rete** (diagnosi): da rivalutare dopo la v1; per ora restano esclusi. Quando se ne parlerà si sceglieranno singoli strumenti di diagnosi utili e non offensivi, ciascuno con una decisione propria | Richiesta dell'utente ("potrebbero tornare utili"); evitare di tornare agli strumenti percepiti come offensivi |
| D-18 | 2026-10-15 | **Rischio accettato** su RUSTSEC-2023-0071 ("Marvin attack", crate `rsa`, usato da russh per le chiavi RSA; nessuna versione corretta disponibile). Si resta sul crate perché le chiavi RSA sono ancora comuni (host key e chiavi utente). Mitigazione: consigliare ed25519/ECDSA; riesame a ogni aggiornamento di russh. Ignorato in `deny.toml` con motivazione | Il rischio richiede osservazioni di timing sulle operazioni con chiave privata RSA; per un client desktop che si autentica con la propria chiave è basso, ma va dichiarato (SEC-12) |
| D-19 | 2026-10-15 | **Editor di testo integrato** per i file remoti, in forma ridotta (7.6): l'esclusione in 2.2 viene tolta. Il file resta in memoria (niente file temporanei), con limite di dimensione e controllo di modifica concorrente. Eccezione limitata alla regola 7 (nessun file intero in memoria), solo per l'editor e sotto il limite | Richiesta dell'utente (come in MobaXterm); riduce i file in chiaro su disco rispetto all'editor di sistema |
| D-20 | 2026-10-16 | **Editor integrato, scelte di realizzazione** (7.6): scheda dell'editor accanto ai terminali, CodeMirror 6 con sintassi per shell, YAML, JSON, XML, INI/conf, TOML, nginx, Python, SQL, Dockerfile, diff; limite 5 MiB; rifiuto dei file con byte NUL; codifiche UTF-8, UTF-8 con BOM e ripiego ISO-8859-1 (senza perdite); fine riga LF/CRLF conservata (se mescolati, uniformati al prevalente con avviso). **Scrittura sul posto** (troncamento e riscrittura, non file temporaneo + rinomina) per non cambiare permessi, proprietario e link; il controllo di modifica concorrente confronta data (precisione di 1 secondo del protocollo SFTP) e dimensione. Limite noto: una modifica esterna nello stesso secondo e con la stessa dimensione non viene rilevata | Mantiene permessi e proprietà dei file di sistema; evita file in chiaro su disco |
| D-21 | 2026-10-16 | **Trasferimenti paralleli, pausa e ripresa** (7.3): in ogni lavoro si copiano fino a 4 file insieme (prima le cartelle, poi i file). Ogni file va in un parziale `<destinazione>.<dimensione>-<data>.pxt-part`: l'impronta dell'originale è nel nome, quindi un parziale di una versione diversa del file non si riusa mai. **Pausa** ferma e tiene i parziali; **annulla** li elimina. La ripresa salta le voci già concluse e continua un parziale solo se gli ultimi 64 KiB combaciano con l'originale; alla fine si controlla la dimensione. I file falliti restano "da fare" e si riprovano con la stessa azione. Scartare un lavoro in pausa elimina i suoi parziali. Limiti noti: il confronto riguarda solo la coda del parziale (non l'interno); un parziale lasciato da un originale poi modificato resta sul disco finché non si cancella a mano | Criterio di accettazione M2 (annullamento e ripresa di un file grande, 1.000 file piccoli); prove: 2 GiB messi in pausa a 411 MiB e ripresi, identici byte per byte, memoria ~15 MiB |
| D-22 | 2026-10-16 | **Apri con il programma di sistema** (7.6): il file remoto (fino a 100 MiB) si scarica in `<temp>/PiCoXterm-edit/<pid>/<id>/`, si apre con il programma associato dal sistema (crate `opener`) e ogni secondo si controlla se è stato salvato (data e dimensione stabili per un secondo). Al salvataggio la UI chiede se caricare (o ricarica da sola con "Carica sempre"); prima di scrivere si controlla che il file remoto non sia cambiato (conflitto: sovrascrivi o scarta). Scrittura sul posto come in D-20. Le copie temporanee si eliminano alla chiusura del file, della connessione e dell'app; all'avvio si tolgono quelle vecchie più di un giorno. **I file eseguibili o script** (exe, bat, cmd, ps1, vbs, js, sh, py, pl, rb, php, lnk, url e simili) **non si aprono così**, perché su molti sistemi il comando "apri" li eseguirebbe: per quelli si usa l'editor integrato. Limite noto: a differenza dell'editor integrato qui il contenuto passa dal disco (permessi 0600 solo su Unix) | Ultimo punto di M2; evita che un file remoto scaricato venga eseguito per sbaglio |
| D-23 | 2026-10-17 | **"Re-inietta" rileva la shell in uso, non quella di login** (7.2): dopo `su - utente` o `sudo su - sidadm` la shell cambia e `$SHELL` letto da un canale a parte resta quello dell'utente di login: iniettava la riga della shell sbagliata. Ora il pulsante scrive nel terminale una sonda (valida per sh/bash/zsh/ksh e per csh/tcsh) che stampa una sequenza OSC privata 7770 con il nome del processo della shell; il lettore del terminale la intercetta (nome ripulito: solo caratteri da nome di programma). Se la sonda non risponde in 3 s si ripiega sulla shell di login; per una shell non supportata si segnala invece di iniettare. L'attivazione automatica dopo la connessione resta quella di prima, che non scrive nulla di visibile | Criterio di accettazione M2; il difetto è emerso dalla prova con `sudo su - tcshuser` |
| D-24 | 2026-10-17 | **Niente "SFTP via sudo"**: il pannello file usa sempre l'identità del login SSH. Dopo `sudo su - utente` il terminale cambia utente, il pannello no, e così resta (limite documentato). Escluso: avviare `sftp-server` con `sudo -u`. Motivi: MobaXterm non lo fa, richiede `sudo` senza password e il percorso di `sftp-server`, con SSH diretto disabilitato per `<sid>adm` i casi da gestire crescono troppo. Chi ha bisogno dei file di un altro utente usa il terminale o permessi di gruppo | Decisione dell'utente. Toglie la voce dal backlog e l'esclusione resta registrata qui |
| D-25 | 2026-10-17 | **Backend FTP/FTPS** (M3, 3.3): `FtpFs` su `suppaftp` (tokio, rustls con `ring`), in `pxt-fs`. FTP ammette un solo trasferimento per connessione di controllo, quindi c'è un **gruppo di al massimo 4 connessioni** (una per file in parallelo), riusate solo se l'ultimo trasferimento è finito bene. Lista con `MLSD` (altrimenti `LIST`), `MLST` per `stat`, `REST` per la ripresa (se il server non la accetta il motore riparte da zero), `MFMT` e `SITE CHMOD` quando ci sono. **TLS**: certificati di sistema oppure **pin dell'impronta SHA-256** accettato dall'utente; un certificato non verificabile e senza pin (o con pin diverso) fa fallire la connessione e riporta l'impronta, mai accettato in silenzio (come per le host key). FTP in chiaro e FTPS implicito sono supportati dal backend; il divieto del chiaro spetta alla policy. La password non compare in `Debug` né nei messaggi d'errore. Con TLS implicito i canali dei dati vanno protetti a mano (`PBSZ 0`, `PROT P`: la libreria lo fa solo con `AUTH TLS`). L'elenco `MLSD` si legge con un parser nostro (il server può omettere la dimensione delle cartelle; nomi e id Unix quando ci sono). Limiti noti: niente FTP attivo | Prove su pure-ftpd (FTP e FTPS esplicito) e ProFTPD (FTPS implicito) in testenv: pin, 40 MiB in entrambi i versi, 120 file in parallelo, ripresa con `REST` |
| D-26 | 2026-10-17 | **Sessioni FTP/FTPS nell'app e prima policy** (M3, 5.4): la sessione ha `ftp` = {`security` (`plain`, `explicit` predefinito, `implicit`), `pinnedCert` (impronta SHA-256, dato pubblico), cartella iniziale, file nascosti}; la password sta nel keyring come per SSH. `ftp_connect` risolve la sessione, applica la policy, collega e, se il certificato non è verificabile, invia alla UI `certUntrusted` (impronta e se è «cambiato») e risponde `certificate-untrusted`: la UI mostra la finestra di conferma, "accetta e salva" scrive il pin nella sessione e riprova, "accetta una volta" riprova col solo pin temporaneo. Un pin salvato che non corrisponde più dà l'avviso forte "certificato cambiato". **Policy**: file `policy.json` nella cartella dati, che l'app legge e non scrive mai: `allowPlainFtp` (predefinito vero) e `allowedProtocols` (assente = tutti). Un file illeggibile o non valido applica la versione più restrittiva (niente FTP in chiaro) e lo segnala. La policy è fatta rispettare dal backend (la UI la rispecchia: opzione in chiaro nascosta, protocolli disattivati). Audit: `connect` con protocollo `ftp` e modalità, `cert_untrusted`, `cert_changed`; l'FTP in chiaro è annotato "senza cifratura". Niente jump host per FTP | Criteri di accettazione M3: certificato autofirmato con pin, `allowPlainFtp: false` blocca il chiaro |
| D-13 | 2026-10-15 | Esito spike M0: **go** sullo stack. Rischio aperto su NFR-03 (memoria, ~273 MB a riposo per via di WebView2) | Vedi `docs/spike-m0.md` |

---

## 13. Domande aperte

1. Il modulo PowerShell Devolutions funziona con il vostro data source MSSQL anche su macOS/Linux, o solo su Windows? (Impatta M6.)
2. Serve un certificato di code signing aziendale (EY) o personale? Chi lo fornisce?
3. Nei landscape dei clienti ci sono ancora sistemi con soli algoritmi SSH legacy (`ssh-rsa` SHA-1, `diffie-hellman-group1`)? Quali?
4. Il port forwarding locale (6.6) serve davvero per SAP GUI/HANA Studio, o si usa già un'altra via (VPN, Devolutions Gateway)?
5. FTP in modalità attiva: si può escludere del tutto?
6. X11 (6.7): per quali applicazioni serve davvero (SAPinst, `hdbsetup`, installer Oracle, altro) e quale X server esterno si usa già (VcXsrv, X410, altro)?
7. ~~Strumenti di rete (D-17, 6.8): lista sul server o scansione?~~ Risolta il 15 ottobre 2026: la **lista delle porte in ascolto sul server** (via SSH); la scansione completa non è prevista.
