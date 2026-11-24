# Changelog

Every release of Persimmon and what changed in it. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Folders as inputs to `prsm convert` and `prsm compress`: a folder stands for the files directly inside it, and `--recursive` adds its subfolders. Files that can't become the target are skipped and counted on a last line, such as `3 files skipped: can't become WebP`.
- A folder dropped on the app adds the files in it.
- `--jobs <n>` and `--jobs auto` on `convert`, `compress` and `extract` work on several files at once. Results still print in the order you gave the files. The default is still one at a time.
- The app converts several files at once, up to one per performance core.
- `prsm watch <folder> --to <format>` converts each file that lands in a folder, until Ctrl-C, which exits 0. A file converts once it has stopped growing for 2 seconds, and partial downloads are never touched. Results go in `converted` inside the folder unless `--out` says otherwise, and that folder is never watched. `--then-move <dir>` moves each original there once its result is safely written. A failed file isn't tried again until it changes, and a folder that goes away is waited for. The other flags are `--match`, `--recursive`, `--existing` and `--no-wait`.
- Watch folders in the app, under Automation in the sidebar. Each rule says which files to match, what to convert them to (with a preset if you like), where results go, and whether to move each original to a done folder after it converts. Rules keep working while the window is closed. Files that arrived while Persimmon wasn't running are converted when it starts, and no file is converted twice. A rule reads its folder when you save it, so macOS asks for access then. Each rule shows what it's doing, its last failure and its `prsm watch` command, which copies with one click.
- Start at login: Persimmon opens with no window and its watch rules run. It's in Settings → General and at the foot of the Watch folders view.
- Notifications for watch folders: one summary when a rule's files stop arriving, such as "~/Inbox — 23 of 24 converted, 1 failed", with Show files and Undo. "Notify on every file" adds one per file. Persimmon asks for permission when you save your first rule, and when notifications are off, History still has every result.
- Watch batches in History, tagged WATCH. Undoing one moves its results to the Trash and puts each moved original back where it was, unless something else is there now or the original changed, and it names each file it left. Undo Last Batch (⌥⌘Z) never undoes a watch batch: it means the newest batch you started.
- Quitting with active watch rules asks first: "Watch folders stop while Persimmon is closed. Quit anyway?"

### Added: in the engine (`persimmon-core`)

- `pool`: `Pool`, `Limits`, `Lane`, `Priority` and `Jobs`, the bounded pool that decides how many tasks run at once.
- `execute_in` runs a plan's tasks in parallel within a pool and still reports them in task order.
- `Claims` and `Job::plan_claimed`, so jobs planned apart never take the same output name.
- `Pool::want`, so a front end's queued work keeps background work from taking a free slot ahead of it.
- `watch`: `WatchSpec`, `Watcher`, `Change`, `Patterns`, `Timing` and `Stamp`. The watcher turns file events into settled, matched files, each converted once.
- `original`: `move_original`, `restore`, `Moved` and `NotRestored`, to move an original after success and move it back on undo.
- `watch::files_in`: the files a watcher would pick up from a folder now, with the same filters.

### Changed (engine, breaking)

- `JobOptions::recursive` is a new field.
- `FixKind::IncludeSubfolders` and `FixKind::ChooseDoneFolder` are new variants.
- `Job::plan` expands folders for convert and compress.
- `Plan::skipped` is a new field.

### Known limitations

- Video and audio still run one at a time.

## [0.2.0] - 2026-11-16

The first release of the macOS app, and `prsm` 0.2.0. This is an alpha: its purpose is to find bugs, so please report what you run into.

### Added: Persimmon.app for macOS

- A DMG for Apple silicon Macs with macOS 14 or later, `Persimmon-0.2.0-arm64.dmg`, with FFmpeg, libvips and libarchive inside the app. Nothing else needs installing.
- **Modes:** Convert, Compress, Extract (archives), and Combine (images and PDFs into one PDF, or files into a ZIP or tar.gz, in an order you set).
- **Ways in:** drop files on the window, on the Dock icon, or on the floating drop window (⌥⌘D, on every Space); or use Open With. Persimmon is the default only for types no other app on your Mac opens.
- **The queue:** each file shows its source and target format, progress, and a reason and a fix in the app's own words when it fails. Each row offers Cancel, Retry or Reveal in Finder as its state allows.
- **Presets:** Web image, Email video and Archive quality, plus your own, each showing the `prsm` command it amounts to. ⌘1–⌘9 pick them.
- **History:** 30 days of batches, searchable, with Reveal, Retry and undo. Undo moves a batch's outputs to the Trash and leaves alone any output changed since.
- **Settings:** General, Output (beside the original, or a folder of your choice, with a fallback folder for originals that can't be written beside, such as the Photos library or Mail), Presets, Engines, Privacy, and About & licences, which links to the sources of everything bundled.
- Light and dark appearance, keyboard operation and VoiceOver labels.
- No network code, as in `prsm`: no update check, no telemetry, no account.

### Added: in the engine (`persimmon-core`)

- An optional `serde` feature for the types the app sends to its window.
- Fixes carry a kind (`FixKind`), so a front end can word them its own way.
- `Backends::with_ffmpeg` to use a given FFmpeg, and `Backends::ffmpeg_info` to report which one.
- `PlanMode::Preview`, planning that creates no folders and writes nothing.
- An optional fallback folder for sources whose own folder can't be written.

### Changed: in the engine (`persimmon-core`), breaking for code that uses it

- `Error::fix` is now an `Option<Fix>` (a kind and the text) instead of an `Option<String>`. `Error::fix_text()` gives the text, and `Error::with_fix` still takes a string.
- `OutputPolicy` has a new public field, `OutputPolicy::fallback`, so code that builds it field by field needs to add it (or use `..Default::default()`).

### Fixed

- A dry run now reports an `--out` that is an existing file.

### Changed: in `prsm`

- Nothing in `prsm`'s commands, flags, output or exit codes, apart from the fix above.

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
