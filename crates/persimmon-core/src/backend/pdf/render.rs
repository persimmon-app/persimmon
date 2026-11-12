// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! PDF → PNG and JPEG, one image per page, drawn by hayro (DD-36).
//!
//! Pages render at 150 dpi on white, at their displayed size (crop box and
//! rotation applied). `--max-size` caps the long edge; a hard cap of
//! 16,384 px on the long edge and 100 megapixels per page shrinks huge
//! pages instead of failing. One page is written as a file; several make a
//! folder of numbered files.

use std::fs::File;
use std::io::BufWriter;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::util::TransformExt;
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render_into};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};

use crate::backend::Done;
use crate::error::{Error, ErrorCode, FixKind};
use crate::exec::CancelToken;
use crate::format::FormatId;
use crate::job::{JobOptions, PlannedTask, Quality, select_pages, stem_of};
use crate::registry::Registry;

/// The resolution pages render at.
const DPI: f64 = 150.0;
/// The longest side a rendered page may have.
const MAX_EDGE: f64 = 16384.0;
/// The most pixels a rendered page may have.
const MAX_PIXELS: f64 = 100_000_000.0;

/// The pixel size of a page `page_pt` points across: 150 dpi, shrunk so the
/// long edge is at most `max_size`, 16,384 px and 100 megapixels in all.
/// Each side is floored and at least 1.
pub(crate) fn render_size(page_pt: (f32, f32), max_size: Option<u32>) -> (u32, u32) {
    let (w, h) = (f64::from(page_pt.0).max(1.0), f64::from(page_pt.1).max(1.0));
    let long = w.max(h);
    let mut scale = DPI / 72.0;
    if let Some(max) = max_size {
        scale = scale.min(f64::from(max) / long);
    }
    scale = scale.min(MAX_EDGE / long);
    let area = w * h * scale * scale;
    if area > MAX_PIXELS {
        scale *= (MAX_PIXELS / area).sqrt();
    }
    // The epsilon keeps an exact fit (800 of 800) from flooring to 799.
    let side = |pt: f64| ((pt * scale + 1e-6).floor() as u32).max(1);
    (side(w), side(h))
}

/// The name of page `page` in a folder of renders: `doc 007.png`, padded to
/// the digit count of `total`, the PDF's page count.
pub(crate) fn page_file(stem: &str, page: u32, total: u32, ext: &str) -> String {
    let width = total.to_string().len();
    format!("{stem} {page:0width$}.{ext}")
}

/// Run `f`, turning a panic while drawing page `page` into an error.
fn guard<T>(page: u32, f: impl FnOnce() -> T) -> Result<T, Error> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| {
        Error::new(
            ErrorCode::CorruptInput,
            format!("couldn't draw page {page}"),
        )
    })
}

/// Render the task's pages to `temp`: a file for one page, a folder for
/// several. Cancel is checked after each page.
pub(crate) fn render(
    task: &PlannedTask,
    options: &JobOptions,
    temp: &Path,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(f32),
) -> Result<Done, Error> {
    let input = &task.inputs[0];
    let reg = Registry::builtin();
    let to = task.to.expect("rendering has a target format");
    let ext = reg.format(to).extensions[0];
    let quality = options.quality.unwrap_or(Quality::High).jpeg();

    let pdf = super::open(input)?;
    let pages = pdf.pages();
    let total = pages.len() as u32;
    let selected = select_pages(options.pages.as_ref(), total).map_err(|e| e.with_file(input))?;
    let folder = selected.len() > 1;
    if folder {
        std::fs::create_dir(temp).map_err(|e| not_writable(&task.output, &e))?;
    }
    let stem = stem_of(input, &reg);

    let cache = RenderCache::new();
    let interpreter = InterpreterSettings::default();
    let settings = RenderSettings::default();
    let mut ctx = RenderContext::new(1, 1);
    let mut pixmap = Pixmap::new(1, 1);
    let mut resources = Resources::default();
    for (done, &n) in selected.iter().enumerate() {
        let page = &pages[(n - 1) as usize];
        let (pw, ph) = page.render_dimensions();
        let (w, h) = render_size((pw, ph), options.max_size);
        let rgb = guard(n, || {
            ctx.reset_and_resize(w as u16, h as u16);
            pixmap.resize(w as u16, h as u16);
            let transform = Affine::scale_non_uniform(
                f64::from(w) / f64::from(pw),
                f64::from(h) / f64::from(ph),
            ) * page.initial_transform(true).to_kurbo();
            render_into(page, &cache, &interpreter, &settings, &mut ctx, transform);
            ctx.flush();
            ctx.render_with(
                &mut pixmap,
                &mut resources,
                RasterizerSettings {
                    target_init: TargetInit::Clear(WHITE),
                    ..Default::default()
                },
            );
            // The background is opaque, so premultiplied RGBA is plain RGBA.
            pixmap
                .data_as_u8_slice()
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&[r, g, b, _]| [r, g, b])
                .collect::<Vec<u8>>()
        })
        .map_err(|e| e.with_file(input))?;
        let path = if folder {
            temp.join(page_file(&stem, n, total, ext))
        } else {
            temp.to_path_buf()
        };
        write(&path, &rgb, (w, h), to, quality).map_err(|e| not_writable(&task.output, &e))?;
        progress((done + 1) as f32 / selected.len() as f32);
        if cancel.is_cancelled() {
            return Err(Error::new(ErrorCode::Cancelled, "cancelled").with_file(input));
        }
    }
    Ok(Done::Written)
}

fn write(
    path: &Path,
    rgb: &[u8],
    (w, h): (u32, u32),
    to: FormatId,
    quality: u8,
) -> Result<(), image::ImageError> {
    let out = BufWriter::new(File::create(path)?);
    match to {
        FormatId::Jpeg => JpegEncoder::new_with_quality(out, quality).write_image(
            rgb,
            w,
            h,
            ExtendedColorType::Rgb8,
        ),
        _ => PngEncoder::new(out).write_image(rgb, w, h, ExtendedColorType::Rgb8),
    }
}

/// Errors name `output`, the file or folder the user asked for.
fn not_writable(output: &Path, e: &dyn std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::OutputNotWritable,
        format!("couldn't write the image ({e})"),
    )
    .with_file(output)
    .with_fix_as(
        FixKind::ChooseOutputFolder,
        "choose another folder with --out <dir>",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::exec::CancelToken;
    use crate::format::FormatId;
    use crate::job::{Job, JobOptions, PageRanges, PlannedTask};
    use crate::output::OutputPolicy;
    use crate::registry::{Operation, Registry};
    use persimmon_corpus::pdf::write_pdf_sample;
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-render-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Plan converting `input` to `to`, as `prsm convert` would.
    fn plan(input: &Path, to: FormatId, pages: Option<&str>) -> (PlannedTask, JobOptions) {
        let options = JobOptions {
            pages: pages.map(|p| PageRanges::parse(p).unwrap()),
            ..JobOptions::default()
        };
        let job = Job {
            op: Operation::Convert,
            inputs: vec![input.to_path_buf()],
            target: Some(to),
            output: OutputPolicy::default(),
            options: options.clone(),
        };
        let mut plan = job.plan(&Registry::builtin());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        (plan.tasks.remove(0), options)
    }

    fn run(task: &PlannedTask, options: &JobOptions, temp: &Path) -> Result<Done, Error> {
        render(task, options, temp, &CancelToken::new(), &mut |_| {})
    }

    fn decode(path: &Path) -> image::RgbImage {
        image::load_from_memory(&std::fs::read(path).unwrap())
            .unwrap()
            .into_rgb8()
    }

    fn mean(img: &image::RgbImage) -> [f64; 3] {
        let mut sum = [0f64; 3];
        for p in img.pixels() {
            for (s, c) in sum.iter_mut().zip(p.0) {
                *s += f64::from(c);
            }
        }
        let n = f64::from(img.width() * img.height());
        sum.map(|s| s / n)
    }

    #[test]
    fn render_size_scales_and_caps() {
        assert_eq!(render_size((595.0, 842.0), None), (1239, 1754));
        assert_eq!(render_size((595.0, 842.0), Some(800)), (565, 800));
        assert_eq!(render_size((595.0, 842.0), Some(4000)), (1239, 1754));
        assert_eq!(render_size((14400.0, 14400.0), None), (10000, 10000));
        let (w, h) = render_size((14400.0, 200.0), None);
        assert_eq!(w.max(h), 16384);
        assert!(h >= 1);
    }

    #[test]
    fn page_files_pad_to_the_page_count() {
        assert_eq!(page_file("doc", 7, 120, "png"), "doc 007.png");
        assert_eq!(page_file("doc", 1, 1, "jpg"), "doc 1.jpg");
    }

    #[test]
    fn text_pdf_renders_three_pngs() {
        let dir = scratch("text");
        let pdf = write_pdf_sample(&dir, "pdf-text");
        let (task, options) = plan(&pdf, FormatId::Png, None);
        let temp = dir.join("out.part");
        assert_eq!(run(&task, &options, &temp).unwrap(), Done::Written);
        for n in 1..=3 {
            let img = decode(&temp.join(format!("text {n}.png")));
            assert_eq!(img.dimensions(), (1239, 1754), "page {n}");
            let dark = img.pixels().filter(|p| p.0[0] < 128).count();
            assert!(dark >= 1000, "page {n} has {dark} dark pixels");
        }
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotated_and_cropped_pages_render_as_shown() {
        let dir = scratch("mixed");
        let pdf = write_pdf_sample(&dir, "pdf-mixed");
        let (task, options) = plan(&pdf, FormatId::Png, None);
        let temp = dir.join("out.part");
        run(&task, &options, &temp).unwrap();
        for (n, size) in [(1, (1275, 1650)), (2, (1754, 1239)), (3, (875, 1239))] {
            let img = decode(&temp.join(format!("mixed {n}.png")));
            assert_eq!(img.dimensions(), size, "page {n}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_pdf_to_jpeg_is_a_file() {
        let dir = scratch("scan");
        let pdf = write_pdf_sample(&dir, "pdf-scan");
        let (task, options) = plan(&pdf, FormatId::Jpeg, Some("1"));
        let temp = dir.join("out.part");
        run(&task, &options, &temp).unwrap();
        assert!(temp.is_file());
        let rendered = mean(&decode(&temp));
        let photo = persimmon_corpus::IMAGE_SAMPLES
            .iter()
            .find(|s| s.0 == "img-jpeg")
            .unwrap()
            .2;
        let original = mean(&image::load_from_memory(photo).unwrap().into_rgb8());
        for c in 0..3 {
            assert!(
                (rendered[c] - original[c]).abs() < 30.0,
                "{rendered:?} vs {original:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn big_page_renders_within_the_cap() {
        let dir = scratch("big");
        let pdf = write_pdf_sample(&dir, "pdf-big-page");
        let (task, options) = plan(&pdf, FormatId::Png, None);
        let temp = dir.join("out.part");
        run(&task, &options, &temp).unwrap();
        assert_eq!(decode(&temp).dimensions(), (10000, 10000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn objstm_and_owner_pdfs_render() {
        let dir = scratch("objstm");
        for id in ["pdf-objstm", "pdf-owner"] {
            let pdf = write_pdf_sample(&dir, id);
            let (task, options) = plan(&pdf, FormatId::Png, None);
            let temp = dir.join(format!("{id}.part"));
            run(&task, &options, &temp).unwrap();
            assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 3, "{id}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_failure_names_the_output() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("write-fail");
        let pdf = write_pdf_sample(&dir, "pdf-text");
        let (task, options) = plan(&pdf, FormatId::Png, Some("1"));
        let ro = dir.join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::File::create(ro.join("probe")).is_ok() {
            return; // running as root
        }
        let e = run(&task, &options, &ro.join("x.part")).unwrap_err();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(e.code, ErrorCode::OutputNotWritable);
        assert_eq!(e.file.as_deref(), Some(task.output.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_between_pages() {
        let dir = scratch("cancel");
        let pdf = write_pdf_sample(&dir, "pdf-many");
        let (task, options) = plan(&pdf, FormatId::Png, None);
        let cancel = CancelToken::new();
        let token = cancel.clone();
        let mut calls = 0;
        let e = render(&task, &options, &dir.join("out.part"), &cancel, &mut |_| {
            calls += 1;
            token.cancel();
        })
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        assert_eq!(calls, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guard_turns_a_panic_into_corrupt_input() {
        assert_eq!(guard(3, || 7).unwrap(), 7);
        let e = guard(3, || -> u32 { panic!("boom") }).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert_eq!(e.reason, "couldn't draw page 3");
    }

    #[test]
    fn truncated_pdf_is_corrupt_or_renders() {
        let dir = scratch("truncated");
        let pdf = write_pdf_sample(&dir, "pdf-truncated");
        let job = Job {
            op: Operation::Convert,
            inputs: vec![pdf],
            target: Some(FormatId::Png),
            output: OutputPolicy::default(),
            options: JobOptions::default(),
        };
        let plan = job.plan(&Registry::builtin());
        if let Some(e) = plan.errors.first() {
            assert_eq!(e.code, ErrorCode::CorruptInput, "{e}");
        } else {
            let temp = dir.join("out.part");
            if let Err(e) = run(&plan.tasks[0], &job.options, &temp) {
                assert_eq!(e.code, ErrorCode::CorruptInput, "{e}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
