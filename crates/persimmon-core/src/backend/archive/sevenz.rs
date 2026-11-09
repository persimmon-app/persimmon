// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! 7z extraction, through `sevenz-rust2`, into the sink. Unix modes come
//! from the attribute's high half when the Unix-extension bit is set, which
//! is how p7zip and 7-Zip on Unix store them; symlinks stored that way are
//! recreated.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use sevenz_rust2::{ArchiveReader, Password};

use super::sink::Sink;
use super::{Counting, Meter, password_protected, unreadable};
use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;

/// FILE_ATTRIBUTE_UNIX_EXTENSION: the high 16 bits hold a Unix mode.
const UNIX_EXTENSION: u32 = 0x8000;
const S_IFMT: u32 = 0o170_000;
const S_IFLNK: u32 = 0o120_000;
/// 100-ns ticks from 1601 to 1970.
const EPOCH_TICKS: u64 = 116_444_736_000_000_000;

fn map_error(input: &Path, e: &sevenz_rust2::Error) -> Error {
    match e {
        sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_) => {
            password_protected(input)
        }
        other => unreadable(input, other),
    }
}

pub(crate) fn extract_7z(
    input: &Path,
    sink: &mut Sink,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(f32),
) -> Result<(), Error> {
    let file = File::open(input).map_err(|e| unreadable(input, &e))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let (reader, count) = Counting::new(BufReader::new(file));
    let mut archive =
        ArchiveReader::new(reader, Password::empty()).map_err(|e| map_error(input, &e))?;
    let last = Cell::new(0.0);
    // The sink's own error, carried out of the reader's callback, which
    // can only return the reader's error type.
    let failed: RefCell<Option<Error>> = RefCell::new(None);
    let result = archive.for_each_entries(|entry, data| {
        let stop = |e: Error| {
            *failed.borrow_mut() = Some(e);
            Err(sevenz_rust2::Error::Unsupported("stopped".into()))
        };
        let name = entry.name().to_string();
        let mtime = entry.has_last_modified_date.then(|| {
            let ticks = u64::from(entry.last_modified_date());
            (ticks.saturating_sub(EPOCH_TICKS) / 10_000_000) as i64
        });
        let mode = (entry.has_windows_attributes && entry.windows_attributes & UNIX_EXTENSION != 0)
            .then_some(entry.windows_attributes >> 16);
        let outcome = if entry.is_directory() {
            sink.dir(&name, mtime)
        } else if mode.is_some_and(|m| m & S_IFMT == S_IFLNK) {
            let mut target = String::new();
            match data.read_to_string(&mut target) {
                Ok(_) => sink.symlink(&name, &target),
                Err(e) => Err(unreadable(input, &e)),
            }
        } else {
            let mut meter = Meter::new(data, count.clone(), size, &last, &mut *progress);
            sink.file(&name, mode, mtime, &mut meter, cancel)
        };
        match outcome {
            Ok(()) if cancel.is_cancelled() => {
                stop(Error::new(ErrorCode::Cancelled, "cancelled").with_file(input))
            }
            Ok(()) => Ok(true),
            Err(e) => stop(e),
        }
    });
    if let Some(e) = failed.into_inner() {
        return Err(e);
    }
    result.map_err(|e| map_error(input, &e))
}
