# Changelog

All notable changes to PiCoXterm are recorded here, in English.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are pre-1.0 and may change freely.

## [Unreleased]

## [0.1.0-beta.3] - 2026-10-20

Third test release. Windows only, unsigned (SmartScreen will warn). **New: RDP in a tab.** The zip now contains a second program, `picoxterm-rdp.exe`, that must stay next to `picoxterm.exe`.

### Added
- **RDP in a tab** (integrated client, built on IronRDP): the session opens inside PiCoXterm like any other tab, with mouse, wheel, full keyboard (physical scancodes, AltGr, F-keys), Ctrl+Alt+Del, fit-to-window and full screen. NLA/CredSSP sign-in; a server certificate that cannot be verified is never accepted silently (you confirm its fingerprint, and it is saved in the session). The RDP engine runs as a separate program (`picoxterm-rdp`) next to the app, which also isolates the protocol parser. Sessions with an RD Gateway, or with the client set to a system one, still use the system client. The keyboard layout of your PC is sent to the server (physical keys are interpreted with it, so `-`, `@`, `#` come out right). **Text clipboard** works both ways (copy on the server, paste on your PC and the other way round; shared only if "Share the clipboard" is on in the session; text only, up to 4 MiB). The desktop is scaled to the tab when the window is resized. If the app dies, the RDP engine is terminated with it. **Automatic resolution**: when you resize the window the remote desktop is re-laid-out at the new size (after a short pause; the "Auto resolution" button turns it off and the desktop is then just scaled). Files in the clipboard and detaching the tab into its own window are not included yet.
### Known limits
- RDP: text clipboard only (no files or images); the tab cannot be detached into its own window yet; no audio or drive sharing in the integrated client; RD Gateway sessions use the system client; Kerberos sign-in is untested (NTLM works).
- `picoxterm-rdp.exe` must stay next to `picoxterm.exe`; without it RDP sessions open the Windows client.
- Directory tracking works with bash, zsh and tcsh only (not ksh, csh, ash).
- S3 uploads cannot be resumed after a pause; S3 with an AWS SSO profile is untested.
- Tested on Windows only; portable mode not verified end to end. Not signed.

## [0.1.0-beta.2] - 2026-10-19

Second test release. Windows only, unsigned (SmartScreen will warn). Now free software under the GPL-3.0.

### Added
- **RDP sessions** (system client): double-click a saved RDP session to open Windows' own Remote Desktop client (`mstsc`; Windows App on macOS, FreeRDP on Linux). Options: domain, full screen, all monitors, resolution, gateway, clipboard and drive sharing (drives off by default). The saved password never touches a file or a command line: on Windows it is handed over through a temporary Credential Manager entry that is removed shortly after the launch. The system client path has not been tested against a real server yet.
- **Licence**: PiCoXterm is now released under the GNU General Public License v3.0 (`GPL-3.0-only`); the full text is in `LICENSE` and the About dialog shows it.
- **SFTP sessions**: a session type that connects over SSH without a shell (also works with SFTP-only accounts) and opens a file browser tab, with an optional start folder and "show hidden files". Jump hosts, passwords/keys/agent, host key checks and the built-in editor work as in SSH sessions. The "SFTP" entry of the new-session dialog is now enabled.
- **Dual-pane view** in SFTP tabs: your computer on the left (starts in Documents, remembers the last folder, lists the drives on Windows), the server on the right. Send files and folders with the toolbar button, the context menu or by dragging between the panes, in both directions, using the same queue (parallel, pause/resume). Double-click a local file to open it with its program (scripts and executables are refused). The divider is draggable.
- **FTP and FTPS sessions**: explicit FTPS (default), implicit FTPS and plain FTP, in the same dual-pane tab as SFTP (editor, transfers with pause/resume, drag between panes). A server certificate that cannot be verified is never accepted silently: a dialog shows its SHA-256 fingerprint and you choose *accept once*, *accept and save* (the fingerprint is stored in the session) or cancel; if a saved certificate later changes, you get a stronger warning. Plain FTP shows a warning banner.
- **Organization policy** (`policy.json` in the data folder, read-only): `allowPlainFtp: false` blocks plain FTP sessions (also hidden in the session dialog) and `allowedProtocols` restricts the protocols. A broken file never loosens the policy.
- **S3 sessions**: browse buckets and prefixes of AWS or any S3-compatible service (custom endpoint, path-style), sign in with access keys (secret key in the system keyring) or an AWS profile including SSO, or open a single "external bucket". Single-pane view with a bucket list, the object list and a detail panel (properties, headers and custom metadata, tags, storage class, text preview); the computer pane can be switched on. Upload files and folders (streamed multipart, nothing left behind when interrupted), download with resume, rename/move (copy + delete), copy the `s3://` address, temporary download links. Objects are not edited in the built-in editor. Versions, ACLs and bucket management are not included yet.
- Test servers for FTP/FTPS (pure-ftpd, ProFTPD) in `testenv/`.
- Local file system backend (`LocalFs`); recursive deletes now refuse Windows drive roots too.
- Transfers copy up to four files at a time per job (200 small files: 1.6 s → 0.4 s against the test server).
- Pause and resume for transfers: pausing keeps the partial files, resuming continues from where each file stopped (checked against the original) and skips files already finished. Failed files can be retried. Cancelling still deletes partial files.
- "Open with system program" in the SFTP context menu: downloads a temporary copy, opens it with the system's associated program and offers to upload it back when it is saved (or "Always upload"). Warns if the file changed on the server. Scripts and executables are refused. Temporary copies are deleted when the file, the connection or the app is closed.

### Fixed
- The remote file list now shows **permissions, user and group**: as columns when the panel is wide enough (about 540 px), otherwise on a small second line under each name. A toolbar button hides or shows them. SFTP only reports numeric ids, so names are read from the server's `/etc/passwd` and `/etc/group`; users that live elsewhere (LDAP) stay numeric. Columns can be sorted by user and group.
- "Re-inject tracking" now detects the shell actually running in the terminal. After `su - user` or `sudo su - user` it used the login shell and injected the wrong syntax; it now asks the running shell, and says so when that shell is unsupported.

### Changed
- Partial files are named `<name>.<size>-<mtime>.pxt-part` so a partial of a different version of the file is never reused.

### Known limits
- Directory tracking works with bash, zsh and tcsh only (not ksh, csh, ash).
- S3 uploads cannot be resumed (an interrupted upload starts again from the beginning); downloads can.
- S3 with an AWS SSO profile and RDP against a real server have not been tested yet.
- Tested on Windows only; portable mode not verified end to end. Not signed.
- The SFTP panel does not follow `su`/`sudo` to another user (by design).

## [0.1.0-beta.1] - 2026-10-16

First test release. Windows only, unsigned (SmartScreen will warn).

### Added
- Tabbed SSH terminal with password, key (OpenSSH, `.ppk`) and agent authentication, chained jump hosts and host key verification (never accepted silently).
- Session tree with folders, colours, per-session themes, duplicate, export/import (no secrets), quick connect and connection test.
- "New session" dialog with protocol picker; only SSH is available, the other protocols are shown as "Soon".
- SFTP panel over the same SSH connection that follows the terminal's directory (bash, zsh, tcsh), with upload/download of files and folders, drag and drop from Explorer, conflict handling, delete, rename, new folder, properties and chmod, and copy path.
- "Go to this folder in the terminal" actions from the SFTP panel.
- Built-in text editor (CodeMirror 6): opens from the context menu or by double-click, syntax highlighting, search/replace, encoding and line-ending preservation, concurrent-modification check, 5 MiB limit, content kept in memory.
- About dialog (F1) with copyable build information.
- Audit log of connections, host keys and file operations (counts only), portable mode via `portable.flag`.
- Italian and English interface.

### Known limits
- Directory tracking works with bash, zsh and tcsh only (not ksh, csh, ash).
- Transfers run one at a time and cannot be resumed after cancelling.
- Tested on Windows only; portable mode not verified end to end.
- Licence not decided yet.
