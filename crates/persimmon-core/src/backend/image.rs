// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The libvips image backend (DD-34).

//!
//! Every input opens through a [`VipsSource`], never a file name, because
//! libvips would read `[…]` in a name as load options. Every output is
//! written by the saver for its format, never chosen from the file name,
//! because the executor's temp files end in `.part`. BMP is decoded here,
//! not by libvips, which would hand it to ImageMagick.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Once;

use image::ImageDecoder;
use image::codecs::bmp::BmpDecoder;
use rs_vips::enums::BandFormat;
use rs_vips::voption::{Setter, VOption};
use rs_vips::{Vips, VipsImage, VipsSource};

use crate::backend::{Done, Run};
use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;
use crate::format::FormatId;
use crate::job::{JobOptions, PlannedTask, Quality};

/// The default pixel limit: 500 megapixels.
const MAX_PIXELS: u64 = 500_000_000;

/// The saver settings for one `--quality` level (DD-34 plan, decision 5).
struct Q {
    jpeg: i32,
    webp: i32,
    avif: i32,
}

fn quality(q: Quality) -> Q {
    match q {
        Quality::Low => Q {
            jpeg: 60,
            webp: 60,
            avif: 40,
        },
        Quality::Medium => Q {
            jpeg: 75,
            webp: 75,
            avif: 50,
        },
        Quality::High => Q {
            jpeg: 85,
            webp: 85,
            avif: 65,
        },
        Quality::Max => Q {
            jpeg: 95,
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

/// libvips's message on one line, without its trailing noise.
fn message(e: &dyn std::fmt::Display) -> String {
    let text = e.to_string();
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut short: String = joined.chars().take(200).collect();
    if short.len() < joined.len() {
        short.push('…');
    }
    short
}

fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // A failure here surfaces as an error on the first operation.
        let _ = Vips::init("persimmon");
    });
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
        init();
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
    /// failed save this tells a bad input from a failed write, without
    /// reading libvips's error text, which is shared between threads.
    fn input_decodes(&self, input: &Path, from: FormatId) -> bool {
        if from == FormatId::Bmp {
            // Already decoded in full by `open`.
            return true;
        }
        VipsSource::new_from_file(input)
            .and_then(|src| {
                VipsImage::new_from_source_with_opts(
                    &src,
                    "",
                    VOption::new().set("fail_on", "truncated"),
                )
            })
            .and_then(|img| img.avg())
            .is_ok()
    }

    /// Open the input with the pixel limit checked before any decoding.
    fn open(&self, input: &Path, from: FormatId) -> Result<VipsImage, Error> {
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
                (img.to_rgba8().into_raw(), 4)
            } else {
                (img.to_rgb8().into_raw(), 3)
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
        let source = |access: &str| -> Result<VipsImage, Error> {
            let src = VipsSource::new_from_file(input).map_err(|e| corrupt(&e))?;
            VipsImage::new_from_source_with_opts(
                &src,
                "",
                VOption::new()
                    .set("access", access)
                    .set("fail_on", "truncated"),
            )
            .map_err(|e| corrupt(&e))
        };
        let img = source("sequential")?;
        self.too_large(input, img.get_width() as u64, img.get_height() as u64)?;
        if img.get_orientation() > 1 {
            // Rotating needs random access to the pixels.
            return source("random")?.autorot().map_err(|e| corrupt(&e));
        }
        Ok(img)
    }
}

impl Run for ImageBackend {
    fn run(
        &self,
        task: &PlannedTask,
        options: &JobOptions,
        temp: &Path,
        _cancel: &CancelToken,
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
        let bands = img.get_bands();
        if target == FormatId::Jpeg && (bands == 2 || bands == 4) {
            // JPEG has no alpha: flatten onto white.
            let white: &[f64] = if bands == 2 {
                &[255.0]
            } else {
                &[255.0, 255.0, 255.0]
            };
            img = img
                .flatten_with_opts(VOption::new().set("background", white))
                .map_err(|e| read_fail(&e))?;
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
            .with_fix("choose another folder with --out <dir>")
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
            let _ = std::fs::remove_file(temp);
            let msg = message(&e);
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
                .with_fix("choose another folder with --out <dir>")
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
        (backend.run(&task, opts, &temp, &CancelToken::new()), temp)
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

    fn pixel(p: &Path, x: i32, y: i32) -> Vec<f64> {
        open(p).getpoint(x, y).unwrap()
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
