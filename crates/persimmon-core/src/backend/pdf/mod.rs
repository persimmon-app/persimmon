// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The PDF backend (DD-36). hayro reads and renders pages; lopdf builds
//! and rewrites documents. Both are pure Rust and compiled in, so PDF work
//! needs nothing installed and runs in builds without libvips.

use std::path::Path;

use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};

use crate::backend::{Done, Run};
use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;
use crate::format::FormatId;
use crate::job::{JobOptions, PlannedTask};

pub(crate) mod assemble;
pub(crate) mod build;
pub(crate) mod images;
pub(crate) mod render;

/// Renders PDF pages to PNG and JPEG, and builds PDFs from images and
/// other PDFs.
#[derive(Debug, Clone, Copy, Default)]
pub struct PdfBackend;

impl Run for PdfBackend {
    fn run(
        &self,
        task: &PlannedTask,
        options: &JobOptions,
        temp: &Path,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(f32),
    ) -> Result<Done, Error> {
        match task.to {
            Some(FormatId::Png | FormatId::Jpeg) => {
                render::render(task, options, temp, cancel, progress)
            }
            _ => build::build(task, options, temp, cancel, progress),
        }
    }
}

/// What planning and `prsm info` need to know about a PDF.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdfInfo {
    pub pages: u32,
    /// Page 1's size as displayed, in points: its crop box, turned by its
    /// rotation.
    pub first_page: (f32, f32),
}

/// Open `path` with hayro's lazy parser. Errors carry the file.
pub(crate) fn open(path: &Path) -> Result<Pdf, Error> {
    let data = std::fs::read(path).map_err(|e| {
        Error::new(
            ErrorCode::CorruptInput,
            format!("can't read this PDF ({e})"),
        )
        .with_file(path)
    })?;
    Pdf::new(data).map_err(|e| load_error(e).with_file(path))
}

fn load_error(e: LoadPdfError) -> Error {
    match e {
        LoadPdfError::Invalid => Error::new(ErrorCode::CorruptInput, "can't read this PDF"),
        LoadPdfError::Decryption(DecryptionError::PasswordProtected) => {
            Error::new(ErrorCode::UnsupportedInput, "it's password-protected")
                .with_fix("remove the password in another app, then try again")
        }
        LoadPdfError::Decryption(_) => Error::new(
            ErrorCode::UnsupportedInput,
            "it uses encryption prsm can't read",
        ),
    }
}

/// The page count and first page size of the PDF at `path`. A PDF that
/// can't be read, needs a password, or has no pages is an error.
pub fn pdf_info(path: &Path) -> Result<PdfInfo, Error> {
    let pdf = open(path)?;
    let pages = pdf.pages();
    let Some(first) = pages.first() else {
        return Err(Error::new(ErrorCode::CorruptInput, "this PDF has no pages").with_file(path));
    };
    Ok(PdfInfo {
        pages: pages.len() as u32,
        first_page: first.render_dimensions(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backends;
    use crate::exec::{CancelToken, Outcome, execute};
    use crate::format::sniff;
    use crate::job::{Job, JobOptions, PageRanges};
    use crate::output::OutputPolicy;
    use crate::registry::{Operation, Registry};
    use persimmon_corpus::pdf::write_pdf_sample;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-pdf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn info_counts_pages_and_sizes_page_one() {
        let dir = scratch("info");
        for (id, pages, first) in [
            ("pdf-text", 3, (595.0, 842.0)),
            ("pdf-mixed", 3, (612.0, 792.0)),
            ("pdf-objstm", 3, (595.0, 842.0)),
            ("pdf-owner", 3, (595.0, 842.0)),
            ("pdf-many", 120, (595.0, 842.0)),
        ] {
            let info = pdf_info(&write_pdf_sample(&dir, id)).unwrap();
            assert_eq!(
                info,
                PdfInfo {
                    pages,
                    first_page: first
                },
                "{id}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Run `op` through the real executor and backends; the one output.
    fn run_job(
        op: Operation,
        inputs: Vec<std::path::PathBuf>,
        to: FormatId,
        pages: Option<&str>,
    ) -> std::path::PathBuf {
        let job = Job {
            op,
            inputs,
            target: Some(to),
            output: OutputPolicy::default(),
            options: JobOptions {
                pages: pages.map(|p| PageRanges::parse(p).unwrap()),
                ..JobOptions::default()
            },
        };
        let plan = job.plan(&Registry::builtin());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        let results = execute(
            &job,
            &plan,
            &Backends::new(),
            &CancelToken::new(),
            &mut |_| {},
        );
        match results.as_slice() {
            [Ok(Outcome::Written(p))] => p.clone(),
            other => panic!("{op:?} → {to:?}: {other:?}"),
        }
    }

    fn sniffed(path: &std::path::Path) -> Option<FormatId> {
        let bytes = std::fs::read(path).unwrap();
        sniff(&bytes[..bytes.len().min(64)])
    }

    /// Gate item 1 for PDF: every DD-28 PDF job, end to end.
    #[test]
    fn every_pdf_pair_converts() {
        let dir = scratch("pairs");
        // Images → PDF. Only JPEG goes in without libvips.
        let mut images = vec!["img-jpeg"];
        if cfg!(feature = "vips") {
            images.extend(["img-webp", "img-heic", "img-tiff", "img-avif", "img-bmp"]);
        }
        let mut inputs: Vec<std::path::PathBuf> = images
            .iter()
            .map(|id| {
                let (_, name, bytes, _) = persimmon_corpus::IMAGE_SAMPLES
                    .iter()
                    .find(|s| s.0 == *id)
                    .unwrap();
                let p = dir.join(name);
                std::fs::write(&p, bytes).unwrap();
                p
            })
            .collect();
        if cfg!(feature = "vips") {
            for (name, bytes) in [
                ("pixel.png", &persimmon_corpus::PNG_1X1[..]),
                ("pixel.gif", persimmon_corpus::GIF_1X1),
            ] {
                std::fs::write(dir.join(name), bytes).unwrap();
                inputs.push(dir.join(name));
            }
        }
        for input in inputs {
            let out = run_job(Operation::Convert, vec![input.clone()], FormatId::Pdf, None);
            assert_eq!(sniffed(&out), Some(FormatId::Pdf), "{input:?}");
        }
        // PDF → PNG and JPEG.
        let pdf = write_pdf_sample(&dir, "pdf-text");
        for to in [FormatId::Png, FormatId::Jpeg] {
            let out = run_job(Operation::Convert, vec![pdf.clone()], to, Some("1"));
            assert_eq!(sniffed(&out), Some(to));
        }
        // Merge and split.
        let other = write_pdf_sample(&dir, "pdf-mixed");
        let merged = run_job(
            Operation::Merge,
            vec![pdf.clone(), other],
            FormatId::Pdf,
            None,
        );
        assert_eq!(pdf_info(&merged).unwrap().pages, 6);
        let split = run_job(Operation::Split, vec![pdf], FormatId::Pdf, Some("1"));
        assert_eq!(pdf_info(&split).unwrap().pages, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
