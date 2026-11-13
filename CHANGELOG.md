# Changelog

Every release of Persimmon and what changed in it. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [0.2.0] - unreleased

The first release of the macOS app, and `prsm` 0.2.0. This is an alpha: its purpose is to find bugs, so please report what you run into.

### Added: Persimmon.app for macOS

- A DMG, `Persimmon-0.2.0-arm64.dmg`, with FFmpeg, libvips and libarchive inside the app. Nothing else needs installing.
- **Modes:** Convert, Compress, Extract (archives), and Combine (images and PDFs into one PDF, or files into a ZIP or tar.gz, in an order you set).
- **Ways in:** drop files on the window, on the Dock icon, or on the floating drop window (⌥⌘D, on every Space); or use Open With. Persimmon is the default only for types no other app on your Mac opens.
- **The queue:** each file shows its source and target format, progress, and a reason and a fix in the app's own words when it fails. Each row offers Cancel, Retry or Reveal in Finder as its state allows.
- **Presets:** Web image, Email video and Archive quality, plus your own, each showing the `prsm` command it amounts to. ⌘1–⌘9 pick them.
- **History:** 30 days of batches, searchable, with Reveal, Retry and undo. Undo moves a batch's outputs to the Trash and leaves alone any output changed since.
- **Settings:** General, Output (beside the original, or a folder of your choice, with a fallback folder for originals that can't be written beside, such as the Photos library or Mail), Presets, Engines, Privacy, and About & licences, which links to the sources of everything bundled.
- Light and dark appearance, keyboard operation and VoiceOver labels.
- No network code, as in `prsm`: no update check, no telemetry, no account.

### Added: in the engine (`persimmon-core`)

All additive:

- An optional `serde` feature for the types the app sends to its window.
- Fixes carry a kind (`FixKind`), so a front end can word them its own way.
- `Backends::with_ffmpeg` to use a given FFmpeg, and `Backends::ffmpeg_info` to report which one.
- `PlanMode::Preview`, planning that creates no folders and writes nothing.
- An optional fallback folder for sources whose own folder can't be written.

### Fixed

- A dry run now reports an `--out` that is an existing file.

### Changed

- Nothing in `prsm`'s commands, flags, output or exit codes.

### Known limitations

- The macOS app is unsigned in this alpha: macOS asks you to allow it once (see the README).
- The app needs macOS 14 or later on Apple silicon. There's no Intel build of the app.
- There's no updater: new versions come from the Releases page.
- Jobs run one at a time, in the app as in `prsm`.

## [0.1.0] - 2026-11-11

The first release: the engine and the `prsm` command line.

### Added

- `prsm convert`, `prsm compress`, `prsm info`, `prsm extract`, `prsm merge`, `prsm split` and `prsm formats`.
- **Images**, through libvips: read JPEG, PNG, WebP, HEIC/HEIF, TIFF, GIF (first frame), AVIF and BMP; write JPEG, PNG, WebP, AVIF and TIFF. `--quality`, `--max-size` and `--strip-metadata`, with colour profiles converted to sRGB first. A 500-megapixel limit is checked before decoding.
- **Audio and video**, through a system FFmpeg 7 or later run as a separate program: MOV, MP4, MKV, WebM and AVI in; MP4, WebM, MOV, GIF, MP3, AAC/M4A, WAV, FLAC and Opus out. Streams are copied when the target takes them; `--preset compatible` and `--preset small` re-encode. Progress is shown on a terminal. FFmpeg may only open local files.
- **PDF**, built in: images to PDF (JPEGs go in untouched, without their metadata), PDF pages to PNG or JPEG at 150 dpi, merge, and split by page range.
- **Archives**: extract ZIP, TAR (plain, gz, bz2, xz, zst), 7z and RAR/RAR5 (RAR through libarchive's own reader); create ZIP and tar.gz. An archive with an unsafe path fails as a whole, and nothing is written.
- Output goes next to each source, or under `--out <dir>`. Nothing is overwritten unless `--overwrite` is given; a clash becomes `name (1).ext`. Every file is written to a temporary name and renamed into place, so a failed or cancelled job leaves nothing behind.
- Ctrl-C, SIGTERM and closing the terminal cancel cleanly.
- Every error names the file, the reason and, where there is one, the fix.

### Exit codes

Exit codes are stable from this release, and each code keeps its meaning for good. The table is in the README. Exit 10 is retired and never reused. A cancelled batch exits 130, even when an earlier input failed.

### Known limitations

- Jobs run one at a time.
- The Windows build has no image conversions and no RAR extraction.
- The release binaries are unsigned. On macOS, the README gives the one-line quarantine workaround.
- The macOS and Linux release binaries link the system's libvips and libarchive, so those must be installed for `prsm` to start. FFmpeg is needed only for audio and video.
