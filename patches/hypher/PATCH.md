# Locus patch of hypher 0.1.8

Upstream: https://crates.io/crates/hypher/0.1.8 (MIT OR Apache-2.0), used by Typst for
hyphenation. Copied unchanged except:

1. `tries/sk.bin` (Slovak patterns, GPL-2.0-or-later) and `tries/hu.bin` (Hungarian
   patterns, MPL-1.1 OR GPL-2.0-only OR LGPL-2.1-only) are deleted.
2. In `Cargo.toml`, `"hungarian"` and `"slovak"` are removed from the `full` feature, and
   the `hungarian` and `slovak` features themselves are removed, so nothing can turn them
   back on (asking for them is a build error).
3. In `src/lang.rs`, the two `include_bytes!` match arms for `hu.bin` and `sk.bin` (with
   their `#[cfg]` lines) are removed, so no code refers to the deleted files.

The whole change, against the published crate: 8 deleted lines and 2 deleted files.

Effect: Typst cannot hyphenate Slovak or Hungarian text. Locus reports are English.

To reapply after a Typst update that moves to a new hypher version:
1. Copy the new version from the cargo registry over `patches/hypher/`.
2. Repeat steps 1–3 above, update the version here, and in the root Cargo.toml's
   `[patch.crates-io]` if the path or version changes.
3. Run `cargo tree -i hypher` (must show the path source) and
   `py tools/notices/notices.py check`, which fails if Slovak or Hungarian pattern data or
   features reappear.
