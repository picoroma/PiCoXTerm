# Local patch (PiCoXterm)

Copy of `ironrdp-session` 0.11.0 (MIT OR Apache-2.0, Devolutions) with one change in `src/fast_path.rs`:
rows of compressed bitmaps are decoded with the padded width (`update.width`, a multiple of 4) but were
copied with the destination rectangle width, which skews the image. The padding is now removed from
each row first (`crop_rows`). See `docs/spike-m7-rdp.md` and `docs/spike-m7-rdp/*.patch`.

Remove this copy (and the `[patch.crates-io]` entry in the root `Cargo.toml`) once the fix is released upstream.
