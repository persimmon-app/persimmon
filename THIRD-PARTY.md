# Third-party components

Every library, codec and tool that Persimmon bundles or links is listed here with its licence and how it is used. A dependency is added to this file in the same commit that adds it. Retrofitting a licence inventory across a bundled FFmpeg, image codecs and libarchive is far harder than keeping one.

Persimmon is GPL-3.0-or-later. Every entry must be compatible with that.

## Bundled or linked

| Component | Version | Licence | Used for | Linked how |
|---|---|---|---|---|
| [clap](https://github.com/clap-rs/clap) | 4 | MIT OR Apache-2.0 | `prsm` argument parsing | Statically linked (Rust crate) |
| [unicode-normalization](https://github.com/unicode-rs/unicode-normalization) | 0.1 | (MIT OR Apache-2.0) AND Unicode-3.0 | `persimmon-core`: comparing output names the way APFS and NTFS do, so `café` in NFC and NFD forms can't overwrite each other | Statically linked (Rust crate) |

`persimmon-corpus` is a test-only crate and is never shipped.

Transitive Rust dependencies (clap's own dependencies, for example) aren't listed one by one. Instead, `cargo deny check licenses` runs in CI on every push and fails the build if any dependency carries a licence not on the list in `deny.toml`. The full inventory, with every transitive crate and its licence, is generated for each release.

## Never

| Component | Why not |
|---|---|
| unRAR (RARLAB) | Its licence forbids using the code to re-create the RAR compression algorithm, which is a restriction GPL-3.0 does not allow. RAR and RAR5 extraction comes from libarchive's own BSD-licensed reader instead. |
