# PiCoXterm

A lightweight remote access client, inspired by MobaXterm but minimal and "auditable by construction".
Windows, Linux, macOS · Tauri 2 + Rust · xterm.js terminal.

Current release: **0.1.0-beta.2** (Windows only, unsigned). See the [changelog](CHANGELOG.md).

## What it does today (M2 nearly complete)

**SSH**
- Tabbed terminal (xterm.js): copy on select, right-click paste, multi-line paste warning, scrollback search (Ctrl+Shift+F), zoom with Ctrl+wheel
- Authentication with password, private key (OpenSSH and `.ppk`, with passphrase) or agent (Pageant, Windows OpenSSH agent, `SSH_AUTH_SOCK`)
- Chained **jump hosts** (up to 5), without opening local ports
- **Host keys**: confirmation with fingerprint (accept once, accept and save, cancel); if a key changes the connection is blocked until the saved entry is removed
- "Test connection" without opening a shell, reconnect in the same tab, Quick connect (`user@host:port`)

**FTP and FTPS sessions** (explicit and implicit TLS, plain FTP only if the policy allows it; self-signed certificates need your explicit confirmation of their fingerprint) open in the same dual-pane tab.

**SFTP sessions**: connect over SSH without a shell and browse the server in a tab, with a **dual-pane** view (your computer on the left, the server on the right; send files with a button or by dragging) plus the same editor and transfers as below.

**SFTP panel** (same SSH connection, no second login)
- Follows the terminal's current directory (OSC 7, with automatic injection for bash, zsh and tcsh) and takes the terminal to a folder ("go to this folder in the terminal")
- Browse, upload and download files and folders (with a conflict dialog), four files at a time, with pause, resume and retry of failed files; drag and drop from Explorer, delete, rename, new folder, properties and chmod (recursive optional), copy path
- **Built-in text editor** (right-click → Edit, or double-click): CodeMirror 6 with syntax highlighting, search and replace, encoding and line-ending preservation, concurrent-modification check, 5 MiB limit. The content stays in memory, no temporary files on disk
- "Open with system program" for any file up to 100 MiB (PDFs, images, spreadsheets...): a temporary copy is opened with the program the system associates, and saving it offers to upload it back. Scripts and executables are refused (use the built-in editor)
- The panel shows the name and colour of the active SSH session
- The file panel always works as the user you logged in with: after `su` or `sudo su -` in the terminal it follows the folder you are in, but file access keeps your login user's permissions

**Sessions and settings**
- Tree of folders and sessions with filter, drag and drop, duplicate; "New session" dialog with protocol picker (SSH, SFTP and FTP/FTPS are available)
- Passwords and passphrases only in the system keyring, never in the JSON file
- Per-session appearance: theme, font, background and tab colour (handy to tell PROD, TEST and DEV apart)
- Themes (dark, light, Solarized, Dracula), font, scrollback and copy/paste options
- Session export and import as JSON, without secrets
- Audit log (JSON Lines, rotated) of connections, host keys and file operations: counts only, no credentials, names or session content
- Italian and English interface
- Portable mode: a `portable.flag` file next to the executable moves the data to the `data/` folder (secrets stay in the system keyring)

## Planned

- **M2** (rest): acceptance checks on real workloads
- **M3**: plain SFTP sessions, FTP/FTPS and S3 with a dual-pane file browser
- **M4**: RDP (external client) and VNC
- **M5 and later**: MobaXterm import, signed installers, Devolutions RDM provider
- **Backlog**: X11 forwarding and, in phases, an X server (spec section 6.7); network diagnostic tools (ping, port check, listening ports of a server over SSH, section 6.8)

Details and acceptance criteria in [docs/ROADMAP.md](docs/ROADMAP.md).

## Security principles

- **Zero listening ports**: outgoing connections only
- No secret on disk in clear text, nor in logs
- Unknown or changed host keys are never accepted silently
- No built-in server, no local shell, no telemetry
- Permissive dependencies checked by `cargo deny` (licences, advisories, sources)

What it deliberately does **not** do: no built-in server, no listening port, no local shell. An X server (in phases) and some network diagnostic tools are in the backlog, after v1.

## Development

You need Rust (stable), Node.js and, on Windows, the Visual Studio Build Tools.

```
cd ui && npm install && npm run build   # frontend, embedded in the executable
cd .. && cargo build --release -p picoxterm
```

The executable is `target/release/picoxterm.exe`.

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd ui && npm run check
```

End-to-end tests (authentication, jump hosts, host keys, SFTP, transfers, editor) run against test SSH servers in Docker and are not part of CI. Instructions in [testenv/README.md](testenv/README.md).

To try the interface in a browser, without Tauri, with a fake backend: `npm --prefix ui run dev` (development only, not part of the executable).

## Documentation

The design documents are written in Italian.

- [Specifications](docs/SPECIFICHE.md): architecture, data model, security and recorded decisions (section 12)
- [Roadmap](docs/ROADMAP.md): milestones and acceptance criteria
- [M0 spike report](docs/spike-m0.md)

## Status

M0 (technical spike) and M1 (tabbed SSH and local sessions) are complete. Current milestone: **M2** (SFTP panel that follows the directory), with the panel, transfers (parallel, resumable), file operations and editor done; "open with system program" is done too.

## Licence

PiCoXterm is free software, released under the [GNU General Public License v3.0](LICENSE) (`GPL-3.0-only`).
Copyright (C) 2026 PiCo.

You may use, study, modify and redistribute it; if you distribute a modified version, you must do so under the same licence and make its source code available.
