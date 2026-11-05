# Persimmon

**The conversion layer for your desktop. Scriptable, offline, everywhere.**

An open-source file conversion toolkit for Linux, Windows and macOS. Everything runs locally; there is no network code in the build.

> Pre-release. Nothing converts files yet — the project is in Phase 0 (de-risk and decide). `prsm --version` is all the CLI does so far.
>
> **macOS comes first.** The engine and CLI are platform-neutral and build on all three from the first commit, but the GUI and shell integration are built out on macOS through to v1.0 before Windows and Linux begin. See WD-10 and DD-21 in `PLAN.md`.

## Scope

**In:** convert, compress, extract, merge, split, batch rename, metadata control, OCR, watch folders, CLI.

**Out:** editing, timelines, colour grading, cloud sync, accounts, AI features, mobile.

This boundary is a decision, not an oversight. See DD-01 in `PLAN.md`.

## Building

You need Rust; the version is pinned in [`rust-toolchain.toml`](rust-toolchain.toml) and `rustup` installs it on first use.

```sh
cargo build --release
./target/release/prsm --version
```

The workspace has two crates. `persimmon-core` is the engine, platform-neutral by rule (DD-22). `persimmon-cli` builds the `prsm` binary. CI builds and tests both on macOS, Windows and Linux on every push. See [`CONTRIBUTING.md`](CONTRIBUTING.md) before sending a change.

## Where things are

| File | What it is |
|---|---|
| [`PLAN.md`](PLAN.md) | **The living document** — directives, phases, design decisions, backlog. Start here. |
| [`ROADMAP.md`](ROADMAP.md) | The detailed phase plan with schedule, costs and risks. |
| [`docs/phase-0-spikes.md`](docs/phase-0-spikes.md) | The current phase, specified. |
| [`docs/phase-0-gate.md`](docs/phase-0-gate.md) | Phase 0's outcome: the interaction model per platform, spike verdicts, go/no-go. |
| [`docs/contrast-audit.md`](docs/contrast-audit.md) | WCAG contrast audit of the palette and the fixes adopted (DD-26). |
| [`docs/naming.md`](docs/naming.md) | Trademark, registry and binary-name checks, and the licence variant (Task 0.1). |
| [`docs/kickoff-prompt.md`](docs/kickoff-prompt.md) | The prompt that starts a development conversation with the right context. |
| `spikes/` | Throwaway code for the Phase 0 spikes, one folder each. |
| [`crates/`](crates) | The Rust workspace: `persimmon-core` and `persimmon-cli` (`prsm`). |
| [`docs/port-notes.md`](docs/port-notes.md) | Every macOS-only capability, recorded as it lands, so the port stays a port (DD-22). |
| [`THIRD-PARTY.md`](THIRD-PARTY.md) | Licence inventory of everything bundled or linked. |

## Licence

GPL-3.0-or-later — see [`LICENSE`](LICENSE). The CLI binary is `prsm`; see [`docs/naming.md`](docs/naming.md) for why not `psm`.

