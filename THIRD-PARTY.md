# Third-party components

Every library, codec and tool that Persimmon bundles or links is listed here with its licence and how it is used. A dependency is added to this file in the same commit that adds it. Retrofitting a licence inventory across a bundled FFmpeg, image codecs and libarchive is far harder than keeping one.

Persimmon is GPL-3.0-or-later. Every entry must be compatible with that.

## Bundled or linked

| Component | Version | Licence | Used for | Linked how |
|---|---|---|---|---|
| [clap](https://github.com/clap-rs/clap) | 4 | MIT OR Apache-2.0 | `prsm` argument parsing | Statically linked (Rust crate) |
| [unicode-normalization](https://github.com/unicode-rs/unicode-normalization) | 0.1 | (MIT OR Apache-2.0) AND Unicode-3.0 | `persimmon-core`: comparing output names the way APFS and NTFS do, so `café` in NFC and NFD forms can't overwrite each other | Statically linked (Rust crate) |
| [rs-vips](https://github.com/mrdkprj/rs-vips) | 0.7 | MIT | `persimmon-core`: Rust bindings to libvips, the image backend | Statically linked (Rust crate) |
| [image](https://github.com/image-rs/image) | 0.25, `bmp` feature only | MIT OR Apache-2.0 | `persimmon-core`: decoding BMP, which libvips can only read through ImageMagick | Statically linked (Rust crate) |
| [libvips](https://www.libvips.org/) | 8.15 or later | LGPL-2.1-or-later | `persimmon-core`: reading, converting and writing images | Dynamically linked; installed separately (Homebrew `vips`, apt `libvips-dev`) until the app bundles it |
| [GLib](https://gitlab.gnome.org/GNOME/glib) | 2 | LGPL-2.1-or-later | Required by libvips | Dynamically linked |

**Codec libraries that libvips loads** for the supported image formats. They're dynamically linked, and each is compatible with GPL-3.0-or-later:

- **JPEG:** libjpeg-turbo (IJG, BSD-3-Clause and zlib). **PNG:** libpng (libpng licence) or spng (BSD-2-Clause). **WebP:** libwebp and sharpyuv (BSD-3-Clause). **TIFF:** libtiff (libtiff licence). **GIF:** libvips's own reader and cgif (MIT).
- **HEIC and AVIF:** libheif and libde265 (LGPL-3.0), dav1d (BSD-2-Clause), and aom (BSD-2-Clause plus the AOMedia patent licence). x265 (GPL-2.0-or-later) is the HEIC *encoder*; Persimmon never writes HEIC, so a bundled libheif can leave it out.
- **Colour and metadata:** lcms2 (MIT), libexif (LGPL-2.1-or-later), highway (Apache-2.0 or BSD-3-Clause).

Persimmon doesn't use libvips's ImageMagick, poppler or libimagequant modules, and a bundled build leaves them out.

`persimmon-corpus` is a test-only crate and is never shipped.

Transitive Rust dependencies (clap's own dependencies, for example) aren't listed one by one. Instead, `cargo deny check licenses` runs in CI on every push and fails the build if any dependency carries a licence not on the list in `deny.toml`. The full inventory, with every transitive crate and its licence, is generated for each release.

## Never

| Component | Why not |
|---|---|
| unRAR (RARLAB) | Its licence forbids using the code to re-create the RAR compression algorithm, which is a restriction GPL-3.0 does not allow. RAR and RAR5 extraction comes from libarchive's own BSD-licensed reader instead. |
