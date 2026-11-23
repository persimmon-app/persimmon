// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Which watched files are ready: settled (unchanged for a while), and not
//! already handled in the state they're in now. A file that converted, or
//! failed, is handed out again only once its size or modification time
//! changes, so a bad file can't loop.

use std::collections::HashMap;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// What a file looked like: its size and modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Stamp {
    pub size: u64,
    pub modified: SystemTime,
}

impl Stamp {
    /// A file's stamp; `None` where the system keeps no modification time.
    pub fn of(meta: &Metadata) -> Option<Stamp> {
        Some(Stamp {
            size: meta.len(),
            modified: meta.modified().ok()?,
        })
    }
}

#[derive(Debug)]
struct Pending {
    stamp: Stamp,
    /// When the stamp last changed.
    since: Instant,
    /// Arrival order.
    seq: u64,
}

/// Settles files and remembers the ones handed out.
#[derive(Debug)]
pub(crate) struct Tracker {
    settle: Option<Duration>,
    known: HashMap<PathBuf, Stamp>,
    pending: HashMap<PathBuf, Pending>,
    seq: u64,
}

impl Tracker {
    /// `settle`: how long a file must stay unchanged; `None` hands a file
    /// out as soon as it's seen.
    pub(crate) fn new(settle: Option<Duration>) -> Tracker {
        Tracker {
            settle,
            known: HashMap::new(),
            pending: HashMap::new(),
            seq: 0,
        }
    }

    /// A file already handled as it is now (for instance, there before
    /// watching started).
    pub(crate) fn baseline(&mut self, path: PathBuf, stamp: Stamp) {
        self.known.insert(path, stamp);
    }

    /// The file at `path` looks like `stamp` at `now`.
    pub(crate) fn observe(&mut self, path: &Path, stamp: Stamp, now: Instant) {
        if self.known.get(path) == Some(&stamp) {
            return;
        }
        match self.pending.get_mut(path) {
            Some(p) if p.stamp != stamp => {
                p.stamp = stamp;
                p.since = now;
            }
            Some(_) => {}
            None => {
                self.seq += 1;
                self.pending.insert(
                    path.to_path_buf(),
                    Pending {
                        stamp,
                        since: now,
                        seq: self.seq,
                    },
                );
            }
        }
    }

    /// The file at `path` is gone: forget it, so one put back converts.
    pub(crate) fn gone(&mut self, path: &Path) {
        self.pending.remove(path);
        self.known.remove(path);
    }

    /// The stamp a file was last handed out (or baselined) with.
    pub(crate) fn known(&self, path: &Path) -> Option<Stamp> {
        self.known.get(path).copied()
    }

    /// Files waiting to settle.
    pub(crate) fn pending(&self) -> Vec<PathBuf> {
        self.pending.keys().cloned().collect()
    }

    /// Files that have settled by `now`, in arrival order, each now handled.
    pub(crate) fn ready(&mut self, now: Instant) -> Vec<PathBuf> {
        let settled = |p: &Pending| {
            self.settle
                .is_none_or(|settle| now.saturating_duration_since(p.since) >= settle)
        };
        let mut done: Vec<(u64, PathBuf)> = self
            .pending
            .iter()
            .filter(|(_, p)| settled(p))
            .map(|(path, p)| (p.seq, path.clone()))
            .collect();
        done.sort();
        done.into_iter()
            .map(|(_, path)| {
                let p = self.pending.remove(&path).expect("pending");
                self.known.insert(path.clone(), p.stamp);
                path
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(size: u64) -> Stamp {
        Stamp {
            size,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000),
        }
    }

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    const SETTLE: Option<Duration> = Some(Duration::from_secs(2));

    #[test]
    fn a_new_file_is_ready_once_it_has_settled() {
        let t0 = Instant::now();
        let mut t = Tracker::new(SETTLE);
        t.observe(Path::new("a"), stamp(1), t0);
        assert!(t.ready(ms(t0, 1_900)).is_empty());
        assert_eq!(t.ready(ms(t0, 2_000)), [PathBuf::from("a")]);
        assert!(t.ready(ms(t0, 9_000)).is_empty());
    }

    #[test]
    fn a_growing_file_waits_for_its_last_change() {
        let t0 = Instant::now();
        let mut t = Tracker::new(SETTLE);
        t.observe(Path::new("a"), stamp(1), t0);
        t.observe(Path::new("a"), stamp(2), ms(t0, 1_500));
        assert!(t.ready(ms(t0, 2_500)).is_empty());
        t.observe(Path::new("a"), stamp(2), ms(t0, 3_000));
        assert_eq!(t.ready(ms(t0, 3_500)), [PathBuf::from("a")]);
    }

    #[test]
    fn without_settling_a_file_is_ready_at_once() {
        let t0 = Instant::now();
        let mut t = Tracker::new(None);
        t.observe(Path::new("a"), stamp(1), t0);
        assert_eq!(t.ready(t0), [PathBuf::from("a")]);
    }

    #[test]
    fn a_handled_file_is_not_ready_again_until_it_changes() {
        let t0 = Instant::now();
        let mut t = Tracker::new(None);
        t.observe(Path::new("a"), stamp(1), t0);
        assert_eq!(t.ready(t0).len(), 1);
        t.observe(Path::new("a"), stamp(1), ms(t0, 10));
        assert!(t.pending().is_empty());
        assert!(t.ready(ms(t0, 10)).is_empty());
        t.observe(Path::new("a"), stamp(5), ms(t0, 20));
        assert_eq!(t.ready(ms(t0, 20)), [PathBuf::from("a")]);
    }

    #[test]
    fn a_baseline_file_is_left_alone_until_it_changes() {
        let t0 = Instant::now();
        let mut t = Tracker::new(None);
        t.baseline(PathBuf::from("old"), stamp(1));
        t.observe(Path::new("old"), stamp(1), t0);
        assert!(t.ready(t0).is_empty());
        t.observe(Path::new("old"), stamp(2), t0);
        assert_eq!(t.ready(t0), [PathBuf::from("old")]);
    }

    #[test]
    fn a_burst_comes_out_in_arrival_order() {
        let t0 = Instant::now();
        let mut t = Tracker::new(SETTLE);
        let names: Vec<PathBuf> = (0..100)
            .rev()
            .map(|i| PathBuf::from(format!("f{i:03}")))
            .collect();
        for (i, n) in names.iter().enumerate() {
            t.observe(n, stamp(1), ms(t0, i as u64));
        }
        assert_eq!(t.ready(ms(t0, 3_000)), names);
    }

    #[test]
    fn a_file_that_goes_away_is_forgotten() {
        let t0 = Instant::now();
        let mut t = Tracker::new(None);
        t.observe(Path::new("a"), stamp(1), t0);
        t.gone(Path::new("a"));
        assert!(t.ready(t0).is_empty());
        t.observe(Path::new("b"), stamp(1), t0);
        assert_eq!(t.ready(t0), [PathBuf::from("b")]);
        t.gone(Path::new("b"));
        t.observe(Path::new("b"), stamp(1), ms(t0, 5));
        assert_eq!(t.ready(ms(t0, 5)), [PathBuf::from("b")]);
    }
}
