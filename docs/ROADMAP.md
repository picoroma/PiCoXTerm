# PiCoXterm: roadmap

> Versione 0.1, 15 ottobre 2026. Riferimento: `SPECIFICHE.md`.

## Principi

- **Una milestone alla volta.** Si passa alla successiva solo quando i criteri di accettazione sono verificati.
- Ogni milestone produce un **eseguibile usabile**, non solo codice.
- Prima si elimina il rischio tecnico (M0), poi si costruisce.
- Le stime sono **indicative**, espresse in giornate di lavoro effettivo con Claude Code, e vanno ricalibrate dopo M0.

## Panoramica

| Milestone | Contenuto | Stima | Uscita |
|---|---|---|---|
| **M0** | Spike tecnico e scheletro del progetto | 2–3 gg | Decisione go/no-go sullo stack |
| **M1** | SSH a schede + sessioni locali | 5–7 gg | **Primo uso quotidiano** |
| **M2** | Pannello SFTP che segue la directory + trasferimenti | 5–7 gg | **MVP "Moba-like"** |
| **M3** | Sessioni SFTP, FTP/FTPS, S3 (file browser a doppio pannello) | 5–7 gg | Tutti i file browser |
| **M4** | RDP (client esterno) e VNC (noVNC) | 3–5 gg | Perimetro v1 completo |
| **M5** | Rifinitura, import MobaXterm, firma, installer | 4–6 gg | **Release 1.0** |
| **M6** | Provider RDM (sola lettura) | 4–6 gg | Sessioni del team |
| **M7+** | RDP integrato (IronRDP), scrittura su RDM, backlog | da definire | 2.x |

```
M0 ──► M1 ──► M2 ══► (MVP)
              │
              ├──► M3 ──► M4 ──► M5 ══► (1.0) ──► M6 ──► M7+
```

---

## M0: spike tecnico e scheletro

**Obiettivo:** dimostrare che le scelte rischiose funzionano prima di costruirci sopra.

Attività:
1. Workspace Cargo con i crate `pxt-core`, `pxt-ssh`, `pxt-fs` e l'app Tauri 2 + Svelte 5 (decidere D-10).
2. Collegamento minimo: una scheda xterm.js connessa via russh a un server di test, con streaming tramite `tauri::ipc::Channel`.
3. Verifiche puntuali (ognuna con un esito scritto in `docs/spike-m0.md`):
   - [ ] chiavi **.ppk** supportate da russh/ssh-key?
   - [ ] **Pageant** e **agent OpenSSH di Windows** (named pipe) utilizzabili?
   - [ ] **OSC 7**: intercettazione nel flusso dati e iniezione su bash e tcsh;
   - [ ] throughput del terminale (`cat` di un file da 50 MB) senza blocchi della UI;
   - [ ] dimensione della build release Windows e memoria a riposo;
   - [ ] crate `keyring` su Windows: salvataggio e lettura di una password.
4. `testenv/docker-compose.yml` con OpenSSH (utenti bash/zsh/ksh/tcsh), FTP TLS, MinIO.
5. CI minima: build su 3 piattaforme, clippy, fmt.

**Criteri di accettazione:**
- Ci si connette a un server SSH di test da una finestra PiCoXterm su Windows e si usano `vim` e `htop` correttamente.
- Il report dello spike documenta ogni verifica con esito e, se negativo, un piano B.
- **Go/no-go**: se russh o xterm.js via IPC risultano inadeguati, si rivaluta lo stack prima di M1.

---

## M1: SSH a schede e sessioni locali

**Obiettivo:** poter sostituire PuTTY/MobaXterm per le connessioni SSH quotidiane.

Attività:
1. `pxt-core`: modello dati (sezione 4), `LocalSessionProvider` con scrittura atomica e backup, `KeyringSecretProvider`, migrazioni dello schema.
2. UI: albero delle sessioni con cartelle, filtro, drag & drop, menu contestuale.
3. Editor della sessione SSH (generale, autenticazione, avanzate, aspetto, note) e "Test connessione".
4. Autenticazione: password, keyboard-interactive, chiave OpenSSH, agent (in base agli esiti di M0).
5. known_hosts proprio, dialoghi per host key nuova o cambiata (SEC-03, SEC-04).
6. Jump host a catena.
7. Terminale: schede, resize, copia su selezione, incolla con tasto destro, avviso multiriga, ricerca, zoom, temi.
8. Disconnessione rilevata, banner e "Riconnetti".
9. Quick connect.
10. Export/import JSON delle sessioni (senza segreti).
11. Log di audit (5.2).
12. Impostazioni di base e i18n (it/en).

**Criteri di accettazione:**
- Creo 20 sessioni in 3 cartelle, chiudo e riapro l'app: tutto è al suo posto; nel file JSON non c'è nessuna password.
- Mi collego a un server tramite jump host con chiave + passphrase salvata nel keyring.
- Una host key cambiata blocca la connessione.
- Dopo una disconnessione di rete, "Riconnetti" ripristina la sessione nella stessa scheda.
- `netstat -ano` non mostra porte in ascolto di PiCoXterm.

---

## M2: pannello SFTP che segue la directory

**Obiettivo:** la funzione distintiva. Alla fine di M2 c'è l'**MVP**.

Attività:
1. `pxt-fs`: trait `RemoteFs`, backend SFTP sulla stessa connessione SSH, backend locale.
2. Componente file browser (modalità pannello laterale).
3. Tracciamento della `cwd`: OSC 7 + iniezione per bash, zsh, ksh, csh/tcsh; rilevamento della shell; pulsante "Re-inietta tracciamento"; icona lucchetto "segui terminale".
4. Operazioni: elenco, crea cartella, rinomina, elimina, chmod, file nascosti, barra del percorso, preferiti.
5. Drag & drop da e verso l'Esplora risorse / Finder.
6. Coda dei trasferimenti: concorrenza, avanzamento, annulla, riprova, gestione dei conflitti, ripresa, streaming.
7. **Editor integrato** (clic destro, Modifica; D-19) e, in alternativa, "Apri con l'editor di sistema" con ricaricamento al salvataggio.

**Criteri di accettazione:**
- Con `cd /usr/sap/<SID>/SYS/profile` nel terminale, il pannello si aggiorna entro 500 ms, con bash e con tcsh.
- Dopo `sudo su - <sid>adm`, il pulsante "Re-inietta" ripristina il tracciamento.
- Upload di una cartella con 1.000 file piccoli e di un file da 2 GB; annullamento e ripresa del file grande.
- Modifico un file remoto con l'editor di sistema e il salvataggio viene ricaricato sul server.
- La memoria resta stabile durante un trasferimento da 2 GB.

> **Punto di controllo:** dopo M2 conviene usare PiCoXterm per qualche giorno nel lavoro reale prima di proseguire. I problemi di UX emersi si correggono qui, non dopo.

---

## M3: sessioni SFTP, FTP/FTPS e S3

**Obiettivo:** tutti i file browser, con lo stesso componente a doppio pannello.

Attività:
1. Modalità **doppio pannello** (locale ↔ remoto) del file browser.
2. Sessione **SFTP** standalone (riusa `pxt-ssh` + backend SFTP).
3. Backend **FTP/FTPS** con suppaftp: TLS esplicito/implicito, passivo, MLSD/LIST, pin dei certificati, avviso per FTP in chiaro.
4. Backend **S3**: profili AWS (incluso SSO), access key nel keyring, elenco bucket, endpoint personalizzato (MinIO), multipart, classe di storage.
5. `capabilities()` per nascondere le funzioni non supportate (per esempio chmod su S3).
6. `policy.json`: prima implementazione (protocolli consentiti, FTP in chiaro).

**Criteri di accettazione:**
- Gli stessi test di trasferimento di M2 passano su SFTP, FTPS (testenv) e MinIO.
- Un certificato FTPS self-signed richiede il pin; al secondo accesso non lo richiede più.
- Con `allowPlainFtp: false`, una sessione FTP in chiaro non parte.
- Accesso a un bucket S3 reale tramite profilo SSO.

---

## M4: RDP e VNC

**Obiettivo:** completare il perimetro dei protocolli della v1.

Attività:
1. `pxt-rdp`: generazione del file `.rdp` e avvio di mstsc / Windows App / FreeRDP; credenziali temporanee `TERMSRV/<host>` su Windows.
2. Spike e implementazione dell'**adattatore noVNC** via IPC (nessun WebSocket locale).
3. Sessione VNC in una scheda: scala, sola visualizzazione, Ctrl+Alt+Canc, clipboard.
4. Opzione VNC tramite tunnel SSH (`direct-tcpip`).

**Criteri di accettazione:**
- Doppio clic su una sessione RDP: si apre il client di sistema già autenticato (Windows), con NLA.
- Sessione VNC verso il server di testenv, utilizzabile, senza porte in ascolto.

---

## M5: rifinitura e release 1.0

**Obiettivo:** un prodotto distribuibile e presentabile all'IT.

Attività:
1. **Import da MobaXterm** (`.mxtsessions` / `MobaXterm.ini`): cartelle, SSH, SFTP, FTP, RDP, VNC; i segreti non vengono importati.
2. Split del terminale (orizzontale/verticale).
3. Policy completa (5.4) e pagina "Informazioni di sicurezza" nell'app (porte, dati salvati, policy attiva).
4. Firma del codice: Windows (Authenticode), macOS (notarizzazione); MSI + ZIP portabile, DMG, AppImage/.deb. **Via da valutare per Windows (idea del 2026-10-16):** pacchetto MSIX con la `winapp` CLI e pubblicazione sul Microsoft Store, che firma il pacchetto al posto nostro (nessun certificato, nessun avviso SmartScreen per chi installa dallo Store). Da verificare: costo e requisiti dell'account sviluppatore in Italia, modalità portabile (non compatibile con MSIX), politiche di certificazione dello Store. Vale anche per PiCoShot.
5. `cargo audit` + `cargo deny` nella CI; elenco delle licenze di terze parti (`THIRD-PARTY-LICENSES`).
6. Documento per l'IT: architettura, flussi di rete, dati salvati, policy (1–2 pagine).
7. Verifica dei requisiti non funzionali (sezione 10).

**Criteri di accettazione:**
- Importo il mio file di sessioni MobaXterm reale e ritrovo la struttura delle cartelle.
- L'installer firmato si installa su un PC aziendale senza avvisi SmartScreen (in base al certificato disponibile).
- Tutti gli NFR verificati e documentati.

---

## M6: provider RDM (sola lettura)

**Obiettivo:** usare le sessioni e le credenziali condivise del team gestite in Remote Desktop Manager Enterprise (data source MSSQL).

Attività:
1. Spike: modulo **Devolutions.PowerShell** con il data source SQL Server: elenco delle voci, lettura delle credenziali rispettando i permessi dell'utente, prestazioni con l'intero albero, comportamento su macOS/Linux.
2. `RdmSessionProvider` (sola lettura): bridge PowerShell che restituisce JSON, mappatura dei tipi di voce RDM verso i protocolli di PiCoXterm, cache locale delle sessioni (senza segreti) e aggiornamento manuale o periodico.
3. `RdmSecretProvider`: credenziali lette **al momento della connessione**, mai salvate da PiCoXterm.
4. Nodo radice "RDM" nell'albero, con icona di sola lettura.
5. Predisposizione per la **CLI ufficiale** Devolutions, quando sarà disponibile (stesso contratto JSON, bridge sostituibile).

**Criteri di accettazione:**
- Vedo l'albero RDM del team in PiCoXterm, con le stesse cartelle.
- Apro una sessione SSH da RDM senza digitare la password; la password non compare in nessun file di PiCoXterm.
- Una voce a cui non ho accesso in RDM non è visibile né apribile.

**Fuori perimetro:** lettura diretta del database MSSQL di RDM (D-07).

---

## M7+: backlog

In ordine di valore stimato, da ripriorizzare dopo l'uso reale:

| Voce | Note |
|---|---|
| RDP integrato con **IronRDP** in una scheda | Spike dedicato (prestazioni, NLA, clipboard) |
| Scrittura su RDM (crea/modifica voci) | Dopo la CLI ufficiale |
| **SFTP via sudo** (sftp-server avviato con `sudo -u <sid>adm`) | Molto utile per SAP Basis; da valutare la sicurezza |
| MultiExec (stesso input su più schede) | Con conferma esplicita |
| Port forwarding locale singolo | Solo se la domanda aperta 4 delle specifiche lo conferma; controllato da policy |
| **X11 forwarding** verso X server esterno (fase 1 di 6.7) | Per SAPinst, `hdbsetup`, installer Oracle; per sessione, disattivato da policy e di default |
| Avvio guidato di un X server installato (fase 2) | Rilevamento e avvio di VcXsrv/X410, senza includerli |
| X server integrato (fase 3) | Solo se 1 e 2 non bastano: richiede un'eccezione esplicita a SEC-01 (D-16) |
| Strumenti di rete di diagnosi | Da rivalutare dopo la v1 (D-17), scegliendo i singoli strumenti utili e non offensivi |
| Import da PuTTY e `~/.ssh/config` | |
| Certificati SSH | |
| Provider KeePass (.kdbx) | Per chi non usa RDM |
| Snippet di comandi salvati | Per esempio comandi Basis frequenti |
