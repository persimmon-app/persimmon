// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! History (spec D3): the batches of the last 30 days, one JSON line each
//! in `~/Library/Application Support/Persimmon/history.jsonl`, and undo,
//! which moves a batch's outputs to the Trash. Nothing is deleted for good.

use std::path::{Path, PathBuf};

use crate::queue::QueueMode;
use crate::wording::Shown;

/// Where a batch came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Window,
    Drop,
    Opened,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum ItemOutcome {
    Done,
    NotSmaller,
    Failed { error: Shown },
    Cancelled,
}

/// One row of a batch.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub inputs: Vec<PathBuf>,
    pub output: Option<PathBuf>,
    pub output_size: Option<u64>,
    /// The output's modification time when written, in nanoseconds.
    pub output_mtime: Option<u64>,
    pub size: u64,
    #[serde(flatten)]
    pub outcome: ItemOutcome,
    #[serde(default)]
    pub undone: bool,
}

/// The rows one Start (or one drop) ran.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub id: String,
    /// Unix seconds.
    pub started: u64,
    pub finished: u64,
    pub source: Source,
    pub preset: Option<String>,
    pub mode: QueueMode,
    pub items: Vec<Item>,
    #[serde(default)]
    pub undone: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filter {
    All,
    Undoable,
    Failed,
}

/// What undo did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoReport {
    pub trashed: usize,
    pub skipped: Vec<String>,
}

/// Moving a path to the Trash.
pub trait Trasher {
    fn trash(&self, path: &Path) -> std::io::Result<()>;
}

/// The system Trash, through macOS's file manager.
pub struct SystemTrash;

impl Trasher for SystemTrash {
    /// Through `NSFileManager`, never by scripting Finder: no Automation
    /// prompt. Finder's "Put Back" may not be offered; the item can be
    /// dragged out of the Trash instead (owner ruling, 2026-11-13).
    fn trash(&self, path: &Path) -> std::io::Result<()> {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = trash::TrashContext::default();
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete(path).map_err(std::io::Error::other)
    }
}

/// How long history keeps a batch.
pub const KEEP_SECONDS: u64 = 30 * 24 * 60 * 60;

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A file or folder's modification time in nanoseconds.
pub fn mtime(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    u64::try_from(since.as_nanos()).ok()
}

pub struct History {
    path: PathBuf,
    batches: Vec<Batch>,
}

impl History {
    /// Load the history: batches older than 30 days are dropped, and a line
    /// that doesn't read as a batch (a write cut short) is kept in
    /// `history.jsonl.bad`. The file is rewritten when anything was dropped.
    pub fn open(path: &Path) -> History {
        let text = std::fs::read(path)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let cutoff = now().saturating_sub(KEEP_SECONDS);
        let mut batches = Vec::new();
        let mut bad = Vec::new();
        let mut dropped = false;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Batch>(line) {
                Ok(b) if b.finished >= cutoff => batches.push(b),
                Ok(_) => dropped = true,
                Err(_) => {
                    bad.push(line.to_string());
                    dropped = true;
                }
            }
        }
        if !bad.is_empty() {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path.with_extension("jsonl.bad"))
            {
                for line in &bad {
                    let _ = writeln!(f, "{line}");
                }
            }
        }
        let history = History {
            path: path.to_path_buf(),
            batches,
        };
        if dropped {
            let _ = history.rewrite();
        }
        history
    }

    /// Write every batch through a temporary file renamed into place.
    fn rewrite(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = String::new();
        for b in &self.batches {
            text.push_str(&serde_json::to_string(b).map_err(std::io::Error::other)?);
            text.push('\n');
        }
        let temp = self.path.with_extension("jsonl.part");
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, &self.path)
    }

    pub fn append(&mut self, batch: Batch) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let line = serde_json::to_string(&batch).map_err(std::io::Error::other)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")?;
        f.sync_all()?;
        self.batches.push(batch);
        Ok(())
    }

    /// Newest first.
    pub fn batches(&self) -> Vec<&Batch> {
        let mut v: Vec<&Batch> = self.batches.iter().collect();
        v.sort_by(|a, b| b.finished.cmp(&a.finished).then(b.id.cmp(&a.id)));
        v
    }

    pub fn get(&self, id: &str) -> Option<&Batch> {
        self.batches.iter().find(|b| b.id == id)
    }

    /// Batches whose file names or preset contain `text` (any case).
    pub fn search(&self, text: &str, filter: Filter) -> Vec<&Batch> {
        let needle = text.trim().to_lowercase();
        let name = |p: &Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };
        self.batches()
            .into_iter()
            .filter(|b| match filter {
                Filter::All => true,
                Filter::Undoable => undoable(b),
                Filter::Failed => b
                    .items
                    .iter()
                    .any(|i| matches!(i.outcome, ItemOutcome::Failed { .. })),
            })
            .filter(|b| {
                needle.is_empty()
                    || b.preset
                        .as_deref()
                        .is_some_and(|p| p.to_lowercase().contains(&needle))
                    || b.items.iter().any(|i| {
                        i.inputs.iter().any(|p| name(p).contains(&needle))
                            || i.output
                                .as_deref()
                                .is_some_and(|p| name(p).contains(&needle))
                    })
            })
            .collect()
    }

    /// Forget every batch. The files stay where they are.
    pub fn clear(&mut self) -> std::io::Result<()> {
        self.batches.clear();
        self.rewrite()
    }

    /// Move the batch's outputs to the Trash. An output changed or moved
    /// since it was written is left alone and named; inputs are never
    /// touched.
    pub fn undo(&mut self, id: &str, trash: &dyn Trasher) -> UndoReport {
        let mut report = UndoReport {
            trashed: 0,
            skipped: Vec::new(),
        };
        let Some(batch) = self.batches.iter_mut().find(|b| b.id == id) else {
            return report;
        };
        for item in batch.items.iter_mut() {
            if item.undone || item.outcome != ItemOutcome::Done {
                continue;
            }
            let Some(output) = item.output.clone() else {
                continue;
            };
            let name = output
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !unchanged(item, &output) {
                report.skipped.push(format!(
                    "{name} was changed or moved since it was converted, so it was left alone"
                ));
                continue;
            }
            match trash.trash(&output) {
                Ok(()) => {
                    item.undone = true;
                    report.trashed += 1;
                }
                Err(e) => report
                    .skipped
                    .push(format!("{name} couldn't be moved to the Trash ({e})")),
            }
        }
        batch.undone = batch
            .items
            .iter()
            .filter(|i| i.outcome == ItemOutcome::Done)
            .all(|i| i.undone);
        let _ = self.rewrite();
        report
    }
}

/// The output is still the file (or folder) the batch wrote.
fn unchanged(item: &Item, output: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(output) else {
        return false;
    };
    let size_ok = match item.output_size {
        Some(size) => meta.is_file() && meta.len() == size,
        None => meta.is_dir(),
    };
    size_ok && item.output_mtime.is_none_or(|t| mtime(output) == Some(t))
}

/// A batch can be undone while one of its outputs is still where it was
/// written and not yet undone.
pub fn undoable(batch: &Batch) -> bool {
    batch.items.iter().any(|i| {
        !i.undone && i.outcome == ItemOutcome::Done && i.output.as_deref().is_some_and(Path::exists)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use persimmon_core::error::ErrorCode;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d = std::env::temp_dir()
                .join(format!("prsm-app-history-{name}-{}", std::process::id()));
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

    /// Moves into a folder of its own instead of the real Trash.
    struct FakeTrash(PathBuf);
    impl Trasher for FakeTrash {
        fn trash(&self, path: &Path) -> std::io::Result<()> {
            std::fs::create_dir_all(&self.0)?;
            std::fs::rename(path, self.0.join(path.file_name().unwrap()))
        }
    }

    fn done(input: &Path, output: &Path) -> Item {
        let meta = std::fs::metadata(output).unwrap();
        Item {
            inputs: vec![input.to_path_buf()],
            output: Some(output.to_path_buf()),
            output_size: meta.is_file().then(|| meta.len()),
            output_mtime: mtime(output),
            size: 10,
            outcome: ItemOutcome::Done,
            undone: false,
        }
    }

    fn batch(id: &str, finished: u64, items: Vec<Item>) -> Batch {
        Batch {
            id: id.into(),
            started: finished.saturating_sub(5),
            finished,
            source: Source::Window,
            preset: Some("Web image".into()),
            mode: QueueMode::Convert,
            items,
            undone: false,
        }
    }

    #[test]
    fn batches_round_trip_and_load_newest_first() {
        let d = Dir::new("round");
        let path = d.0.join("history.jsonl");
        let mut h = History::open(&path);
        h.append(batch("a", now() - 100, vec![])).unwrap();
        h.append(batch("b", now() - 10, vec![])).unwrap();
        let ids: Vec<String> = History::open(&path)
            .batches()
            .iter()
            .map(|b| b.id.clone())
            .collect();
        assert_eq!(ids, ["b", "a"]);
    }

    #[test]
    fn old_batches_are_pruned_on_open() {
        let d = Dir::new("prune");
        let path = d.0.join("history.jsonl");
        let mut h = History::open(&path);
        h.append(batch("old", now() - KEEP_SECONDS - 60, vec![]))
            .unwrap();
        h.append(batch("new", now(), vec![])).unwrap();
        assert_eq!(History::open(&path).batches().len(), 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().lines().count(),
            1,
            "the file is rewritten"
        );
    }

    #[test]
    fn a_torn_last_line_is_skipped() {
        let d = Dir::new("torn");
        let path = d.0.join("history.jsonl");
        let mut h = History::open(&path);
        h.append(batch("a", now(), vec![])).unwrap();
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(r#"{"id":"x","sta"#);
        std::fs::write(&path, text).unwrap();
        let mut h = History::open(&path);
        assert_eq!(h.batches().len(), 1);
        assert!(
            std::fs::read_to_string(d.0.join("history.jsonl.bad"))
                .unwrap()
                .contains(r#""sta"#)
        );
        h.append(batch("b", now(), vec![])).unwrap();
        assert_eq!(History::open(&path).batches().len(), 2);
    }

    #[test]
    fn undo_trashes_unchanged_outputs() {
        let d = Dir::new("undo");
        let input = d.put("a.png", b"in");
        let output = d.put("a.jpg", b"out");
        let mut h = History::open(&d.0.join("history.jsonl"));
        h.append(batch("a", now(), vec![done(&input, &output)]))
            .unwrap();
        assert!(undoable(h.batches()[0]));
        let report = h.undo("a", &FakeTrash(d.0.join("trash")));
        assert_eq!(
            report,
            UndoReport {
                trashed: 1,
                skipped: vec![]
            }
        );
        assert!(!output.exists() && d.0.join("trash/a.jpg").exists());
        let reopened = History::open(&d.0.join("history.jsonl"));
        assert!(reopened.batches()[0].undone);
        assert!(!undoable(reopened.batches()[0]));
    }

    #[test]
    fn undo_skips_changed_outputs() {
        let d = Dir::new("skip");
        let input = d.put("a.png", b"in");
        let edited = d.put("edited.jpg", b"out");
        let moved = d.put("moved.jpg", b"out");
        let kept = d.put("kept.jpg", b"out");
        let items = vec![
            done(&input, &edited),
            done(&input, &moved),
            done(&input, &kept),
        ];
        let mut h = History::open(&d.0.join("history.jsonl"));
        h.append(batch("a", now(), items)).unwrap();
        std::fs::write(&edited, b"edited since, longer").unwrap();
        std::fs::rename(&moved, d.0.join("elsewhere.jpg")).unwrap();
        let report = h.undo("a", &FakeTrash(d.0.join("trash")));
        assert_eq!(report.trashed, 1);
        assert_eq!(
            report.skipped,
            vec![
                "edited.jpg was changed or moved since it was converted, so it was left alone"
                    .to_string(),
                "moved.jpg was changed or moved since it was converted, so it was left alone"
                    .to_string(),
            ]
        );
        assert!(edited.exists());
    }

    #[test]
    fn undo_trashes_a_folder_output_whole() {
        let d = Dir::new("folder");
        let input = d.put("a.zip", b"zip");
        let folder = d.0.join("a");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("x.txt"), b"x").unwrap();
        let mut h = History::open(&d.0.join("history.jsonl"));
        h.append(batch("a", now(), vec![done(&input, &folder)]))
            .unwrap();
        let report = h.undo("a", &FakeTrash(d.0.join("trash")));
        assert_eq!(report.trashed, 1);
        assert!(!folder.exists() && d.0.join("trash/a/x.txt").exists());
    }

    #[test]
    fn undo_never_touches_inputs() {
        let d = Dir::new("inputs");
        let input = d.put("a.png", b"in");
        let output = d.put("a.jpg", b"out");
        let mut h = History::open(&d.0.join("history.jsonl"));
        h.append(batch("a", now(), vec![done(&input, &output)]))
            .unwrap();
        h.undo("a", &FakeTrash(d.0.join("trash")));
        assert!(input.exists());
    }

    #[test]
    fn search_and_filters() {
        let d = Dir::new("search");
        let input = d.put("holiday.png", b"in");
        let output = d.put("holiday.jpg", b"out");
        let mut h = History::open(&d.0.join("history.jsonl"));
        h.append(batch("ok", now() - 5, vec![done(&input, &output)]))
            .unwrap();
        let failed = Item {
            inputs: vec![d.0.join("broken.png")],
            output: None,
            output_size: None,
            output_mtime: None,
            size: 1,
            outcome: ItemOutcome::Failed {
                error: Shown {
                    code: ErrorCode::CorruptInput,
                    file: None,
                    reason: "x".into(),
                    fix: None,
                },
            },
            undone: false,
        };
        h.append(batch("bad", now(), vec![failed])).unwrap();
        let ids = |v: Vec<&Batch>| v.iter().map(|b| b.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(h.search("", Filter::All)), ["bad", "ok"]);
        assert_eq!(ids(h.search("", Filter::Failed)), ["bad"]);
        assert_eq!(ids(h.search("", Filter::Undoable)), ["ok"]);
        assert_eq!(ids(h.search("HOLIDAY", Filter::All)), ["ok"]);
        assert_eq!(ids(h.search("web image", Filter::All)).len(), 2);
    }

    #[test]
    fn clear_keeps_files() {
        let d = Dir::new("clear");
        let input = d.put("a.png", b"in");
        let output = d.put("a.jpg", b"out");
        let path = d.0.join("history.jsonl");
        let mut h = History::open(&path);
        h.append(batch("a", now(), vec![done(&input, &output)]))
            .unwrap();
        h.clear().unwrap();
        assert!(h.batches().is_empty() && History::open(&path).batches().is_empty());
        assert!(output.exists() && input.exists());
    }
}
