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
}
