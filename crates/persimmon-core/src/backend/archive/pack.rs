// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Packing files and folders into ZIP or tar.gz (DD-37).
//!
//! Each input goes at the archive's root under its own name (a clash gets
//! `name (1)`); folders are walked in sorted order with empty folders
//! kept. Symlinks are stored as links, never followed. Finder litter
//! (`.DS_Store`, `._*`) inside folders is left out. The job's own
//! temporary file and output are never packed, even when they sit inside
//! an input folder. ZIP uses Deflate level 6 with UTF-8 names, Unix modes,
//! modification times and ZIP64 when needed; tar.gz uses gzip level 6 with
//! modes and times, and no owners.

use std::cell::Cell;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::backend::Done;
use crate::error::{Error, ErrorCode, FixKind};
use crate::exec::CancelToken;
use crate::format::FormatId;
use crate::job::PlannedTask;
use crate::platform;

/// The error text a cancelled read carries back out of the writers.
const CANCELLED: &str = "prsm: cancelled";

/// The error text a source that shrank while being packed carries back.
pub(crate) const CHANGED: &str = "prsm: the file changed while it was packed";

/// Marks a read error from a source file, so it's reported against the
/// source rather than the archive being written.
const READ_FAILED: &str = "prsm: read failed: ";

/// Exactly `len` bytes of `inner`: a file that grew is cut at the size its
/// tar header already states, and one that shrank is an error rather than
/// an archive whose header and data disagree.
pub(crate) struct Exact<R> {
    inner: R,
    left: u64,
}

impl<R> Exact<R> {
    pub(crate) fn new(inner: R, len: u64) -> Exact<R> {
        Exact { inner, left: len }
    }
}

impl<R: Read> Read for Exact<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Ok(0);
        }
        let want = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..want])?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                CHANGED,
            ));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

/// What an archive entry is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind {
    Dir,
    File,
    Symlink(PathBuf),
}

/// One entry to pack: its name in the archive, where it comes from, what
/// it is.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub name: String,
    pub source: PathBuf,
    pub kind: Kind,
}

fn unreadable(path: &Path, e: &dyn std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::UnsupportedInput,
        format!("can't read this file ({e})"),
    )
    .with_file(path)
}

fn not_writable(output: &Path, e: &dyn std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::OutputNotWritable,
        format!("couldn't write the archive ({e})"),
    )
    .with_file(output)
    .with_fix_as(
        FixKind::ChooseOutputFolder,
        "choose another folder with --out <dir>",
    )
}

fn litter(name: &str) -> bool {
    name == ".DS_Store" || name.starts_with("._")
}

fn fold(name: &str) -> String {
    name.nfc().collect::<String>().to_lowercase()
}

/// `path` with its folder resolved, for comparing against what to skip.
fn resolved(path: &Path) -> PathBuf {
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let parent = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    match path.file_name() {
        Some(name) => parent.join(name),
        None => parent,
    }
}

/// Every entry to pack, in order, with `skip` (the temp file and the
/// output) left out.
pub(crate) fn entries(inputs: &[PathBuf], skip: &[&Path]) -> Result<Vec<Entry>, Error> {
    let skip: HashSet<PathBuf> = skip.iter().map(|p| resolved(p)).collect();
    let mut out = Vec::new();
    let mut roots = HashSet::new();
    for input in inputs {
        let meta = std::fs::symlink_metadata(input).map_err(|e| unreadable(input, &e))?;
        let base = resolved(input)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "files".to_string());
        let mut name = base.clone();
        let mut n = 1;
        while !roots.insert(fold(&name)) {
            name = match base.rfind('.') {
                Some(dot) if dot > 0 && !meta.is_dir() => {
                    format!("{} ({n}){}", &base[..dot], &base[dot..])
                }
                _ => format!("{base} ({n})"),
            };
            n += 1;
        }
        walk(input, name, &meta, &skip, &mut out)?;
    }
    Ok(out)
}

fn walk(
    path: &Path,
    name: String,
    meta: &std::fs::Metadata,
    skip: &HashSet<PathBuf>,
    out: &mut Vec<Entry>,
) -> Result<(), Error> {
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(path).map_err(|e| unreadable(path, &e))?;
        out.push(Entry {
            name,
            source: path.to_path_buf(),
            kind: Kind::Symlink(target),
        });
    } else if meta.is_dir() {
        out.push(Entry {
            name: name.clone(),
            source: path.to_path_buf(),
            kind: Kind::Dir,
        });
        let here = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut children: Vec<_> = std::fs::read_dir(path)
            .map_err(|e| unreadable(path, &e))?
            .collect::<Result<_, _>>()
            .map_err(|e| unreadable(path, &e))?;
        children.sort_by_key(|c| c.file_name());
        for child in children {
            let child_name = child.file_name().to_string_lossy().into_owned();
            if litter(&child_name) || skip.contains(&here.join(child.file_name())) {
                continue;
            }
            let child_path = child.path();
            let child_meta =
                std::fs::symlink_metadata(&child_path).map_err(|e| unreadable(&child_path, &e))?;
            walk(
                &child_path,
                format!("{name}/{child_name}"),
                &child_meta,
                skip,
                out,
            )?;
        }
    } else if meta.is_file() {
        out.push(Entry {
            name,
            source: path.to_path_buf(),
            kind: Kind::File,
        });
    }
    // FIFOs, sockets and devices are left out: opening a FIFO blocks
    // forever, and a device like /dev/zero never ends.
    Ok(())
}

/// What every entry's writing shares: where the archive goes (for
/// errors), progress so far, and the cancel token.
struct Packing<'a> {
    output: &'a Path,
    done: Cell<u64>,
    total: u64,
    last: Cell<f32>,
    cancel: &'a CancelToken,
}

/// A source file's bytes on their way into the archive: counts them for
/// progress (at most every 1%) and stops with an error once cancelled.
struct Feed<'a> {
    file: File,
    packing: &'a Packing<'a>,
    progress: &'a mut dyn FnMut(f32),
}

impl Read for Feed<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let p = self.packing;
        if p.cancel.is_cancelled() {
            return Err(std::io::Error::other(CANCELLED));
        }
        let n = self.file.read(buf).map_err(|e| match e.kind() {
            // io::copy retries these.
            std::io::ErrorKind::Interrupted => e,
            _ => std::io::Error::other(format!("{READ_FAILED}{e}")),
        })?;
        p.done.set(p.done.get() + n as u64);
        let f = (p.done.get() as f32 / p.total.max(1) as f32).min(1.0);
        if f >= p.last.get() + 0.01 {
            p.last.set(f);
            (self.progress)(f);
        }
        Ok(n)
    }
}

/// Unix seconds as a ZIP (DOS) time, clamped to what DOS times can hold.
fn zip_time(secs: i64) -> ::zip::DateTime {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    match u16::try_from(y) {
        Ok(y) if (1980..=2107).contains(&y) => {
            ::zip::DateTime::from_date_and_time(y, m as u8, d as u8, h as u8, mi as u8, s as u8)
                .unwrap_or_default()
        }
        Ok(y) if y > 2107 => {
            ::zip::DateTime::from_date_and_time(2107, 12, 31, 23, 59, 58).unwrap_or_default()
        }
        _ => ::zip::DateTime::default(),
    }
}

fn mtime(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

/// Pack the task's inputs into a ZIP or tar.gz at `temp`.
pub(crate) fn pack(
    task: &PlannedTask,
    temp: &Path,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(f32),
) -> Result<Done, Error> {
    let file = File::create(temp).map_err(|e| not_writable(&task.output, &e))?;
    let list = entries(&task.inputs, &[temp, &task.output])?;
    let total: u64 = list
        .iter()
        .filter(|e| e.kind == Kind::File)
        .filter_map(|e| std::fs::metadata(&e.source).ok())
        .map(|m| m.len())
        .sum();
    let packing = Packing {
        output: &task.output,
        done: Cell::new(0),
        total,
        last: Cell::new(0.0),
        cancel,
    };
    let out = BufWriter::new(file);
    let result = match task.to {
        Some(FormatId::Zip) => write_zip(out, &list, &packing, progress),
        _ => write_tar_gz(out, &list, &packing, progress),
    };
    match result {
        Ok(()) => {
            progress(1.0);
            Ok(Done::Written)
        }
        Err(_) if cancel.is_cancelled() => Err(Error::new(ErrorCode::Cancelled, "cancelled")),
        Err(e) => Err(e),
    }
}

/// Map a writer failure: a source read error names the source, anything
/// else is the output's.
fn write_error(output: &Path, source: &Path, e: &dyn std::fmt::Display) -> Error {
    let text = e.to_string();
    if text.contains(CANCELLED) {
        Error::new(ErrorCode::Cancelled, "cancelled")
    } else if let Some((_, cause)) = text.split_once(READ_FAILED) {
        unreadable(source, &cause)
    } else if text.contains(CHANGED) {
        Error::new(
            ErrorCode::UnsupportedInput,
            "it changed while it was being packed",
        )
        .with_file(source)
        .with_fix("try again once nothing is writing to it")
    } else {
        not_writable(output, &format!("{} ({text})", source.display()))
    }
}

fn write_zip(
    out: BufWriter<File>,
    list: &[Entry],
    packing: &Packing,
    progress: &mut dyn FnMut(f32),
) -> Result<(), Error> {
    use ::zip::write::FullFileOptions;
    let output = packing.output;
    let mut w = ::zip::ZipWriter::new(out);
    for entry in list {
        let meta =
            std::fs::symlink_metadata(&entry.source).map_err(|e| unreadable(&entry.source, &e))?;
        let secs = mtime(&meta);
        let mut base = FullFileOptions::default()
            .compression_method(::zip::CompressionMethod::Deflated)
            .compression_level(Some(6))
            .last_modified_time(zip_time(secs));
        let fail = |e: &dyn std::fmt::Display| write_error(output, &entry.source, e);
        // An extended timestamp (0x5455) carries the time in UTC, to the
        // second, which other tools read before the local DOS time.
        if let Ok(secs) = u32::try_from(secs) {
            let mut field = vec![1u8];
            field.extend(secs.to_le_bytes());
            base.add_extra_field(0x5455, field, false)
                .map_err(|e| fail(&e))?;
        }
        match &entry.kind {
            Kind::Dir => w
                .add_directory(&entry.name, base.unix_permissions(0o755))
                .map_err(|e| fail(&e))?,
            Kind::Symlink(target) => w
                .add_symlink(
                    &entry.name,
                    target.to_string_lossy(),
                    base.unix_permissions(0o777),
                )
                .map_err(|e| fail(&e))?,
            Kind::File => {
                let options = base
                    .unix_permissions(platform::file_mode(&meta))
                    .large_file(meta.len() >= u64::from(u32::MAX));
                w.start_file(&entry.name, options).map_err(|e| fail(&e))?;
                let file = File::open(&entry.source).map_err(|e| unreadable(&entry.source, &e))?;
                let mut feed = Feed {
                    file,
                    packing,
                    progress: &mut *progress,
                };
                std::io::copy(&mut feed, &mut w).map_err(|e| fail(&e))?;
            }
        }
    }
    let mut out = w.finish().map_err(|e| not_writable(output, &e))?;
    out.flush().map_err(|e| not_writable(output, &e))
}

fn write_tar_gz(
    out: BufWriter<File>,
    list: &[Entry],
    packing: &Packing,
    progress: &mut dyn FnMut(f32),
) -> Result<(), Error> {
    let output = packing.output;
    let gz = flate2::GzBuilder::new()
        .mtime(0)
        .write(out, flate2::Compression::new(6));
    let mut tar = ::tar::Builder::new(gz);
    for entry in list {
        let meta =
            std::fs::symlink_metadata(&entry.source).map_err(|e| unreadable(&entry.source, &e))?;
        let fail = |e: &dyn std::fmt::Display| write_error(output, &entry.source, e);
        let mut header = ::tar::Header::new_gnu();
        header.set_mtime(mtime(&meta).max(0) as u64);
        header.set_uid(0);
        header.set_gid(0);
        match &entry.kind {
            Kind::Dir => {
                header.set_entry_type(::tar::EntryType::Directory);
                header.set_mode(0o755);
                header.set_size(0);
                tar.append_data(&mut header, &entry.name, std::io::empty())
                    .map_err(|e| fail(&e))?;
            }
            Kind::Symlink(target) => {
                header.set_entry_type(::tar::EntryType::Symlink);
                header.set_mode(0o777);
                header.set_size(0);
                tar.append_link(&mut header, &entry.name, target)
                    .map_err(|e| fail(&e))?;
            }
            Kind::File => {
                header.set_entry_type(::tar::EntryType::Regular);
                header.set_mode(platform::file_mode(&meta));
                header.set_size(meta.len());
                let file = File::open(&entry.source).map_err(|e| unreadable(&entry.source, &e))?;
                let feed = Feed {
                    file,
                    packing,
                    progress: &mut *progress,
                };
                tar.append_data(&mut header, &entry.name, Exact::new(feed, meta.len()))
                    .map_err(|e| fail(&e))?;
            }
        }
    }
    let gz = tar.into_inner().map_err(|e| not_writable(output, &e))?;
    let mut out = gz.finish().map_err(|e| not_writable(output, &e))?;
    out.flush().map_err(|e| not_writable(output, &e))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{assert_tree, extract, scratch};
    use super::{Feed, Packing, write_error};
    use crate::error::{Error, ErrorCode};
    use crate::exec::{CancelToken, Outcome, execute};
    use crate::format::FormatId;
    use crate::job::{Job, JobOptions};
    use crate::output::OutputPolicy;
    use crate::registry::{Operation, Registry};
    use persimmon_corpus::archives::write_archive_sample;
    use std::cell::Cell;
    use std::fs::File;
    use std::path::{Path, PathBuf};

    /// Pack `inputs` into `to` through the executor; the archive's path.
    fn pack_to(inputs: Vec<PathBuf>, to: FormatId, out: Option<&Path>) -> Result<PathBuf, Error> {
        let job = Job {
            op: Operation::Merge,
            inputs,
            target: Some(to),
            output: OutputPolicy {
                dir: out.map(Path::to_path_buf),
                overwrite: false,
                fallback: None,
            },
            options: JobOptions::default(),
        };
        let plan = job.plan(&Registry::builtin());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        let backend = super::super::ArchiveBackend;
        match execute(&job, &plan, &backend, &CancelToken::new(), &mut |_| {}).pop() {
            Some(Ok(Outcome::Written(p))) => Ok(p),
            Some(Err(e)) => Err(e),
            other => panic!("{other:?}"),
        }
    }

    fn zip_names(path: &Path) -> Vec<String> {
        let mut zip = ::zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        (0..zip.len())
            .map(|i| zip.by_index_raw(i).unwrap().name().unwrap().into_owned())
            .collect()
    }

    fn write(dir: &Path, name: &str, data: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn round_trip_keeps_the_tree() {
        let dir = scratch("round-trip");
        let unpacked = extract(&dir, write_archive_sample(&dir, "arc-tar")).unwrap();
        let tree = dir.join("tree");
        std::fs::rename(unpacked, &tree).unwrap();
        for to in [FormatId::Zip, FormatId::TarGz] {
            let packed = pack_to(vec![tree.clone()], to, None).unwrap();
            let out = extract(&dir, packed.clone()).unwrap();
            // merged.* holds a folder "tree": not the archive's own name,
            // so it isn't unwrapped.
            assert_tree(&out.join("tree"), true);
            std::fs::remove_file(packed).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_never_contains_itself() {
        let dir = scratch("itself");
        let photos = dir.join("photos");
        write(&photos, "a.txt", "a");
        write(&photos, "b.txt", "b");
        let packed = pack_to(vec![photos.clone()], FormatId::Zip, Some(&photos)).unwrap();
        assert_eq!(packed, photos.join("merged.zip"));
        assert_eq!(
            zip_names(&packed),
            ["photos/", "photos/a.txt", "photos/b.txt"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finder_litter_is_left_out() {
        let dir = scratch("litter");
        let folder = dir.join("notes");
        write(&folder, "a.txt", "a");
        write(&folder, ".DS_Store", "litter");
        write(&folder, "._a.txt", "litter");
        write(&folder, ".env", "kept");
        let packed = pack_to(vec![folder], FormatId::Zip, None).unwrap();
        assert_eq!(zip_names(&packed), ["notes/", "notes/.env", "notes/a.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn root_name_clash_is_numbered() {
        let dir = scratch("clash");
        let a = write(&dir.join("x"), "a.txt", "first");
        let b = write(&dir.join("y"), "a.txt", "second");
        let packed = pack_to(vec![a, b], FormatId::Zip, None).unwrap();
        assert_eq!(zip_names(&packed), ["a.txt", "a (1).txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tar_gz_has_no_owners() {
        let dir = scratch("owners");
        let folder = dir.join("f");
        write(&folder, "a.txt", "a");
        let packed = pack_to(vec![folder], FormatId::TarGz, None).unwrap();
        let gz = flate2::read::GzDecoder::new(std::fs::File::open(packed).unwrap());
        let mut tar = ::tar::Archive::new(gz);
        let mut n = 0;
        for entry in tar.entries().unwrap() {
            let entry = entry.unwrap();
            let h = entry.header();
            assert_eq!((h.uid().unwrap(), h.gid().unwrap()), (0, 0));
            assert!(h.username().unwrap().unwrap_or("").is_empty());
            assert!(h.groupname().unwrap().unwrap_or("").is_empty());
            n += 1;
        }
        assert_eq!(n, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_to_zip_and_tar_gz_end_to_end() {
        let dir = scratch("e2e");
        let a = write(&dir, "a.txt", "a");
        for (to, name) in [
            (FormatId::Zip, "merged.zip"),
            (FormatId::TarGz, "merged.tar.gz"),
        ] {
            let packed = pack_to(vec![a.clone()], to, None).unwrap();
            assert_eq!(packed, dir.join(name));
            let out = extract(&dir, packed).unwrap();
            assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "a");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A source that fails while it's being read is the source's fault,
    /// not the output folder's.
    #[test]
    fn source_read_error_names_the_source() {
        let dir = scratch("read-error");
        let source = write(&dir, "a.txt", "text");
        let output = dir.join("out.zip");
        let cancel = CancelToken::new();
        let packing = Packing {
            output: &output,
            done: Cell::new(0),
            total: 4,
            last: Cell::new(0.0),
            cancel: &cancel,
        };
        // Opened for writing only, so reading it fails.
        let file = File::options().write(true).open(&source).unwrap();
        let mut feed = Feed {
            file,
            packing: &packing,
            progress: &mut |_| {},
        };
        let e = std::io::copy(&mut feed, &mut std::io::sink()).unwrap_err();
        let e = write_error(&output, &source, &e);
        assert_eq!(e.code, ErrorCode::UnsupportedInput, "{}", e.reason);
        assert!(e.reason.starts_with("can't read this file"), "{}", e.reason);
        assert_eq!(e.file.as_deref(), Some(source.as_path()));
        assert_eq!(e.fix, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_input_names_it() {
        let dir = scratch("unreadable");
        let missing = dir.join("gone.txt");
        let e = super::entries(std::slice::from_ref(&missing), &[]).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput);
        assert!(e.reason.starts_with("can't read this file"), "{}", e.reason);
        assert_eq!(e.file.as_deref(), Some(missing.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn private_files_stay_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("private");
        let key = write(&dir.join("keys"), "id", "secret");
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o4600)).unwrap();
        let packed = pack_to(vec![key], FormatId::Zip, None).unwrap();
        let mut zip = ::zip::ZipArchive::new(std::fs::File::open(packed).unwrap()).unwrap();
        let mode = zip.by_index_raw(0).unwrap().unix_mode().unwrap() & 0o7777;
        assert_eq!(mode, 0o600, "kept private, set-uid dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn special_files_are_skipped() {
        let dir = scratch("fifo");
        let folder = dir.join("in");
        write(&folder, "a.txt", "a");
        let status = std::process::Command::new("mkfifo")
            .arg(folder.join("pipe"))
            .status()
            .unwrap();
        assert!(status.success());
        let (tx, rx) = std::sync::mpsc::channel();
        let folder2 = folder.clone();
        std::thread::spawn(move || {
            let _ = tx.send(pack_to(vec![folder2], FormatId::Zip, None).map(|p| zip_names(&p)));
        });
        let names = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("packing a folder with a FIFO finishes")
            .unwrap();
        assert_eq!(names, ["in/", "in/a.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exact_reads_the_stated_length() {
        use std::io::Read;
        let mut out = Vec::new();
        super::Exact::new(&b"abcdef"[..], 4)
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out, b"abcd", "a file that grew is cut at its stated size");
        let e = super::Exact::new(&b"ab"[..], 4)
            .read_to_end(&mut Vec::new())
            .unwrap_err();
        assert!(e.to_string().contains(super::CHANGED), "{e}");
    }

    #[test]
    fn packed_zip_carries_utc_time() {
        let dir = scratch("utc");
        let f = write(&dir, "a.txt", "a");
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_767_323_047);
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(when)
            .unwrap();
        let packed = pack_to(vec![f], FormatId::Zip, None).unwrap();
        let mut zip = ::zip::ZipArchive::new(std::fs::File::open(packed).unwrap()).unwrap();
        let entry = zip.by_index(0).unwrap();
        let utc = entry.extra_data_fields().find_map(|f| match f {
            ::zip::extra_fields::ExtraField::ExtendedTimestamp(t) => t.mod_time(),
            _ => None,
        });
        assert_eq!(
            utc,
            Some(1_767_323_047),
            "an odd second, which DOS times can't hold"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
