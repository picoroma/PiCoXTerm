# testenv

Ambiente di test **solo per sviluppo** (credenziali fittizie, porta su loopback). Non fa parte dell'app.

## Server SSH

```
docker compose up -d
```

Ascolta su `127.0.0.1:2222`. Utente `test`, password `test`. Accetta anche le chiavi in `keys/authorized.pub`.

## Server FTP e FTPS

```
docker compose up -d ftp
```

Ascolta su `127.0.0.1:2121` (utente `test`, password `test`), modo passivo sulle porte 30000-30009, TLS esplicito opzionale con certificato **autofirmato** generato all'avvio (la prima volta serve qualche decina di secondi). Prove: `cargo test -p pxt-fs --test ftp_e2e -- --ignored --test-threads=1`.

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
