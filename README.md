# Persimmon

**The conversion layer for your desktop. Scriptable, offline, everywhere.**

An open-source file conversion toolkit for Linux, Windows and macOS. Everything runs locally; there is no network code in the build.

> Pre-release. `prsm` converts and compresses images, audio and video, and makes, merges, splits and renders PDFs today. Archives come next: `--dry-run` already plans them, and running them exits with `NOT_IMPLEMENTED` until their backend lands.
>
> **macOS comes first.** The engine and CLI are platform-neutral and build on all three from the first commit, but the GUI and shell integration are built out on macOS through to v1.0 before Windows and Linux begin.

## Scope

**In:** convert, compress, extract, merge, split, batch rename, metadata control, OCR, watch folders, CLI.

**Out:** editing, timelines, colour grading, cloud sync, accounts, AI features, mobile.

This boundary is a decision, not an oversight.

## Building

You need Rust; the version is pinned in [`rust-toolchain.toml`](rust-toolchain.toml) and `rustup` installs it on first use.

Images go through [libvips](https://www.libvips.org/) 8.15 or later, which must be installed first. On Debian and Ubuntu, the libheif plugins read HEIC and AVIF and write AVIF:

```sh
brew install vips                    # macOS
sudo apt-get install libvips-dev libheif-plugin-libde265 libheif-plugin-dav1d libheif-plugin-aomenc   # Debian and Ubuntu
```

Then:

```sh
cargo build --release
./target/release/prsm --version
```

To build without image support, and without needing libvips, add `--no-default-features`. Image conversions then fail with `BACKEND_MISSING`.

Audio and video go through [FFmpeg](https://ffmpeg.org/) 7 or later, which `prsm` runs as a separate program; it is never built in. `prsm` uses the `ffmpeg` on your `PATH`, with `ffprobe` beside it, or the one you name with `--ffmpeg <path>`:

```sh
brew install ffmpeg                  # macOS
sudo apt-get install ffmpeg          # Ubuntu 26.04 or later, Debian 13 or later
winget install Gyan.FFmpeg           # Windows
```

Without it, everything else still works, and audio and video conversions fail with `BACKEND_MISSING` and the install command.

PDFs need nothing installed: reading, rendering and writing them is built in. Turning images other than JPEG into a PDF uses libvips, like other image conversions.

The workspace has two crates. `persimmon-core` is the engine, platform-neutral by rule. `persimmon-cli` builds the `prsm` binary. CI builds and tests both on macOS, Windows and Linux on every push. See [`CONTRIBUTING.md`](CONTRIBUTING.md) before sending a change.

## Using it

```sh
prsm convert holiday.heic --to jpeg              # holiday.jpg, next to the original
prsm convert *.png --to webp --quality medium    # low, medium, high (the default) or max
prsm convert photo.jpg --to webp --max-size 2048 # shrink so the long edge is at most 2048 px
prsm convert photo.jpg --to png --strip-metadata # drop EXIF, XMP and colour profiles (colours go to sRGB first)
prsm compress scan.png                           # "scan (compressed).png", only if it's smaller
prsm convert clip.mov --to mp4                   # copies the video stream when MP4 already takes it: fast and lossless
prsm convert clip.mov --to mp4 --preset small    # HEVC: smaller, slower. --preset compatible is H.264, which plays anywhere
prsm convert clip.mov --to webm                  # VP9 and Opus; --preset small makes AV1
prsm convert talk.mp4 --to m4a                   # just the audio
prsm convert clip.mp4 --to gif --max-size 480    # an animated GIF, at most 480 px on the long edge
prsm compress clip.mp4                           # re-encoded, kept only if it's smaller
prsm info clip.mov                               # duration, codecs, size and what it can become
prsm convert scan.jpg --to pdf                   # one page per image; JPEGs go in untouched, without their EXIF or GPS
prsm merge --to pdf a.pdf photo.jpg b.pdf        # merged.pdf, pages in argument order
prsm split report.pdf --pages 1-3,7-             # "report (pages 1-3,7-).pdf"
prsm convert report.pdf --to png                 # one PNG per page at 150 dpi, in a folder named report
prsm convert report.pdf --to jpeg --pages 2      # just page 2: "report (page 2).jpg"
prsm convert clip.mov --to mp4 --dry-run         # show what would be written, write nothing
```

`prsm` never overwrites a file unless you pass `--overwrite`. A clash becomes `name (1).ext`; a folder of rendered pages is never replaced, even with `--overwrite`. Output goes next to each source unless `--out <dir>` says otherwise. Ctrl-C stops a video at once and a PDF after the page it's on, lets an image in progress finish, skips the rest, and leaves no partial files behind. Closing the terminal or sending SIGTERM does the same. On a terminal, audio, video and PDFs show their progress as they convert.

`prsm formats` lists every conversion, and `prsm info <file>` says what a file is and what it can become.

## Exit codes

`prsm` exits with a stable code for every kind of failure, so scripts can tell them apart. These codes are a public contract: once v0.1.0 ships, a code never changes meaning and is never reused.

| Error code | Exit | Meaning |
|---|---|---|
| — | 0 | Every input succeeded |
| `INTERNAL` | 1 | A bug in Persimmon |
| `USAGE` | 2 | The command line is wrong: an unknown flag, a missing argument, a bad page range |
| `UNSUPPORTED_INPUT` | 3 | The input isn't a file type Persimmon reads, it's a folder, an image is over the 500-megapixel limit, or a PDF is password-protected |
| `BACKEND_MISSING` | 4 | A tool the conversion needs isn't installed or is too old, such as FFmpeg (7 or later, with the needed encoder), or this `prsm` was built without image support |
| `CORRUPT_INPUT` | 5 | The file is empty, truncated, or doesn't match its type |
| `OUTPUT_NOT_WRITABLE` | 6 | The output folder can't be written to; `--out <dir>` is the fix |
| `INPUT_NOT_FOUND` | 7 | The input path doesn't exist |
| `UNSUPPORTED_CONVERSION` | 8 | The input type can't become the requested type; `prsm formats` lists what can |
| `OUTPUT_EXISTS` | 9 | The output appeared while the job ran, and `--overwrite` wasn't given |
| `NOT_IMPLEMENTED` | 10 | Before v0.1.0 only: the archive backend isn't built yet. This code is removed before v0.1.0 |
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

