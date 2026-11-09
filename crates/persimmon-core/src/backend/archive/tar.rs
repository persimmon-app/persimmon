// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! TAR extraction, plain or compressed with gzip, bzip2, xz or zstd,
//! through the `tar` crate, into the sink. A compressed file that doesn't
//! hold a tar archive is refused before anything is written.

use std::cell::Cell;
use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::Path;

use super::sink::Sink;
use super::{Counting, Meter, unreadable};
use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;
use crate::format::FormatId;

/// Whether `block`, the first 512 bytes, starts a tar archive: an empty
/// one (all zeros) or a header whose checksum is right.
fn looks_like_tar(block: &[u8]) -> bool {
    if block.len() < 512 {
        return false;
    }
    if block.iter().all(|&b| b == 0) {
        return true;
    }
    let field = &block[148..156];
    let digits: String = field
        .iter()
        .take_while(|&&b| b != 0 && b != b' ')
        .map(|&b| b as char)
        .collect();
    let Ok(stored) = u32::from_str_radix(digits.trim(), 8) else {
        return false;
    };
    let unsigned: u32 = block
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                32
            } else {
                u32::from(b)
            }
        })
        .sum();
    let signed: i32 = block
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                32
            } else {
                i32::from(b as i8)
            }
        })
        .sum();
    stored == unsigned || stored as i32 == signed
}

pub(crate) fn extract_tar(
    input: &Path,
    from: FormatId,
    sink: &mut Sink,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(f32),
) -> Result<(), Error> {
    let file = File::open(input).map_err(|e| unreadable(input, &e))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let (counting, count) = Counting::new(BufReader::new(file));
    let mut decoded: Box<dyn Read> = match from {
        FormatId::TarGz => Box::new(flate2::read::MultiGzDecoder::new(counting)),
        FormatId::TarBz2 => Box::new(bzip2::read::MultiBzDecoder::new(counting)),
        FormatId::TarXz => Box::new(lzma_rust2::XzReader::new(counting, true)),
        FormatId::TarZst => Box::new(
            ruzstd::decoding::StreamingDecoder::new(counting).map_err(|e| unreadable(input, &e))?,
        ),
        _ => Box::new(counting),
    };
    let mut head = Vec::with_capacity(512);
    (&mut decoded)
        .take(512)
        .read_to_end(&mut head)
        .map_err(|e| unreadable(input, &e))?;
    if !looks_like_tar(&head) {
        return Err(if from == FormatId::Tar {
            unreadable(input, &"no tar header")
        } else {
            Error::new(
                ErrorCode::UnsupportedInput,
                "it's a compressed file, not a tar archive",
            )
            .with_file(input)
        });
    }
    let mut archive = ::tar::Archive::new(Cursor::new(head).chain(decoded));
    let last = Cell::new(0.0);
    for entry in archive.entries().map_err(|e| unreadable(input, &e))? {
        let mut entry = entry.map_err(|e| unreadable(input, &e))?;
        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let link = entry
            .link_name_bytes()
            .map(|l| String::from_utf8_lossy(&l).into_owned());
        let header = entry.header();
        let kind = header.entry_type();
        let mtime = header.mtime().ok().and_then(|t| i64::try_from(t).ok());
        let mode = header.mode().ok();
        use ::tar::EntryType as T;
        match kind {
            T::Directory => sink.dir(&name, mtime)?,
            T::Regular | T::Continuous | T::GNUSparse => {
                let mut meter = Meter::new(&mut entry, count.clone(), size, &last, &mut *progress);
                sink.file(&name, mode, mtime, &mut meter, cancel)?;
            }
            T::Symlink => sink.symlink(&name, link.as_deref().unwrap_or(""))?,
            T::Link => sink.hardlink(&name, link.as_deref().unwrap_or(""))?,
            // Devices, FIFOs and sockets are skipped; long-name and PAX
            // headers are read by the tar crate itself.
            _ => {}
        }
        if cancel.is_cancelled() {
            return Err(Error::new(ErrorCode::Cancelled, "cancelled").with_file(input));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::looks_like_tar;

    #[test]
    fn tar_headers_are_recognised() {
        let mut header = ::tar::Header::new_ustar();
        header.set_path("a.txt").unwrap();
        header.set_size(0);
        header.set_cksum();
        assert!(looks_like_tar(header.as_bytes()));
        assert!(looks_like_tar(&[0u8; 512]));
        assert!(!looks_like_tar(b"notes\n"));
        let mut bad = header.as_bytes().to_vec();
        bad[0] ^= 1;
        assert!(!looks_like_tar(&bad));
    }
}
