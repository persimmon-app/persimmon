# Persimmon

**The conversion layer for your desktop. Scriptable, offline, everywhere.**

An open-source file conversion toolkit for Linux, Windows and macOS. Everything runs locally; there is no network code in the build.

> Version 0.2: the macOS app and the command line. Both convert and compress images, audio and video; make, merge, split and render PDFs; and extract and create archives.
>
> **macOS comes first.** The engine and CLI are platform-neutral and build on all three from the first commit, but the GUI and shell integration are built out on macOS through to v1.0 before Windows and Linux begin.

## Scope

**In:** convert, compress, extract, merge, split, batch rename, metadata control, OCR, watch folders, CLI.

**Out:** editing, timelines, colour grading, cloud sync, accounts, AI features, mobile.

This boundary is a decision, not an oversight.

## Installing

### The macOS app

Each [GitHub Release](https://github.com/persimmon-app/persimmon/releases) from 0.2.0 has `Persimmon-<version>-arm64.dmg`, with a `.sha256` checksum beside it. It needs macOS 14 or later on Apple silicon, and nothing else: FFmpeg, libvips and libarchive are inside the app. Open the DMG and drag Persimmon to Applications.

The app isn't signed yet. macOS blocks apps from developers it doesn't know. Open Persimmon once (it will refuse), then go to System Settings → Privacy & Security, scroll to Security, and choose **Open Anyway**. Or clear the quarantine flag in Terminal:

```sh
xattr -dr com.apple.quarantine /Applications/Persimmon.app
```

Updates: download the new DMG from the Releases page. Persimmon never checks for updates itself (it has no network code). Its settings, presets and history live in `~/Library/Application Support/Persimmon`. Each release also carries `Persimmon-<version>-sources.tar`, the source of everything the app bundles, and FFmpeg's configure line.

### Release archives

Each [GitHub Release](https://github.com/persimmon-app/persimmon/releases) has one archive per platform, each with a `.sha256` checksum beside it. Every archive holds `prsm`, this README, the licence, `THIRD-PARTY.md` and `THIRD-PARTY-LICENSES.html`.

| Archive | For | Install first |
|---|---|---|
| `prsm-<version>-aarch64-apple-darwin.tar.gz` | macOS 14 or later, Apple silicon | `brew install vips libarchive ffmpeg` |
| `prsm-<version>-x86_64-apple-darwin.tar.gz` | macOS 14 or later, Intel | `brew install vips libarchive ffmpeg` |
| `prsm-<version>-x86_64-unknown-linux-gnu.tar.gz` | Ubuntu 26.04 or later, x86-64 | `sudo apt-get install libvips42t64 libheif-plugin-libde265 libheif-plugin-dav1d libheif-plugin-aomenc libarchive13t64 ffmpeg` |
| `prsm-<version>-x86_64-pc-windows-msvc.zip` | Windows 10 or later, x86-64 | `winget install Gyan.FFmpeg` |

The macOS and Linux binaries link the system's libvips and libarchive, so **without those installed, `prsm` won't start at all**. FFmpeg is needed only for audio and video; without it, those conversions fail with `BACKEND_MISSING` and everything else works. The Windows build has no image conversions and no RAR extraction yet: those fail with `BACKEND_MISSING`.

The binaries aren't signed. On macOS, after unpacking, clear the quarantine flag once, or Gatekeeper refuses to run `prsm`:

```sh
xattr -d com.apple.quarantine prsm
```

Then put `prsm` somewhere on your `PATH`, such as `/usr/local/bin` or `~/.local/bin`.

### With Cargo

```sh
brew install vips libarchive pkgconf    # macOS; on Debian and Ubuntu, the -dev packages under Building
export PKG_CONFIG_PATH="$(brew --prefix libarchive)/lib/pkgconfig"   # macOS: Homebrew keeps libarchive out of pkg-config's sight
cargo install persimmon-cli
```

Without libvips and libarchive, use `cargo install persimmon-cli --no-default-features`: image conversions and RAR extraction then fail with `BACKEND_MISSING`, and everything else works.

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

RAR archives are read by [libarchive](https://www.libarchive.org/)'s own reader (never unRAR), which must be installed too: `brew install libarchive` on macOS, `sudo apt-get install libarchive-dev` on Debian and Ubuntu. ZIP, TAR (plain, gz, bz2, xz, zst) and 7z need nothing installed.

A default build links libvips and libarchive, so it won't start where they aren't installed. To build without them, add `--no-default-features`. Image conversions and RAR extraction then fail with `BACKEND_MISSING`; everything else works.

Audio and video go through [FFmpeg](https://ffmpeg.org/) 7 or later, which `prsm` runs as a separate program; it is never built in. `prsm` uses the `ffmpeg` on your `PATH`, with `ffprobe` beside it, or the one you name with `--ffmpeg <path>`:

```sh
brew install ffmpeg                  # macOS
sudo apt-get install ffmpeg          # Ubuntu 26.04 or later, Debian 13 or later
winget install Gyan.FFmpeg           # Windows
```

Without it, everything else still works, and audio and video conversions fail with `BACKEND_MISSING` and the install command.

PDFs need nothing installed: reading, rendering and writing them is built in. Turning images other than JPEG into a PDF uses libvips, like other image conversions.

The workspace has four crates. `persimmon-core` is the engine, platform-neutral by rule. `persimmon-cli` builds the `prsm` binary. `persimmon-app` is the macOS app; elsewhere it builds as an empty stub. `persimmon-corpus` generates the test files and is never published. CI builds and tests the workspace on macOS, Windows and Linux on every push. See [`CONTRIBUTING.md`](CONTRIBUTING.md) before sending a change.

### The macOS app

The app is a [Tauri](https://tauri.app/) shell over the engine, with its interface in [`crates/persimmon-app/ui`](crates/persimmon-app/ui) (Svelte and TypeScript). It needs Node 22 as well as the libraries above. A development build links the libraries Homebrew installed and uses the FFmpeg on your `PATH`:

```sh
npm ci --prefix crates/persimmon-app/ui
cd crates/persimmon-app && ui/node_modules/.bin/tauri build --debug --bundles app
```

The DMG a release ships is built by the scripts in [`crates/persimmon-app/packaging`](crates/persimmon-app/packaging), the same way on a Mac as in the release workflow. They download pinned, checksummed sources and build libvips, libarchive and a static FFmpeg from them for macOS 14, so they need Homebrew's `meson`, `ninja`, `cmake` and `pkgconf`:

```sh
bash crates/persimmon-app/packaging/build-libs.sh
bash crates/persimmon-app/packaging/build-ffmpeg.sh
bash crates/persimmon-app/packaging/bundle.sh target/native dist
```

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
prsm extract photos.zip                          # a folder "photos" next to it; never "photos/photos"
prsm extract src.tar.zst --out ~/src             # ZIP, TAR, TAR.GZ/BZ2/XZ/ZST, 7z and RAR
prsm merge --to zip notes/ photo.jpg             # merged.zip; also --to tar.gz
prsm convert clip.mov --to mp4 --dry-run         # show what would be written, write nothing
```

`prsm` never overwrites a file unless you pass `--overwrite`. A clash becomes `name (1).ext`; a folder (rendered pages, an extracted archive) is never replaced, even with `--overwrite`. Extracting never writes outside its folder: an archive with an absolute path, a `..`, or a path through a link fails as a whole, and nothing is written. Output goes next to each source unless `--out <dir>` says otherwise. Ctrl-C stops a video at once and a PDF after the page it's on, lets an image in progress finish, skips the rest, and leaves no partial files behind. Closing the terminal or sending SIGTERM does the same. A second Ctrl-C quits at once and may leave a `.part` file. On a terminal, audio, video and PDFs show their progress as they convert.

`prsm formats` lists every conversion, and `prsm info <file>` says what a file is and what it can become.

## Exit codes

`prsm` exits with a stable code for every kind of failure, so scripts can tell them apart. These codes are a public contract: once v0.1.0 ships, a code never changes meaning and is never reused.

| Error code | Exit | Meaning |
|---|---|---|
| — | 0 | Every input succeeded |
| `INTERNAL` | 1 | A bug in Persimmon |
| `USAGE` | 2 | The command line is wrong: an unknown flag, a missing argument, a bad page range |
| `UNSUPPORTED_INPUT` | 3 | The input isn't a file type Persimmon reads, it's a folder, an image is over the 500-megapixel limit, or a PDF or archive is password-protected |
| `BACKEND_MISSING` | 4 | A tool the conversion needs isn't installed or is too old, such as FFmpeg (7 or later, with the needed encoder), or this `prsm` was built without image or RAR support (libvips, libarchive) |
| `CORRUPT_INPUT` | 5 | The file is empty, truncated, or doesn't match its type, or an archive has an unsafe path |
| `OUTPUT_NOT_WRITABLE` | 6 | The output folder can't be written to; `--out <dir>` is the fix |
| `INPUT_NOT_FOUND` | 7 | The input path doesn't exist |
| `UNSUPPORTED_CONVERSION` | 8 | The input type can't become the requested type; `prsm formats` lists what can |
| `OUTPUT_EXISTS` | 9 | The output appeared while the job ran, and `--overwrite` wasn't given |
| `CANCELLED` | 130 | The job was cancelled (the shell convention for Ctrl-C) |

Exit 10 was `NOT_IMPLEMENTED` before every backend existed. It is retired and never reused.

### Batches

When a command takes several inputs, `prsm` keeps going after a failure. It exits 0 only if every input succeeded. Otherwise it exits with the code of the **first failing input, in argument order**. The one exception is cancelling: a batch you cancel exits 130, even if an input before it failed, so a script can always tell it was interrupted. Only the first input that was cancelled or skipped gets a `cancelled` line.

### Error messages

Errors go to stderr as `prsm: {file}: {reason}`, followed by `  fix: {fix}` on the next line when there is one.

## Where things are

| File | What it is |
|---|---|
| [`crates/`](crates) | The Rust workspace: `persimmon-core`, `persimmon-cli` (`prsm`), `persimmon-app` (the macOS app, with its interface in `ui/` and its packaging scripts in `packaging/`) and `persimmon-corpus` (test files). |
| [`THIRD-PARTY.md`](THIRD-PARTY.md) | Licence inventory of everything bundled or linked. |
| [`CHANGELOG.md`](CHANGELOG.md) | What changed in each release. |

## Licence

GPL-3.0-or-later — see [`LICENSE`](LICENSE). The CLI binary is `prsm`, not `psm`, because macOS already ships `/usr/bin/psm`.

