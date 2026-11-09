# Image samples

Small real images for testing the image backend. `persimmon-corpus` embeds them and writes them into every generated corpus under `images/`. They were made once, on macOS 26.6, and are committed so that tests on every platform use identical bytes.

All of them are original and synthetic, made for this repository. Like the rest of the repository, they are GPL-3.0-or-later.

## How they were made

The base image is a 320×240 synthetic photo:
- an 80×80 patch of red (220, 30, 30) in the top-left corner;
- horizontal and vertical gradients;
- hard-edged squares;
- mild deterministic noise.

| File | Made with | What it tests |
|---|---|---|
| `photo.jpg` | `sips -s format jpeg -s formatOptions 85` | JPEG input |
| `photo.heic` | `sips -s format heic` | HEIC input |
| `photo.tif` | `sips -s format tiff` | TIFF input |
| `photo.bmp` | `sips -s format bmp` | BMP input |
| `photo.webp` | `cwebp -q 90` | WebP input |
| `photo.avif` | `avifenc -q 80 -s 6` | AVIF input |
| `cmyk.jpg` | `sips --matchTo "Generic CMYK Profile.icc"`; 4-component JPEG | CMYK colour conversion |
| `p3.jpg` | `sips --matchTo "Display P3.icc"` | Colour-profile handling |
| `orient-6.jpg` | 300×200 quadrants (top-left red, top-right green, bottom-left blue, bottom-right white), saved by `sips`, then its EXIF segment replaced by one saying Orientation = 6 | EXIF rotation |
| `deep-16.png` | A 16-bit RGB gradient | 16-bit input |
| `alpha.png` | 400×300, left half transparent, right half opaque red | Transparency |
| `anim.gif` | Three 100×100 frames: red, green, blue | First-frame extraction |
| `truncated.jpg` | The first half of `photo.jpg` | Rejecting truncated files |

PNG and GIF input are covered by the corpus's 1×1 files.

## Audio and video

Audio and video samples are not committed. `persimmon_corpus::media` makes them at test time with FFmpeg's own test sources (`testsrc2` and `sine`), using the `ffmpeg` on `PATH`. Without one, the tests that need them skip, unless `PERSIMMON_REQUIRE_FFMPEG=1` is set, as it is in CI.

## PDFs

PDF samples are not committed either. `persimmon_corpus::pdf` writes them at test time with lopdf: plain and inherited-attribute pages, mixed page sizes with rotation and a crop box, object streams, a scanned page, links with an outline and a form field, 120 pages, a 200-inch page, an owner-password and a password-locked file, and broken ones. The same id always gives the same bytes.

## Archives

Archive samples are not committed either. `persimmon_corpus::archives` writes them at test time: a standard tree (a text file, an executable script, a subfolder, an NFD name, an empty folder and a symlink) as ZIP, TAR, TAR.GZ, TAR.BZ2, TAR.XZ, TAR.ZST and 7z, plus hostile and odd ones: a `../` entry, an absolute path, a write through a symlink, hard links inside and outside, a case clash, a repeated name, password-protected ZIP and 7z, a gzip that isn't a tar, and 3 MiB of noise for progress. Fixed times, no owners and fixed salts make the same id give the same bytes.

## RAR

Nothing free writes RAR, so four small RAR samples are committed. They come unchanged (uudecoded) from libarchive 3.8.1's test suite, `libarchive/test/` in https://github.com/libarchive/libarchive, under libarchive's BSD-2-Clause licence:

| File | From | What it tests |
|---|---|---|
| `rar4.rar` | `test_read_format_rar.rar.uu` | RAR4: files, a folder, an empty folder and a symlink |
| `rar5.rar` | `test_read_format_rar5_stored.rar.uu` | RAR5, stored |
| `rar5-unicode.rar` | `test_read_format_rar5_unicode.rar.uu` | RAR5 with emoji names, a hard link and a symlink |
| `rar4-locked.rar` | `test_read_format_rar4_encrypted.rar.uu` | RAR4 with encrypted files |
