# Changelog

Every release of Persimmon and what changed in it. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

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
