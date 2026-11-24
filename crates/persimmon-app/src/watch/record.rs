// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Each rule's record of the files it has handled, so a file converts
//! once: across restarts, and when an undo puts an original back.

use std::collections::BTreeMap;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use persimmon_core::watch::Stamp;

/// A file's size and modification time, as History and the records keep
/// them: whole seconds from 1970 (negative before it) plus nanoseconds,
/// which JSON can hold for any date, unlike `SystemTime`'s own form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStamp {
    pub size: u64,
    pub secs: i64,
    pub nanos: u32,
}

impl FileStamp {
    /// A file's stamp; `None` where the system keeps no modification time.
    pub fn of(meta: &Metadata) -> Option<FileStamp> {
        Stamp::of(meta).map(|s| FileStamp::from_core(&s))
    }

    pub fn from_core(s: &Stamp) -> FileStamp {
        let (secs, nanos) = match s.modified.duration_since(UNIX_EPOCH) {
            Ok(d) => (d.as_secs() as i64, d.subsec_nanos()),
            Err(e) => {
                // Before 1970: whole seconds rounded down, nanos counted up.
                let d = e.duration();
                match d.subsec_nanos() {
                    0 => (-(d.as_secs() as i64), 0),
                    n => (-(d.as_secs() as i64) - 1, 1_000_000_000 - n),
                }
            }
        };
        FileStamp {
            size: s.size,
            secs,
            nanos,
        }
    }

    pub fn to_core(&self) -> Stamp {
        let nanos = Duration::from_nanos(u64::from(self.nanos));
        let modified = if self.secs >= 0 {
            UNIX_EPOCH + Duration::from_secs(self.secs as u64) + nanos
        } else {
            UNIX_EPOCH - Duration::from_secs(self.secs.unsigned_abs()) + nanos
        };
        Stamp {
            size: self.size,
            modified,
        }
    }
}

/// Where rule `rule_id` keeps its record; `data` is
/// `~/Library/Application Support/Persimmon`.
pub fn record_path(data: &Path, rule_id: &str) -> PathBuf {
    data.join("watch").join(format!("{rule_id}.json"))
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    files: BTreeMap<PathBuf, FileStamp>,
}

/// The files a rule has handled (converted, failed, or found already
/// there), each as it was then.
#[derive(Debug)]
pub struct Record {
    path: PathBuf,
    files: BTreeMap<PathBuf, FileStamp>,
    damaged: bool,
}

impl Record {
    /// The record at `path`: empty when there's none, and when it's
    /// damaged, which is then kept aside as `<id>.json.bad`.
    pub fn open(path: &Path) -> Record {
        let (files, damaged) = match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Stored>(&bytes) {
                Ok(stored) => (stored.files, false),
                Err(_) => {
                    let _ = std::fs::rename(path, path.with_extension("json.bad"));
                    (BTreeMap::new(), true)
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (BTreeMap::new(), false),
            Err(_) => {
                let _ = std::fs::rename(path, path.with_extension("json.bad"));
                (BTreeMap::new(), true)
            }
        };
        Record {
            path: path.to_path_buf(),
            files,
            damaged,
        }
    }

    /// Whether the record couldn't be read when opened.
    pub fn damaged(&self) -> bool {
        self.damaged
    }

    /// Whether `file` was handled as it is now.
    pub fn handled(&self, file: &Path, stamp: FileStamp) -> bool {
        self.files.get(file) == Some(&stamp)
    }

    pub fn mark(&mut self, file: &Path, stamp: FileStamp) {
        self.files.insert(file.to_path_buf(), stamp);
    }

    /// Mark each file as it is now: files already there are left alone.
    pub fn baseline(&mut self, files: &[PathBuf]) {
        for file in files {
            if let Some(stamp) = std::fs::metadata(file).ok().and_then(|m| FileStamp::of(&m)) {
                self.mark(file, stamp);
            }
        }
    }

    /// Forget files that are gone.
    pub fn prune(&mut self) {
        self.files.retain(|path, _| path.exists());
    }

    /// Write through a temporary file renamed into place.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let stored = Stored {
            files: self.files.clone(),
        };
        let text = serde_json::to_vec(&stored).map_err(std::io::Error::other)?;
        let temp = self.path.with_extension("json.part");
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, &self.path)
    }

    /// Delete a removed rule's record.
    pub fn forget(path: &Path) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d =
                std::env::temp_dir().join(format!("prsm-app-record-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
        fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn stamp(size: u64) -> FileStamp {
        FileStamp {
            size,
            secs: 1_800_000_000,
            nanos: 5,
        }
    }

    #[test]
    fn stamps_round_trip_before_1970() {
        let core = persimmon_core::watch::Stamp {
            size: 7,
            modified: UNIX_EPOCH - Duration::new(90, 250),
        };
        let mine = FileStamp::from_core(&core);
        let json = serde_json::to_string(&mine).unwrap();
        let back: FileStamp = serde_json::from_str(&json).unwrap();
        assert_eq!(back.to_core(), core);
        let after = persimmon_core::watch::Stamp {
            size: 7,
            modified: UNIX_EPOCH + Duration::new(1_800_000_000, 999_999_999),
        };
        assert_eq!(FileStamp::from_core(&after).to_core(), after);
    }

    #[test]
    fn a_marked_file_is_handled_until_it_changes() {
        let d = Dir::new("mark");
        let mut r = Record::open(&record_path(&d.0, "r1"));
        let a = d.0.join("a.png");
        r.mark(&a, stamp(10));
        assert!(r.handled(&a, stamp(10)));
        assert!(!r.handled(&a, stamp(11)));
        assert!(!r.handled(&d.0.join("b.png"), stamp(10)));
    }

    #[test]
    fn the_record_survives_a_reopen() {
        let d = Dir::new("reopen");
        let path = record_path(&d.0, "r1");
        let a = d.0.join("a.png");
        let mut r = Record::open(&path);
        r.mark(&a, stamp(10));
        r.save().unwrap();
        assert_eq!(path, d.0.join("watch").join("r1.json"));
        let again = Record::open(&path);
        assert!(!again.damaged());
        assert!(again.handled(&a, stamp(10)));
    }

    #[test]
    fn a_damaged_record_is_kept_aside() {
        let d = Dir::new("damaged");
        let path = record_path(&d.0, "r1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not json {").unwrap();
        let r = Record::open(&path);
        assert!(r.damaged());
        assert!(!r.handled(&d.0.join("a.png"), stamp(1)));
        assert_eq!(
            std::fs::read(path.with_extension("json.bad")).unwrap(),
            b"not json {"
        );
    }

    #[test]
    fn prune_drops_files_that_are_gone() {
        let d = Dir::new("prune");
        let here = d.put("here.png", b"x");
        let gone = d.0.join("gone.png");
        let mut r = Record::open(&record_path(&d.0, "r1"));
        r.mark(&here, stamp(1));
        r.mark(&gone, stamp(1));
        r.prune();
        assert!(r.handled(&here, stamp(1)));
        assert!(!r.handled(&gone, stamp(1)));
    }

    #[test]
    fn baseline_marks_files_as_they_are_now() {
        let d = Dir::new("baseline");
        let a = d.put("a.png", b"abc");
        let mut r = Record::open(&record_path(&d.0, "r1"));
        r.baseline(std::slice::from_ref(&a));
        let now = FileStamp::of(&std::fs::metadata(&a).unwrap()).unwrap();
        assert!(r.handled(&a, now));
    }

    #[test]
    fn forget_deletes_a_record() {
        let d = Dir::new("forget");
        let path = record_path(&d.0, "r1");
        let r = Record::open(&path);
        r.save().unwrap();
        Record::forget(&path);
        assert!(!path.exists());
    }
}
