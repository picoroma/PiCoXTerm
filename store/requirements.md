# Microsoft Store: what's missing

Updated 2026-10-23. **Upload to Partner Center**: `msix/PiCoDev.PiCoXTerm_0.1.8.0_x64.msix` —
built with `--no-sign` (the Store signs it; the `--cert devcert.pfx` builds elsewhere in this
folder are for local test installs only, never for submission). **The Revision (4th number) must
stay 0** — Partner Center rejects any other value; the package counter lives in Build instead
(0.1.8.0, not 0.1.0.8). Not tracked in git (`.gitignore`); rebuild with
`cd msix && winapp pack dist --no-sign` after staging a fresh release build and bumping `Version`
in `Package.appxmanifest`.

**Restricted capability justification** (`runFullTrust` — Partner Center asks for this on every
submission that declares it, contrary to what an earlier note here assumed): same text each time,
reuse it.

> PiCoXterm is a desktop remote-access client (SSH/SFTP, FTP/FTPS, S3, RDP, VNC) for IT
> administrators and developers. It is built with Tauri, a native Win32 application using the
> system WebView2 runtime — not a UWP app — so it requires `runFullTrust` to run as a classic
> desktop application packaged via the Desktop Bridge.
>
> Full trust is needed to:
> - Open outbound network connections to the servers the user configures (SSH, FTP, RDP, VNC, S3)
>   — the app opens no listening ports and runs no local server.
> - Read and write its own configuration files (session list, settings) in the user's AppData
>   folder.
> - Store and retrieve credentials via the Windows Credential Manager (passwords are never written
>   to disk in plain text).
> - Launch a companion process included in the same package (`picoxterm-rdp.exe`) that implements
>   the RDP protocol.
> - Access the local file system when the user explicitly browses, uploads or downloads files
>   through an SFTP/FTP/S3 session they configured themselves.
>
> The app accesses only what the user explicitly configures or requests. No telemetry, no
> background services, no administrator/elevated privileges are used.

Account open, app name reserved (identity: `PiCoDev.PiCoXTerm`, publisher
`CN=47CF9EDF-34F1-4B44-8BB0-606DED1FE4C3`, display name "PiCoXTerm"). `msix/Package.appxmanifest`
is written with real icons, and `scripts/build-msix.ps1` builds and packages a local x64 MSIX —
tested once end to end with a development certificate (`winapp cert generate`,
`winapp pack dist --cert devcert.pfx`): packaging itself works.

## Only you can do these (account, money, identity)

1. ~~Microsoft Partner Center developer account.~~ **Done.**
2. ~~Reserve the app name.~~ **Done.** Partner Center still shows "PiCoXTerm" (capital T) as the
   reserved name, but `Properties/DisplayName` in the manifest is "PiCoXterm" (2026-10-21 decision)
   — the `Identity/Name` itself (`PiCoDev.PiCoXTerm`) is fixed regardless, so this is cosmetic only.
3. ~~Trust the dev certificate locally.~~ **Done**, verified the installed MSIX launches correctly.
4. **IARC age-rating questionnaire**, filled in Partner Center under your account.
5. ~~Support contact and privacy policy URL.~~ **Done**: contact is `picodev@cocco.biz` (now in
   `store/privacy-policy.md` and `store/listing.md`); the privacy policy itself stays in the repo
   rather than being hosted separately (2026-10-22 decision) — see "Privacy policy hosting" below.
6. **Original hero image and tagline** — you're preparing these; `store/listing.md` has
   placeholders waiting for them.
7. **Make the repository public** before submitting — the Store listing's privacy policy URL
   points into the repo itself (see below), so it must be reachable by Microsoft's reviewers and
   anyone who installs the app, which a private repo isn't. Do this "at the appropriate time", at
   your call, not automatically.
8. First submission review and any back-and-forth with Microsoft's certification team.

## Privacy policy hosting

Decided 2026-10-22: no separate hosting (no GitHub Pages, no external site). The Store listing's
privacy policy URL is the repo file itself:

```
https://github.com/picoroma/PiCoXTerm/blob/main/store/privacy-policy.md
```

This only resolves once the repository is public (item 7 above) — until then it 404s. Submitting
the Store listing with this URL while the repo is private would fail review, so this is a hard
prerequisite, not just a nice-to-have.

## Verified 2026-10-21, installed and trusted on this machine

- The MSIX launches (`PiCoDev.PiCoXTerm_mzb3h6p4gck8g!PiCoXterm`), window opens and responds.
- **No file system virtualization to work around**: a `runFullTrust` packaged app gets normal,
  non-virtualized access. `sessions.json`, `settings.json`, `known_hosts` and the audit log all
  landed in the same real `%APPDATA%\PiCoXterm` the portable build already used — same data, same
  place, nothing to change in the code.
- `picoxterm-rdp.exe` is staged correctly beside `picoxterm.exe` inside the package.
- Not yet verified: the keyring (Windows Credential Manager) from inside the package, an actual SSH
  connection, and the RDP engine actually launching end-to-end — worth a real pass before relying on
  this for anything but packaging.

## What I'll do next

1. **arm64 build**: today's `build-msix.ps1` only packages the architecture it's built on (x64).
   Producing an arm64 package too (and a `.msixbundle` combining both, which is what the Store
   actually wants, via `winapp pack x64-folder arm64-folder`) needs a build on, or cross-compiled
   for, arm64 — not done yet.
2. **CI**: no MSIX packaging step in `.github/workflows/ci.yml` yet; add one once the manual flow
   is proven stable (and decide whether it packages on every push or only on a release tag).
3. Bump the manifest `Version` (`msix/Package.appxmanifest`) on every future beta, same place the
   `Cargo.toml`/`tauri.conf.json` version already gets bumped.

## Done since the above

- **Icon**: finalized (network globe + terminal prompt, orange/blue), resized once for taskbar
  legibility, shipped in the app and the MSIX package (D-41).
- **Screenshots**: four real ones in [`screenshots/`](screenshots/) (1456×819 PNG), captured from
  the installed MSIX with disposable demo data — not the older `promo/` set.

## Already true, nothing to add

- No code-signing certificate to buy for the Store — it signs the package. `devcert.pfx` here is
  only for local test installs, is gitignored, and uses the publicly-known default password
  (`winapp cert generate`'s own default) — fine for that purpose, never meant as a real secret.
- No Tauri updater plugin in the app (already true before this decision) — the Store handles
  updates for the MSIX install; the portable ZIP (outside the Store) stays manual, as today.
- Capabilities: `msix/Package.appxmanifest` only declares `runFullTrust` (required for any packaged
  Win32/Tauri app) — checked against `src-tauri/capabilities/default.json`
  (clipboard-manager text read/write, dialog open/save): neither needs a separate MSIX capability.
- Privacy: checked the codebase for telemetry/analytics/crash-reporting/update-check network
  calls — there are none. `store/privacy-policy.md` reflects that. (The `winapp` CLI itself reports
  anonymous usage telemetry about the *tool*, not the app; every command above sets
  `WINAPP_CLI_TELEMETRY_OPTOUT=1` to turn that off on this machine — it has no bearing on
  PiCoXterm's own privacy, since the CLI never ships inside the package.)
- The portable ZIP distribution (today's beta releases on GitHub) is unaffected and continues
  alongside the Store listing, unsigned, as now.
