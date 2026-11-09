// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! ZIP extraction, through the `zip` crate, into the sink. An encrypted
//! entry fails the archive before anything is written.

use std::cell::Cell;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use super::sink::Sink;
use super::{Counting, Meter, password_protected, unreadable};
use crate::error::Error;
use crate::exec::CancelToken;

pub(crate) fn extract_zip(
    input: &Path,
    sink: &mut Sink,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(f32),
) -> Result<(), Error> {
    let file = File::open(input).map_err(|e| unreadable(input, &e))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let (reader, count) = Counting::new(BufReader::new(file));
    let mut zip = ::zip::ZipArchive::new(reader).map_err(|e| unreadable(input, &e))?;
    for i in 0..zip.len() {
        if zip
            .by_index_raw(i)
            .map_err(|e| unreadable(input, &e))?
            .encrypted()
        {
            return Err(password_protected(input));
        }
    }
    let last = Cell::new(0.0);
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| unreadable(input, &e))?;
        let name = entry
            .name()
            .map_err(|e| unreadable(input, &e))?
            .into_owned();
        // The extended-timestamp field (0x5455) holds real UTC, as Finder,
        // Info-ZIP and 7-Zip write it; without one, the DOS time.
        let utc = entry.extra_data_fields().find_map(|f| match f {
            ::zip::extra_fields::ExtraField::ExtendedTimestamp(t) => t.mod_time(),
            _ => None,
        });
        let mtime = utc
            .map(i64::from)
            .or_else(|| entry.last_modified().and_then(dos_time));
        if entry.is_dir() {
            sink.dir(&name, mtime)?;
        } else if entry.is_symlink() {
            let mut target = String::new();
            entry
                .read_to_string(&mut target)
                .map_err(|e| unreadable(input, &e))?;
            sink.symlink(&name, &target)?;
        } else {
            let mode = entry.unix_mode();
            let mut meter = Meter::new(&mut entry, count.clone(), size, &last, &mut *progress);
            sink.file(&name, mode, mtime, &mut meter, cancel)?;
        }
    }
    Ok(())
}

/// A ZIP's DOS timestamp as Unix seconds, for entries without an
/// extended timestamp. DOS times carry no time zone; they're read as UTC.
fn dos_time(t: ::zip::DateTime) -> Option<i64> {
    let (y, m, d) = (
        i64::from(t.year()),
        i64::from(t.month()),
        i64::from(t.day()),
    );
    if !(1..=12).contains(&m) || d == 0 {
        return None;
    }
    // Days from 1970-01-01 to y-m-d (proleptic Gregorian).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(
        days * 86_400
            + i64::from(t.hour()) * 3_600
            + i64::from(t.minute()) * 60
            + i64::from(t.second()),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn dos_time_is_utc() {
        let t = ::zip::DateTime::from_date_and_time(2026, 1, 2, 3, 4, 6).unwrap();
        assert_eq!(super::dos_time(t), Some(1_767_323_046));
        let t = ::zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(super::dos_time(t), Some(315_532_800));
    }

    #[cfg(test)]
    mod utc_tests {
        use crate::backend::archive::tests::{extract, scratch};
        use std::io::Write;

        #[test]
        fn extended_timestamp_wins_over_dos_time() {
            use ::zip::write::FullFileOptions;
            let dir = scratch("zip-utc");
            let path = dir.join("t.zip");
            let mut w = ::zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            let mut options = FullFileOptions::default().last_modified_time(
                ::zip::DateTime::from_date_and_time(2026, 1, 2, 13, 4, 6).unwrap(),
            );
            let mut ut = vec![1u8];
            ut.extend(1_767_323_047u32.to_le_bytes());
            options.add_extra_field(0x5455, ut, false).unwrap();
            w.start_file("a.txt", options).unwrap();
            w.write_all(b"a").unwrap();
            w.finish().unwrap();
            let out = extract(&dir, path).unwrap();
            let mtime = std::fs::metadata(out.join("a.txt"))
                .unwrap()
                .modified()
                .unwrap()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            assert_eq!(mtime, 1_767_323_047);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
