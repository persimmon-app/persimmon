// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The watch runner (spec D2, "The Tauri shell"): each enabled rule runs a
//! watcher thread and a worker thread. The worker converts one file at a
//! time in the shared pool, behind work you started (DD-56), records it,
//! moves the original, and adds it to the rule's burst. A clock thread
//! turns quiet bursts into History batches and summary notifications.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use persimmon_core::backend::Run;
use persimmon_core::error::{Error, ErrorCode};
use persimmon_core::exec::{CancelToken, Outcome, execute};
use persimmon_core::original::move_original;
use persimmon_core::output::Claims;
use persimmon_core::pool::{Lane, Pool, Priority};
use persimmon_core::registry::Registry;
use persimmon_core::watch::{Change, Timing, WatchSpec, Watcher, files_in};

use crate::history::{self, Batch, Item, ItemOutcome, MovedOriginal};
use crate::queue::QueueMode;
use crate::watch::burst::{Bursts, file_note, summary};
use crate::watch::record::{FileStamp, Record, record_path};
use crate::watch::rule::{ALLOW, CANT_READ, Rule};
use crate::wording::{Shown, app_error, shown};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Telling the person about unattended work (spec D9).
pub trait Notifier: Send + Sync {
    /// Whether notifications may be shown now: read before each one.
    fn allowed(&self) -> bool;
    /// `batch`: a burst's summary (Show files and Undo) rather than one
    /// file's note; `reveal` is what Show files shows.
    fn post(&self, id: &str, body: &str, batch: bool, reveal: Option<PathBuf>);
}

/// What the runner works with.
pub struct Deps {
    pub reg: Registry,
    pub engine: Arc<dyn Run + Send + Sync>,
    /// The queue's pool: watch files take its slots as background work.
    pub pool: Pool,
    /// The queue's output-name claims, so a watch output and a row never
    /// share a name.
    pub claims: Claims,
    /// `~/Library/Application Support/Persimmon`.
    pub data: PathBuf,
    /// How often a missing or unreadable folder is tried again.
    pub timing: Timing,
    /// How long a rule is quiet before its burst becomes a batch.
    pub quiet: Duration,
    pub on_batch: Box<dyn Fn(Batch) + Send + Sync>,
    pub on_status: Box<dyn Fn(RuleStatus) + Send + Sync>,
    pub notifier: Arc<dyn Notifier>,
}

/// A rule as it runs: the rule, its spec, and its preset's name.
#[derive(Debug, Clone)]
pub struct Running {
    pub rule: Rule,
    pub spec: WatchSpec,
    pub preset_name: Option<String>,
}

impl PartialEq for Running {
    fn eq(&self, other: &Running) -> bool {
        self.rule == other.rule
            && self.spec.options == other.spec.options
            && self.preset_name == other.preset_name
    }
}

/// What a rule is doing, for its line in the Watch folders view.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum WatchState {
    Watching,
    Converting { done: usize, total: usize },
    Missing,
    Unreadable { error: Shown },
    Paused,
}

/// The last file a rule couldn't convert (or move).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleStatus {
    pub id: String,
    #[serde(flatten)]
    pub state: WatchState,
    /// Files converted in the burst not yet in History.
    pub burst_done: usize,
    pub last_failed: Option<Failure>,
    pub note: Option<String>,
}

impl RuleStatus {
    fn new(id: &str, state: WatchState) -> RuleStatus {
        RuleStatus {
            id: id.into(),
            state,
            burst_done: 0,
            last_failed: None,
            note: None,
        }
    }
}

/// What a damaged record leaves the rule's line saying.
pub const DAMAGED_NOTE: &str =
    "Couldn't read what this rule had converted, so files already here were left alone";

/// One running rule's shared parts.
struct RuleShared {
    running: Running,
    cancel: CancelToken,
    record: Mutex<Record>,
    bursts: Mutex<Bursts>,
    queue: Mutex<VecDeque<PathBuf>>,
    wake: Condvar,
    /// The watched folder, resolved, once watching has started.
    root: Mutex<Option<PathBuf>>,
    /// Files converted since the queue was last empty.
    streak: Mutex<usize>,
}

struct RuleRun {
    shared: Arc<RuleShared>,
    threads: Vec<JoinHandle<()>>,
}

struct Shared {
    deps: Deps,
    runs: Mutex<BTreeMap<String, RuleRun>>,
    statuses: Mutex<BTreeMap<String, RuleStatus>>,
    stop: CancelToken,
}

/// Runs the enabled watch rules.
pub struct WatchRunner {
    shared: Arc<Shared>,
    clock: Mutex<Option<JoinHandle<()>>>,
}

impl WatchRunner {
    pub fn new(deps: Deps) -> WatchRunner {
        let shared = Arc::new(Shared {
            deps,
            runs: Mutex::new(BTreeMap::new()),
            statuses: Mutex::new(BTreeMap::new()),
            stop: CancelToken::new(),
        });
        let clock = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || shared.clock())
        };
        WatchRunner {
            shared,
            clock: Mutex::new(Some(clock)),
        }
    }

    /// Run exactly these rules: enabled ones that are new or changed
    /// (re)start; disabled, removed or changed ones stop first, their open
    /// burst becoming a batch.
    pub fn apply(&self, rules: Vec<Running>) {
        let mut ended = Vec::new();
        let mut runs = lock(&self.shared.runs);
        let ids: Vec<String> = runs.keys().cloned().collect();
        for id in ids {
            let keep = rules.iter().any(|r| {
                r.rule.id == id
                    && r.rule.enabled
                    && runs.get(&id).is_some_and(|run| run.shared.running == *r)
            });
            if !keep && let Some(run) = runs.remove(&id) {
                ended.extend(self.shared.stop_run(run));
            }
        }
        lock(&self.shared.statuses).retain(|id, _| rules.iter().any(|r| r.rule.id == *id));
        for r in rules {
            if !r.rule.enabled {
                self.shared
                    .set_status(RuleStatus::new(&r.rule.id, WatchState::Paused));
            } else if !runs.contains_key(&r.rule.id) {
                let run = self.shared.start_run(r);
                let id = run.shared.running.rule.id.clone();
                runs.insert(id, run);
            }
        }
        drop(runs);
        // Handed on with no lock held: History takes its own lock.
        for batch in ended {
            self.shared.deliver(batch);
        }
    }

    /// Mark the files in the rule's folder now as handled, so they're left
    /// alone; how many there were.
    pub fn baseline(&self, rule_id: &str, spec: &WatchSpec) -> usize {
        let files = files_in(spec).unwrap_or_default();
        self.baseline_files(rule_id, &files);
        files.len()
    }

    /// Mark `files` in the rule's record as they are now, so they're left
    /// alone.
    pub fn baseline_files(&self, rule_id: &str, files: &[PathBuf]) {
        let runs = lock(&self.shared.runs);
        match runs.get(rule_id) {
            Some(run) => {
                let mut record = lock(&run.shared.record);
                record.baseline(files);
                let _ = record.save();
            }
            None => {
                let mut record = Record::open(&record_path(&self.shared.deps.data, rule_id));
                record.baseline(files);
                let _ = record.save();
            }
        }
    }

    /// Look at the rule's folder again now.
    pub fn scan(&self, id: &str) {
        let mut runs = lock(&self.shared.runs);
        let Some(run) = runs.remove(id) else {
            return;
        };
        let running = run.shared.running.clone();
        let ended = self.shared.stop_run(run);
        let run = self.shared.start_run(running);
        runs.insert(id.to_string(), run);
        drop(runs);
        if let Some(batch) = ended {
            self.shared.deliver(batch);
        }
    }

    pub fn statuses(&self) -> Vec<RuleStatus> {
        lock(&self.shared.statuses).values().cloned().collect()
    }

    /// How many rules are running.
    pub fn active(&self) -> usize {
        lock(&self.shared.runs).len()
    }

    /// An undo is about to put `from` back as `stamp`: the rule must not
    /// take it for a new file.
    pub fn mark_restored(&self, rule: &str, from: &Path, stamp: FileStamp) {
        let runs = lock(&self.shared.runs);
        match runs.get(rule) {
            Some(run) => {
                let mut record = lock(&run.shared.record);
                record.mark(from, stamp);
                let _ = record.save();
            }
            None if !rule.is_empty() => {
                let mut record = Record::open(&record_path(&self.shared.deps.data, rule));
                record.mark(from, stamp);
                let _ = record.save();
            }
            None => {}
        }
    }

    /// Stop every rule (a file mid-way is cancelled, leaving nothing, and
    /// isn't recorded), turn open bursts into batches, and stop the clock.
    pub fn shutdown(&self) {
        self.shared.stop.cancel();
        if let Some(clock) = lock(&self.clock).take() {
            let _ = clock.join();
        }
        let runs: Vec<RuleRun> = std::mem::take(&mut *lock(&self.shared.runs))
            .into_values()
            .collect();
        for run in runs {
            if let Some(batch) = self.shared.stop_run(run) {
                self.shared.deliver(batch);
            }
        }
    }
}

impl Drop for WatchRunner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Sleep for `d`, or less if `cancel` fires; false if it fired.
fn nap(cancel: &CancelToken, d: Duration) -> bool {
    let end = Instant::now() + d;
    while Instant::now() < end {
        if cancel.is_cancelled() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20).min(d));
    }
    !cancel.is_cancelled()
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Shared {
    fn set_status(&self, status: RuleStatus) {
        lock(&self.statuses).insert(status.id.clone(), status.clone());
        (self.deps.on_status)(status);
    }

    /// Change a running rule's status.
    fn update(&self, rs: &RuleShared, f: impl FnOnce(&mut RuleStatus)) {
        let id = &rs.running.rule.id;
        let status = {
            let mut statuses = lock(&self.statuses);
            let s = statuses
                .entry(id.clone())
                .or_insert_with(|| RuleStatus::new(id, WatchState::Watching));
            f(s);
            s.clone()
        };
        (self.deps.on_status)(status);
    }

    fn start_run(self: &Arc<Self>, running: Running) -> RuleRun {
        let id = running.rule.id.clone();
        // Pruned and, when damaged, baselined only once the folder can be
        // read (in the watcher thread): a folder that's away must not
        // empty the record.
        let record = Record::open(&record_path(&self.deps.data, &id));
        let damaged = record.damaged();
        let rs = Arc::new(RuleShared {
            bursts: Mutex::new(Bursts::new(
                id.clone(),
                running.rule.folder.clone(),
                running.preset_name.clone(),
                self.deps.quiet,
            )),
            running,
            cancel: CancelToken::new(),
            record: Mutex::new(record),
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            root: Mutex::new(None),
            streak: Mutex::new(0),
        });
        let mut status = RuleStatus::new(&id, WatchState::Watching);
        status.note = damaged.then(|| DAMAGED_NOTE.to_string());
        self.set_status(status);
        let watch = {
            let (shared, rs) = (Arc::clone(self), Arc::clone(&rs));
            std::thread::spawn(move || shared.watch(&rs))
        };
        let work = {
            let (shared, rs) = (Arc::clone(self), Arc::clone(&rs));
            std::thread::spawn(move || shared.work(&rs))
        };
        RuleRun {
            shared: rs,
            threads: vec![watch, work],
        }
    }

    /// Stop a rule; its open burst comes back as a batch, for the caller
    /// to deliver once it holds no lock.
    fn stop_run(&self, run: RuleRun) -> Option<Batch> {
        run.shared.cancel.cancel();
        run.shared.wake.notify_all();
        for t in run.threads {
            let _ = t.join();
        }
        let _ = lock(&run.shared.record).save();
        lock(&run.shared.bursts).close()
    }

    /// A finished burst: into History, and its summary notification.
    fn deliver(&self, batch: Batch) {
        if self.deps.notifier.allowed() {
            let reveal = batch.items.iter().rev().find_map(|i| i.output.clone());
            self.deps
                .notifier
                .post(&batch.id, &summary(&batch), true, reveal);
        }
        (self.deps.on_batch)(batch);
    }

    /// The clock: quiet bursts become batches.
    fn clock(&self) {
        let tick = (self.deps.quiet / 2).min(Duration::from_secs(1));
        while nap(&self.stop, tick) {
            let rules: Vec<Arc<RuleShared>> = lock(&self.runs)
                .values()
                .map(|r| Arc::clone(&r.shared))
                .collect();
            for rs in rules {
                let due = lock(&rs.bursts).due(Instant::now());
                if let Some(batch) = due {
                    self.update(&rs, |s| s.burst_done = 0);
                    self.deliver(batch);
                }
            }
        }
    }

    /// A rule's watcher thread: start watching (waiting for a missing or
    /// unreadable folder), then queue each ready file the record doesn't
    /// already hold as it is.
    fn watch(&self, rs: &RuleShared) {
        let spec = &rs.running.spec;
        loop {
            if rs.cancel.is_cancelled() {
                return;
            }
            let mut watcher = match Watcher::start(spec) {
                Ok(w) => w,
                Err(e) => {
                    let state = match e.code {
                        ErrorCode::InputNotFound => WatchState::Missing,
                        ErrorCode::UnsupportedInput => WatchState::Unreadable {
                            error: app_error(
                                ErrorCode::UnsupportedInput,
                                &spec.folder,
                                CANT_READ,
                                Some(ALLOW),
                            ),
                        },
                        _ => WatchState::Unreadable { error: shown(&e) },
                    };
                    self.update(rs, |s| s.state = state);
                    if !nap(&rs.cancel, self.deps.timing.rescan) {
                        return;
                    }
                    continue;
                }
            };
            *lock(&rs.root) = Some(watcher.root().to_path_buf());
            {
                // The folder is there and readable: now the record can be
                // tidied, and a damaged one leave the files here alone,
                // before anything is handed out.
                let mut record = lock(&rs.record);
                if record.take_damaged() {
                    record.baseline(&files_in(spec).unwrap_or_default());
                }
                record.prune(watcher.root());
                let _ = record.save();
            }
            self.update(rs, |s| {
                if !matches!(s.state, WatchState::Converting { .. }) {
                    s.state = WatchState::Watching;
                }
            });
            while let Some(change) = watcher.next(&rs.cancel) {
                match change {
                    Change::Missing => self.update(rs, |s| s.state = WatchState::Missing),
                    Change::Back => self.update(rs, |s| s.state = WatchState::Watching),
                    Change::Ready(path) => {
                        let stamp = std::fs::metadata(&path)
                            .ok()
                            .and_then(|m| FileStamp::of(&m));
                        let handled = stamp.is_none_or(|st| lock(&rs.record).handled(&path, st));
                        let mut queue = lock(&rs.queue);
                        if !handled && !queue.contains(&path) {
                            queue.push_back(path);
                            rs.wake.notify_all();
                        }
                    }
                }
            }
            return;
        }
    }

    /// A rule's worker thread: one file at a time.
    fn work(&self, rs: &RuleShared) {
        loop {
            let path = {
                let mut queue = lock(&rs.queue);
                loop {
                    if rs.cancel.is_cancelled() {
                        return;
                    }
                    if let Some(p) = queue.pop_front() {
                        break p;
                    }
                    queue = rs
                        .wake
                        .wait_timeout(queue, Duration::from_millis(100))
                        .unwrap_or_else(|p| p.into_inner())
                        .0;
                }
            };
            self.convert(rs, &path);
            if rs.cancel.is_cancelled() {
                return;
            }
            if lock(&rs.queue).is_empty() {
                *lock(&rs.streak) = 0;
                self.update(rs, |s| {
                    if matches!(s.state, WatchState::Converting { .. }) {
                        s.state = WatchState::Watching;
                    }
                });
            }
        }
    }

    fn convert(&self, rs: &RuleShared, path: &Path) {
        let spec = &rs.running.spec;
        let rule = &rs.running.rule;
        let Some(stamp) = std::fs::metadata(path)
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| FileStamp::of(&m))
        else {
            return;
        };
        if lock(&rs.record).handled(path, stamp) {
            return;
        }
        let done = *lock(&rs.streak);
        let total = done + lock(&rs.queue).len() + 1;
        self.update(rs, |s| s.state = WatchState::Converting { done, total });
        let batch_id = lock(&rs.bursts).open_id(Instant::now());
        let root = lock(&rs.root)
            .clone()
            .unwrap_or_else(|| spec.folder.clone());
        let job = spec.job_for(&root, path);
        let lane = job
            .plan(&self.deps.reg)
            .tasks
            .first()
            .map_or(Lane::Cpu, |t| Lane::of(t.backend));
        let Some(permit) = self
            .deps
            .pool
            .acquire(lane, Priority::Background, &rs.cancel)
        else {
            lock(&rs.bursts).idle();
            return;
        };
        let (plan, claim) = job.plan_claimed(&self.deps.reg, &self.deps.claims);
        let ended = match plan.errors.first() {
            Some(e) => Some((ItemOutcome::Failed { error: shown(e) }, None)),
            None => {
                let results = execute(&job, &plan, &*self.deps.engine, &rs.cancel, &mut |_| {});
                match results.into_iter().next() {
                    Some(Ok(Outcome::Written(out))) => Some((ItemOutcome::Done, Some(out))),
                    Some(Ok(Outcome::NotSmaller)) => Some((ItemOutcome::NotSmaller, None)),
                    Some(Err(e)) if e.code == ErrorCode::Cancelled => None,
                    Some(Err(e)) => Some((ItemOutcome::Failed { error: shown(&e) }, None)),
                    None if rs.cancel.is_cancelled() => None,
                    None => Some((
                        ItemOutcome::Failed {
                            error: shown(
                                &Error::new(ErrorCode::Internal, "the converter gave no result")
                                    .with_file(path),
                            ),
                        },
                        None,
                    )),
                }
            }
        };
        drop(claim);
        drop(permit);
        // Cancelled: nothing recorded, so it converts next time.
        let Some((outcome, output)) = ended else {
            lock(&rs.bursts).idle();
            return;
        };
        {
            // Recorded before the original moves, so a stop between the two
            // never converts it twice.
            let mut record = lock(&rs.record);
            record.mark(path, stamp);
            let _ = record.save();
        }
        let mut failure = match &outcome {
            ItemOutcome::Failed { error } => Some(Failure {
                name: name_of(path),
                reason: error.reason.clone(),
            }),
            _ => None,
        };
        let moved = match (&outcome, &spec.done) {
            (ItemOutcome::Done, Some(done)) => match move_original(path, &root, done) {
                Ok(m) => Some(MovedOriginal::from_core(&m)),
                Err(e) => {
                    failure = Some(Failure {
                        name: name_of(path),
                        reason: shown(&e).reason,
                    });
                    None
                }
            },
            _ => None,
        };
        let output_size = output
            .as_deref()
            .and_then(|o| std::fs::metadata(o).ok())
            .filter(|m| m.is_file())
            .map(|m| m.len());
        let item = Item {
            inputs: vec![path.to_path_buf()],
            output_mtime: output.as_deref().and_then(history::stamp),
            output: output.clone(),
            output_size,
            size: stamp.size,
            outcome,
            undone: false,
            to: Some(rule.target),
            mode: Some(QueueMode::Convert),
            moved,
            restored: false,
        };
        if rule.notify_each && self.deps.notifier.allowed() {
            let n = lock(&rs.bursts).len();
            self.deps
                .notifier
                .post(&format!("{batch_id}/{n}"), &file_note(&item), false, output);
        }
        let burst_done = {
            let mut bursts = lock(&rs.bursts);
            bursts.push(item, Instant::now());
            bursts.done()
        };
        let done = {
            let mut streak = lock(&rs.streak);
            *streak += 1;
            *streak
        };
        let total = done + lock(&rs.queue).len();
        self.update(rs, |s| {
            s.burst_done = burst_done;
            s.state = WatchState::Converting { done, total };
            if failure.is_some() {
                s.last_failed = failure;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{History, Trasher};
    use crate::presets::Presets;
    use crate::watch::rule::{self, Rule};
    use persimmon_core::backend::{Done, Run};
    use persimmon_core::error::{Error, ErrorCode};
    use persimmon_core::exec::CancelToken;
    use persimmon_core::format::FormatId;
    use persimmon_core::job::{JobOptions, PlannedTask};
    use persimmon_core::pool::{Limits, Pool};
    use persimmon_core::registry::Registry;
    use persimmon_core::watch::Timing;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Copies the input; fails `bad*`; holds `slow*` on its first try
    /// until cancelled. Counts tries per file name.
    #[derive(Default)]
    struct Fake {
        tries: Mutex<HashMap<String, usize>>,
    }

    impl Fake {
        fn tries(&self, name: &str) -> usize {
            self.tries.lock().unwrap().get(name).copied().unwrap_or(0)
        }
    }

    impl Run for Fake {
        fn run(
            &self,
            task: &PlannedTask,
            _: &JobOptions,
            temp: &Path,
            cancel: &CancelToken,
            _: &mut dyn FnMut(f32),
        ) -> Result<Done, Error> {
            let input = &task.inputs[0];
            let name = input.file_name().unwrap().to_string_lossy().into_owned();
            let n = {
                let mut t = self.tries.lock().unwrap();
                let n = t.entry(name.clone()).or_insert(0);
                *n += 1;
                *n
            };
            if name.starts_with("bad") {
                return Err(Error::new(ErrorCode::CorruptInput, "it's damaged").with_file(input));
            }
            if name.starts_with("slow") && n == 1 {
                std::fs::write(temp, b"partial").unwrap();
                let end = Instant::now() + Duration::from_secs(10);
                while !cancel.is_cancelled() {
                    assert!(Instant::now() < end, "never cancelled");
                    std::thread::sleep(Duration::from_millis(10));
                }
                return Err(Error::new(ErrorCode::Cancelled, "cancelled"));
            }
            std::fs::copy(input, temp).unwrap();
            Ok(Done::Written)
        }
    }

    #[derive(Default)]
    struct Notes {
        off: bool,
        posts: Mutex<Vec<(String, String, bool)>>,
    }

    impl Notifier for Notes {
        fn allowed(&self) -> bool {
            !self.off
        }
        fn post(&self, id: &str, body: &str, batch: bool, _reveal: Option<PathBuf>) {
            self.posts
                .lock()
                .unwrap()
                .push((id.into(), body.into(), batch));
        }
    }

    struct Rig {
        dir: PathBuf,
        engine: Arc<Fake>,
        notes: Arc<Notes>,
        batches: Arc<Mutex<Vec<crate::history::Batch>>>,
        statuses: Arc<Mutex<Vec<RuleStatus>>>,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn rig(name: &str) -> Rig {
        rig_with(name, Notes::default())
    }

    fn rig_with(name: &str, notes: Notes) -> Rig {
        let dir =
            std::env::temp_dir().join(format!("prsm-app-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        Rig {
            dir,
            engine: Arc::new(Fake::default()),
            notes: Arc::new(notes),
            batches: Arc::default(),
            statuses: Arc::default(),
        }
    }

    impl Rig {
        fn input(&self) -> PathBuf {
            self.dir.join("in")
        }

        fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let p = self.input().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
            p
        }

        fn png(&self, name: &str) -> PathBuf {
            self.put(name, &persimmon_corpus::PNG_1X1)
        }

        fn runner(&self) -> WatchRunner {
            let batches = Arc::clone(&self.batches);
            self.runner_with(Box::new(move |b| batches.lock().unwrap().push(b)))
        }

        fn runner_with(
            &self,
            on_batch: Box<dyn Fn(crate::history::Batch) + Send + Sync>,
        ) -> WatchRunner {
            let statuses = Arc::clone(&self.statuses);
            WatchRunner::new(Deps {
                reg: Registry::builtin(),
                engine: self.engine.clone(),
                pool: Pool::new(Limits { total: 2, media: 1 }),
                claims: Default::default(),
                data: self.dir.join("data"),
                timing: fast(),
                quiet: Duration::from_millis(400),
                on_batch,
                on_status: Box::new(move |s| statuses.lock().unwrap().push(s)),
                notifier: self.notes.clone(),
            })
        }

        fn rule(&self) -> Rule {
            let mut r = rule::draft(&Registry::builtin(), self.input(), FormatId::Webp);
            r.id = "r1".into();
            r
        }

        fn running(&self, r: &Rule) -> Running {
            let presets = Presets::load(&self.dir.join("presets"));
            let (mut spec, preset_name) = rule::spec(r, &presets, &Registry::builtin());
            spec.timing = fast();
            Running {
                rule: r.clone(),
                spec,
                preset_name,
            }
        }

        fn results(&self) -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(self.input().join("converted"))
                .map(|d| {
                    d.filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            v.sort();
            v
        }

        fn batches(&self) -> Vec<crate::history::Batch> {
            self.batches.lock().unwrap().clone()
        }

        fn last_state(&self) -> Option<WatchState> {
            self.statuses
                .lock()
                .unwrap()
                .last()
                .map(|s| s.state.clone())
        }
    }

    fn fast() -> Timing {
        Timing {
            settle: Duration::from_millis(150),
            tick: Duration::from_millis(30),
            rescan: Duration::from_millis(300),
        }
    }

    fn wait(what: &str, cond: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(15);
        while !cond() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Long enough for several rescans to have handed out anything left.
    fn settle_down() {
        std::thread::sleep(Duration::from_millis(1_200));
    }

    #[test]
    fn each_file_converts_once() {
        let r = rig("once");
        let w = r.runner();
        let rule = r.rule();
        w.apply(vec![r.running(&rule)]);
        for i in 0..20 {
            r.png(&format!("p{i:02}.png"));
        }
        wait("20 outputs", || r.results().len() == 20);
        wait("one batch", || r.batches().len() == 1);
        assert_eq!(r.batches()[0].items.len(), 20);
        w.scan("r1");
        settle_down();
        assert_eq!(r.results().len(), 20);
        assert!((0..20).all(|i| r.engine.tries(&format!("p{i:02}.png")) == 1));
        assert_eq!(w.active(), 1);
    }

    #[test]
    fn a_restart_converts_what_arrived_meanwhile_and_nothing_twice() {
        let r = rig("restart");
        let rule = r.rule();
        {
            let w = r.runner();
            w.apply(vec![r.running(&rule)]);
            r.png("a.png");
            wait("a converted", || r.results() == ["a.webp"]);
            w.shutdown();
        }
        r.png("b.png");
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        wait("b converted", || r.results().len() == 2);
        settle_down();
        assert_eq!(r.results(), ["a.webp", "b.webp"]);
        assert_eq!(r.engine.tries("a.png"), 1);
    }

    #[test]
    fn files_already_there_are_left_alone_after_baseline() {
        let r = rig("baseline");
        r.png("old.png");
        let rule = r.rule();
        let w = r.runner();
        let running = r.running(&rule);
        assert_eq!(w.baseline("r1", &running.spec), 1);
        w.apply(vec![running]);
        settle_down();
        assert!(r.results().is_empty());
        r.png("new.png");
        wait("new converted", || r.results() == ["new.webp"]);
    }

    #[test]
    fn without_a_baseline_they_convert() {
        let r = rig("no-baseline");
        r.png("old.png");
        let rule = r.rule();
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        wait("old converted", || r.results() == ["old.webp"]);
    }

    #[test]
    fn a_failed_file_is_not_retried_until_it_changes() {
        let r = rig("failed");
        let rule = r.rule();
        {
            let w = r.runner();
            w.apply(vec![r.running(&rule)]);
            r.png("bad.png");
            wait("bad tried", || r.engine.tries("bad.png") == 1);
            wait("a batch", || r.batches().len() == 1);
            let item = &r.batches()[0].items[0];
            assert!(matches!(
                item.outcome,
                crate::history::ItemOutcome::Failed { .. }
            ));
            let s = w.statuses();
            assert_eq!(
                s[0].last_failed.as_ref().map(|f| f.name.as_str()),
                Some("bad.png")
            );
            w.shutdown();
        }
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        settle_down();
        assert_eq!(r.engine.tries("bad.png"), 1);
        let mut bytes = persimmon_corpus::PNG_1X1.to_vec();
        bytes.extend_from_slice(b"changed");
        r.put("bad.png", &bytes);
        wait("bad tried again", || r.engine.tries("bad.png") == 2);
    }

    /// Moves into a folder of its own instead of the real Trash.
    struct FakeTrash(PathBuf);
    impl Trasher for FakeTrash {
        fn trash(&self, path: &Path) -> std::io::Result<()> {
            std::fs::create_dir_all(&self.0)?;
            std::fs::rename(path, self.0.join(path.file_name().unwrap()))
        }
    }

    #[test]
    fn moved_originals_come_back_on_undo_and_are_not_converted_again() {
        let r = rig("restore");
        let mut rule = r.rule();
        rule.done = Some(r.input().join("done"));
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        let original = r.png("a.png");
        wait("a converted", || r.results() == ["a.webp"]);
        wait("a batch", || r.batches().len() == 1);
        assert!(!original.exists());
        let batch = r.batches()[0].clone();
        assert!(batch.items[0].moved.is_some());
        let mut h = History::open(&r.dir.join("history.jsonl"));
        h.append(batch.clone()).unwrap();
        let report = h.undo_restoring(
            &batch.id,
            &FakeTrash(r.dir.join("trash")),
            &mut |rule, from, stamp| w.mark_restored(rule, from, stamp),
        );
        assert_eq!(
            (report.trashed, report.restored),
            (1, 1),
            "{:?}",
            report.skipped
        );
        assert!(original.exists());
        settle_down();
        assert_eq!(
            r.engine.tries("a.png"),
            1,
            "the restored original converted again"
        );
        assert!(r.results().is_empty());
    }

    #[test]
    fn quitting_mid_file_leaves_no_part_and_converts_it_next_time() {
        let r = rig("quit");
        let rule = r.rule();
        {
            let w = r.runner();
            w.apply(vec![r.running(&rule)]);
            r.png("slow.png");
            wait("slow started", || r.engine.tries("slow.png") == 1);
            w.shutdown();
        }
        let leftovers: Vec<_> = std::fs::read_dir(r.input().join("converted"))
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        wait("slow converted", || r.results() == ["slow.webp"]);
    }

    #[test]
    fn a_burst_ends_with_one_summary() {
        let r = rig("summary");
        let mut rule = r.rule();
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        for n in ["a", "b", "c"] {
            r.png(&format!("{n}.png"));
        }
        wait("a batch", || r.batches().len() == 1);
        let posts = r.notes.posts.lock().unwrap().clone();
        assert_eq!(posts.len(), 1, "{posts:?}");
        assert!(posts[0].2);
        assert_eq!(posts[0].0, r.batches()[0].id);
        assert!(posts[0].1.ends_with("— 3 converted"), "{}", posts[0].1);
        rule.notify_each = true;
        w.apply(vec![r.running(&rule)]);
        r.png("d.png");
        wait("a second batch", || r.batches().len() == 2);
        let posts = r.notes.posts.lock().unwrap().clone();
        assert_eq!(posts.len(), 3, "{posts:?}");
        assert_eq!(posts[1].1, "d.png → d.webp");
        assert!(!posts[1].2 && posts[2].2);
    }

    #[test]
    fn notifications_off_loses_nothing() {
        let r = rig_with(
            "notes-off",
            Notes {
                off: true,
                ..Notes::default()
            },
        );
        let mut rule = r.rule();
        rule.notify_each = true;
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        r.png("a.png");
        wait("a batch", || r.batches().len() == 1);
        assert!(r.notes.posts.lock().unwrap().is_empty());
    }

    #[test]
    fn a_damaged_record_leaves_files_alone() {
        let r = rig("damaged");
        r.png("old.png");
        let path = crate::watch::record::record_path(&r.dir.join("data"), "r1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ damaged").unwrap();
        let rule = r.rule();
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        wait("the note", || {
            w.statuses().first().is_some_and(|s| s.note.is_some())
        });
        settle_down();
        assert!(r.results().is_empty());
        r.png("new.png");
        wait("new converted", || r.results() == ["new.webp"]);
    }

    #[test]
    fn a_folder_missing_at_launch_is_waited_for() {
        let r = rig("missing");
        std::fs::remove_dir_all(r.input()).unwrap();
        let rule = r.rule();
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        wait("missing", || r.last_state() == Some(WatchState::Missing));
        assert_eq!(w.active(), 1);
        std::fs::create_dir_all(r.input()).unwrap();
        r.png("a.png");
        wait("a converted", || r.results() == ["a.webp"]);
        assert_eq!(r.last_state(), Some(WatchState::Watching));
    }

    #[test]
    fn an_unreadable_folder_says_so_and_recovers() {
        use std::os::unix::fs::PermissionsExt;
        let r = rig("unreadable");
        let set = |mode| {
            std::fs::set_permissions(r.input(), std::fs::Permissions::from_mode(mode)).unwrap()
        };
        set(0o000);
        if std::fs::read_dir(r.input()).is_ok() {
            set(0o755);
            return; // root reads anything
        }
        let rule = r.rule();
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        wait("unreadable", || {
            matches!(r.last_state(), Some(WatchState::Unreadable { .. }))
        });
        let Some(WatchState::Unreadable { error }) = r.last_state() else {
            unreachable!()
        };
        set(0o755);
        assert_eq!(error.reason, "Persimmon can't read this folder");
        assert_eq!(
            error.fix.as_deref(),
            Some("Allow Persimmon in System Settings → Privacy & Security → Files and Folders")
        );
        r.png("a.png");
        wait("a converted", || r.results() == ["a.webp"]);
    }

    #[test]
    fn changing_a_rule_closes_its_burst_and_converts_nothing_twice() {
        let r = rig("change");
        let mut rule = r.rule();
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        r.png("a.png");
        wait("a converted", || r.results() == ["a.webp"]);
        rule.target = FormatId::Jpeg;
        rule.patterns = "*.png".into();
        w.apply(vec![r.running(&rule)]);
        assert_eq!(r.batches().len(), 1, "the open burst closed at once");
        r.png("b.png");
        wait("b converted", || r.results() == ["a.webp", "b.jpg"]);
        settle_down();
        assert_eq!(r.engine.tries("a.png"), 1);
    }

    #[test]
    fn a_deleted_results_folder_is_made_again() {
        let r = rig("results-gone");
        let rule = r.rule();
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        r.png("a.png");
        wait("a converted", || r.results() == ["a.webp"]);
        std::fs::remove_dir_all(r.input().join("converted")).unwrap();
        r.png("b.png");
        wait("b converted", || r.results() == ["b.webp"]);
    }

    #[test]
    fn a_paused_rule_says_so_and_converts_nothing() {
        let r = rig("paused");
        let mut rule = r.rule();
        rule.enabled = false;
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        assert_eq!(w.statuses()[0].state, WatchState::Paused);
        assert_eq!(w.active(), 0);
        r.png("a.png");
        settle_down();
        assert!(r.results().is_empty());
        w.apply(vec![]);
        assert!(w.statuses().is_empty());
    }

    /// History takes its own lock in `on_batch`, and an undo holds it while
    /// telling the runner about a restored original: stopping a rule must
    /// hand its batch on with no runner lock held, or the two deadlock.
    #[test]
    fn a_stopped_rules_batch_is_handed_on_with_no_lock_held() {
        let r = rig("no-deadlock");
        let history = Arc::new(Mutex::new(()));
        let w = {
            let history = Arc::clone(&history);
            Arc::new(r.runner_with(Box::new(move |_| drop(history.lock().unwrap()))))
        };
        let rule = r.rule();
        w.apply(vec![r.running(&rule)]);
        let a = r.png("a.png");
        wait("a converted", || r.results() == ["a.webp"]);
        let held = history.lock().unwrap();
        let stopper = {
            let w = Arc::clone(&w);
            std::thread::spawn(move || w.apply(vec![]))
        };
        std::thread::sleep(Duration::from_millis(300));
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let w = Arc::clone(&w);
            let stamp =
                crate::watch::record::FileStamp::of(&std::fs::metadata(&a).unwrap()).unwrap();
            std::thread::spawn(move || {
                w.mark_restored("r1", &a, stamp);
                let _ = tx.send(());
            });
        }
        let done = rx.recv_timeout(Duration::from_secs(5));
        drop(held);
        stopper.join().unwrap();
        assert!(
            done.is_ok(),
            "mark_restored waited on a runner lock held while delivering"
        );
    }

    /// A rule started while its folder is away (a drive unplugged) keeps
    /// what it had converted: nothing converts again when it's back.
    #[test]
    fn a_restart_while_the_folder_is_away_converts_nothing_twice() {
        let r = rig("away");
        let rule = r.rule();
        {
            let w = r.runner();
            w.apply(vec![r.running(&rule)]);
            r.png("a.png");
            wait("a converted", || r.results() == ["a.webp"]);
            w.shutdown();
        }
        let away = r.dir.join("away");
        std::fs::rename(r.input(), &away).unwrap();
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        wait("missing", || r.last_state() == Some(WatchState::Missing));
        std::thread::sleep(Duration::from_millis(400));
        std::fs::rename(&away, r.input()).unwrap();
        wait("watching", || r.last_state() == Some(WatchState::Watching));
        settle_down();
        assert_eq!(r.engine.tries("a.png"), 1, "a.png converted again");
        assert_eq!(r.results(), ["a.webp"]);
    }

    /// The same when the folder can't be read at start (D10: access reset
    /// after an update), until it's allowed again.
    #[test]
    fn a_restart_while_the_folder_is_unreadable_converts_nothing_twice() {
        use std::os::unix::fs::PermissionsExt;
        let r = rig("unreadable-restart");
        let set = |mode| {
            std::fs::set_permissions(r.input(), std::fs::Permissions::from_mode(mode)).unwrap()
        };
        let rule = r.rule();
        {
            let w = r.runner();
            w.apply(vec![r.running(&rule)]);
            r.png("a.png");
            wait("a converted", || r.results() == ["a.webp"]);
            w.shutdown();
        }
        set(0o000);
        if std::fs::read_dir(r.input()).is_ok() {
            set(0o755);
            return; // root reads anything
        }
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        wait("unreadable", || {
            matches!(r.last_state(), Some(WatchState::Unreadable { .. }))
        });
        std::thread::sleep(Duration::from_millis(400));
        set(0o755);
        wait("watching", || r.last_state() == Some(WatchState::Watching));
        settle_down();
        assert_eq!(r.engine.tries("a.png"), 1, "a.png converted again");
    }

    /// A damaged record found while the folder is away still leaves the
    /// files there alone once it's back.
    #[test]
    fn a_damaged_record_while_the_folder_is_away_leaves_files_alone() {
        let r = rig("damaged-away");
        r.png("old.png");
        let away = r.dir.join("away");
        std::fs::rename(r.input(), &away).unwrap();
        let path = crate::watch::record::record_path(&r.dir.join("data"), "r1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ damaged").unwrap();
        let rule = r.rule();
        let w = r.runner();
        w.apply(vec![r.running(&rule)]);
        wait("missing", || r.last_state() == Some(WatchState::Missing));
        std::fs::rename(&away, r.input()).unwrap();
        wait("watching", || r.last_state() == Some(WatchState::Watching));
        settle_down();
        assert_eq!(r.engine.tries("old.png"), 0, "old.png converted");
    }

    /// Editing a rule (here: matching JPEGs too) leaves the files it now
    /// matches alone, as a new rule does; files that arrive after convert.
    #[test]
    fn an_edited_rule_leaves_the_files_it_newly_matches_alone() {
        let r = rig("edit-scope");
        let mut rule = r.rule();
        rule.patterns = "*.png".into();
        r.put("x.jpg", &persimmon_corpus::PNG_1X1);
        let w = r.runner();
        let old = r.running(&rule);
        w.apply(vec![old.clone()]);
        rule.patterns = "*.png,*.jpg".into();
        let new = r.running(&rule);
        w.baseline_files(
            "r1",
            &crate::watch::rule::newly_matched(&old.spec, &new.spec),
        );
        w.apply(vec![new]);
        settle_down();
        assert_eq!(r.engine.tries("x.jpg"), 0, "x.jpg converted after the edit");
        r.put("y.jpg", &persimmon_corpus::PNG_1X1);
        wait("y converted", || r.results() == ["y.webp"]);
    }
}
