// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Where output goes, what it's called, and how it lands (DD-31).
//!
//! Output goes next to the source unless `--out` says otherwise. Nothing is
//! ever overwritten without `--overwrite`: a clash becomes `stem (1).ext`.
//! Every output is written to a hidden `.part` file in the destination
//! folder and renamed into place only when complete, so a cancelled or
//! crashed job never leaves a half-written file under the final name.

use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, ErrorCode, FixKind};

/// The longest file name, in bytes, that APFS, ext4 and NTFS all accept.
const NAME_MAX: usize = 255;

const NOT_WRITABLE: &str = "can't write to this folder";
const OUT_FIX: &str = "choose another folder with --out <dir>";

/// Where a job's outputs go.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, rename_all = "camelCase"))]
pub struct OutputPolicy {
    /// `--out <dir>`: write here instead of next to each source.
    #[cfg_attr(
        feature = "serde",
        serde(serialize_with = "crate::serde_util::opt_path")
    )]
    pub dir: Option<PathBuf>,
    /// `--overwrite`: replace existing files instead of numbering around them.
    /// Never deserialised: a client such as the app can't turn it on.
    #[cfg_attr(feature = "serde", serde(skip_deserializing))]
    pub overwrite: bool,
}

impl OutputPolicy {
    /// The folder the output for `input` goes in. With `--out`, the folder
    /// is created if it's missing. Either way it must be writable.
    pub fn output_dir(&self, input: &Path) -> Result<PathBuf, Error> {
        let dir = self.preview_dir(input)?;
        if self.dir.is_some() {
            std::fs::create_dir_all(&dir).map_err(|e| {
                Error::new(
                    ErrorCode::OutputNotWritable,
                    format!("can't create this folder ({e})"),
                )
                .with_file(&dir)
                .with_fix_as(FixKind::ChooseOutputFolder, OUT_FIX)
            })?;
        }
        ensure_writable(&dir)?;
        Ok(dir)
    }

    /// The folder the output for `input` would go in, touching nothing: no
    /// folder is created and nothing checks that it's writable. Only an
    /// `--out` that is an existing file is an error here.
    pub fn preview_dir(&self, input: &Path) -> Result<PathBuf, Error> {
        match &self.dir {
            Some(dir) if dir.exists() && !dir.is_dir() => Err(Error::new(
                ErrorCode::OutputNotWritable,
                "it's a file, not a folder",
            )
            .with_file(dir)
            .with_fix_as(FixKind::ChooseOutputFolder, OUT_FIX)),
            Some(dir) => Ok(dir.clone()),
            None => Ok(match input.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
                _ => PathBuf::from("."),
            }),
        }
    }
}

/// Check that files can be created in `dir`, by creating and removing a
/// probe file. A folder that refuses (Photos and Mail containers, read-only
/// volumes) is an error naming `--out` as the fix.
pub fn ensure_writable(dir: &Path) -> Result<(), Error> {
    let probe = dir.join(format!(".prsm-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => {
            let reason = match e.kind() {
                ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
                    NOT_WRITABLE.to_string()
                }
                _ => format!("{NOT_WRITABLE} ({e})"),
            };
            Err(Error::new(ErrorCode::OutputNotWritable, reason)
                .with_file(dir)
                .with_fix_as(FixKind::ChooseOutputFolder, OUT_FIX))
        }
    }
}

/// The identity of a path for clash checks. Default APFS and NTFS treat
/// names that differ only in case or in Unicode normalisation (NFC vs NFD)
/// as one file, so the key folds both, after resolving the folder. On a
/// case-sensitive volume this only ever adds a needless counter.
fn clash_key(path: &Path) -> PathBuf {
    let fold = |s: &str| s.nfc().collect::<String>().to_lowercase();
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    let name = path
        .file_name()
        .map(|n| fold(&n.to_string_lossy()))
        .unwrap_or_default();
    PathBuf::from(fold(&dir.to_string_lossy())).join(name)
}

/// Mark `path` as taken, so no output in this batch is given its name.
/// Planning reserves every input this way: `--overwrite` replaces earlier
/// outputs, never the file being read.
pub fn reserve(taken: &mut HashSet<PathBuf>, path: &Path) {
    taken.insert(clash_key(path));
}

/// A free output path in `dir`: `stem.ext`, else the first `stem (n).ext`
/// that is neither on disk nor already `taken` by this batch (compared
/// ignoring case and normalisation). The result is added to `taken`. With
/// `overwrite`, files on disk don't count as clashes, but names taken by
/// the batch still do. An empty `ext` (an extracted folder) gives a name
/// without a dot. A stem too long for a 255-byte name is shortened.
pub fn free_name(
    dir: &Path,
    stem: &str,
    ext: &str,
    overwrite: bool,
    taken: &mut HashSet<PathBuf>,
) -> PathBuf {
    let name = |suffix: String| {
        let tail = suffix.len() + if ext.is_empty() { 0 } else { ext.len() + 1 };
        let mut cut = stem.len().min(NAME_MAX.saturating_sub(tail));
        while !stem.is_char_boundary(cut) {
            cut -= 1;
        }
        let stem = &stem[..cut];
        if ext.is_empty() {
            dir.join(format!("{stem}{suffix}"))
        } else {
            dir.join(format!("{stem}{suffix}.{ext}"))
        }
    };
    let is_free = |p: &PathBuf| {
        !taken.contains(&clash_key(p)) && (overwrite || std::fs::symlink_metadata(p).is_err())
    };
    let mut candidate = name(String::new());
    let mut n = 1u32;
    while !is_free(&candidate) {
        candidate = name(format!(" ({n})"));
        n += 1;
    }
    reserve(taken, &candidate);
    candidate
}

/// An output being written. Write to [`temp_path`](Self::temp_path), then
/// [`commit`](Self::commit). Dropped without a commit, it removes the
/// temporary file or folder.
#[derive(Debug)]
pub struct PendingOutput {
    dest: PathBuf,
    temp: PathBuf,
    committed: bool,
}

impl PendingOutput {
    /// Prepare to write `dest`. The temporary path is
    /// `{dir}/.prsm-{pid}-{n}.part`: in the same folder, so the final rename
    /// never crosses volumes, and short, so it fits wherever `dest` does.
    pub fn new(dest: PathBuf) -> PendingOutput {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let temp = dest.with_file_name(format!(".prsm-{}-{n}.part", std::process::id()));
        PendingOutput {
            dest,
            temp,
            committed: false,
        }
    }

    pub fn temp_path(&self) -> &Path {
        &self.temp
    }

    /// Move the finished output into place. Without `overwrite`, an
    /// existing destination is left untouched and the result is
    /// `OUTPUT_EXISTS`.
    pub fn commit(mut self, overwrite: bool) -> Result<PathBuf, Error> {
        if !overwrite {
            let exists = || {
                Error::new(ErrorCode::OutputExists, "already exists")
                    .with_file(&self.dest)
                    .with_fix_as(FixKind::Overwrite, "use --overwrite to replace it")
            };
            if std::fs::symlink_metadata(&self.temp).is_ok_and(|m| m.is_file()) {
                // A hard link refuses to replace an existing file, atomically.
                match std::fs::hard_link(&self.temp, &self.dest) {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&self.temp);
                        self.committed = true;
                        return Ok(self.dest.clone());
                    }
                    Err(e) if e.kind() == ErrorKind::AlreadyExists => return Err(exists()),
                    // Volumes without hard links fall through to check-then-rename.
                    Err(_) => {}
                }
            }
            if std::fs::symlink_metadata(&self.dest).is_ok() {
                return Err(exists());
            }
        }
        std::fs::rename(&self.temp, &self.dest).map_err(|e| {
            Error::new(
                ErrorCode::OutputNotWritable,
                format!("couldn't move the finished file into place ({e})"),
            )
            .with_file(&self.dest)
            .with_fix_as(FixKind::ChooseOutputFolder, OUT_FIX)
        })?;
        self.committed = true;
        Ok(self.dest.clone())
    }
}

impl Drop for PendingOutput {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        match std::fs::symlink_metadata(&self.temp) {
            Ok(m) if m.is_dir() => {
                let _ = std::fs::remove_dir_all(&self.temp);
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&self.temp);
            }
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-output-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn free_name_appends_counter() {
        let dir = scratch("counter");
        std::fs::write(dir.join("a.webp"), b"x").unwrap();
        let mut taken = HashSet::new();
        assert_eq!(
            free_name(&dir, "a", "webp", false, &mut taken),
            dir.join("a (1).webp")
        );
    }

    #[test]
    fn free_name_respects_taken() {
        let dir = scratch("taken");
        let mut taken = HashSet::new();
        assert_eq!(
            free_name(&dir, "a", "webp", false, &mut taken),
            dir.join("a.webp")
        );
        assert_eq!(
            free_name(&dir, "a", "webp", false, &mut taken),
            dir.join("a (1).webp")
        );
    }

    #[test]
    fn free_name_overwrite_skips_disk_but_not_taken() {
        let dir = scratch("overwrite");
        std::fs::write(dir.join("a.webp"), b"x").unwrap();
        let mut taken = HashSet::new();
        assert_eq!(
            free_name(&dir, "a", "webp", true, &mut taken),
            dir.join("a.webp")
        );
        assert_eq!(
            free_name(&dir, "a", "webp", true, &mut taken),
            dir.join("a (1).webp")
        );
    }

    #[test]
    fn free_name_folds_case_and_normalisation() {
        let dir = scratch("fold");
        let mut taken = HashSet::new();
        assert_eq!(
            free_name(&dir, "Snap", "webp", false, &mut taken),
            dir.join("Snap.webp")
        );
        assert_eq!(
            free_name(&dir, "snap", "webp", false, &mut taken),
            dir.join("snap (1).webp")
        );
        assert_eq!(
            free_name(&dir, "caf\u{e9}", "webp", false, &mut taken),
            dir.join("caf\u{e9}.webp")
        );
        assert_eq!(
            free_name(&dir, "cafe\u{301}", "webp", false, &mut taken),
            dir.join("cafe\u{301} (1).webp")
        );
    }

    #[test]
    fn reserved_inputs_are_never_output_names() {
        let dir = scratch("reserve");
        std::fs::write(dir.join("photo.jpg"), b"x").unwrap();
        let mut taken = HashSet::new();
        reserve(&mut taken, &dir.join("photo.jpg"));
        assert_eq!(
            free_name(&dir, "photo", "jpg", true, &mut taken),
            dir.join("photo (1).jpg")
        );
    }

    #[test]
    fn free_name_fits_the_name_limit() {
        let dir = scratch("long");
        let mut taken = HashSet::new();
        let stem = "\u{e9}".repeat(126); // 252 bytes
        let first = free_name(&dir, &stem, "webp", false, &mut taken);
        let second = free_name(&dir, &stem, "webp", false, &mut taken);
        for p in [&first, &second] {
            let name = p.file_name().unwrap().to_str().unwrap();
            assert!(name.len() <= 255, "{} bytes", name.len());
            assert!(name.ends_with(".webp"));
        }
        assert!(
            second
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with(" (1).webp")
        );
        assert_ne!(first, second);
    }

    #[test]
    fn pending_temp_name_is_short_and_works_for_long_names() {
        let dir = scratch("long-temp");
        let dest = dir.join(format!("{}.webp", "a".repeat(250)));
        let pending = PendingOutput::new(dest.clone());
        assert!(pending.temp_path().file_name().unwrap().len() < 64);
        std::fs::write(pending.temp_path(), b"done").unwrap();
        assert_eq!(pending.commit(false).unwrap(), dest);
        let other = PendingOutput::new(dest.clone());
        assert_ne!(other.temp_path(), PendingOutput::new(dest).temp_path());
    }

    #[test]
    fn free_name_folder_without_ext() {
        let dir = scratch("folder");
        std::fs::create_dir(dir.join("photos")).unwrap();
        let mut taken = HashSet::new();
        assert_eq!(
            free_name(&dir, "photos", "", false, &mut taken),
            dir.join("photos (1)")
        );
    }

    #[test]
    fn out_dir_is_created() {
        let dir = scratch("outdir");
        let input = dir.join("a.png");
        let policy = OutputPolicy {
            dir: Some(dir.join("new/nested")),
            ..Default::default()
        };
        assert_eq!(policy.output_dir(&input).unwrap(), dir.join("new/nested"));
        assert!(dir.join("new/nested").is_dir());
    }

    #[test]
    fn default_output_dir_is_the_inputs_folder() {
        let dir = scratch("default");
        let input = dir.join("a.png");
        assert_eq!(OutputPolicy::default().output_dir(&input).unwrap(), dir);
        assert_eq!(
            OutputPolicy::default()
                .output_dir(Path::new("a.png"))
                .unwrap(),
            Path::new(".")
        );
    }

    #[test]
    fn pending_dropped_leaves_nothing() {
        let dir = scratch("drop");
        let dest = dir.join("out.webp");
        let pending = PendingOutput::new(dest.clone());
        let temp = pending.temp_path().to_path_buf();
        assert_eq!(temp.parent(), Some(dir.as_path()));
        let name = temp.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with(&format!(".prsm-{}-", std::process::id())),
            "{name}"
        );
        assert!(name.ends_with(".part"), "{name}");
        std::fs::write(&temp, b"half").unwrap();
        drop(pending);
        assert!(!temp.exists());
        assert!(!dest.exists());
    }

    #[test]
    fn pending_commit_moves_into_place() {
        let dir = scratch("commit");
        let dest = dir.join("out.webp");
        let pending = PendingOutput::new(dest.clone());
        std::fs::write(pending.temp_path(), b"done").unwrap();
        assert_eq!(pending.commit(false).unwrap(), dest);
        assert_eq!(std::fs::read(&dest).unwrap(), b"done");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn pending_commit_never_clobbers() {
        let dir = scratch("clobber");
        let dest = dir.join("out.webp");
        std::fs::write(&dest, b"original").unwrap();
        let pending = PendingOutput::new(dest.clone());
        std::fs::write(pending.temp_path(), b"new").unwrap();
        let e = pending.commit(false).unwrap_err();
        assert_eq!(e.code, ErrorCode::OutputExists);
        assert_eq!(std::fs::read(&dest).unwrap(), b"original");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "no .part file remains"
        );
    }

    #[test]
    fn pending_commit_overwrite_replaces() {
        let dir = scratch("replace");
        let dest = dir.join("out.webp");
        std::fs::write(&dest, b"original").unwrap();
        let pending = PendingOutput::new(dest.clone());
        std::fs::write(pending.temp_path(), b"new").unwrap();
        pending.commit(true).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn unwritable_dir_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("readonly");
        let locked = dir.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = ensure_writable(&locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = result.unwrap_err();
        assert_eq!(e.code, ErrorCode::OutputNotWritable);
        assert_eq!(e.file.as_deref(), Some(locked.as_path()));
        assert_eq!(e.reason, "can't write to this folder");
        assert_eq!(e.fix_text(), Some("choose another folder with --out <dir>"));
    }
}
