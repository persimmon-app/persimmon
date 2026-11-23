// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Watching a folder: file events (through `notify`), a short tick that
//! re-checks files still settling, and a slower full rescan that catches
//! events that never arrive (network shares) and notices the folder going
//! away and coming back.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use notify::Watcher as _;

use crate::detect::unreadable;
use crate::error::{Error, ErrorCode, FixKind};
use crate::exec::CancelToken;
use crate::format::FormatId;
use crate::inputs::is_package;
use crate::job::{Job, JobOptions};
use crate::output::OutputPolicy;
use crate::registry::{Operation, Registry};

use super::patterns::Patterns;
use super::tracker::{Stamp, Tracker};

/// Name endings of files still being written: never touched.
const IN_PROGRESS: &[&str] = &[".part", ".download", ".crdownload", ".tmp"];

/// How often the watcher looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How long a file must stay unchanged before it converts.
    pub settle: Duration,
    /// How often files still settling are checked.
    pub tick: Duration,
    /// How often the whole folder is listed again.
    pub rescan: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            settle: Duration::from_secs(2),
            tick: Duration::from_millis(250),
            rescan: Duration::from_secs(10),
        }
    }
}

/// A watched folder and what happens to the files that land in it.
#[derive(Debug, Clone)]
pub struct WatchSpec {
    pub folder: PathBuf,
    pub target: FormatId,
    pub options: JobOptions,
    pub patterns: Patterns,
    /// Where outputs go; never watched.
    pub results: PathBuf,
    /// Where originals move after success; never watched.
    pub done: Option<PathBuf>,
    pub recursive: bool,
    /// Wait for a file to stop growing.
    pub settle: bool,
    /// Also convert the files there when watching starts.
    pub existing: bool,
    pub timing: Timing,
}

impl WatchSpec {
    /// Watching `folder` for files that can become `target`, results in
    /// `<folder>/converted`.
    pub fn new(reg: &Registry, folder: PathBuf, target: FormatId) -> WatchSpec {
        WatchSpec {
            results: folder.join("converted"),
            folder,
            target,
            options: JobOptions::default(),
            patterns: Patterns::for_target(reg, target),
            done: None,
            recursive: false,
            settle: true,
            existing: false,
            timing: Timing::default(),
        }
    }

    /// The job for `path`, found under `root`: its output mirrors the
    /// subfolder it came from inside the results folder.
    pub fn job_for(&self, root: &Path, path: &Path) -> Job {
        let sub = path
            .parent()
            .and_then(|p| p.strip_prefix(root).ok())
            .unwrap_or(Path::new(""));
        let dir = if sub.as_os_str().is_empty() {
            self.results.clone()
        } else {
            self.results.join(sub)
        };
        Job {
            op: Operation::Convert,
            inputs: vec![path.to_path_buf()],
            target: Some(self.target),
            output: OutputPolicy {
                dir: Some(dir),
                overwrite: false,
                fallback: None,
            },
            options: self.options.clone(),
        }
    }
}

/// What the watcher saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A file is ready to convert.
    Ready(PathBuf),
    /// The folder went away; watching waits for it.
    Missing,
    /// The folder is back; watching resumes.
    Back,
}

/// A folder being watched.
pub struct Watcher {
    root: PathBuf,
    excluded: Vec<PathBuf>,
    patterns: Patterns,
    recursive: bool,
    timing: Timing,
    tracker: Tracker,
    tx: Sender<notify::Result<notify::Event>>,
    rx: Receiver<notify::Result<notify::Event>>,
    events: Option<notify::RecommendedWatcher>,
    last_scan: Instant,
    missing: bool,
    queue: VecDeque<Change>,
}

impl Watcher {
    /// Start watching. Fails when the folder is missing or unreadable, or
    /// when the results or done folder is the watched folder itself.
    pub fn start(spec: &WatchSpec) -> Result<Watcher, Error> {
        let folder = &spec.folder;
        match std::fs::metadata(folder) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => {
                return Err(Error::new(ErrorCode::Usage, "isn't a folder").with_file(folder));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::new(ErrorCode::InputNotFound, "no such file").with_file(folder));
            }
            Err(e) => return Err(unreadable(folder, &e)),
        }
        let root = std::fs::canonicalize(folder).map_err(|e| unreadable(folder, &e))?;
        let is_root = |p: &Path| std::fs::canonicalize(p).is_ok_and(|c| c == root);
        if is_root(&spec.results) {
            return Err(
                Error::new(ErrorCode::Usage, "results would be converted again")
                    .with_file(folder)
                    .with_fix_as(
                        FixKind::ChooseOutputFolder,
                        "choose another folder with --out <dir>",
                    ),
            );
        }
        if spec.done.as_deref().is_some_and(is_root) {
            return Err(
                Error::new(ErrorCode::Usage, "moved originals would be converted again")
                    .with_file(folder)
                    .with_fix_as(
                        FixKind::ChooseDoneFolder,
                        "choose another folder with --then-move <dir>",
                    ),
            );
        }
        let mut excluded = Vec::new();
        for dir in std::iter::once(&spec.results).chain(spec.done.as_ref()) {
            std::fs::create_dir_all(dir).map_err(|e| {
                Error::new(
                    ErrorCode::OutputNotWritable,
                    format!("can't create this folder ({e})"),
                )
                .with_file(dir)
            })?;
            excluded.push(std::fs::canonicalize(dir).map_err(|e| unreadable(dir, &e))?);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut w = Watcher {
            root,
            excluded,
            patterns: spec.patterns.clone(),
            recursive: spec.recursive,
            timing: spec.timing,
            tracker: Tracker::new(spec.settle.then_some(spec.timing.settle)),
            tx,
            rx,
            events: None,
            last_scan: Instant::now(),
            missing: false,
            queue: VecDeque::new(),
        };
        // Read the folder now, so a refusal shows while someone is there.
        std::fs::read_dir(&w.root).map_err(|e| unreadable(folder, &e))?;
        w.arm();
        let now = Instant::now();
        let root = w.root.clone();
        for (path, stamp) in w.files_in(&root) {
            if spec.existing {
                w.tracker.observe(&path, stamp, now);
            } else {
                w.tracker.baseline(path, stamp);
            }
        }
        Ok(w)
    }

    /// The watched folder, resolved.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Wait for the next change; `None` once `cancel` fires.
    pub fn next(&mut self, cancel: &CancelToken) -> Option<Change> {
        loop {
            if cancel.is_cancelled() {
                return None;
            }
            if let Some(change) = self.queue.pop_front() {
                return Some(change);
            }
            if let Ok(event) = self.rx.recv_timeout(self.timing.tick) {
                self.on_event(event);
                while let Ok(event) = self.rx.try_recv() {
                    self.on_event(event);
                }
            }
            self.step(Instant::now());
        }
    }

    /// Start (or restart) file events; without them, rescans do the work.
    fn arm(&mut self) {
        let mode = if self.recursive {
            notify::RecursiveMode::Recursive
        } else {
            notify::RecursiveMode::NonRecursive
        };
        self.events = notify::recommended_watcher(self.tx.clone())
            .and_then(|mut w| w.watch(&self.root, mode).map(|()| w))
            .ok();
    }

    fn on_event(&mut self, event: notify::Result<notify::Event>) {
        if self.missing {
            return;
        }
        let Ok(event) = event else { return };
        let now = Instant::now();
        for path in event.paths {
            match std::fs::metadata(&path) {
                Ok(m) if m.is_dir() => {
                    if self.recursive && self.considered_dir(&path) {
                        for (file, stamp) in self.files_in(&path) {
                            self.tracker.observe(&file, stamp, now);
                        }
                    }
                }
                Ok(m) => {
                    if let Some(stamp) = Stamp::of(&m).filter(|_| m.is_file())
                        && self.considered(&path)
                    {
                        self.tracker.observe(&path, stamp, now);
                    }
                }
                Err(_) => self.tracker.gone(&path),
            }
        }
    }

    /// One tick: the folder's presence, files still settling, the rescan,
    /// and what's now ready.
    fn step(&mut self, now: Instant) {
        if !self.root.is_dir() {
            if !self.missing {
                self.missing = true;
                self.events = None;
                self.queue.push_back(Change::Missing);
            }
            return;
        }
        if self.missing {
            self.missing = false;
            self.arm();
            self.rescan(now);
            self.queue.push_back(Change::Back);
        }
        for path in self.tracker.pending() {
            match std::fs::metadata(&path) {
                Ok(m) if m.is_file() => match Stamp::of(&m) {
                    Some(stamp) => self.tracker.observe(&path, stamp, now),
                    None => self.tracker.gone(&path),
                },
                _ => self.tracker.gone(&path),
            }
        }
        if now.saturating_duration_since(self.last_scan) >= self.timing.rescan {
            self.rescan(now);
        }
        for path in self.tracker.ready(now) {
            self.queue.push_back(Change::Ready(path));
        }
    }

    fn rescan(&mut self, now: Instant) {
        self.last_scan = now;
        let root = self.root.clone();
        for (path, stamp) in self.files_in(&root) {
            self.tracker.observe(&path, stamp, now);
        }
    }

    /// The considered files in `dir` (and below it, when recursive), in
    /// name order. Symlinked folders are never followed.
    fn files_in(&self, dir: &Path) -> Vec<(PathBuf, Stamp)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut paths: Vec<PathBuf> =
                entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            paths.sort();
            let mut subfolders = Vec::new();
            for path in paths {
                let Ok(link) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if link.is_dir() {
                    if self.recursive && self.considered_dir(&path) {
                        subfolders.push(path);
                    }
                    continue;
                }
                if let Ok(m) = std::fs::metadata(&path)
                    && m.is_file()
                    && self.considered(&path)
                    && let Some(stamp) = Stamp::of(&m)
                {
                    out.push((path, stamp));
                }
            }
            stack.extend(subfolders.into_iter().rev());
        }
        out
    }

    /// Whether a folder below the root may hold files to convert.
    fn considered_dir(&self, dir: &Path) -> bool {
        let Ok(rel) = dir.strip_prefix(&self.root) else {
            return false;
        };
        !self.excluded.iter().any(|e| dir.starts_with(e))
            && rel.components().all(|c| visible(c.as_os_str()))
            && !self.in_package(rel, true)
    }

    /// Whether a file is one to convert: under the root (directly, unless
    /// recursive), outside the results and done folders and packages,
    /// visible, finished being written, and matching.
    fn considered(&self, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return false;
        };
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        let lower = name.to_lowercase();
        (self.recursive || rel.components().count() == 1)
            && !self.excluded.iter().any(|e| path.starts_with(e))
            && rel.components().all(|c| visible(c.as_os_str()))
            && !self.in_package(rel, false)
            && !IN_PROGRESS.iter().any(|end| lower.ends_with(end))
            && self.patterns.matches(name)
    }

    /// Whether `rel` lies inside a package (an `.app`, a Photos library);
    /// `whole` also counts `rel` itself.
    fn in_package(&self, rel: &Path, whole: bool) -> bool {
        let mut at = self.root.clone();
        let parts: Vec<_> = rel.components().collect();
        let upto = if whole {
            parts.len()
        } else {
            parts.len().saturating_sub(1)
        };
        parts[..upto].iter().any(|c| {
            at.push(c);
            is_package(&at)
        })
    }
}

/// A name that isn't hidden and isn't Finder's `Icon\r`.
fn visible(name: &std::ffi::OsStr) -> bool {
    let bytes = name.as_encoded_bytes();
    !bytes.starts_with(b".") && bytes != b"Icon\r"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            let d = std::env::temp_dir().join(format!("prsm-watch-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("in")).unwrap();
            Dir(d)
        }
        fn input(&self) -> PathBuf {
            self.0.join("in")
        }
        fn put(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let p = self.input().join(rel);
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

    fn spec(d: &Dir) -> WatchSpec {
        let mut s = WatchSpec::new(&Registry::builtin(), d.input(), FormatId::Webp);
        s.timing = Timing {
            settle: Duration::from_millis(300),
            tick: Duration::from_millis(50),
            rescan: Duration::from_secs(1),
        };
        s
    }

    /// Every change within `within`, files by name.
    fn collect(w: &mut Watcher, within: Duration) -> Vec<String> {
        let cancel = CancelToken::new();
        let c = cancel.clone();
        let timer = std::thread::spawn(move || {
            std::thread::sleep(within);
            c.cancel();
        });
        let mut seen = Vec::new();
        while let Some(change) = w.next(&cancel) {
            seen.push(match change {
                Change::Ready(p) => p.file_name().unwrap().to_string_lossy().into_owned(),
                Change::Missing => "<missing>".into(),
                Change::Back => "<back>".into(),
            });
        }
        timer.join().unwrap();
        seen
    }

    /// Wait up to `within` for the first ready file; its name and when.
    fn first_ready(w: &mut Watcher, within: Duration) -> Option<(String, Instant)> {
        let cancel = CancelToken::new();
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(within);
            c.cancel();
        });
        while let Some(change) = w.next(&cancel) {
            if let Change::Ready(p) = change {
                return Some((
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    Instant::now(),
                ));
            }
        }
        None
    }

    const SOON: Duration = Duration::from_millis(1_500);

    #[test]
    fn a_file_that_appears_is_ready_once() {
        let d = Dir::new("appears");
        let mut w = Watcher::start(&spec(&d)).unwrap();
        d.put("a.png", b"png");
        assert_eq!(collect(&mut w, SOON), ["a.png"]);
        assert!(collect(&mut w, SOON).is_empty());
    }

    #[test]
    fn a_file_still_growing_waits() {
        let d = Dir::new("growing");
        let mut w = Watcher::start(&spec(&d)).unwrap();
        let path = d.input().join("a.png");
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            let mut f = std::fs::File::create(&path).unwrap();
            for _ in 0..5 {
                f.write_all(&[0; 1024]).unwrap();
                f.flush().unwrap();
                std::thread::sleep(Duration::from_millis(150));
            }
            Instant::now() - Duration::from_millis(150)
        });
        let (name, at) = first_ready(&mut w, Duration::from_secs(4)).expect("never ready");
        let last_write = writer.join().unwrap();
        assert_eq!(name, "a.png");
        assert!(
            at.duration_since(last_write) >= Duration::from_millis(300),
            "ready {:?} after the last write",
            at.duration_since(last_write)
        );
    }

    #[test]
    fn a_download_renamed_when_done_is_ready_under_its_final_name() {
        let d = Dir::new("download");
        let mut w = Watcher::start(&spec(&d)).unwrap();
        let part = d.put("a.png.crdownload", b"png");
        std::thread::sleep(Duration::from_millis(500));
        std::fs::rename(&part, d.input().join("a.png")).unwrap();
        assert_eq!(collect(&mut w, SOON), ["a.png"]);
    }

    #[test]
    fn an_atomic_save_converts_once_under_its_final_name() {
        let d = Dir::new("atomic");
        let mut s = spec(&d);
        s.patterns = Patterns::parse("*").unwrap();
        let mut w = Watcher::start(&s).unwrap();
        let temp = d.put(".a.png.sb-1234", b"png");
        std::fs::rename(&temp, d.input().join("a.png")).unwrap();
        assert_eq!(collect(&mut w, SOON), ["a.png"]);
    }

    #[test]
    fn a_burst_of_100_is_each_ready_once() {
        let d = Dir::new("burst");
        let mut w = Watcher::start(&spec(&d)).unwrap();
        for i in 0..100 {
            d.put(&format!("f{i:03}.png"), b"png");
        }
        let mut seen = collect(&mut w, Duration::from_secs(3));
        seen.sort();
        let want: Vec<String> = (0..100).map(|i| format!("f{i:03}.png")).collect();
        assert_eq!(seen, want);
    }

    #[test]
    fn outputs_and_the_done_folder_are_never_matched() {
        let d = Dir::new("own");
        let mut s = spec(&d);
        s.recursive = true;
        s.done = Some(d.input().join("done"));
        let mut w = Watcher::start(&s).unwrap();
        d.put("converted/x.png", b"png");
        d.put("converted/sub/y.png", b"png");
        d.put("done/z.png", b"png");
        assert!(collect(&mut w, SOON).is_empty());
    }

    #[test]
    fn hidden_litter_and_temp_files_are_ignored() {
        let d = Dir::new("litter");
        let mut s = spec(&d);
        s.patterns = Patterns::parse("*").unwrap();
        s.recursive = true;
        let mut w = Watcher::start(&s).unwrap();
        for name in [
            ".DS_Store",
            "._a.png",
            ".prsm-1-0.part",
            "Icon\r",
            "a.tmp",
            "b.png.part",
            "c.png.download",
            ".hidden/d.png",
            "Some.app/e.png",
        ] {
            d.put(name, b"x");
        }
        assert!(collect(&mut w, SOON).is_empty());
    }

    #[test]
    fn files_already_there_are_left_alone_unless_existing() {
        let d = Dir::new("existing");
        d.put("old.png", b"png");
        let mut w = Watcher::start(&spec(&d)).unwrap();
        assert!(collect(&mut w, SOON).is_empty());
        let mut s = spec(&d);
        s.existing = true;
        let mut w = Watcher::start(&s).unwrap();
        assert_eq!(collect(&mut w, SOON), ["old.png"]);
    }

    #[test]
    fn a_file_that_fails_is_not_ready_again_until_it_changes() {
        let d = Dir::new("again");
        let mut w = Watcher::start(&spec(&d)).unwrap();
        let a = d.put("a.png", b"broken");
        assert_eq!(collect(&mut w, SOON), ["a.png"]);
        assert!(
            collect(&mut w, SOON).is_empty(),
            "a rescan handed it out again"
        );
        std::fs::write(&a, b"broken, now longer").unwrap();
        assert_eq!(collect(&mut w, SOON), ["a.png"]);
    }

    #[test]
    fn a_file_moved_out_before_it_settles_is_never_converted() {
        let d = Dir::new("moved-out");
        let mut w = Watcher::start(&spec(&d)).unwrap();
        let a = d.put("a.png", b"png");
        std::thread::sleep(Duration::from_millis(100));
        std::fs::rename(&a, d.0.join("a.png")).unwrap();
        assert!(collect(&mut w, SOON).is_empty());
    }

    #[test]
    fn the_folder_removed_and_restored_resumes() {
        let d = Dir::new("restored");
        d.put("old.png", b"png");
        let mut w = Watcher::start(&spec(&d)).unwrap();
        std::fs::remove_dir_all(d.input()).unwrap();
        assert_eq!(collect(&mut w, SOON), ["<missing>"]);
        std::fs::create_dir_all(d.input()).unwrap();
        d.put("b.png", b"png");
        assert_eq!(collect(&mut w, Duration::from_secs(3)), ["<back>", "b.png"]);
    }

    #[test]
    fn subfolders_are_watched_only_when_recursive() {
        let d = Dir::new("subfolders");
        let mut w = Watcher::start(&spec(&d)).unwrap();
        d.put("sub/a.png", b"png");
        assert!(collect(&mut w, SOON).is_empty());
        let mut s = spec(&d);
        s.recursive = true;
        let mut w = Watcher::start(&s).unwrap();
        d.put("sub/deeper/b.png", b"png");
        assert_eq!(collect(&mut w, SOON), ["b.png"]);
    }

    #[test]
    fn results_in_the_watched_folder_itself_is_refused() {
        let d = Dir::new("refused");
        let mut s = spec(&d);
        s.results = d.input();
        let e = Watcher::start(&s).err().unwrap();
        assert_eq!(e.code, ErrorCode::Usage);
        assert_eq!(e.reason, "results would be converted again");
        let mut s = spec(&d);
        s.done = Some(d.input().join("."));
        let e = Watcher::start(&s).err().unwrap();
        assert_eq!(e.code, ErrorCode::Usage);
        assert_eq!(e.reason, "moved originals would be converted again");
    }

    #[test]
    fn a_missing_folder_is_input_not_found() {
        let d = Dir::new("missing");
        let mut s = spec(&d);
        s.folder = d.0.join("nowhere");
        let e = Watcher::start(&s).err().unwrap();
        assert_eq!(e.code, ErrorCode::InputNotFound);
    }

    #[test]
    fn job_for_mirrors_the_subfolder() {
        let d = Dir::new("mirror");
        let s = spec(&d);
        let root = d.input();
        let job = s.job_for(&root, &root.join("2026/a.jpg"));
        assert_eq!(job.output.dir, Some(s.results.join("2026")));
        assert_eq!(job.inputs, [root.join("2026/a.jpg")]);
        assert_eq!(job.target, Some(FormatId::Webp));
        assert!(!job.output.overwrite);
        let top = s.job_for(&root, &root.join("b.jpg"));
        assert_eq!(top.output.dir, Some(s.results.clone()));
    }
}
