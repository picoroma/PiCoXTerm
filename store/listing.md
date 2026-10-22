# Microsoft Store listing — draft

Everything below is a draft for the Partner Center "Store listing" page. Fields marked
**[placeholder]** need the original tagline/image or an account-specific value; everything else is
ready to paste in, in English (Store listings are published in English first; additional
languages can be added later as separate listing translations).

## App name

PiCoXterm

*(Subject to availability when reserved in Partner Center — see [requirements.md](requirements.md).)*

## Tagline / short description (up to ~100 characters)

**[placeholder — your original tagline goes here]**

Draft alternatives, if useful as a starting point:
- "A light, fast, portable remote access client — SSH, SFTP, FTP, S3, RDP, VNC."
- "SSH, SFTP, RDP and VNC in tabs. No bloat, no listening ports."

## Description (long)

> PiCoXterm is a light, fast, portable remote access client for IT administrators and developers,
> in the spirit of tools like MobaXterm — built from scratch, free and open source.
>
> **Everything in tabs.** SSH terminals with an integrated SFTP panel that follows your current
> directory, FTP/FTPS, Amazon S3 (and S3-compatible storage), RDP and VNC sessions all live
> side by side in one window. Any tab — a file session, an RDP session, even a running SSH
> terminal — can be detached into its own window and docked back later, without reconnecting or
> losing your session.
>
> **Built for safety.** Unknown or changed server host keys are never accepted silently. Passwords,
> passphrases and keys are kept in Windows' own secure credential store, never written to a plain
> file or to a log. PiCoXterm opens no listening ports and runs no local server: every connection
> is outbound, to the server you choose.
>
> **Protocols:** SSH (password, key, SSH agent, jump hosts in a chain) with integrated SFTP; FTP
> and FTPS; Amazon S3 and S3-compatible object storage; RDP (built-in client, no separate Remote
> Desktop window); VNC.
>
> **Also:** session tree with folders, quick connect, per-session themes and fonts, directory
> tracking in the terminal (bash, zsh, tcsh), import your existing sessions from PuTTY or
> MobaXterm, connection audit log, English and Italian interface.
>
> PiCoXterm is free software under the GNU GPL-3.0. Source code, issue tracker and full changelog:
> https://github.com/picoroma/PiCoXTerm

## Category

Primary: **Developer tools** (alternative: **Productivity**)

## Features list (bullet points, Store "Features" field)

- SSH terminal with integrated, directory-following SFTP panel
- FTP, FTPS and Amazon S3 / S3-compatible storage
- Built-in RDP and VNC sessions, no separate windows
- Detach any tab into its own window, dock it back without reconnecting
- Passwords and keys kept in Windows' secure credential store, never in a plain file
- Unknown or changed server host keys are never accepted silently
- No listening ports, no local server — only outbound connections you configure
- Import sessions from PuTTY and MobaXterm
- Free and open source (GPL-3.0)

## System requirements

- Windows 10 version 22H2 or later, or Windows 11
- x64 or ARM64
- Internet (or local network) connection to the servers you connect to

*(Figures to confirm once MSIX builds exist — disk space, architectures actually packaged.)*

## Age rating

To be completed by the IARC questionnaire in Partner Center (the account owner fills it in — see
[requirements.md](requirements.md)). Expected outcome: suitable for general audiences; no
objectionable content, no ads, no in-app purchases, no user-to-user chat.

## Privacy policy URL

Decided 2026-10-22: stays in the repo, no separate hosting. Once the repo is public:

```
https://github.com/picoroma/PiCoXTerm/blob/main/store/privacy-policy.md
```

Not usable yet — the repo is still private, so this URL 404s for Microsoft's reviewers (and
anyone else) until it's made public. See requirements.md.

## Support contact / website

**picodev@cocco.biz**. Project page: https://github.com/picoroma/PiCoXTerm

## Screenshots

Store requirements: at least 1 screenshot, PNG or JPEG, 1366×768 minimum (desktop), up to 10
images. Ready in [`screenshots/`](screenshots/), 1456×819 PNG, captured from the installed MSIX
build with the new icon, against disposable demo data (not real sessions): SSH terminal with the
SFTP panel, the S3 browser, a tab detached into its own window, and the "new session" picker.
`promo/` still has six older screenshots from the LinkedIn announcement if more variety is wanted.
The original hero image you're preparing can double as the Store's main app image/icon artwork.
