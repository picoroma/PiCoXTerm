# picoxterm-rdp

The RDP engine of PiCoXterm (built on [IronRDP](https://github.com/Devolutions/IronRDP)). It is **not** meant
to be run by hand: the app starts it and talks to it over stdin/stdout (`crates/pxt-rdp-proto`).
No listening ports; logs go to stderr and never contain the password.

It is a separate program with its own `Cargo.lock` because IronRDP pins release-candidate versions of the
RustCrypto libraries that cannot share a lockfile with `russh` (decision D-33 in `docs/SPECIFICHE.md`).

```
cargo build --release          # produces target/release/picoxterm-rdp(.exe)
```

Copy the executable next to `picoxterm(.exe)`.

`third_party/ironrdp-session` is a copy of `ironrdp-session` 0.11.0 with a small fix (see
`PICOXTERM-PATCH.md` there).
