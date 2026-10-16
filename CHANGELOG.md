# Changelog

All notable changes to PiCoXterm are recorded here, in English.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are pre-1.0 and may change freely.

## [Unreleased]

### Added
- Transfers copy up to four files at a time per job (200 small files: 1.6 s → 0.4 s against the test server).
- Pause and resume for transfers: pausing keeps the partial files, resuming continues from where each file stopped (checked against the original) and skips files already finished. Failed files can be retried. Cancelling still deletes partial files.
- "Open with system program" in the SFTP context menu: downloads a temporary copy, opens it with the system's associated program and offers to upload it back when it is saved (or "Always upload"). Warns if the file changed on the server. Scripts and executables are refused. Temporary copies are deleted when the file, the connection or the app is closed.

### Changed
- Partial files are named `<name>.<size>-<mtime>.pxt-part` so a partial of a different version of the file is never reused.

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
