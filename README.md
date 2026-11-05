# Persimmon

**The conversion layer for your desktop. Scriptable, offline, everywhere.**

An open-source file conversion toolkit for Linux, Windows and macOS. Everything runs locally; there is no network code in the build.

> Pre-release. Nothing converts files yet — the project is in Phase 0 (de-risk and decide). `prsm --version` is all the CLI does so far.
>
> **macOS comes first.** The engine and CLI are platform-neutral and build on all three from the first commit, but the GUI and shell integration are built out on macOS through to v1.0 before Windows and Linux begin.

## Scope

**In:** convert, compress, extract, merge, split, batch rename, metadata control, OCR, watch folders, CLI.

**Out:** editing, timelines, colour grading, cloud sync, accounts, AI features, mobile.

This boundary is a decision, not an oversight.

## Building

You need Rust; the version is pinned in [`rust-toolchain.toml`](rust-toolchain.toml) and `rustup` installs it on first use.

```sh
cargo build --release
./target/release/prsm --version
```

The workspace has two crates. `persimmon-core` is the engine, platform-neutral by rule. `persimmon-cli` builds the `prsm` binary. CI builds and tests both on macOS, Windows and Linux on every push. See [`CONTRIBUTING.md`](CONTRIBUTING.md) before sending a change.

## Exit codes

`prsm` exits with a stable code for every kind of failure, so scripts can tell them apart. These codes are a public contract: once v0.1.0 ships, a code never changes meaning and is never reused.

| Error code | Exit | Meaning |
|---|---|---|
| — | 0 | Every input succeeded |
| `INTERNAL` | 1 | A bug in Persimmon |
| `USAGE` | 2 | The command line is wrong: an unknown flag, a missing argument, a bad page range |
| `UNSUPPORTED_INPUT` | 3 | The input isn't a file type Persimmon reads, or it's a folder |
| `BACKEND_MISSING` | 4 | A tool the conversion needs, such as FFmpeg, isn't installed |
| `CORRUPT_INPUT` | 5 | The file is empty, truncated, or doesn't match its type |
| `OUTPUT_NOT_WRITABLE` | 6 | The output folder can't be written to; `--out <dir>` is the fix |
| `INPUT_NOT_FOUND` | 7 | The input path doesn't exist |
| `UNSUPPORTED_CONVERSION` | 8 | The input type can't become the requested type; `prsm formats` lists what can |
| `OUTPUT_EXISTS` | 9 | The output appeared while the job ran, and `--overwrite` wasn't given |
| `NOT_IMPLEMENTED` | 10 | Before v0.1.0 only: the backend for this conversion isn't built yet. This code is removed before v0.1.0 |
| `CANCELLED` | 130 | The job was cancelled (the shell convention for Ctrl-C) |

### Batches

When a command takes several inputs, `prsm` keeps going after a failure. It exits 0 only if every input succeeded. Otherwise it exits with the code of the **first failing input, in argument order**.

### Error messages

Errors go to stderr as `prsm: {file}: {reason}`, followed by `  fix: {fix}` on the next line when there is one.

## Where things are

| File | What it is |
|---|---|
| [`crates/`](crates) | The Rust workspace: `persimmon-core` and `persimmon-cli` (`prsm`). |
| [`THIRD-PARTY.md`](THIRD-PARTY.md) | Licence inventory of everything bundled or linked. |

## Licence

GPL-3.0-or-later — see [`LICENSE`](LICENSE). The CLI binary is `prsm`, not `psm`, because macOS already ships `/usr/bin/psm`.

