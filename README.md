# PiCoXterm

A lightweight remote access client, inspired by MobaXterm but minimal and "auditable by construction".
Windows, Linux, macOS · Tauri 2 + Rust · xterm.js terminal.

Current release: **0.1.0-beta.4** (Windows only, unsigned). See the [changelog](CHANGELOG.md).

## Download

Download the latest zip from the [releases page](https://github.com/picoroma/PiCoXTerm/releases), extract it and run `picoxterm.exe`. Keep `picoxterm-rdp.exe` next to it — it is the integrated RDP engine, a separate program; without it RDP sessions fall back to the Windows client. For portable mode (settings and sessions next to the executable instead of your user profile), create an empty `portable.flag` file next to `picoxterm.exe`.

The executable is **not signed**: Windows will show a SmartScreen warning ("More info" → "Run anyway"). A Microsoft Store listing, which Microsoft signs, is in preparation.

## What it does today

**SSH**
- Tabbed terminal (xterm.js): copy on select, right-click paste, multi-line paste warning, scrollback search (Ctrl+Shift+F), zoom with Ctrl+wheel
- Authentication with password, private key (OpenSSH and `.ppk`, with passphrase) or agent (Pageant, Windows OpenSSH agent, `SSH_AUTH_SOCK`)
- Chained **jump hosts** (up to 5), without opening local ports
- **Host keys**: confirmation with fingerprint (accept once, accept and save, cancel); if a key changes the connection is blocked until the saved entry is removed
- "Test connection" without opening a shell, reconnect in the same tab, Quick connect (`user@host:port`)

**SFTP, FTP/FTPS and S3** sessions (explicit/implicit TLS for FTP, plain FTP only if the policy allows it; self-signed certificates need your explicit confirmation of their fingerprint; S3 and S3-compatible services such as MinIO, with an AWS profile or access keys) open in a dual-pane tab (your computer on the left, the remote side on the right; send files with a button or by dragging).

**RDP and VNC**
- RDP sessions open **inside a tab**, with an integrated client (IronRDP): full keyboard, Ctrl+Alt+Del, fit-to-window and full screen, text clipboard, automatic remote resolution, NLA sign-in. A server certificate that cannot be verified is never accepted silently. Sessions with an RD Gateway, or set to a system client, still use Windows' own Remote Desktop
- VNC sessions also open in a tab (noVNC); the password is kept in the system keyring and never reaches the interface code. VNC does not encrypt traffic — a warning says so

**SFTP panel** (same SSH connection, no second login)
- Follows the terminal's current directory (OSC 7, with automatic injection for bash, zsh and tcsh) and takes the terminal to a folder ("go to this folder in the terminal")
- Browse, upload and download files and folders (with a conflict dialog), four files at a time, with pause, resume and retry of failed files; drag and drop from Explorer, delete, rename, new folder, properties and chmod (recursive optional), copy path
- **Built-in text editor** (right-click → Edit, or double-click): CodeMirror 6 with syntax highlighting, search and replace, encoding and line-ending preservation, concurrent-modification check, 5 MiB limit. The content stays in memory, no temporary files on disk
- "Open with system program" for any file up to 100 MiB (PDFs, images, spreadsheets...): a temporary copy is opened with the program the system associates, and saving it offers to upload it back. Scripts and executables are refused (use the built-in editor)
- The panel shows the name and colour of the active SSH session
- The file panel always works as the user you logged in with: after `su` or `sudo su -` in the terminal it follows the folder you are in, but file access keeps your login user's permissions

**Detach a tab into its own window**: SFTP/FTP/S3, RDP and SSH terminal tabs can move to a separate window — button, tab context menu, or drag the tab out of the bar. The live session moves as it is: no reconnection, no new login, no lost scrollback. Closing the detached window asks whether to dock it back or end the session. VNC and editor tabs cannot be detached yet.

**Collapsible sidebar**: an arrow on the splitter hides the session tree and SFTP panel so the active tab gets the full window. For an SSH terminal, the SFTP panel doesn't disappear — it moves below the terminal instead.

**Import sessions** from PuTTY (Windows registry, an exported `.reg` file, or `~/.putty/sessions`) and MobaXterm (`MobaXterm.ini` or `.mxtsessions`), keeping folders. You pick which sessions to bring in from a preview; they land in a new folder and nothing existing is touched. Passwords are not imported — type them at the first connection.

**System tray**: mark sessions as favorites and the tray menu lists them, plus your most recently opened sessions, for a one-click launch. Closing the main window minimizes it to the tray; "Quit" in the tray menu exits for real.

**Sessions and settings**
- Tree of folders and sessions with filter, drag and drop, duplicate; "New session" dialog with a protocol picker (SSH, SFTP, FTP/FTPS, S3, RDP, VNC)
- Passwords and passphrases only in the system keyring, never in the JSON file
- Per-session appearance: theme, font, background and tab colour (handy to tell PROD, TEST and DEV apart)
- Themes (dark, light, Solarized, Dracula), font, scrollback and copy/paste options
- Session export and import as JSON, without secrets
- Audit log (JSON Lines, rotated) of connections, host keys and file operations: counts only, no credentials, names or session content
- Italian and English interface
- Portable mode: a `portable.flag` file next to the executable moves the data to the `data/` folder (secrets stay in the system keyring)

## Planned

- **M5**: code signing for macOS and Linux installers, a Microsoft Store listing for Windows (in preparation — see `store/`), a security-info page in the app, full policy enforcement, non-functional requirements verified
- **M6**: read-only Devolutions RDM provider (shared team sessions)
- **Backlog**: X11 forwarding and, in phases, an X server (spec section 6.7); network diagnostic tools (ping, port check, listening ports of a server over SSH, section 6.8); SSH certificates; a KeePass provider

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

The executable is `target/release/picoxterm.exe`. The integrated RDP client is a separate program with its own `Cargo.lock`:

```
cargo build --release --manifest-path rdp-engine/Cargo.toml
```

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd ui && npm run check
```

End-to-end tests (authentication, jump hosts, host keys, SFTP, transfers, editor, S3, VNC) run against test servers in Docker and are not part of CI. Instructions in [testenv/README.md](testenv/README.md).

To try the interface in a browser, without Tauri, with a fake backend: `npm --prefix ui run dev` (development only, not part of the executable).

## Documentation

The design documents are written in Italian.

- [Specifications](docs/SPECIFICHE.md): architecture, data model, security and recorded decisions (section 12)
- [Roadmap](docs/ROADMAP.md): milestones and acceptance criteria
- [M0 spike report](docs/spike-m0.md)

## Status

M0 through M4 are complete (SSH, SFTP/FTP/S3, RDP, VNC). Current milestone: **M5**, polishing the project for a 1.0 release — code signing, the Microsoft Store listing, and the remaining non-functional requirements.

## Licence

PiCoXterm is free software, released under the [GNU General Public License v3.0](LICENSE) (`GPL-3.0-only`).
Copyright (C) 2026 PiCo.

You may use, study, modify and redistribute it; if you distribute a modified version, you must do so under the same licence and make its source code available.
