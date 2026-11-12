// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The libvips image backend (DD-34).
//!
//! Every input opens through a [`VipsSource`] and the loader for its
//! detected format, never through a file name or libvips's own format
//! guessing: libvips would read `[…]` in a name as load options, and its
//! guessing falls back to ImageMagick for files its own loaders refuse.
//! Every output is written by the saver for its format, because the
//! executor's temp files end in `.part`. BMP is decoded here, since
//! libvips has no BMP loader of its own.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::OnceLock;

use image::ImageDecoder;
use image::codecs::bmp::BmpDecoder;
use rs_vips::enums::{BandFormat, Interpretation};
use rs_vips::voption::{Setter, VOption};
use rs_vips::{Vips, VipsImage, VipsSource};

use crate::backend::{Done, Run};
use crate::error::{Error, ErrorCode, FixKind};
use crate::exec::CancelToken;
use crate::format::FormatId;
use crate::job::{JobOptions, PlannedTask, Quality};

/// The default pixel limit: 500 megapixels.
const MAX_PIXELS: u64 = 500_000_000;

/// The widest or tallest image WebP can hold.
const WEBP_MAX: i32 = 16383;

/// The saver settings for one `--quality` level.
struct Q {
    jpeg: i32,
    webp: i32,
    avif: i32,
}

fn quality(q: Quality) -> Q {
    match q {
        Quality::Low => Q {
            jpeg: q.jpeg().into(),
            webp: 60,
            avif: 40,
        },
        Quality::Medium => Q {
            jpeg: q.jpeg().into(),
            webp: 75,
            avif: 50,
        },
        Quality::High => Q {
            jpeg: q.jpeg().into(),
            webp: 85,
            avif: 65,
        },
        Quality::Max => Q {
            jpeg: q.jpeg().into(),
            webp: 95,
            avif: 80,
        },
    }
}

/// A limit in megapixels: a whole number when it is one, else up to two
/// decimals without trailing zeros (`1.5`, `0.06`).
pub(crate) fn megapixels(pixels: u64) -> String {
    if pixels.is_multiple_of(1_000_000) {
        return format!("{}", pixels / 1_000_000);
    }
    let s = format!("{:.2}", pixels as f64 / 1_000_000.0);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The useful part of a libvips failure: the first line libvips itself
/// wrote, without the bindings' wrapper text or any temp-file path.
fn message(e: &dyn std::fmt::Display) -> String {
    let text = e.to_string();
    // rs-vips formats errors as "vips error: <Kind> - <its own sentence>. <libvips's buffer>".
    let body = text
        .split_once(" - ")
        .map_or(text.as_str(), |(_, rest)| rest);
    let libvips = body.split_once(". ").map_or(body, |(_, rest)| rest);
    let first = libvips
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(body.trim());
    let cleaned: Vec<&str> = first
        .split_whitespace()
        .filter(|w| !w.contains(".part"))
        .collect();
    let mut short: String = cleaned.join(" ").chars().take(160).collect();
    if short.len() < cleaned.join(" ").len() {
        short.push('…');
    }
    short
}

/// Whether a libvips version string (`8.15.1`) is new enough: 8.15 added
/// the `keep` option that `--strip-metadata` relies on.
fn version_supported(version: &str) -> bool {
    let mut parts = version.split('.').map(|p| p.trim().parse::<u32>());
    match (parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor))) => (major, minor) >= (8, 15),
        _ => false,
    }
}

/// Start libvips once. An error means image tasks can't run in this process.
fn init() -> Result<(), String> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(|| {
        Vips::init("persimmon")
            .map_err(|e| format!("libvips failed to start ({})", message(&e)))?;
        let version = Vips::version_string().unwrap_or_default();
        if version_supported(&version) {
            Ok(())
        } else {
            Err(format!(
                "libvips {version} is too old; 8.15 or later is needed"
            ))
        }
    })
    .clone()
}

/// Open `input` with the libvips loader for `from`, never by guessing.
fn load(input: &Path, from: FormatId, access: &str) -> Result<VipsImage, rs_vips::error::Error> {
    load_page(input, from, access, 0)
}

/// [`load`] page `page` of a multi-page TIFF; other formats ignore it.
fn load_page(
    input: &Path,
    from: FormatId,
    access: &str,
    page: u32,
) -> Result<VipsImage, rs_vips::error::Error> {
    let src = VipsSource::new_from_file(input)?;
    let opts = || {
        VOption::new()
            .set("access", access)
            .set("fail_on", "truncated")
    };
    match from {
        FormatId::Jpeg => VipsImage::jpegload_source_with_opts(&src, opts()),
        FormatId::Png => VipsImage::pngload_source_with_opts(&src, opts()),
        FormatId::Webp => VipsImage::webpload_source_with_opts(&src, opts()),
        FormatId::Heic | FormatId::Avif => VipsImage::heifload_source_with_opts(&src, opts()),
        FormatId::Tiff => {
            VipsImage::tiffload_source_with_opts(&src, opts().set("page", page as i32))
        }
        FormatId::Gif => VipsImage::gifload_source_with_opts(&src, opts()),
        // Every other format is routed to another backend by the planner.
        _ => VipsImage::new_from_source_with_opts(&src, "", opts()),
    }
}

/// Converts and compresses the DD-28 image formats through libvips.
#[derive(Debug, Clone)]
pub struct ImageBackend {
    max_pixels: u64,
}

impl Default for ImageBackend {
    fn default() -> Self {
        ImageBackend::new()
    }
}

impl ImageBackend {
    pub fn new() -> ImageBackend {
        ImageBackend::with_max_pixels(MAX_PIXELS)
    }

    /// A backend that refuses images over `max_pixels` before decoding them.
    pub fn with_max_pixels(max_pixels: u64) -> ImageBackend {
        let _ = init();
        ImageBackend { max_pixels }
    }

    fn too_large(&self, input: &Path, w: u64, h: u64) -> Result<(), Error> {
        if w * h > self.max_pixels {
            return Err(Error::new(
                ErrorCode::UnsupportedInput,
                format!(
                    "image is too large ({w}×{h}, over {} megapixels)",
                    megapixels(self.max_pixels)
                ),
            )
            .with_file(input));
        }
        Ok(())
    }

    /// Whether the input decodes completely. libvips decodes lazily, so a
    /// truncated file only fails once the saver pulls its pixels; after a
    /// failed save this tells a bad input from a failed write.
    fn input_decodes(&self, input: &Path, from: FormatId) -> bool {
        if from == FormatId::Bmp {
            // Already decoded in full by `open`.
            return true;
        }
        let ok = load(input, from, "random")
            .and_then(|img| img.avg())
            .is_ok();
        Vips::error_clear();
        ok
    }

    /// Open the input with the pixel limit checked before any decoding.
    fn open(&self, input: &Path, from: FormatId) -> Result<VipsImage, Error> {
        self.open_page(input, from, 0)
    }

    /// [`open`](Self::open) page `page` of a multi-page TIFF.
    fn open_page(&self, input: &Path, from: FormatId, page: u32) -> Result<VipsImage, Error> {
        let corrupt = |e: &dyn std::fmt::Display| {
            Error::new(
                ErrorCode::CorruptInput,
                format!("can't read this image ({})", message(e)),
            )
            .with_file(input)
        };
        if from == FormatId::Bmp {
            let file = File::open(input).map_err(|e| corrupt(&e))?;
            let decoder = BmpDecoder::new(BufReader::new(file)).map_err(|e| corrupt(&e))?;
            let (w, h) = decoder.dimensions();
            self.too_large(input, w.into(), h.into())?;
            let img = image::DynamicImage::from_decoder(decoder).map_err(|e| corrupt(&e))?;
            let (buf, bands) = if img.color().has_alpha() {
                (img.into_rgba8().into_raw(), 4)
            } else {
                (img.into_rgb8().into_raw(), 3)
            };
            return VipsImage::new_from_memory_copy(
                &buf,
                w as i32,
                h as i32,
                bands,
                BandFormat::Uchar,
            )
            .map_err(|e| corrupt(&e));
        }
        let img = load_page(input, from, "sequential", page).map_err(|e| corrupt(&e))?;
        self.too_large(input, img.get_width() as u64, img.get_height() as u64)?;
        if img.get_orientation() > 1 {
            // Rotating needs random access to the pixels.
            return load_page(input, from, "random", page)
                .and_then(|img| img.autorot())
                .map_err(|e| corrupt(&e));
        }
        Ok(img)
    }
}

/// An image decoded for embedding in a PDF: 8-bit sRGB or gray, alpha
/// last, upright.
pub(crate) struct Decoded {
    pub width: u32,
    pub height: u32,
    /// 1 (gray), 2 (gray and alpha), 3 (RGB) or 4 (RGB and alpha).
    pub bands: u8,
    pub pixels: Vec<u8>,
    /// The recorded resolution, when it's a plausible one (50–4,800 dpi).
    pub dpi: Option<f32>,
}

/// Decode `input` for a PDF page, as `convert` opens it: the pixel limit is
/// checked first, EXIF rotation applied, the long edge shrunk to
/// `max_size`, colour profiles and CMYK converted to sRGB, and 16-bit
/// samples reduced to 8.
pub(crate) fn decode_srgb8(
    input: &Path,
    from: FormatId,
    max_size: Option<u32>,
    page: u32,
) -> Result<Decoded, Error> {
    started(input)?;
    Vips::error_clear();
    let result = decode(input, from, max_size, page);
    Vips::error_clear();
    result
}

/// libvips started, or `BACKEND_MISSING` naming `input`.
fn started(input: &Path) -> Result<(), Error> {
    init().map_err(|reason| {
        Error::new(ErrorCode::BackendMissing, reason)
            .with_file(input)
            .with_fix_as(
                FixKind::InstallLibvips,
                "install or update libvips (brew upgrade vips, or a newer libvips-dev)",
            )
    })
}

/// How many pages the TIFF at `input` holds: each becomes a PDF page.
pub(crate) fn tiff_pages(input: &Path) -> Result<u32, Error> {
    started(input)?;
    let img = load(input, FormatId::Tiff, "sequential").map_err(|e| {
        Error::new(
            ErrorCode::CorruptInput,
            format!("can't read this image ({})", message(&e)),
        )
        .with_file(input)
    });
    Vips::error_clear();
    Ok(img?.get_n_pages().max(1) as u32)
}

fn decode(
    input: &Path,
    from: FormatId,
    max_size: Option<u32>,
    page: u32,
) -> Result<Decoded, Error> {
    let fail = |e: &dyn std::fmt::Display| {
        Error::new(
            ErrorCode::CorruptInput,
            format!("can't read this image ({})", message(e)),
        )
        .with_file(input)
    };
    let mut img = ImageBackend::new().open_page(input, from, page)?;
    if let Some(max) = max_size
        && img.get_width().max(img.get_height()) > max as i32
    {
        img = img
            .thumbnail_image_with_opts(
                max as i32,
                VOption::new().set("height", max as i32).set("size", "down"),
            )
            .map_err(|e| fail(&e))?;
    }
    if img.get_typeof("icc-profile-data").unwrap_or(0) != 0 {
        img = img
            .icc_transform_with_opts("srgb", VOption::new().set("embedded", true))
            .map_err(|e| fail(&e))?;
    }
    let gray = matches!(
        img.guess_interpretation(),
        Ok(Interpretation::BW | Interpretation::Grey16)
    );
    img = img
        .colourspace(if gray {
            Interpretation::BW
        } else {
            Interpretation::Srgb
        })
        .map_err(|e| fail(&e))?;
    if !matches!(img.get_format(), Ok(BandFormat::Uchar)) {
        img = img.cast(BandFormat::Uchar).map_err(|e| fail(&e))?;
    }
    // Rounded: PNG stores 72 dpi as 2,835 pixels per metre, 72.009 dpi.
    let dpi = Some((img.get_xres() * 25.4).round() as f32).filter(|d| (50.0..=4800.0).contains(d));
    let (width, height, bands) = (
        img.get_width() as u32,
        img.get_height() as u32,
        img.get_bands() as u8,
    );
    // Decoding happens here; a file that fails partway gives fewer bytes.
    let pixels = img.write_to_memory();
    if pixels.len() != width as usize * height as usize * usize::from(bands) {
        let buffer = Vips::error_buffer().unwrap_or_default();
        let reason = buffer
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("it's truncated")
            .to_string();
        return Err(fail(&reason));
    }
    Ok(Decoded {
        width,
        height,
        bands,
        pixels,
        dpi,
    })
}

impl Run for ImageBackend {
    fn run(
        &self,
        task: &PlannedTask,
        options: &JobOptions,
        temp: &Path,
        _cancel: &CancelToken,
        _progress: &mut dyn FnMut(f32),
    ) -> Result<Done, Error> {
        let input = &task.inputs[0];
        init().map_err(|reason| {
            Error::new(ErrorCode::BackendMissing, reason)
                .with_file(input)
                .with_fix_as(
                    FixKind::InstallLibvips,
                    "install or update libvips (brew upgrade vips, or a newer libvips-dev)",
                )
        })?;
        // libvips keeps one error buffer per process; start each task clean.
        Vips::error_clear();
        let result = self.convert(task, options, temp);
        Vips::error_clear();
        result
    }
}

impl ImageBackend {
    fn convert(
        &self,
        task: &PlannedTask,
        options: &JobOptions,
        temp: &Path,
    ) -> Result<Done, Error> {
        let input = &task.inputs[0];
        let from = task.from[0];
        let compress = task.to.is_none();
        let target = task.to.unwrap_or(from);
        let read_fail = |e: &dyn std::fmt::Display| {
            Error::new(
                ErrorCode::CorruptInput,
                format!("can't read this image ({})", message(e)),
            )
            .with_file(input)
        };

        let mut img = self.open(input, from)?;
        if let Some(max) = options.max_size
            && img.get_width().max(img.get_height()) > max as i32
        {
            img = img
                .thumbnail_image_with_opts(
                    max as i32,
                    VOption::new().set("height", max as i32).set("size", "down"),
                )
                .map_err(|e| read_fail(&e))?;
        }
        if options.strip_metadata && img.get_typeof("icc-profile-data").unwrap_or(0) != 0 {
            // Dropping a profile without converting would change the colours.
            img = img
                .icc_transform_with_opts("srgb", VOption::new().set("embedded", true))
                .map_err(|e| read_fail(&e))?;
        }
        if target == FormatId::Jpeg && img.hasalpha() {
            // JPEG has no alpha: flatten onto white, at the image's depth.
            // (A CMYK image also has four bands; it has no alpha.)
            let white = match img.get_format() {
                Ok(BandFormat::Ushort) => 65535.0,
                _ => 255.0,
            };
            let background = vec![white; (img.get_bands() - 1).max(1) as usize];
            img = img
                .flatten_with_opts(VOption::new().set("background", background.as_slice()))
                .map_err(|e| read_fail(&e))?;
        }
        if target == FormatId::Webp && img.get_width().max(img.get_height()) > WEBP_MAX {
            return Err(Error::new(
                ErrorCode::UnsupportedConversion,
                format!(
                    "WebP can't be wider or taller than {WEBP_MAX} pixels (this image is {}×{})",
                    img.get_width(),
                    img.get_height()
                ),
            )
            .with_file(input)
            .with_fix_as(
                FixKind::ShrinkOrPickTarget {
                    max: WEBP_MAX as u32,
                },
                format!("add --max-size {WEBP_MAX}, or convert to png or jpeg"),
            ));
        }

        let q = quality(options.quality.unwrap_or(if compress {
            Quality::Medium
        } else {
            Quality::High
        }));
        let mut opts = VOption::new();
        if options.strip_metadata {
            // `keep` is a flags argument; 0 is VIPS_FOREIGN_KEEP_NONE.
            opts = opts.set("keep", 0);
        }
        let path = temp.to_str().ok_or_else(|| {
            Error::new(
                ErrorCode::OutputNotWritable,
                "the output folder's path isn't valid UTF-8",
            )
            .with_file(temp)
            .with_fix_as(
                FixKind::ChooseOutputFolder,
                "choose another folder with --out <dir>",
            )
        })?;
        let saved = match target {
            FormatId::Jpeg => img.jpegsave_with_opts(path, opts.set("Q", q.jpeg)),
            FormatId::Webp => img.webpsave_with_opts(path, opts.set("Q", q.webp)),
            FormatId::Avif => img.heifsave_with_opts(
                path,
                opts.set("compression", "av1")
                    .set("Q", q.avif)
                    .set("effort", 4),
            ),
            FormatId::Png if compress => img.pngsave_with_opts(path, opts.set("compression", 9)),
            FormatId::Png => img.pngsave_with_opts(path, opts),
            FormatId::Tiff if compress => img.tiffsave_with_opts(
                path,
                opts.set("compression", "deflate")
                    .set("predictor", "horizontal"),
            ),
            FormatId::Tiff => img.tiffsave_with_opts(path, opts.set("compression", "lzw")),
            other => {
                return Err(Error::new(
                    ErrorCode::UnsupportedConversion,
                    format!("the image backend can't write {}", other.as_str()),
                )
                .with_file(input));
            }
        };
        if let Err(e) = saved {
            let msg = message(&e);
            let _ = std::fs::remove_file(temp);
            return Err(if !self.input_decodes(input, from) {
                Error::new(
                    ErrorCode::CorruptInput,
                    format!("can't read this image ({msg})"),
                )
                .with_file(input)
            } else {
                Error::new(
                    ErrorCode::OutputNotWritable,
                    format!("couldn't write the image ({msg})"),
                )
                .with_file(input)
                .with_fix_as(
                    FixKind::ChooseOutputFolder,
                    "choose another folder with --out <dir>",
                )
            });
        }

        if compress {
            let before = std::fs::metadata(input)
                .map(|m| m.len())
                .unwrap_or(u64::MAX);
            let after = std::fs::metadata(temp).map(|m| m.len()).unwrap_or(u64::MAX);
            if after >= before {
                let _ = std::fs::remove_file(temp);
                return Ok(Done::NotSmaller);
            }
        }
        Ok(Done::Written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Done, Run};
    use crate::error::ErrorCode;
    use crate::exec::CancelToken;
    use crate::format::{FormatId, sniff};
    use crate::job::{JobOptions, PlannedTask, Quality};
    use crate::registry::Backend;
    use persimmon_corpus::{Options, generate};
    use rs_vips::VipsImage;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    fn corpus() -> &'static (PathBuf, Vec<persimmon_corpus::Case>) {
        static C: OnceLock<(PathBuf, Vec<persimmon_corpus::Case>)> = OnceLock::new();
        C.get_or_init(|| {
            let dir =
                std::env::temp_dir().join(format!("prsm-image-corpus-{}", std::process::id()));
            let cases = generate(&dir, &Options { large: false }).unwrap();
            (dir, cases)
        })
    }

    fn case(id: &str) -> PathBuf {
        corpus()
            .1
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("no case {id}"))
            .path
            .clone()
    }

    fn from_of(id: &str) -> FormatId {
        crate::detect::detect(&case(id), crate::format::all_formats())
            .unwrap()
            .format
    }

    fn out_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("prsm-image-out-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Run the backend on a corpus case. `to: None` is compress. The temp
    /// path ends in `.part`, like the executor's.
    fn run_with(
        backend: &ImageBackend,
        id: &str,
        to: Option<FormatId>,
        opts: &JobOptions,
    ) -> (Result<Done, crate::error::Error>, PathBuf) {
        let tag = to.map_or("compress", |t| t.as_str());
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temp = out_dir().join(format!(".{id}-{tag}-{n}.part"));
        let _ = std::fs::remove_file(&temp);
        let task = PlannedTask {
            inputs: vec![case(id)],
            from: vec![from_of(id)],
            to,
            backend: Backend::Image,
            output: temp.with_extension("out"),
        };
        (
            backend.run(&task, opts, &temp, &CancelToken::new(), &mut |_| {}),
            temp,
        )
    }

    fn run_to(
        id: &str,
        to: Option<FormatId>,
        opts: &JobOptions,
    ) -> (Result<Done, crate::error::Error>, PathBuf) {
        run_with(&ImageBackend::new(), id, to, opts)
    }

    fn open(p: &Path) -> VipsImage {
        let src = rs_vips::VipsSource::new_from_file(p).unwrap();
        VipsImage::new_from_source(&src, "").unwrap()
    }

    fn dims(p: &Path) -> (i32, i32) {
        let img = open(p);
        (img.get_width(), img.get_height())
    }

    /// A pixel in 8-bit sRGB (plus alpha), whatever the file stores:
    /// a CMYK JPEG or a 16-bit PNG is converted first.
    fn pixel(p: &Path, x: i32, y: i32) -> Vec<f64> {
        open(p)
            .colourspace(rs_vips::enums::Interpretation::Srgb)
            .unwrap()
            .getpoint(x, y)
            .unwrap()
    }

    fn near(px: &[f64], target: [f64; 3], tol: f64) -> bool {
        (0..3).all(|i| (px[i] - target[i]).abs() <= tol)
    }

    fn head(p: &Path) -> Vec<u8> {
        let bytes = std::fs::read(p).unwrap();
        bytes[..bytes.len().min(512)].to_vec()
    }

    const WRITE: [FormatId; 5] = [
        FormatId::Jpeg,
        FormatId::Png,
        FormatId::Webp,
        FormatId::Avif,
        FormatId::Tiff,
    ];

    #[test]
    fn every_image_pair_converts() {
        let opts = JobOptions::default();
        for id in [
            "img-jpeg", "png-1x1", "img-webp", "img-heic", "img-tiff", "gif-1x1", "img-avif",
            "img-bmp",
        ] {
            for to in WRITE {
                let (r, temp) = run_to(id, Some(to), &opts);
                assert_eq!(r, Ok(Done::Written), "{id} → {to:?}");
                assert_eq!(
                    sniff(&head(&temp)),
                    Some(to),
                    "{id} → {to:?} wrote the wrong format"
                );
            }
        }
    }

    #[test]
    fn orientation_is_applied() {
        let (r, temp) = run_to("img-orient-6", Some(FormatId::Png), &JobOptions::default());
        assert_eq!(r, Ok(Done::Written));
        assert_eq!(dims(&temp), (200, 300));
        let p = pixel(&temp, 10, 10);
        assert!(
            near(&p, [30.0, 30.0, 220.0], 40.0),
            "top-left {p:?}, expected blue"
        );
    }

    #[test]
    fn cmyk_keeps_colour() {
        let (_, temp) = run_to("img-cmyk", Some(FormatId::Png), &JobOptions::default());
        let p = pixel(&temp, 20, 20);
        assert!(
            p[0] > 180.0 && p[1] < 90.0 && p[2] < 90.0,
            "red patch {p:?}"
        );
    }

    #[test]
    fn p3_is_converted_when_stripping() {
        let opts = JobOptions {
            strip_metadata: true,
            ..Default::default()
        };
        let (_, temp) = run_to("img-p3", Some(FormatId::Webp), &opts);
        let p = pixel(&temp, 20, 20);
        assert!(near(&p, [220.0, 30.0, 30.0], 12.0), "red patch {p:?}");
        assert_eq!(open(&temp).get_typeof("icc-profile-data").unwrap_or(0), 0);
    }

    #[test]
    fn p3_profile_kept_by_default() {
        let (_, temp) = run_to("img-p3", Some(FormatId::Png), &JobOptions::default());
        assert_ne!(open(&temp).get_typeof("icc-profile-data").unwrap_or(0), 0);
    }

    #[test]
    fn strip_removes_exif() {
        let opts = JobOptions {
            strip_metadata: true,
            ..Default::default()
        };
        let (_, temp) = run_to("img-orient-6", Some(FormatId::Jpeg), &opts);
        assert_eq!(open(&temp).get_typeof("exif-data").unwrap_or(0), 0);
    }

    #[test]
    fn alpha_survives() {
        for to in [
            FormatId::Png,
            FormatId::Webp,
            FormatId::Avif,
            FormatId::Tiff,
        ] {
            let (_, temp) = run_to("img-alpha", Some(to), &JobOptions::default());
            let (l, r) = (pixel(&temp, 50, 150), pixel(&temp, 350, 150));
            assert!(
                l.len() == 4 && l[3] < 16.0 && r[3] > 240.0,
                "{to:?}: left {l:?}, right {r:?}"
            );
        }
    }

    #[test]
    fn first_gif_frame() {
        let (_, temp) = run_to("img-anim", Some(FormatId::Png), &JobOptions::default());
        assert_eq!(dims(&temp), (100, 100));
        let p = pixel(&temp, 50, 50);
        assert!(near(&p, [220.0, 30.0, 30.0], 40.0), "centre {p:?}");
    }

    #[test]
    fn truncated_is_corrupt() {
        let (r, temp) = run_to("img-truncated", Some(FormatId::Png), &JobOptions::default());
        let e = r.unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert!(
            e.reason.starts_with("can't read this image"),
            "{}",
            e.reason
        );
        assert_eq!(e.file.as_deref(), Some(case("img-truncated").as_path()));
        assert!(!temp.exists());
    }

    #[test]
    fn brackets_and_commas_in_names() {
        for id in ["brackets", "comma"] {
            let (r, temp) = run_to(id, Some(FormatId::Webp), &JobOptions::default());
            assert_eq!(r, Ok(Done::Written), "{id}");
            assert_eq!(sniff(&head(&temp)), Some(FormatId::Webp));
        }
    }

    #[test]
    fn pixel_limit_refuses_before_decoding() {
        let (r, temp) = run_with(
            &ImageBackend::with_max_pixels(60_000),
            "img-jpeg",
            Some(FormatId::Png),
            &JobOptions::default(),
        );
        let e = r.unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput);
        assert_eq!(
            e.reason,
            "image is too large (320×240, over 0.06 megapixels)"
        );
        assert!(!temp.exists());
        assert_eq!(ImageBackend::new().max_pixels, 500_000_000);
        assert_eq!(megapixels(500_000_000), "500");
        assert_eq!(megapixels(1_500_000), "1.5");
    }

    #[test]
    fn bmp_respects_the_pixel_limit() {
        let (r, _) = run_with(
            &ImageBackend::with_max_pixels(60_000),
            "img-bmp",
            Some(FormatId::Png),
            &JobOptions::default(),
        );
        assert_eq!(r.unwrap_err().code, ErrorCode::UnsupportedInput);
    }

    #[test]
    fn max_size_downscales_only() {
        let small = JobOptions {
            max_size: Some(100),
            ..Default::default()
        };
        let (_, temp) = run_to("img-jpeg", Some(FormatId::Png), &small);
        assert_eq!(dims(&temp), (100, 75));
        let big = JobOptions {
            max_size: Some(1000),
            ..Default::default()
        };
        let (_, temp) = run_to("img-heic", Some(FormatId::Png), &big);
        assert_eq!(dims(&temp), (320, 240));
    }

    #[test]
    fn compress_not_smaller_writes_nothing() {
        let (r, temp) = run_to("png-1x1", None, &JobOptions::default());
        assert_eq!(r, Ok(Done::NotSmaller));
        assert!(!temp.exists());
    }

    #[test]
    fn compress_shrinks_a_jpeg() {
        let (r, temp) = run_to("img-cmyk", None, &JobOptions::default());
        assert_eq!(r, Ok(Done::Written));
        assert!(
            std::fs::metadata(&temp).unwrap().len()
                < std::fs::metadata(case("img-cmyk")).unwrap().len()
        );
        assert_eq!(sniff(&head(&temp)), Some(FormatId::Jpeg));
    }

    /// Write `img` as PNG into the test output folder and return the path.
    fn save_png(img: &VipsImage, name: &str) -> PathBuf {
        let p = out_dir().join(name);
        img.pngsave_with_opts(p.to_str().unwrap(), rs_vips::voption::VOption::new())
            .unwrap();
        p
    }

    /// Run the backend on an arbitrary file.
    fn run_file(
        path: &Path,
        from: FormatId,
        to: FormatId,
    ) -> (Result<Done, crate::error::Error>, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temp = out_dir().join(format!(".file-{n}.part"));
        let task = PlannedTask {
            inputs: vec![path.to_path_buf()],
            from: vec![from],
            to: Some(to),
            backend: Backend::Image,
            output: temp.with_extension("out"),
        };
        (
            ImageBackend::new().run(
                &task,
                &JobOptions::default(),
                &temp,
                &CancelToken::new(),
                &mut |_| {},
            ),
            temp,
        )
    }

    #[test]
    fn cmyk_to_jpeg_keeps_colour() {
        for (to, label) in [(Some(FormatId::Jpeg), "convert"), (None, "compress")] {
            let (r, temp) = run_to("img-cmyk", to, &JobOptions::default());
            assert_eq!(r, Ok(Done::Written), "{label}");
            let p = pixel(&temp, 20, 20);
            assert!(
                p[0] > 180.0 && p[1] < 90.0 && p[2] < 90.0,
                "{label}: red patch {p:?}"
            );
        }
    }

    #[test]
    fn transparent_16_bit_to_jpeg_is_white() {
        // A fully transparent 16-bit RGBA image.
        let img =
            VipsImage::black_with_opts(8, 8, rs_vips::voption::VOption::new().set("bands", 4))
                .unwrap()
                .cast(rs_vips::enums::BandFormat::Ushort)
                .unwrap()
                .copy_with_opts(rs_vips::voption::VOption::new().set("interpretation", "rgb16"))
                .unwrap();
        let png = save_png(&img, "clear16.png");
        let (r, temp) = run_file(&png, FormatId::Png, FormatId::Jpeg);
        assert_eq!(r, Ok(Done::Written));
        let p = pixel(&temp, 4, 4);
        assert!(p.iter().take(3).all(|v| *v > 250.0), "background {p:?}");
    }

    #[test]
    fn errors_do_not_carry_earlier_files() {
        let backend = ImageBackend::new();
        let opts = JobOptions::default();
        let (first, first_temp) = run_with(&backend, "img-truncated", Some(FormatId::Png), &opts);
        let (second, _) = run_with(&backend, "img-truncated", Some(FormatId::Png), &opts);
        let (a, b) = (first.unwrap_err().reason, second.unwrap_err().reason);
        let first_name = first_temp
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            !b.contains(&first_name),
            "second error mentions the first file: {b}"
        );
        for reason in [&a, &b] {
            assert!(!reason.contains(".part"), "temp path leaked: {reason}");
            assert!(
                !reason.contains("OperationError"),
                "binding noise leaked: {reason}"
            );
        }
    }

    #[test]
    fn webp_size_limit_is_unsupported() {
        let img = VipsImage::black(17000, 8).unwrap();
        let png = save_png(&img, "wide.png");
        let (r, temp) = run_file(&png, FormatId::Png, FormatId::Webp);
        let e = r.unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedConversion);
        assert_eq!(
            e.reason,
            "WebP can't be wider or taller than 16383 pixels (this image is 17000×8)"
        );
        assert_eq!(
            e.fix_text(),
            Some("add --max-size 16383, or convert to png or jpeg")
        );
        assert!(!temp.exists());
    }

    #[test]
    fn imagemagick_is_never_used() {
        // A TIFF cut short: libvips's own TIFF loader refuses it, and
        // without the loader check libvips would fall back to ImageMagick.
        let bytes = std::fs::read(case("img-tiff")).unwrap();
        let bad = out_dir().join("cut.tif");
        std::fs::write(&bad, &bytes[..15000]).unwrap();
        let (r, _) = run_file(&bad, FormatId::Tiff, FormatId::Jpeg);
        let e = r.unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert!(!e.reason.to_lowercase().contains("magick"), "{}", e.reason);
    }

    #[test]
    fn libvips_8_15_or_later_is_required() {
        assert!(version_supported("8.15.0"));
        assert!(version_supported("8.18.7"));
        assert!(version_supported("9.0.1"));
        assert!(!version_supported("8.14.2"));
        assert!(!version_supported("8.12.1"));
        assert!(!version_supported("garbage"));
    }

    #[test]
    fn quality_changes_size() {
        let size = |q| {
            let (_, temp) = run_to(
                "img-jpeg",
                Some(FormatId::Webp),
                &JobOptions {
                    quality: Some(q),
                    ..Default::default()
                },
            );
            let n = std::fs::metadata(&temp).unwrap().len();
            std::fs::remove_file(temp).unwrap();
            n
        };
        assert!(size(Quality::Low) < size(Quality::Max));
    }
}
