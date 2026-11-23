// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Moving an original after its output is safely written (a watch rule's
//! "then move to done"), and moving it back on undo. Neither ever replaces
//! a file: a clash in the done folder is numbered, and a move back refuses
//! when the old place is taken or the file has changed.

use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, ErrorCode};
use crate::output::free_name;
use crate::watch::Stamp;

/// An original that was moved.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Moved {
    pub from: PathBuf,
    pub to: PathBuf,
    /// The file as it was at `to` right after the move.
    pub stamp: Stamp,
}

/// Why a moved original wasn't put back.
#[derive(Debug)]
pub enum NotRestored {
    /// Something else is at its old place now.
    Taken,
    /// It changed since it was moved.
    Changed,
    /// It's no longer where it was moved to.
    Gone,
    /// Moving it failed.
    Failed(Error),
}

/// Move `original`, found under `root`, into `done`, mirroring the
/// subfolder it came from. A clash is numbered, as an output's would be.
pub fn move_original(original: &Path, root: &Path, done: &Path) -> Result<Moved, Error> {
    let failed = |e: std::io::Error| {
        Error::new(
            ErrorCode::OutputNotWritable,
            format!("couldn't move the original to the done folder ({e})"),
        )
        .with_file(original)
    };
    let sub = original
        .parent()
        .and_then(|p| p.strip_prefix(root).ok())
        .unwrap_or(Path::new(""));
    let dir = done.join(sub);
    std::fs::create_dir_all(&dir).map_err(failed)?;
    let name = Path::new(original.file_name().unwrap_or_default());
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let ext = name
        .extension()
        .map(|e| e.to_string_lossy())
        .unwrap_or_default();
    let mut taken = HashSet::new();
    // A name taken between choosing it and moving there is skipped.
    for _ in 0..100 {
        let to = free_name(&dir, &stem, &ext, false, &mut taken);
        match transfer(original, &to) {
            Ok(()) => {
                let meta = std::fs::metadata(&to).map_err(failed)?;
                let stamp = Stamp::of(&meta)
                    .ok_or_else(|| failed(std::io::Error::other("no modification time")))?;
                return Ok(Moved {
                    from: original.to_path_buf(),
                    to,
                    stamp,
                });
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(failed(e)),
        }
    }
    Err(failed(std::io::Error::other("no free name")))
}

/// Put a moved original back where it was.
pub fn restore(moved: &Moved) -> Result<(), NotRestored> {
    let now = match std::fs::metadata(&moved.to) {
        Ok(meta) => Stamp::of(&meta),
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(NotRestored::Gone),
        Err(e) => return Err(NotRestored::Failed(restore_failed(moved, e))),
    };
    if now != Some(moved.stamp) {
        return Err(NotRestored::Changed);
    }
    if std::fs::symlink_metadata(&moved.from).is_ok() {
        return Err(NotRestored::Taken);
    }
    if let Some(parent) = moved.from.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| NotRestored::Failed(restore_failed(moved, e)))?;
    }
    match transfer(&moved.to, &moved.from) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(NotRestored::Taken),
        Err(e) => Err(NotRestored::Failed(restore_failed(moved, e))),
    }
}

fn restore_failed(moved: &Moved, e: std::io::Error) -> Error {
    Error::new(
        ErrorCode::OutputNotWritable,
        format!("couldn't move the original back ({e})"),
    )
    .with_file(&moved.from)
}

/// Move `from` to `to`, never replacing an existing `to`: a hard link
/// refuses atomically; another volume, or one without hard links, copies.
fn transfer(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(from, to) {
        Ok(()) => std::fs::remove_file(from).inspect_err(|_| {
            let _ = std::fs::remove_file(to);
        }),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(e),
        Err(_) => move_by_copy(from, to),
    }
}

/// Move by copying (for another volume), never replacing `to`.
fn move_by_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(ErrorKind::AlreadyExists.into());
    }
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let temp = to.with_file_name(format!(".prsm-{}-m{n}.part", std::process::id()));
    let placed = std::fs::copy(from, &temp).and_then(|_| {
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(ErrorKind::AlreadyExists.into());
        }
        std::fs::rename(&temp, to)
    });
    if let Err(e) = placed {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    std::fs::remove_file(from).inspect_err(|_| {
        let _ = std::fs::remove_file(to);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            let d =
                std::env::temp_dir().join(format!("prsm-original-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
        fn put(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
            p
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_original_moves_into_done_mirroring_its_subfolder() {
        let d = Dir::new("mirror");
        let a = d.put("in/2026/a.jpg", b"jpeg");
        let m = move_original(&a, &d.0.join("in"), &d.0.join("done")).unwrap();
        assert_eq!(m.from, a);
        assert_eq!(m.to, d.0.join("done/2026/a.jpg"));
        assert!(!a.exists());
        assert_eq!(std::fs::read(&m.to).unwrap(), b"jpeg");
        assert_eq!(m.stamp.size, 4);
    }

    #[test]
    fn a_clash_in_done_is_numbered() {
        let d = Dir::new("clash");
        let a = d.put("in/a.jpg", b"new");
        d.put("done/a.jpg", b"old");
        let m = move_original(&a, &d.0.join("in"), &d.0.join("done")).unwrap();
        assert_eq!(m.to, d.0.join("done/a (1).jpg"));
        assert_eq!(std::fs::read(d.0.join("done/a.jpg")).unwrap(), b"old");
        assert_eq!(std::fs::read(&m.to).unwrap(), b"new");
    }

    #[test]
    fn the_copy_fallback_moves_the_bytes_and_removes_the_original() {
        let d = Dir::new("copy");
        let a = d.put("a.jpg", b"jpeg");
        std::fs::create_dir_all(d.0.join("other")).unwrap();
        move_by_copy(&a, &d.0.join("other/a.jpg")).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read(d.0.join("other/a.jpg")).unwrap(), b"jpeg");
        let leftovers: Vec<_> = std::fs::read_dir(d.0.join("other")).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "a temp file was left behind");
        let b = d.put("b.jpg", b"b");
        let e = move_by_copy(&b, &d.0.join("other/a.jpg")).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::AlreadyExists);
        assert!(b.exists());
    }

    #[test]
    fn restore_puts_it_back() {
        let d = Dir::new("restore");
        let a = d.put("in/sub/a.jpg", b"jpeg");
        let m = move_original(&a, &d.0.join("in"), &d.0.join("done")).unwrap();
        std::fs::remove_dir(d.0.join("in/sub")).unwrap();
        restore(&m).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), b"jpeg");
        assert!(!m.to.exists());
    }

    #[test]
    fn restore_refuses_when_its_place_is_taken() {
        let d = Dir::new("taken");
        let a = d.put("in/a.jpg", b"jpeg");
        let m = move_original(&a, &d.0.join("in"), &d.0.join("done")).unwrap();
        d.put("in/a.jpg", b"someone else");
        assert!(matches!(restore(&m), Err(NotRestored::Taken)));
        assert_eq!(std::fs::read(&a).unwrap(), b"someone else");
        assert_eq!(std::fs::read(&m.to).unwrap(), b"jpeg");
    }

    #[test]
    fn restore_refuses_when_it_changed() {
        let d = Dir::new("changed");
        let a = d.put("in/a.jpg", b"jpeg");
        let m = move_original(&a, &d.0.join("in"), &d.0.join("done")).unwrap();
        std::fs::write(&m.to, b"edited since").unwrap();
        assert!(matches!(restore(&m), Err(NotRestored::Changed)));
        assert!(!a.exists());
        assert_eq!(std::fs::read(&m.to).unwrap(), b"edited since");
    }

    #[test]
    fn restore_refuses_when_it_is_gone() {
        let d = Dir::new("gone");
        let a = d.put("in/a.jpg", b"jpeg");
        let m = move_original(&a, &d.0.join("in"), &d.0.join("done")).unwrap();
        std::fs::remove_file(&m.to).unwrap();
        assert!(matches!(restore(&m), Err(NotRestored::Gone)));
        assert!(!a.exists());
    }
}
