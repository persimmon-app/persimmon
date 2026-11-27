// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! A watch rule's files become History batches (spec D8): a batch is a
//! burst, the files a rule converts until it has been quiet for 30
//! seconds. Pure: the time is always passed in.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::history::{self, Batch, Item, ItemOutcome, Source};
use crate::queue::{QueueMode, tilde};

/// How long a rule is quiet before its burst becomes a batch.
pub const QUIET: Duration = Duration::from_secs(30);

static BATCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct Open {
    id: String,
    started: u64,
    last: Instant,
    /// A file is converting: the burst can't end yet.
    busy: bool,
    items: Vec<Item>,
}

/// One rule's open burst.
pub struct Bursts {
    rule: String,
    folder: PathBuf,
    /// The preset's name, as History shows it.
    preset: Option<String>,
    quiet: Duration,
    open: Option<Open>,
}

impl Bursts {
    pub fn new(rule: String, folder: PathBuf, preset: Option<String>, quiet: Duration) -> Bursts {
        Bursts {
            rule,
            folder,
            preset,
            quiet,
            open: None,
        }
    }

    /// The open burst's batch id, opening one if none is open. A file is
    /// starting: the burst stays open until it's pushed.
    pub fn open_id(&mut self, now: Instant) -> String {
        let open = self.open.get_or_insert_with(|| {
            let started = history::now();
            let n = BATCHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Open {
                id: format!("w{started}-{n}"),
                started,
                last: now,
                busy: false,
                items: Vec::new(),
            }
        });
        open.last = open.last.max(now);
        open.busy = true;
        open.id.clone()
    }

    pub fn push(&mut self, item: Item, now: Instant) {
        self.open_id(now);
        if let Some(open) = self.open.as_mut() {
            open.items.push(item);
            open.last = now;
            open.busy = false;
        }
    }

    /// The file that started didn't make an item (it was cancelled).
    pub fn idle(&mut self) {
        if let Some(open) = self.open.as_mut() {
            open.busy = false;
        }
    }

    /// The burst as a batch, once it has been quiet long enough.
    pub fn due(&mut self, now: Instant) -> Option<Batch> {
        let quiet = self
            .open
            .as_ref()
            .is_some_and(|o| !o.busy && now.saturating_duration_since(o.last) >= self.quiet);
        if quiet { self.close() } else { None }
    }

    /// The open burst as a batch now: for a pause, a change or quitting.
    /// A burst with no files makes no batch.
    pub fn close(&mut self) -> Option<Batch> {
        let open = self.open.take()?;
        if open.items.is_empty() {
            return None;
        }
        Some(Batch {
            id: open.id,
            started: open.started,
            finished: history::now().max(open.started),
            source: Source::Watch,
            preset: self.preset.clone(),
            mode: QueueMode::Convert,
            items: open.items,
            undone: false,
            rule: Some(self.rule.clone()),
            folder: Some(self.folder.clone()),
        })
    }

    /// Files converted in the open burst.
    pub fn done(&self) -> usize {
        self.open.as_ref().map_or(0, |o| {
            o.items
                .iter()
                .filter(|i| i.outcome == ItemOutcome::Done)
                .count()
        })
    }

    /// Files in the open burst.
    pub fn len(&self) -> usize {
        self.open.as_ref().map_or(0, |o| o.items.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What the notification at the end of a burst says.
pub fn summary(batch: &Batch) -> String {
    let folder = batch.folder.as_deref().map(tilde).unwrap_or_default();
    let n = batch.items.len();
    let done = batch
        .items
        .iter()
        .filter(|i| i.outcome == ItemOutcome::Done)
        .count();
    let failed = batch
        .items
        .iter()
        .filter(|i| matches!(i.outcome, ItemOutcome::Failed { .. }))
        .count();
    if failed == 0 {
        format!("{folder} — {done} converted")
    } else {
        format!("{folder} — {done} of {n} converted, {failed} failed")
    }
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What a per-file notification says.
pub fn file_note(item: &Item) -> String {
    let input = item.inputs.first().map(|p| name(p)).unwrap_or_default();
    match (&item.outcome, &item.output) {
        (ItemOutcome::Done, Some(out)) => match &item.note {
            Some(note) => format!("{input} → {} ({note})", name(out)),
            None => format!("{input} → {}", name(out)),
        },
        (ItemOutcome::Failed { error }, _) => format!("{input} failed: {}", error.reason),
        _ => format!("{input}: nothing written"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wording::Shown;
    use persimmon_core::error::ErrorCode;

    #[test]
    fn file_note_appends_the_note() {
        let mut i = item("clip.mov", Some("clip.mp4"));
        i.note = Some("hardware encoder unavailable — used software".into());
        assert_eq!(
            file_note(&i),
            "clip.mov → clip.mp4 (hardware encoder unavailable — used software)"
        );
        assert_eq!(file_note(&item("a.png", Some("a.webp"))), "a.png → a.webp");
    }

    fn item(name: &str, out: Option<&str>) -> Item {
        Item {
            inputs: vec![PathBuf::from("/in").join(name)],
            output: out.map(|o| PathBuf::from("/in/converted").join(o)),
            output_size: out.map(|_| 1),
            output_mtime: None,
            size: 2,
            outcome: match out {
                Some(_) => ItemOutcome::Done,
                None => ItemOutcome::Failed {
                    error: Shown {
                        code: ErrorCode::CorruptInput,
                        file: None,
                        reason: "it's damaged".into(),
                        fix: None,
                    },
                },
            },
            undone: false,
            to: None,
            mode: None,
            moved: None,
            restored: false,
            note: None,
        }
    }

    fn bursts() -> Bursts {
        Bursts::new(
            "r1".into(),
            PathBuf::from("/in"),
            Some("Web image".into()),
            QUIET,
        )
    }

    #[test]
    fn a_burst_ends_after_quiet() {
        let t = Instant::now();
        let mut b = bursts();
        let id = b.open_id(t);
        b.push(item("a.jpg", Some("a.webp")), t);
        b.push(item("b.jpg", Some("b.webp")), t + Duration::from_secs(10));
        assert_eq!((b.len(), b.done()), (2, 2));
        assert!(b.due(t + Duration::from_secs(39)).is_none());
        let batch = b.due(t + Duration::from_secs(40)).unwrap();
        assert_eq!(batch.id, id);
        assert_eq!(batch.items.len(), 2);
        assert_eq!(batch.source, Source::Watch);
        assert_eq!(batch.mode, QueueMode::Convert);
        assert_eq!(batch.rule.as_deref(), Some("r1"));
        assert_eq!(batch.folder.as_deref(), Some(std::path::Path::new("/in")));
        assert_eq!(batch.preset.as_deref(), Some("Web image"));
        assert!(batch.finished >= batch.started);
        assert_eq!(b.len(), 0);
    }

    #[test]
    fn a_new_file_after_the_end_opens_a_new_batch() {
        let t = Instant::now();
        let mut b = bursts();
        let first = b.open_id(t);
        b.push(item("a.jpg", Some("a.webp")), t);
        // A file starting keeps the burst open until it's pushed.
        assert_eq!(b.open_id(t + Duration::from_secs(5)), first);
        assert!(b.due(t + Duration::from_secs(120)).is_none());
        b.push(item("a2.jpg", Some("a2.webp")), t + Duration::from_secs(6));
        assert_eq!(
            b.due(t + Duration::from_secs(6) + QUIET)
                .unwrap()
                .items
                .len(),
            2
        );
        let t = t + Duration::from_secs(6);
        let later = t + QUIET + Duration::from_secs(1);
        let second = b.open_id(later);
        assert_ne!(second, first);
        b.push(item("b.jpg", Some("b.webp")), later);
        assert_eq!(b.due(later + QUIET).unwrap().items.len(), 1);
    }

    #[test]
    fn close_returns_the_open_burst_once() {
        let t = Instant::now();
        let mut b = bursts();
        b.open_id(t);
        b.push(item("a.jpg", None), t);
        assert_eq!(b.close().unwrap().items.len(), 1);
        assert!(b.close().is_none());
    }

    #[test]
    fn nothing_open_closes_to_nothing() {
        let mut b = bursts();
        assert!(b.close().is_none());
        assert!(b.due(Instant::now() + QUIET).is_none());
        // Opened for a file that then wasn't recorded: no empty batch.
        b.open_id(Instant::now());
        assert!(b.close().is_none());
        // A file that started and was cancelled doesn't hold the burst open.
        let t = Instant::now();
        b.push(item("a.jpg", Some("a.webp")), t);
        b.open_id(t);
        assert!(b.due(t + QUIET).is_none());
        b.idle();
        assert_eq!(b.due(t + QUIET).unwrap().items.len(), 1);
    }

    #[test]
    fn the_summary_reads_as_drawn() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let t = Instant::now();
        let mut b = Bursts::new("r1".into(), home.join("Inbox"), None, QUIET);
        b.open_id(t);
        for i in 0..23 {
            b.push(item(&format!("{i}.jpg"), Some(&format!("{i}.webp"))), t);
        }
        b.push(item("scan-0231.jpg", None), t);
        let batch = b.close().unwrap();
        assert_eq!(summary(&batch), "~/Inbox — 23 of 24 converted, 1 failed");
        b.open_id(t);
        for i in 0..24 {
            b.push(item(&format!("{i}.jpg"), Some(&format!("{i}.webp"))), t);
        }
        assert_eq!(summary(&b.close().unwrap()), "~/Inbox — 24 converted");
    }

    #[test]
    fn file_notes_name_the_file() {
        assert_eq!(file_note(&item("a.jpg", Some("a.webp"))), "a.jpg → a.webp");
        assert_eq!(
            file_note(&item("scan.jpg", None)),
            "scan.jpg failed: it's damaged"
        );
    }
}
