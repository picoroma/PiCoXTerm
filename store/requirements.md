# Microsoft Store: what's missing

Updated 2026-10-21. Account open, app name reserved (identity: `PiCoDev.PiCoXTerm`, publisher
`CN=47CF9EDF-34F1-4B44-8BB0-606DED1FE4C3`, display name "PiCoXTerm"). `msix/Package.appxmanifest`
is written with real icons, and `scripts/build-msix.ps1` builds and packages a local x64 MSIX —
tested once end to end with a development certificate (`winapp cert generate`,
`winapp pack dist --cert devcert.pfx`): packaging itself works.

## Only you can do these (account, money, identity)

1. ~~Microsoft Partner Center developer account.~~ **Done.**
2. ~~Reserve the app name.~~ **Done** — but check the capitalisation: Partner Center shows
   **PiCoXTerm** (capital T), while the rest of the project (binary, `tauri.conf.json`, README,
   GitHub) uses **PiCoXterm**. The manifest's `Identity/Name` is fixed as registered
   (`PiCoDev.PiCoXTerm`, cannot change without reserving a new name), but `Properties/DisplayName` —
   the name shown to users — is currently set to "PiCoXterm" to match the rest of the project. Say
   if you'd rather match the Store's "PiCoXTerm" instead; it's a one-line change.
3. **Trust the dev certificate once, locally, as administrator** (to actually launch the test
   package — packaging itself doesn't need this):
   ```powershell
   winapp cert install msix\devcert.pfx
   Add-AppxPackage msix\PiCoDev.PiCoXTerm_0.1.0.4_x64.msix
   ```
   Not needed for the Store submission itself — only to run the MSIX build on this machine.
4. **IARC age-rating questionnaire**, filled in Partner Center under your account.
5. **Support contact** Microsoft will display (email or page) and a **public URL for the privacy
   policy** (`store/privacy-policy.md` is ready to publish — GitHub Pages, or just a raw link to
   this file in the repo, both work for Microsoft). The policy also still has a
   **[support contact — to confirm]** placeholder I didn't fill in on my own.
6. **Original hero image and tagline** — you're preparing these; `store/listing.md` has
   placeholders waiting for them.
7. First submission review and any back-and-forth with Microsoft's certification team.

## What I'll do next

1. **Verify every path PiCoXterm writes to** (`sessions.json`, `settings.json`, `known_hosts`, the
   audit log, the keyring) actually works once the MSIX is installed and trusted — this needs you
   to run the two commands above first, then I can drive the installed app and check.
2. **Package `picoxterm-rdp.exe` correctly**: it's staged next to `picoxterm.exe` inside the MSIX
   (not its own `Application` entry — it's a child process, never launched from Start) and
   `pxt_rdp::engine_path()` looks for it beside the running exe, read-only, which should work as-is
   under MSIX; still needs the real installed-app check to be sure.
3. **arm64 build**: today's `build-msix.ps1` only packages the architecture it's built on (x64).
   Producing an arm64 package too (and a `.msixbundle` combining both, which is what the Store
   actually wants, via `winapp pack x64-folder arm64-folder`) needs a build on, or cross-compiled
   for, arm64 — not done yet.
4. **CI**: no MSIX packaging step in `.github/workflows/ci.yml` yet; add one once the manual flow
   is proven stable (and decide whether it packages on every push or only on a release tag).
5. Resize/relabel the existing `promo/` screenshots for the Store's size requirements once a
   trusted, installed MSIX build exists to screenshot from.
6. Bump the manifest `Version` (`msix/Package.appxmanifest`) on every future beta, same place the
   `Cargo.toml`/`tauri.conf.json` version already gets bumped.

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
