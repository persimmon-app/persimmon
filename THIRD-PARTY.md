# Third-party components

Every library, codec and tool that Persimmon bundles or links is listed here with its licence and how it is used. A dependency is added to this file in the same commit that adds it. Retrofitting a licence inventory across a bundled FFmpeg, image codecs and libarchive is far harder than keeping one.

Persimmon is GPL-3.0-or-later. Every entry must be compatible with that.

## Bundled or linked

| Component | Version | Licence | Used for | Linked how |
|---|---|---|---|---|
| [clap](https://github.com/clap-rs/clap) | 4 | MIT OR Apache-2.0 | `prsm` argument parsing | Statically linked (Rust crate) |
| [unicode-normalization](https://github.com/unicode-rs/unicode-normalization) | 0.1 | (MIT OR Apache-2.0) AND Unicode-3.0 | `persimmon-core`: comparing output names the way APFS and NTFS do, so `café` in NFC and NFD forms can't overwrite each other | Statically linked (Rust crate) |
| [ctrlc](https://github.com/Detegr/rust-ctrlc) | 3, `termination` feature | MIT OR Apache-2.0 | `prsm`: Ctrl-C, SIGTERM and SIGHUP cancel cleanly, leaving no partial files | Statically linked (Rust crate) |
| [rs-vips](https://github.com/mrdkprj/rs-vips) | 0.7 | MIT | `persimmon-core`: Rust bindings to libvips, the image backend | Statically linked (Rust crate) |
| [image](https://github.com/image-rs/image) | 0.25, `bmp`, `png` and `jpeg` features | MIT OR Apache-2.0 | `persimmon-core`: decoding BMP, which libvips can only read through ImageMagick; writing rendered PDF pages as PNG and JPEG | Statically linked (Rust crate) |
| [hayro](https://github.com/LaurenzV/hayro) | 0.9, with its `hayro-*` crates | Apache-2.0 OR MIT | `persimmon-core`: reading PDFs and rendering their pages | Statically linked (Rust crate) |
| [lopdf](https://github.com/J-F-Liu/lopdf) | 0.47, no default features | MIT | `persimmon-core`: building PDFs from images, merging and splitting them | Statically linked (Rust crate) |
| Foxit standard fonts, from [PDFium](https://pdfium.googlesource.com/pdfium/) | — | BSD-3-Clause | Stand-ins for the 14 standard PDF fonts when a PDF doesn't embed them | Compiled in, as data inside `hayro-font` |
| [Adobe CMap resources](https://github.com/adobe-type-tools/cmap-resources) | — | BSD-3-Clause | Character maps for Chinese, Japanese and Korean text in PDFs | Compiled in, as data inside `hayro-cmap` |
| [zip](https://github.com/zip-rs/zip2) | 9.5, Deflate, Deflate64, bzip2, LZMA and XZ only | MIT | `persimmon-core`: reading and writing ZIP archives | Statically linked (Rust crate) |
| [tar](https://github.com/alexcrichton/tar-rs) | 0.4 | MIT OR Apache-2.0 | `persimmon-core`: reading and writing tar archives | Statically linked (Rust crate) |
| [flate2](https://github.com/rust-lang/flate2-rs) with [zlib-rs](https://github.com/trifectatechfoundation/zlib-rs) | 1, `zlib-rs` backend | MIT OR Apache-2.0; zlib-rs is Zlib | `persimmon-core`: gzip and Deflate, for `.tar.gz` and ZIP | Statically linked (Rust crate) |
| [bzip2](https://github.com/trifectatechfoundation/bzip2-rs) with [libbz2-rs-sys](https://github.com/trifectatechfoundation/libbzip2-rs) | 0.6 | MIT OR Apache-2.0; libbz2-rs-sys is bzip2-1.0.6 | `persimmon-core`: bzip2, for `.tar.bz2` and bzip2 in ZIP | Statically linked (Rust crate) |
| [lzma-rust2](https://github.com/hasenbanck/lzma-rust2) | 0.25 | Apache-2.0 | `persimmon-core`: XZ and LZMA, for `.tar.xz` and ZIP | Statically linked (Rust crate) |
| [ruzstd](https://github.com/KillingSpark/zstd-rs) | 0.9 | MIT | `persimmon-core`: Zstandard, for `.tar.zst` | Statically linked (Rust crate) |
| [sevenz-rust2](https://github.com/hasenbanck/sevenz-rust2) with [ppmd-rust](https://github.com/hasenbanck/ppmd-rust) | 0.25, bzip2, PPMd and Deflate | Apache-2.0; ppmd-rust is CC0-1.0 OR MIT-0 | `persimmon-core`: reading 7z archives | Statically linked (Rust crate) |
| [compress-tools](https://github.com/OSSystems/compress-tools-rs) | 0.16, behind the default `rar` feature | MIT OR Apache-2.0 | `persimmon-core`: Rust bindings to libarchive, for reading RAR | Statically linked (Rust crate) |
| [libarchive](https://www.libarchive.org/) | 3.6 or later | BSD-2-Clause | `persimmon-core`: reading RAR and RAR5, with libarchive's own reader, never unRAR | Dynamically linked; installed separately (Homebrew `libarchive`, apt `libarchive-dev`) until the app bundles it |
| [libvips](https://www.libvips.org/) | 8.15 or later | LGPL-2.1-or-later | `persimmon-core`: reading, converting and writing images | Dynamically linked; installed separately (Homebrew `vips`, apt `libvips-dev`) until the app bundles it |
| [GLib](https://gitlab.gnome.org/GNOME/glib) | 2 | LGPL-2.1-or-later | Required by libvips | Dynamically linked |

**Codec libraries that libvips loads** for the supported image formats. They're dynamically linked, and each is compatible with GPL-3.0-or-later:

- **JPEG:** libjpeg-turbo (IJG, BSD-3-Clause and zlib). **PNG:** libpng (libpng licence) or spng (BSD-2-Clause). **WebP:** libwebp and sharpyuv (BSD-3-Clause). **TIFF:** libtiff (libtiff licence). **GIF:** libvips's own reader and cgif (MIT).
- **HEIC and AVIF:** libheif and libde265 (LGPL-3.0), dav1d (BSD-2-Clause), and aom (BSD-2-Clause plus the AOMedia patent licence). x265 (GPL-2.0-or-later) is the HEIC *encoder*; Persimmon never writes HEIC, so a bundled libheif can leave it out.
- **Colour and metadata:** lcms2 (MIT), libexif (LGPL-2.1-or-later), highway (Apache-2.0 or BSD-3-Clause).

Persimmon doesn't use libvips's ImageMagick, poppler or libimagequant modules, and a bundled build leaves them out.

**The macOS app** (`persimmon-app`, not yet released) adds these:

| Component | Version | Licence | Used for | Linked how |
|---|---|---|---|---|
| [Tauri](https://tauri.app/) (`tauri`, `tauri-build`, `@tauri-apps/api`, `@tauri-apps/cli`) | 2.12 | Apache-2.0 OR MIT | The app's window, its web view (the system WKWebView) and the bridge to the engine | Statically linked (Rust crates); the API package is bundled into the app's UI; the CLI is a build tool only |
| [Svelte](https://svelte.dev/) | 5 | MIT | The app's interface | Compiled into the app's UI |
| [Inter](https://rsms.me/inter/), from [Fontsource](https://fontsource.org/) | 5.3 (variable) | OFL-1.1 | The interface typeface | Bundled font files |
| [JetBrains Mono](https://www.jetbrains.com/lp/mono/), from [Fontsource](https://fontsource.org/) | 5.3 (variable) | OFL-1.1 | File names, sizes, formats and commands | Bundled font files |

Build tools that never ship in the app: Vite (MIT), TypeScript (Apache-2.0), svelte-check (MIT). Every npm package is checked by `crates/persimmon-app/ui/scripts/check-licences.mjs`, the counterpart of `deny.toml`.

## Run as a separate program

| Component | Version | Licence | Used for | How |
|---|---|---|---|---|
| [FFmpeg](https://ffmpeg.org/) and ffprobe | 7.0 or later | LGPL-2.1-or-later, or GPL-2.0-or-later / GPL-3.0-or-later in builds with x264 and x265, as most are | Audio and video: probing, converting, compressing | Never linked or bundled in v0.1. `prsm` runs the `ffmpeg` the user installed as a child process. Bundling a GPL build arrives with the app (DD-04) |

`persimmon-corpus` is a test-only crate and is never shipped. Its audio and video samples are made at test time by the test machine's FFmpeg.

Transitive Rust dependencies (clap's own dependencies, for example) aren't listed one by one. Instead, `cargo deny check licenses` runs in CI on every push and fails the build if any dependency carries a licence not on the list in `deny.toml`. The full inventory, with every crate compiled into `prsm` and its licence text, is generated for each release by [cargo-about](https://github.com/EmbarkStudios/cargo-about) (MIT OR Apache-2.0, a build tool, not shipped) and included in every release archive as `THIRD-PARTY-LICENSES.html`. Its configuration is `about.toml`, whose accepted licences mirror `deny.toml`.

## Never

| Component | Why not |
|---|---|
| unRAR (RARLAB) | Its licence forbids using the code to re-create the RAR compression algorithm, which is a restriction GPL-3.0 does not allow. RAR and RAR5 extraction comes from libarchive's own BSD-licensed reader instead. |
