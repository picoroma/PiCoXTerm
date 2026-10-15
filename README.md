# PiCoXterm

Client di accesso remoto leggero, ispirato a MobaXterm ma essenziale e "auditabile per costruzione".

- **SSH** con pannello **SFTP che segue la directory del terminale**
- **SFTP**, **FTP/FTPS**, **S3** in un file browser a doppio pannello
- **RDP** e **VNC**
- Sessioni salvate in locale (JSON) e password nel keyring di sistema; integrazione con Devolutions Remote Desktop Manager in roadmap
- Windows, Linux, macOS · Tauri 2 + Rust

Cosa **non** fa, per scelta: nessun server integrato, nessuna porta in ascolto, nessuno strumento di rete, nessuna shell locale, nessun X server.

## Documentazione

- [Specifiche](docs/SPECIFICHE.md)
- [Roadmap](docs/ROADMAP.md)

## Stato

M0 (spike tecnico) concluso. Milestone corrente: M1 (SSH a schede e sessioni locali).
