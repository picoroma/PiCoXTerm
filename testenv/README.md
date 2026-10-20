# testenv

Ambiente di test **solo per sviluppo** (credenziali fittizie, porta su loopback). Non fa parte dell'app.

## Server SSH

```
docker compose up -d
```

Ascolta su `127.0.0.1:2222`. Utente `test`, password `test`. Accetta anche le chiavi in `keys/authorized.pub`.

## Server FTP e FTPS

```
docker compose up -d ftp ftps
powershell -ExecutionPolicy Bypass -File testenv/seed-ftp.ps1    # file di esempio (una volta)
```

| Servizio | Indirizzo | Protocollo | Software |
|---|---|---|---|
| `ftp` | `127.0.0.1:2121` | FTP in chiaro **e** FTPS esplicito (`AUTH TLS`) | pure-ftpd |
| `ftps` | `127.0.0.1:2990` | FTPS **implicito** (TLS subito) | ProFTPD |

Utente `test`, password `test`, modo passivo (porte 30000-30009 per `ftp`, 30010-30019 per `ftps`), tutto solo su loopback. I certificati TLS sono **autofirmati** e generati all'avvio: il primo accesso deve chiedere di fidarsi dell'impronta (pin); `ftp` impiega qualche decina di secondi a generarlo. Il server `ftps` contiene già `LEGGIMI.txt`, `Documenti` e `Immagini`; `ftp` si popola con lo script.

Prove automatiche (usano entrambi): `cargo test -p pxt-fs --test ftp_e2e -- --ignored --test-threads=1`.

## Server S3

```
docker compose up -d s3
```

Servizio `s3` (`127.0.0.1:9000`, RustFS: MinIO non ha più immagini pubbliche). Chiave di accesso `pxtaccess`, chiave segreta `pxtsecret-1234`, indirizzi *path-style*, regione qualsiasi (`us-east-1`). Solo credenziali di prova, solo loopback. Prove automatiche: `cargo test -p pxt-fs --test s3_e2e -- --ignored --test-threads=1`.

## Server RDP

```
docker compose up -d rdp
```

Servizio `rdp` (`127.0.0.1:3390`, xrdp + Xfce). Utente `pxtuser`, password `pxtpass-1234`; l'autenticazione NLA riesce e poi xrdp mostra la propria schermata di login. Solo credenziali di prova, solo loopback. Serve allo spike dell'RDP integrato (`docs/spike-m7-rdp.md`).

## Server con più shell

Servizio `ssh-shells` (porta 2223): utenti `bashuser`, `zshuser`, `tcshuser`, `ashuser` (password `test`); `bashuser` può usare `sudo`.

## Chiavi di prova

Le chiavi non sono nel repository (cartella `keys/` ignorata da git). Per generarle:

```
mkdir keys && cd keys
ssh-keygen -t ed25519 -N "" -C pxt-test-plain -f id_plain
ssh-keygen -t ed25519 -N "testpass" -C pxt-test-enc -f id_enc
cat id_plain.pub id_enc.pub > authorized.pub
```

`id_plain` è senza passphrase; `id_enc` ha la passphrase `testpass`. Poi `docker compose up -d --force-recreate`.

## Test end-to-end

```
cargo test -p pxt-ssh --test auth_e2e -- --ignored --test-threads=1
```

## Verifica SEC-01 (nessuna porta in ascolto)

```
powershell -ExecutionPolicy Bypass -File testenv/check-no-listeners.ps1
```

Avvia l'exe, confronta tutti i socket del PC prima e dopo e controlla che nessuno appartenga a PiCoXterm o al suo WebView2. Con una sessione già aperta (anche con jump host o trasferimenti) si usa `-Attach`. Esce con codice 0 se tutto è in regola.
