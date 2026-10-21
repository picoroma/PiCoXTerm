# PiCoXterm Privacy Policy

*Last updated: 2026-10-21*

PiCoXterm is a remote access client (SSH/SFTP, FTP/FTPS, S3, RDP, VNC). This policy explains what
data it handles and where it goes. The short version: **PiCoXterm does not collect, transmit or
sell any of your data.** It only connects to the servers you tell it to connect to.

## What PiCoXterm stores, and where

Everything PiCoXterm stores stays **on your own device**, in your user profile:

- **Session list** (names, hosts, ports, usernames, options you set): a local file in the app's
  data folder.
- **Passwords, passphrases and private-key references**: your operating system's own secure
  credential store (Windows Credential Manager, macOS Keychain, or the Linux Secret Service/keyring
  — via the `keyring` crate). PiCoXterm never writes a password to a plain file or to its logs.
- **Known host keys**, used to detect a server impersonating one you trust: a local file, in the
  standard SSH `known_hosts` format.
- **A local audit log** of connection attempts (date, host, outcome) for your own troubleshooting.
  It never leaves your device.
- **Settings** (theme, font, behaviour): a local file.

None of this is uploaded anywhere by PiCoXterm. Nothing is shared between devices unless you
explicitly export your session list yourself (Settings → Export sessions…) and send that file
somewhere — in which case the export already excludes your saved passwords.

## Network connections

PiCoXterm makes **only the outbound connections you configure**: to the SSH, SFTP, FTP/FTPS, S3,
RDP or VNC server you add as a session, directly, using the protocol you chose. It does not open
any local network port or server, and it has:

- **no telemetry**,
- **no analytics**,
- **no crash reporting**,
- **no "check for updates" or "phone home" traffic**

to the developer or to any third party. The only exception is a connection to Amazon S3 (or an
S3-compatible service) you set up yourself: that traffic goes directly between your device and
AWS (or the service you chose), under their own privacy terms, not the developer's.

## Clipboard

If you use clipboard sharing in an RDP or VNC session, PiCoXterm reads or writes your system
clipboard only to carry out that copy/paste, between your device and the remote server you are
connected to. It is never sent anywhere else.

## Imported sessions

PiCoXterm can import a session list from PuTTY or MobaXterm, reading files already on your own
device (the Windows registry, or files you point it to). Saved passwords from those programs are
**not** imported; nothing from this import step is sent anywhere.

## Microsoft Store

If you install PiCoXterm from the Microsoft Store, Microsoft collects its own standard
installation and diagnostic data under the [Microsoft Privacy
Statement](https://privacy.microsoft.com/privacystatement) — that collection is Microsoft's, not
PiCoXterm's, and this policy does not cover it.

## Children's privacy

PiCoXterm is a technical tool for system administrators and developers, not directed at children.
It does not knowingly collect data from anyone, children included — see above: it does not collect
data from anyone at all.

## Open source

PiCoXterm is free software, licensed under the GPL-3.0. Its full source code is public, so this
policy can be checked against the code itself: <https://github.com/picoroma/PiCoXTerm>.

## Changes to this policy

If this policy changes, the date at the top will change and the new version will be published at
the same place. Significant changes will also be noted in the app's changelog.

## Contact

Questions about this policy: **[support contact — to confirm]**.
