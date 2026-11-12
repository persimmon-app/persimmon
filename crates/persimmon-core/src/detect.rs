// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! What is this file? Content first, name second, and never more than the
//! first [`HEAD_LEN`] bytes read: a 4 GB video is identified as fast as a
//! 4 KB icon.

use std::io::Read;
use std::path::Path;

use crate::error::{Error, ErrorCode};
use crate::format::{Format, FormatId, format_for_name, sniff};

/// The most bytes detection ever reads from a file.
pub const HEAD_LEN: usize = 512;

/// The result of detecting a file's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detected {
    pub format: FormatId,
    /// True when the file's extension names a different format than its
    /// content: a PNG called `photo.jpg`.
    pub extension_mismatch: bool,
}

/// Read up to [`HEAD_LEN`] bytes from the start of a file.
pub(crate) fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(HEAD_LEN);
    std::fs::File::open(path)?
        .take(HEAD_LEN as u64)
        .read_to_end(&mut head)?;
    Ok(head)
}

/// Identify the format of the file at `path`.
pub fn detect(path: &Path, formats: &[Format]) -> Result<Detected, Error> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::new(ErrorCode::InputNotFound, "no such file").with_file(path));
        }
        Err(e) => return Err(unreadable(path, &e)),
    };
    if meta.is_dir() {
        return Err(Error::new(ErrorCode::UnsupportedInput, "is a folder")
            .with_file(path)
            .with_fix("pass the files inside it"));
    }
    if meta.len() == 0 {
        return Err(Error::new(ErrorCode::CorruptInput, "file is empty").with_file(path));
    }

    let head = read_head(path).map_err(|e| unreadable(path, &e))?;
    let by_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| format_for_name(n, formats));

    match (sniff(&head), by_name) {
        (Some(format), by_name) => Ok(Detected {
            format,
            extension_mismatch: by_name.is_some_and(|n| n != format),
        }),
        (None, Some(named)) => {
            let name = formats
                .iter()
                .find(|f| f.id == named)
                .map_or(named.as_str(), |f| f.name);
            Err(Error::new(
                ErrorCode::CorruptInput,
                format!("doesn't look like a {name} file"),
            )
            .with_file(path))
        }
        (None, None) => {
            Err(Error::new(ErrorCode::UnsupportedInput, "unrecognised file type").with_file(path))
        }
    }
}

fn unreadable(path: &Path, e: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::UnsupportedInput,
        format!("can't read this file ({e})"),
    )
    .with_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::format::{FormatId, all_formats};
    use std::path::PathBuf;

    const PNG_HEAD: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-detect-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detect_png_named_jpg_reports_mismatch() {
        let p = scratch("mismatch").join("photo.jpg");
        std::fs::write(&p, PNG_HEAD).unwrap();
        assert_eq!(
            detect(&p, all_formats()).unwrap(),
            Detected {
                format: FormatId::Png,
                extension_mismatch: true
            }
        );
    }

    #[test]
    fn detect_matching_extension_is_not_a_mismatch() {
        let p = scratch("match").join("photo.PNG");
        std::fs::write(&p, PNG_HEAD).unwrap();
        assert!(!detect(&p, all_formats()).unwrap().extension_mismatch);
    }

    #[test]
    fn detect_empty_is_corrupt() {
        let p = scratch("empty").join("empty.png");
        std::fs::write(&p, b"").unwrap();
        let e = detect(&p, all_formats()).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert_eq!(e.reason, "file is empty");
        assert_eq!(e.file.as_deref(), Some(p.as_path()));
    }

    #[test]
    fn detect_truncated_png_is_corrupt() {
        let p = scratch("truncated").join("cut.png");
        std::fs::write(&p, [0x89, 0x50, 0x4E]).unwrap();
        let e = detect(&p, all_formats()).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert_eq!(e.reason, "doesn't look like a PNG file");
    }

    #[test]
    fn detect_folder_is_unsupported() {
        let p = scratch("folder").join("album.png");
        std::fs::create_dir(&p).unwrap();
        let e = detect(&p, all_formats()).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput);
        assert_eq!(e.reason, "is a folder");
        assert_eq!(e.fix_text(), Some("pass the files inside it"));
    }

    #[test]
    fn detect_missing_is_not_found() {
        let p = scratch("missing").join("nope.png");
        assert_eq!(
            detect(&p, all_formats()).unwrap_err().code,
            ErrorCode::InputNotFound
        );
    }

    #[test]
    fn detect_unknown_is_unsupported() {
        let p = scratch("unknown").join("notes.xyz");
        std::fs::write(&p, b"just some text").unwrap();
        let e = detect(&p, all_formats()).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput);
        assert_eq!(e.reason, "unrecognised file type");
    }

    #[test]
    fn detect_reads_only_the_head() {
        let p = scratch("head").join("big.png");
        let mut bytes = PNG_HEAD.to_vec();
        bytes.resize(512, 0);
        bytes.extend(std::iter::repeat_n(0xAB, 88));
        std::fs::write(&p, &bytes).unwrap();
        assert_eq!(detect(&p, all_formats()).unwrap().format, FormatId::Png);
        assert_eq!(read_head(&p).unwrap().len(), 512);
    }
}
