# Third-party components

Every library, codec and tool that Persimmon bundles or links is listed here with its licence and how it is used. A dependency is added to this file in the same commit that adds it. Retrofitting a licence inventory across a bundled FFmpeg, image codecs and libarchive is far harder than keeping one.

Persimmon is GPL-3.0-or-later (DD-04). Every entry must be compatible with that.

## Bundled or linked

| Component | Version | Licence | Used for | Linked how |
|---|---|---|---|---|

*None yet. `persimmon-core` and `persimmon-cli` use only the Rust standard library.*

## Never

| Component | Why not |
|---|---|
| unRAR (RARLAB) | Its licence forbids using the code to re-create the RAR compression algorithm, which is a restriction GPL-3.0 does not allow. RAR and RAR5 extraction comes from libarchive's own BSD-licensed reader instead (BL-T20). |
