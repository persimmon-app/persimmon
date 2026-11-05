# Contributing to Persimmon

Thank you for looking. Persimmon is pre-release: it is in Phase 0, and nothing converts files yet. If you are planning anything larger than a small fix, open an issue first so the change can be agreed before you write it.

By taking part you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Licence of contributions

Persimmon is licensed GPL-3.0-or-later. Anything you contribute is licensed under the same terms. There is no CLA. New source files start with:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors
```

## Building

The toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml); `rustup` installs it on first use.

```sh
cargo build
cargo run --bin prsm -- --version
```

Before sending a change, run what CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs those on macOS, Windows and Linux for every push. All three must pass, even though only macOS ships before v1.0.

## Rules that are easy to miss

- **Scope.** Persimmon converts, compresses and extracts, plus rename, metadata, OCR and watch folders. Editing, timelines, colour grading, cloud sync, accounts and AI features are out. A change outside that boundary will be declined however good it is.
- **Offline.** No network code in the build.
- **Platform code stays in one place.** `persimmon-core` and `prsm` contain no platform-conditional code outside `crates/persimmon-core/src/platform/`. A macOS-only feature above that boundary must say so in its pull request description, with the macOS API it rests on, so the Windows and Linux port can be tracked.
- **Dependencies are listed.** Anything bundled or linked goes into [`THIRD-PARTY.md`](THIRD-PARTY.md) in the same change, with its licence. It must be compatible with GPL-3.0-or-later. unRAR is never used.
- **Markdown is not hard-wrapped.** One paragraph, list item or table row is one line, however long.
