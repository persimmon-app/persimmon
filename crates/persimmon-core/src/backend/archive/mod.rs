// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The archive backend (DD-37): extracting ZIP, TAR (plain, gz, bz2, xz,
//! zst), 7z and RAR, and packing ZIP and tar.gz. Every reader feeds one
//! [`sink::Sink`], the only code that writes extracted files, into the
//! executor's temporary folder; a failed, unsafe or cancelled extraction
//! leaves nothing.

#[cfg(feature = "rar")]
mod rar;
#[cfg(any(feature = "rar", test))]
mod rar_scan;
mod sevenz;
pub(crate) mod sink;
mod tar;
mod zip;

use std::cell::Cell;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::rc::Rc;

use crate::backend::{Done, Run};
use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;
use crate::format::FormatId;
use crate::job::{JobOptions, PlannedTask, stem_of};
use crate::registry::Registry;
use sink::Sink;

/// Extracts and packs archives.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArchiveBackend;

impl Run for ArchiveBackend {
    fn run(
        &self,
        task: &PlannedTask,
        _options: &JobOptions,
        temp: &Path,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(f32),
    ) -> Result<Done, Error> {
        if task.to.is_some() {
            return Err(Error::new(
                ErrorCode::NotImplemented,
                "packing archives isn't built yet",
            ));
        }
        let input = &task.inputs[0];
        let from = task.from[0];
        let mut sink = Sink::new(temp.to_path_buf(), input.clone());
        match from {
            FormatId::Zip => zip::extract_zip(input, &mut sink, cancel, progress)?,
            FormatId::Tar
            | FormatId::TarGz
            | FormatId::TarBz2
            | FormatId::TarXz
            | FormatId::TarZst => tar::extract_tar(input, from, &mut sink, cancel, progress)?,
            FormatId::SevenZ => sevenz::extract_7z(input, &mut sink, cancel, progress)?,
            #[cfg(feature = "rar")]
            FormatId::Rar => rar::extract_rar(input, &mut sink, cancel, progress)?,
            #[cfg(not(feature = "rar"))]
            FormatId::Rar => {
                return Err(
                    Error::new(ErrorCode::BackendMissing, "this build can't read RAR")
                        .with_file(input)
                        .with_fix(
                            "install libarchive (brew install libarchive, or libarchive-dev), \
                             or use a build of prsm made with it",
                        ),
                );
            }
            other => {
                return Err(Error::new(
                    ErrorCode::NotImplemented,
                    format!("extracting {} isn't built yet", other.as_str()),
                )
                .with_file(input));
            }
        }
        sink.finish(&stem_of(input, &Registry::builtin()))?;
        progress(1.0);
        Ok(Done::Written)
    }
}

/// `CORRUPT_INPUT` for a reader's failure.
pub(crate) fn unreadable(input: &Path, e: &dyn std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::CorruptInput,
        format!("can't read this archive ({e})"),
    )
    .with_file(input)
}

/// `UNSUPPORTED_INPUT` for an encrypted archive (decision 6).
pub(crate) fn password_protected(input: &Path) -> Error {
    Error::new(ErrorCode::UnsupportedInput, "it's password-protected")
        .with_file(input)
        .with_fix("extract it with the app that made it")
}

/// A reader that counts the bytes read through it, for progress. Seeking
/// passes through, so ZIP's random access works too.
pub(crate) struct Counting<R> {
    inner: R,
    count: Rc<Cell<u64>>,
}

impl<R> Counting<R> {
    pub(crate) fn new(inner: R) -> (Counting<R>, Rc<Cell<u64>>) {
        let count = Rc::new(Cell::new(0));
        (
            Counting {
                inner,
                count: count.clone(),
            },
            count,
        )
    }
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.set(self.count.get() + n as u64);
        Ok(n)
    }
}

impl<R: Seek> Seek for Counting<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// Reports progress as the archive is read: `count / size`, at most
/// every 1%, never going back. Wraps an entry's data on its way to the
/// sink, so progress moves within large files.
pub(crate) struct Meter<'p, R> {
    inner: R,
    count: Rc<Cell<u64>>,
    size: u64,
    last: &'p Cell<f32>,
    progress: &'p mut dyn FnMut(f32),
}

impl<'p, R> Meter<'p, R> {
    pub(crate) fn new(
        inner: R,
        count: Rc<Cell<u64>>,
        size: u64,
        last: &'p Cell<f32>,
        progress: &'p mut dyn FnMut(f32),
    ) -> Meter<'p, R> {
        Meter {
            inner,
            count,
            size,
            last,
            progress,
        }
    }

    pub(crate) fn tick(&mut self) {
        let f = (self.count.get() as f32 / self.size.max(1) as f32).min(1.0);
        if f >= self.last.get() + 0.01 {
            self.last.set(f);
            (self.progress)(f);
        }
    }
}

impl<R: Read> Read for Meter<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.tick();
        Ok(n)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::backend::Run;
    use crate::error::ErrorCode;
    use crate::exec::{CancelToken, execute};
    use crate::format::FormatId;
    use crate::job::{Job, JobOptions};
    use crate::output::OutputPolicy;
    use crate::registry::{Operation, Registry};
    use persimmon_corpus::archives::{MTIME, TREE, TreeItem, write_archive_sample};
    use std::path::{Path, PathBuf};

    pub(crate) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-arc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn extract_job(input: PathBuf) -> Job {
        Job {
            op: Operation::Extract,
            inputs: vec![input],
            target: None,
            output: OutputPolicy::default(),
            options: JobOptions::default(),
        }
    }

    /// Extract `input` with the backend into `dir/out`; the result.
    pub(crate) fn extract(dir: &Path, input: PathBuf) -> Result<PathBuf, Error> {
        let job = extract_job(input);
        let plan = job.plan(&Registry::builtin());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        let out = dir.join("out");
        let _ = std::fs::remove_dir_all(&out);
        ArchiveBackend
            .run(
                &plan.tasks[0],
                &job.options,
                &out,
                &CancelToken::new(),
                &mut |_| {},
            )
            .map(|_| out)
    }

    /// Extract through the executor, as prsm does; the error, if any.
    fn execute_extract(input: PathBuf) -> Option<Error> {
        let job = extract_job(input);
        let plan = job.plan(&Registry::builtin());
        let mut results = execute(
            &job,
            &plan,
            &ArchiveBackend,
            &CancelToken::new(),
            &mut |_| {},
        );
        results.pop().unwrap().err()
    }

    fn mtime(p: &Path) -> u64 {
        std::fs::symlink_metadata(p)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    /// `out` holds exactly `TREE` (without the symlink when `link` is false).
    pub(crate) fn assert_tree(out: &Path, link: bool) {
        for (name, item) in TREE {
            let p = out.join(name);
            match *item {
                TreeItem::File(data, mode) => {
                    assert_eq!(std::fs::read(&p).unwrap(), data, "{name}");
                    assert_eq!(mtime(&p), MTIME, "{name} mtime");
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let got = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
                        assert_eq!(got, mode, "{name} mode");
                    }
                    let _ = mode;
                }
                TreeItem::Dir => {
                    assert!(p.is_dir(), "{name}");
                    assert_eq!(mtime(&p), MTIME, "{name} mtime");
                }
                TreeItem::Symlink(target) if link => {
                    #[cfg(unix)]
                    assert_eq!(std::fs::read_link(&p).unwrap(), Path::new(target), "{name}");
                    let _ = target;
                }
                TreeItem::Symlink(_) => assert!(std::fs::symlink_metadata(&p).is_err()),
            }
        }
        let count = |d: &Path| std::fs::read_dir(d).unwrap().count();
        let top = TREE
            .iter()
            .filter(|(n, i)| !n.contains('/') && (link || !matches!(i, TreeItem::Symlink(_))));
        assert_eq!(count(out), top.count(), "nothing extra at the top");
    }

    #[test]
    fn every_tree_archive_extracts() {
        let dir = scratch("trees");
        for id in [
            "arc-zip", "arc-tar", "arc-tgz", "arc-tbz2", "arc-txz", "arc-tzst",
        ] {
            let input = write_archive_sample(&dir, id);
            let out = extract(&dir, input).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_tree(&out, true);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hostile_archives_write_nothing() {
        let dir = scratch("hostile");
        let mut cases = vec![
            ("arc-slip", "../evil.txt"),
            ("arc-abs", "/tmp/prsm-evil-abs.txt"),
            ("arc-hard-out", "/etc/passwd"),
        ];
        if cfg!(unix) {
            cases.push(("arc-through", "link/prsm-evil-through.txt"));
        }
        for (id, name) in cases {
            let input = write_archive_sample(&dir, id);
            let e = execute_extract(input).unwrap_or_else(|| panic!("{id} extracted"));
            assert_eq!(e.code, ErrorCode::CorruptInput, "{id}: {e}");
            assert!(e.reason.contains(name), "{id}: {}", e.reason);
            let left: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .filter(|n| !n.to_string_lossy().contains('.'))
                .collect();
            assert!(left.is_empty(), "{id} left {left:?}");
            assert!(
                std::fs::read_dir(&dir).unwrap().all(|e| !e
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".part"))
            );
        }
        for f in ["prsm-evil-abs.txt", "prsm-evil-through.txt"] {
            assert!(!Path::new("/tmp").join(f).exists(), "{f}");
        }
        assert!(!dir.parent().unwrap().join("evil.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_folder_unwraps() {
        let dir = scratch("nested");
        let out = extract(&dir, write_archive_sample(&dir, "arc-nested")).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "a\n");
        assert!(!out.join("photos").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hard_and_dup() {
        let dir = scratch("hard-dup");
        let out = extract(&dir, write_archive_sample(&dir, "arc-hard")).unwrap();
        assert_eq!(std::fs::read(out.join("b.txt")).unwrap(), b"shared\n");
        let out = extract(&dir, write_archive_sample(&dir, "arc-dup")).unwrap();
        assert_eq!(std::fs::read(out.join("x.txt")).unwrap(), b"second\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locked_zip_is_password_protected() {
        let dir = scratch("locked-zip");
        let e = extract(&dir, write_archive_sample(&dir, "arc-locked-zip")).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput);
        assert_eq!(e.reason, "it's password-protected");
        assert_eq!(
            e.fix.as_deref(),
            Some("extract it with the app that made it")
        );
        assert!(!dir.join("out/secret.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_gz_is_not_a_tar() {
        let dir = scratch("plain-gz");
        let e = extract(&dir, write_archive_sample(&dir, "arc-plain-gz")).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput);
        assert_eq!(e.reason, "it's a compressed file, not a tar archive");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Cut `path` to `percent` of its bytes.
    pub(crate) fn truncate(path: &Path, percent: usize) {
        let bytes = std::fs::read(path).unwrap();
        std::fs::write(path, &bytes[..bytes.len() * percent / 100]).unwrap();
    }

    #[test]
    fn truncated_archives_leave_nothing() {
        let dir = scratch("truncated");
        for id in ["arc-tgz", "arc-zip", "arc-big"] {
            let input = write_archive_sample(&dir, id);
            truncate(&input, 60);
            let e = execute_extract(input.clone()).unwrap_or_else(|| panic!("{id} extracted"));
            assert_eq!(e.code, ErrorCode::CorruptInput, "{id}: {e}");
            let names: Vec<String> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(names.len(), 1, "{id} left {names:?}");
            std::fs::remove_file(input).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_and_cancel() {
        let dir = scratch("progress");
        let input = write_archive_sample(&dir, "arc-big");
        let job = extract_job(input);
        let plan = job.plan(&Registry::builtin());
        let mut seen = Vec::new();
        ArchiveBackend
            .run(
                &plan.tasks[0],
                &job.options,
                &dir.join("out"),
                &CancelToken::new(),
                &mut |f| seen.push(f),
            )
            .unwrap();
        assert!(seen.len() > 2, "{seen:?}");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert_eq!(*seen.last().unwrap(), 1.0);

        let cancel = CancelToken::new();
        let token = cancel.clone();
        let e = ArchiveBackend
            .run(
                &plan.tasks[0],
                &job.options,
                &dir.join("out2"),
                &cancel,
                &mut |_| token.cancel(),
            )
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_folder_is_never_replaced() {
        let dir = scratch("never-replace");
        let input = write_archive_sample(&dir, "arc-tgz");
        std::fs::create_dir(dir.join("tree")).unwrap();
        let mut job = extract_job(input);
        job.output.overwrite = true;
        let plan = job.plan(&Registry::builtin());
        assert_eq!(plan.tasks[0].output, dir.join("tree (1)"));
        assert_eq!(plan.tasks[0].from, vec![FormatId::TarGz]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sevenz_extracts_the_tree() {
        let dir = scratch("7z");
        let out = extract(&dir, write_archive_sample(&dir, "arc-7z")).unwrap();
        assert_tree(&out, false);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locked_7z_is_password_protected() {
        let dir = scratch("locked-7z");
        let e = extract(&dir, write_archive_sample(&dir, "arc-locked-7z")).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput, "{e}");
        assert_eq!(e.reason, "it's password-protected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncated_7z_leaves_nothing() {
        let dir = scratch("truncated-7z");
        let input = write_archive_sample(&dir, "arc-7z");
        truncate(&input, 60);
        let e = execute_extract(input).expect("a truncated 7z fails");
        assert_eq!(e.code, ErrorCode::CorruptInput, "{e}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Copy the committed RAR sample `id` into `dir`.
    fn rar(dir: &Path, id: &str) -> PathBuf {
        let (_, name, bytes) = persimmon_corpus::RAR_SAMPLES
            .iter()
            .find(|s| s.0 == id)
            .unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[cfg(feature = "rar")]
    #[test]
    fn rar4_and_rar5_extract() {
        let dir = scratch("rar");
        let out = extract(&dir, rar(&dir, "rar4")).unwrap();
        let text = b"test text document\r\n".as_slice();
        assert_eq!(std::fs::read(out.join("test.txt")).unwrap(), text);
        assert_eq!(std::fs::read(out.join("testdir/test.txt")).unwrap(), text);
        assert!(out.join("testemptydir").is_dir());
        // RAR links are skipped: the binding doesn't expose their targets.
        assert!(std::fs::symlink_metadata(out.join("testlink")).is_err());
        let out = extract(&dir, rar(&dir, "rar5")).unwrap();
        assert_eq!(
            std::fs::read(out.join("helloworld.txt")).unwrap(),
            b"hello libarchive test suite!\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "rar")]
    #[test]
    fn rar5_unicode_names() {
        let dir = scratch("rar-unicode");
        let out = extract(&dir, rar(&dir, "rar5-unicode")).unwrap();
        assert_eq!(
            std::fs::read(out.join("\u{1f44b}\u{1f30e}.txt")).unwrap(),
            b"Hello, world!"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "rar")]
    #[test]
    fn locked_rar_is_password_protected() {
        let dir = scratch("rar-locked");
        let e = extract(&dir, rar(&dir, "rar4-locked")).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput, "{e}");
        assert_eq!(e.reason, "it's password-protected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(feature = "rar"))]
    #[test]
    fn rar_without_libarchive_is_backend_missing() {
        let dir = scratch("rar-missing");
        let e = extract(&dir, rar(&dir, "rar5")).unwrap_err();
        assert_eq!(e.code, ErrorCode::BackendMissing);
        assert_eq!(e.reason, "this build can't read RAR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rar_encryption_is_detected_from_headers() {
        for (id, _, bytes) in persimmon_corpus::RAR_SAMPLES {
            assert_eq!(
                rar_scan::encrypted(&mut std::io::Cursor::new(bytes)),
                id == "rar4-locked",
                "{id}"
            );
        }
    }
}
