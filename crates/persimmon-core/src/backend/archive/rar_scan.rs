// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Whether a RAR archive is password-protected, read from its headers
//! alone. libarchive reports encrypted RAR data as decoding errors
//! ("Invalid PPMd sequence"), and the binding has no encryption query, so
//! this walks the RAR4 block chain or the RAR5 header chain looking for an
//! encrypted file or encrypted headers. Pure Rust: builds without
//! libarchive.

use std::io::{self, Read, Seek, SeekFrom};

/// The most headers walked before giving up (a malformed chain).
const MAX_HEADERS: usize = 1_000_000;

/// Whether the RAR archive in `r` has encrypted headers or files. Anything
/// that isn't a RAR, or doesn't parse, counts as not encrypted: the
/// extractor then reports it.
pub(crate) fn encrypted<R: Read + Seek>(r: &mut R) -> bool {
    scan(r).unwrap_or(false)
}

fn scan<R: Read + Seek>(r: &mut R) -> io::Result<bool> {
    let mut sig = [0u8; 8];
    r.seek(SeekFrom::Start(0))?;
    r.read_exact(&mut sig[..7])?;
    if &sig[..7] == b"Rar!\x1a\x07\x00" {
        return rar4(r, 7);
    }
    r.read_exact(&mut sig[7..])?;
    if &sig == b"Rar!\x1a\x07\x01\x00" {
        return rar5(r, 8);
    }
    Ok(false)
}

fn u32_at<R: Read + Seek>(r: &mut R, at: u64) -> io::Result<u64> {
    let mut b = [0u8; 4];
    r.seek(SeekFrom::Start(at))?;
    r.read_exact(&mut b)?;
    Ok(u64::from(u32::from_le_bytes(b)))
}

/// RAR 1.5–4: 7-byte block headers (CRC, type, flags, size), with an extra
/// data size after them when flag 0x8000 is set; a file header's packed
/// size sits there.
fn rar4<R: Read + Seek>(r: &mut R, mut pos: u64) -> io::Result<bool> {
    for _ in 0..MAX_HEADERS {
        let mut h = [0u8; 7];
        r.seek(SeekFrom::Start(pos))?;
        if r.read_exact(&mut h).is_err() {
            return Ok(false);
        }
        let kind = h[2];
        let flags = u16::from_le_bytes([h[3], h[4]]);
        let size = u64::from(u16::from_le_bytes([h[5], h[6]]));
        if size < 7 {
            return Ok(false);
        }
        let mut add = 0;
        match kind {
            // Main header with encrypted block headers.
            0x73 if flags & 0x0080 != 0 => return Ok(true),
            0x74 => {
                if flags & 0x0004 != 0 {
                    return Ok(true);
                }
                add = u32_at(r, pos + 7)?;
                if flags & 0x0100 != 0 {
                    add |= u32_at(r, pos + 32)? << 32;
                }
            }
            0x7B => return Ok(false),
            _ if flags & 0x8000 != 0 => add = u32_at(r, pos + 7)?,
            _ => {}
        }
        pos += size + add;
    }
    Ok(false)
}

/// A RAR5 variable-length integer and how many bytes it took.
fn vint<R: Read>(r: &mut R) -> io::Result<(u64, u64)> {
    let mut value = 0u64;
    for i in 0..10 {
        let mut b = [0u8; 1];
        r.read_exact(&mut b)?;
        value |= u64::from(b[0] & 0x7F) << (7 * i);
        if b[0] & 0x80 == 0 {
            return Ok((value, i + 1));
        }
    }
    Err(io::Error::new(io::ErrorKind::InvalidData, "bad vint"))
}

/// RAR5: each header is a CRC32, then its size, type and flags as vints,
/// then optional extra-area and data sizes. Type 4 means encrypted
/// headers; a file header (type 2) with an extra record of type 1 means an
/// encrypted file.
fn rar5<R: Read + Seek>(r: &mut R, mut pos: u64) -> io::Result<bool> {
    for _ in 0..MAX_HEADERS {
        r.seek(SeekFrom::Start(pos + 4))?;
        let Ok((size, size_len)) = vint(r) else {
            return Ok(false);
        };
        let body = pos + 4 + size_len;
        let (kind, _) = vint(r)?;
        let (flags, _) = vint(r)?;
        let extra = if flags & 1 != 0 { vint(r)?.0 } else { 0 };
        let data = if flags & 2 != 0 { vint(r)?.0 } else { 0 };
        match kind {
            4 => return Ok(true),
            5 => return Ok(false),
            2 if extra > 0 && extra <= size => {
                let start = body + size - extra;
                let mut done = 0;
                while done < extra {
                    r.seek(SeekFrom::Start(start + done))?;
                    let (record, record_len) = vint(r)?;
                    let (record_type, _) = vint(r)?;
                    if record_type == 1 {
                        return Ok(true);
                    }
                    if record == 0 {
                        break;
                    }
                    done += record_len + record;
                }
            }
            _ => {}
        }
        if size == 0 {
            return Ok(false);
        }
        pos = body + size + data;
    }
    Ok(false)
}
