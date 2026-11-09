// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! RAR and RAR5 extraction through the system libarchive's own
//! BSD-licensed reader, never unRAR (BL-T20), via `compress-tools`. The
//! binding exposes each entry's path and stat but no link targets, so
//! symlinks are skipped and a hard link can't be told from an empty file.

use std::cell::Cell;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use compress_tools::{ArchiveContents, ArchiveIterator};

use super::sink::Sink;
use super::{Counting, Meter, password_protected, rar_scan, unreadable};
use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;

const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;

fn map_error(input: &Path, e: &compress_tools::Error) -> Error {
    let text = e.to_string().to_lowercase();
    if text.contains("passphrase") || text.contains("encrypt") {
        password_protected(input)
    } else {
        unreadable(input, e)
    }
}

/// One entry's data as a `Read`, pulled chunk by chunk from the iterator
/// until its end-of-entry.
struct Chunks<'a, R: Read + std::io::Seek> {
    iter: &'a mut ArchiveIterator<R>,
    chunk: Vec<u8>,
    at: usize,
    done: bool,
}

impl<R: Read + std::io::Seek> Read for Chunks<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.at == self.chunk.len() {
            if self.done {
                return Ok(0);
            }
            match self.iter.next() {
                Some(ArchiveContents::DataChunk(c)) => {
                    self.chunk = c;
                    self.at = 0;
                }
                Some(ArchiveContents::Err(e)) => {
                    self.done = true;
                    return Err(std::io::Error::other(e.to_string()));
                }
                _ => self.done = true,
            }
        }
        let n = buf.len().min(self.chunk.len() - self.at);
        buf[..n].copy_from_slice(&self.chunk[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

pub(crate) fn extract_rar(
    input: &Path,
    sink: &mut Sink,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(f32),
) -> Result<(), Error> {
    let mut probe = BufReader::new(File::open(input).map_err(|e| unreadable(input, &e))?);
    if rar_scan::encrypted(&mut probe) {
        return Err(password_protected(input));
    }
    let file = File::open(input).map_err(|e| unreadable(input, &e))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let (reader, count) = Counting::new(BufReader::new(file));
    let mut iter = ArchiveIterator::from_read(reader).map_err(|e| map_error(input, &e))?;
    let last = Cell::new(0.0);
    while let Some(item) = iter.next() {
        match item {
            ArchiveContents::StartOfEntry(name, stat) => {
                let mode = stat.st_mode as u32;
                // time_t is i64 here but narrower on some platforms.
                #[allow(clippy::useless_conversion)]
                let mtime = Some(i64::from(stat.st_mtime));
                let mut chunks = Chunks {
                    iter: &mut iter,
                    chunk: Vec::new(),
                    at: 0,
                    done: false,
                };
                match mode & S_IFMT {
                    S_IFDIR => sink.dir(&name, mtime)?,
                    S_IFREG | 0 => {
                        let mut meter =
                            Meter::new(&mut chunks, count.clone(), size, &last, &mut *progress);
                        sink.file(&name, Some(mode & 0o7777), mtime, &mut meter, cancel)?;
                    }
                    // Links and devices: skipped.
                    _ => {}
                }
                // Drain whatever the entry still has, up to its end.
                std::io::copy(&mut chunks, &mut std::io::sink())
                    .map_err(|e| unreadable(input, &e))?;
            }
            ArchiveContents::Err(e) => return Err(map_error(input, &e)),
            ArchiveContents::DataChunk(_) | ArchiveContents::EndOfEntry => {}
        }
        if cancel.is_cancelled() {
            return Err(Error::new(ErrorCode::Cancelled, "cancelled").with_file(input));
        }
    }
    Ok(())
}
