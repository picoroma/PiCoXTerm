# Changelog

All notable changes to PiCoXterm are recorded here, in English.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are pre-1.0 and may change freely.

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
