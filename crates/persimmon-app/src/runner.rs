// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The runner: a dispatcher thread starts queued rows, in queue order,
//! whenever the pool has a slot for their kind of work (D6): images, PDFs
//! and archives several at once, video and audio one at a time. Each row
//! runs on its own thread as a one-task job with its own cancel token, so
//! one row can be cancelled while the rest go on.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use persimmon_core::backend::{Backends, Run};
use persimmon_core::error::{Error, ErrorCode};
use persimmon_core::exec::{CancelToken, Event, Outcome, execute};
use persimmon_core::job::Job;
use persimmon_core::output::Claims;
use persimmon_core::pool::{Lane, Limits, Pool, Wanting};
use persimmon_core::registry::Registry;

use crate::history::{self, Batch, Item, ItemOutcome, Source};
use crate::queue::{Queue, RowId, RowState, RowView};
use crate::wording::shown;

/// Progress reaches the window at most this often per row.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// How often the dispatcher looks again when it's waiting for a slot that
/// someone outside this runner may free.
const RECHECK: Duration = Duration::from_millis(100);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

type Sink = Box<dyn Fn(RowView) + Send + Sync>;

struct State {
    running: HashMap<RowId, CancelToken>,
    go: bool,
    stop: bool,
    /// The lanes queued rows are waiting for, [cpu, media], held as wants
    /// so background work never takes a slot ahead of them (DD-56).
    wants: [Option<Wanting>; 2],
}

struct Inner {
    queue: Arc<Mutex<Queue>>,
    engine: Arc<dyn Run + Send + Sync>,
    reg: Registry,
    pool: Pool,
    /// Output names held by rows that are running, so two never share one.
    claims: Claims,
    sink: Sink,
    state: Mutex<State>,
    wake: Condvar,
    /// Told about each batch once nothing is queued or running (history).
    on_batch: Mutex<Option<BatchSink>>,
    /// Where the next batch comes from.
    source: Mutex<Source>,
    /// How each row of the running batch ended, in the order they ended.
    ended: Mutex<Vec<RowView>>,
    /// The threads of rows that are running or have just finished.
    rows: Mutex<Vec<JoinHandle<()>>>,
}

type BatchSink = Box<dyn Fn(Batch) + Send + Sync>;

pub struct Runner {
    inner: Arc<Inner>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Runner {
    pub fn new(
        queue: Arc<Mutex<Queue>>,
        backends: Backends,
        sink: impl Fn(RowView) + Send + Sync + 'static,
    ) -> Runner {
        Runner::with_engine(queue, Arc::new(backends), Limits::auto(), sink)
    }

    /// A runner over `engine`, running at most what `limits` allows at once.
    pub fn with_engine(
        queue: Arc<Mutex<Queue>>,
        engine: Arc<dyn Run + Send + Sync>,
        limits: Limits,
        sink: impl Fn(RowView) + Send + Sync + 'static,
    ) -> Runner {
        let inner = Arc::new(Inner {
            queue,
            engine,
            reg: Registry::builtin(),
            pool: Pool::new(limits),
            claims: Claims::new(),
            sink: Box::new(sink),
            state: Mutex::new(State {
                running: HashMap::new(),
                go: false,
                stop: false,
                wants: [None, None],
            }),
            wake: Condvar::new(),
            on_batch: Mutex::new(None),
            source: Mutex::new(Source::Window),
            ended: Mutex::new(Vec::new()),
            rows: Mutex::new(Vec::new()),
        });
        let worker = {
            let inner = Arc::clone(&inner);
            std::thread::spawn(move || inner.work())
        };
        Runner {
            inner,
            worker: Mutex::new(Some(worker)),
        }
    }

    /// Be told about each finished batch.
    pub fn set_on_batch(&self, f: impl Fn(Batch) + Send + Sync + 'static) {
        *lock(&self.inner.on_batch) = Some(Box::new(f));
    }

    /// Run every queued row, several at once within the pool's limits,
    /// recording where the batch came from.
    pub fn start_from(&self, source: Source) {
        *lock(&self.inner.source) = source;
        lock(&self.inner.state).go = true;
        self.inner.wake.notify_all();
    }

    /// Start from the main window.
    pub fn start(&self) {
        self.start_from(Source::Window);
    }

    /// Cancel row `id`: a running one stops mid-file where the backend
    /// can; a queued one is marked cancelled. False if it's neither.
    pub fn cancel(&self, id: RowId) -> bool {
        // Locks are always taken queue first, then state.
        let mut queue = lock(&self.inner.queue);
        match queue.get(id).map(|v| v.state) {
            Some(RowState::Queued) => {
                let v = queue.update(id, RowState::Cancelled);
                drop(queue);
                if let Some(v) = v {
                    (self.inner.sink)(v);
                }
                self.inner.wake.notify_all();
                true
            }
            Some(RowState::Running { .. }) => {
                let state = lock(&self.inner.state);
                match state.running.get(&id) {
                    Some(token) => {
                        token.cancel();
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// Queue a failed or cancelled row again and start.
    pub fn retry(&self, id: RowId) -> Option<RowView> {
        let v = lock(&self.inner.queue).requeue(id)?;
        (self.inner.sink)(v.clone());
        self.start();
        Some(v)
    }

    /// The pool the rows run in, for work that shares it (watch folders).
    pub fn pool(&self) -> &Pool {
        &self.inner.pool
    }

    /// The engine rows run on, for work that shares it (watch folders).
    pub fn engine(&self) -> Arc<dyn Run + Send + Sync> {
        Arc::clone(&self.inner.engine)
    }

    /// The output names held by running rows, shared with watch rules so
    /// a row and a watch output never share a name.
    pub fn claims(&self) -> Claims {
        self.inner.claims.clone()
    }

    /// How many rows are running.
    pub fn running(&self) -> usize {
        lock(&self.inner.state).running.len()
    }

    /// Cancel every running row, wait for them, and stop. Rows still
    /// queued stay queued.
    pub fn shutdown(&self) {
        {
            let mut state = lock(&self.inner.state);
            state.stop = true;
            for token in state.running.values() {
                token.cancel();
            }
        }
        self.inner.wake.notify_all();
        if let Some(worker) = lock(&self.worker).take() {
            let _ = worker.join();
        }
        let rows: Vec<JoinHandle<()>> = lock(&self.inner.rows).drain(..).collect();
        for row in rows {
            let _ = row.join();
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Inner {
    fn work(self: &Arc<Self>) {
        loop {
            {
                let mut state = lock(&self.state);
                while !state.go && !state.stop {
                    state = self.wake.wait(state).unwrap_or_else(|p| p.into_inner());
                }
                if state.stop {
                    return;
                }
                state.go = false;
            }
            let started = history::now();
            let source = *lock(&self.source);
            let stopping = self.drain();
            // Recorded even when quitting cut the batch short.
            let ended: Vec<RowView> = lock(&self.ended).drain(..).collect();
            if !ended.is_empty() {
                let rows = in_queue_order(&lock(&self.queue).rows(), ended);
                let batch = record(started, source, &rows);
                if let Some(f) = lock(&self.on_batch).as_ref() {
                    f(batch);
                }
            }
            if stopping {
                return;
            }
        }
    }

    /// Start rows until nothing is queued or running; true if stopping.
    fn drain(self: &Arc<Self>) -> bool {
        loop {
            let token = CancelToken::new();
            let mut queue = lock(&self.queue);
            let mut state = lock(&self.state);
            if state.stop {
                // Wait for the cancelled rows to end before recording; they
                // need the queue to record how they ended.
                drop(queue);
                state.wants = [None, None];
                while !state.running.is_empty() {
                    state = self.wake.wait(state).unwrap_or_else(|p| p.into_inner());
                }
                return true;
            }
            let mut permit = None;
            let next = queue.next_queued_where(|row| {
                permit = self.pool.try_acquire(self.lane_of(row));
                permit.is_some()
            });
            if let (Some((id, job)), Some(permit)) = (next, permit) {
                state.running.insert(id, token.clone());
                let view = queue.get(id);
                drop(state);
                drop(queue);
                if let Some(v) = view {
                    (self.sink)(v);
                }
                let inner = Arc::clone(self);
                let handle = std::thread::spawn(move || {
                    // A backend that panics fails its own row; the slot, the
                    // name claim and the temp file go with the unwinding.
                    let end = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        inner.run(id, &job, &token)
                    }))
                    .unwrap_or_else(|_| crashed(&job));
                    let view = inner.set(id, end);
                    drop(permit);
                    // Into `ended` before leaving `running`: once nothing is
                    // running, every row that ended is there to record.
                    if let Some(v) = view {
                        lock(&inner.ended).push(v);
                    }
                    lock(&inner.state).running.remove(&id);
                    inner.wake.notify_all();
                });
                let mut rows = lock(&self.rows);
                rows.retain(|h| !h.is_finished());
                rows.push(handle);
                continue;
            }
            if state.running.is_empty() && !queue.any_queued() {
                state.wants = [None, None];
                return false;
            }
            // Rows still queued wait for their lanes, ahead of background work.
            let mut wanted = [false, false];
            for row in queue.rows().iter().filter(|r| r.state == RowState::Queued) {
                wanted[lane_index(self.lane_of(row))] = true;
            }
            for (i, lane) in [Lane::Cpu, Lane::Media].into_iter().enumerate() {
                if !wanted[i] {
                    state.wants[i] = None;
                } else if state.wants[i].is_none() {
                    state.wants[i] = Some(self.pool.want(lane));
                }
            }
            drop(queue);
            // A row ending, a start, a cancel, a retry or a stop wakes this.
            drop(
                self.wake
                    .wait_timeout(state, RECHECK)
                    .unwrap_or_else(|p| p.into_inner()),
            );
        }
    }

    /// Which pool lane a row's work takes.
    fn lane_of(&self, row: &RowView) -> Lane {
        row.from
            .and_then(|from| self.reg.find(row.op, from, row.to))
            .map_or(Lane::Cpu, |cap| Lane::of(cap.backend))
    }

    /// Plan the row for real, claiming its output name, then run it;
    /// returns how it ended.
    fn run(&self, id: RowId, job: &Job, token: &CancelToken) -> RowState {
        let (plan, _claim) = job.plan_claimed(&self.reg, &self.claims);
        if let Some(e) = plan.errors.first() {
            return RowState::Failed { error: shown(e) };
        }
        let mut last = Instant::now() - PROGRESS_EVERY;
        let results = execute(job, &plan, &*self.engine, token, &mut |event| {
            if let Event::Progress { fraction, .. } = event
                && last.elapsed() >= PROGRESS_EVERY
            {
                last = Instant::now();
                self.set(
                    id,
                    RowState::Running {
                        fraction: Some(fraction),
                    },
                );
            }
        });
        outcome_of(results, token.is_cancelled())
    }

    fn set(&self, id: RowId, state: RowState) -> Option<RowView> {
        let v = lock(&self.queue).update(id, state);
        if let Some(v) = &v {
            (self.sink)(v.clone());
        }
        v
    }
}

/// A lane's slot in [`State::wants`].
fn lane_index(lane: Lane) -> usize {
    match lane {
        Lane::Cpu => 0,
        Lane::Media => 1,
    }
}

/// How a row whose backend panicked ends.
fn crashed(job: &Job) -> RowState {
    let e = Error::new(ErrorCode::Internal, "the converter stopped unexpectedly");
    let e = match job.inputs.as_slice() {
        [one] => e.with_file(one),
        _ => e,
    };
    RowState::Failed { error: shown(&e) }
}

/// The batch's rows, each once as it last ended, in the queue's order;
/// rows no longer in the queue go last, in the order they ended.
fn in_queue_order(queue: &[RowView], ended: Vec<RowView>) -> Vec<RowView> {
    let mut last: Vec<RowView> = Vec::new();
    for v in ended {
        last.retain(|r| r.id != v.id);
        last.push(v);
    }
    let position = |id: RowId| queue.iter().position(|r| r.id == id).unwrap_or(usize::MAX);
    last.sort_by_key(|r| position(r.id));
    last
}

static BATCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A batch from the rows one drain of the queue ran.
fn record(started: u64, source: Source, rows: &[RowView]) -> Batch {
    use persimmon_core::registry::Operation;
    let mode = match rows[0].op {
        Operation::Compress => crate::queue::QueueMode::Compress,
        Operation::Extract => crate::queue::QueueMode::Extract,
        Operation::Merge => crate::queue::QueueMode::Combine,
        _ => crate::queue::QueueMode::Convert,
    };
    let first = rows[0].preset.clone();
    let preset = rows
        .iter()
        .all(|r| r.preset == first)
        .then_some(first)
        .flatten();
    let items = rows
        .iter()
        .map(|r| {
            let (outcome, output_size) = match &r.state {
                RowState::Done { output_size, .. } => (ItemOutcome::Done, *output_size),
                RowState::NotSmaller => (ItemOutcome::NotSmaller, None),
                RowState::Failed { error } => (
                    ItemOutcome::Failed {
                        error: error.clone(),
                    },
                    None,
                ),
                _ => (ItemOutcome::Cancelled, None),
            };
            let output = match outcome {
                ItemOutcome::Done => r.output.clone(),
                _ => None,
            };
            Item {
                inputs: r.inputs.clone(),
                output_mtime: output.as_deref().and_then(history::stamp),
                output,
                output_size,
                size: r.size,
                outcome,
                undone: false,
                to: r.to,
                mode: Some(match r.op {
                    Operation::Compress => crate::queue::QueueMode::Compress,
                    Operation::Extract => crate::queue::QueueMode::Extract,
                    Operation::Merge => crate::queue::QueueMode::Combine,
                    _ => crate::queue::QueueMode::Convert,
                }),
                moved: None,
                restored: false,
            }
        })
        .collect();
    let n = BATCHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Batch {
        id: format!("{started}-{n}"),
        started,
        finished: history::now(),
        source,
        preset,
        mode,
        items,
        undone: false,
        rule: None,
        folder: None,
    }
}

/// How a row ended, from what the engine returned for its one task, and
/// whether its cancel token fired.
fn outcome_of(
    results: Vec<Result<Outcome, persimmon_core::error::Error>>,
    cancelled: bool,
) -> RowState {
    match results.into_iter().next() {
        Some(Ok(outcome @ (Outcome::Written(_) | Outcome::WrittenNoted(..)))) => {
            let (output, note) = match outcome {
                Outcome::WrittenNoted(output, note) => (output, Some(note.text)),
                Outcome::Written(output) => (output, None),
                Outcome::NotSmaller => unreachable!(),
            };
            let size = std::fs::metadata(&output)
                .ok()
                .filter(|m| m.is_file())
                .map(|m| m.len());
            RowState::Done {
                output,
                output_size: size,
                note,
            }
        }
        Some(Ok(Outcome::NotSmaller)) => RowState::NotSmaller,
        Some(Err(e)) if e.code == ErrorCode::Cancelled => RowState::Cancelled,
        Some(Err(e)) => RowState::Failed { error: shown(&e) },
        // Nothing ran: a cancel before the task started, or else the
        // engine gave no answer, which is a failure, not a cancel.
        None if cancelled => RowState::Cancelled,
        None => RowState::Failed {
            error: shown(&persimmon_core::error::Error::new(
                ErrorCode::Internal,
                "Persimmon stopped this file without a result",
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{QueueMode, RowState};
    #[cfg(feature = "vips")]
    use persimmon_core::error::ErrorCode;
    use persimmon_core::format::FormatId;
    use persimmon_corpus::PNG_1X1;
    use persimmon_corpus::media::{media_sample, test_ffmpeg};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d =
                std::env::temp_dir().join(format!("prsm-app-runner-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
        fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        }
        fn no_partials(&self) -> bool {
            std::fs::read_dir(&self.0).unwrap().all(|e| {
                let n = e.unwrap().file_name().to_string_lossy().into_owned();
                !n.ends_with(".part") && !n.starts_with(".prsm-")
            })
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    type Seen = Arc<Mutex<Vec<RowView>>>;

    fn setup(backends: Backends) -> (Arc<Mutex<Queue>>, Runner, Seen) {
        let queue = Arc::new(Mutex::new(Queue::new(
            Registry::builtin(),
            persimmon_core::output::OutputPolicy::default(),
        )));
        let seen: Seen = Arc::default();
        let s = Arc::clone(&seen);
        let runner = Runner::new(Arc::clone(&queue), backends, move |v| {
            s.lock().unwrap().push(v)
        });
        (queue, runner, seen)
    }

    fn wait(what: &str, secs: u64, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(secs);
        while !done() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn state(q: &Arc<Mutex<Queue>>, id: RowId) -> RowState {
        q.lock().unwrap().get(id).unwrap().state
    }

    fn finished(s: &RowState) -> bool {
        !matches!(s, RowState::Queued | RowState::Running { .. })
    }

    /// FFmpeg for the tests that need it; None skips them, unless CI
    /// requires FFmpeg.
    fn ffmpeg() -> Option<PathBuf> {
        let found = test_ffmpeg();
        if found.is_none() && std::env::var_os("PERSIMMON_REQUIRE_FFMPEG").is_some() {
            panic!("PERSIMMON_REQUIRE_FFMPEG is set but no FFmpeg was found");
        }
        found
    }

    fn no_ffmpeg_child() -> bool {
        !std::process::Command::new("/usr/bin/pgrep")
            .args(["-P", &std::process::id().to_string(), "ffmpeg"])
            .status()
            .unwrap()
            .success()
    }

    /// A long video, queued to re-encode to WebM, then a PNG.
    fn video_then_png(d: &Dir, q: &Arc<Mutex<Queue>>, ff: &Path) -> (RowId, RowId) {
        let _ = ff;
        let src = media_sample("vid-long").unwrap();
        let video = d.0.join(src.file_name().unwrap());
        std::fs::copy(&src, &video).unwrap();
        let mut q = q.lock().unwrap();
        let v = q.add(&[video], QueueMode::Convert)[0].id;
        q.set_target(&[v], FormatId::Webm);
        let p = q.add(&[d.put("b.png", &PNG_1X1)], QueueMode::Convert)[0].id;
        (v, p)
    }

    // Converts images, so needs the image backend (libvips).
    #[cfg(feature = "vips")]
    #[test]
    fn runs_rows_in_order() {
        let d = Dir::new("order");
        let (q, r, seen) = setup(Backends::new());
        let ids: Vec<RowId> = q
            .lock()
            .unwrap()
            .add(
                &[d.put("a.png", &PNG_1X1), d.put("b.png", &PNG_1X1)],
                QueueMode::Convert,
            )
            .iter()
            .map(|v| v.id)
            .collect();
        r.start();
        wait("both rows", 30, || {
            ids.iter().all(|&i| finished(&state(&q, i)))
        });
        for &i in &ids {
            let RowState::Done {
                output,
                output_size: size,
                ..
            } = state(&q, i)
            else {
                panic!("{:?}", state(&q, i))
            };
            assert!(output.is_file());
            assert!(size.unwrap() > 0);
        }
        let seen = seen.lock().unwrap();
        let runs = |id| {
            seen.iter()
                .position(|v: &RowView| v.id == id && matches!(v.state, RowState::Running { .. }))
                .unwrap()
        };
        assert!(
            runs(ids[0]) < runs(ids[1]),
            "the second row started before the first"
        );
    }

    // Converts images, so needs the image backend (libvips).
    #[cfg(feature = "vips")]
    #[test]
    fn start_replans_and_reports_a_vanished_file() {
        let d = Dir::new("vanished");
        let (q, r, _) = setup(Backends::new());
        let a = d.put("a.png", &PNG_1X1);
        let ids: Vec<RowId> = q
            .lock()
            .unwrap()
            .add(&[a.clone(), d.put("b.png", &PNG_1X1)], QueueMode::Convert)
            .iter()
            .map(|v| v.id)
            .collect();
        std::fs::remove_file(&a).unwrap();
        r.start();
        wait("both rows", 30, || {
            ids.iter().all(|&i| finished(&state(&q, i)))
        });
        let RowState::Failed { error } = state(&q, ids[0]) else {
            panic!("{:?}", state(&q, ids[0]))
        };
        assert_eq!(error.code, ErrorCode::InputNotFound);
        assert!(matches!(state(&q, ids[1]), RowState::Done { .. }));
    }

    // Converts images, so needs the image backend (libvips).
    #[cfg(feature = "vips")]
    #[test]
    fn cancel_one_row_leaves_the_rest_running() {
        let Some(ff) = ffmpeg() else { return };
        let _serial = crate::FFMPEG_TESTS
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let d = Dir::new("cancel");
        let (q, r, _) = setup(Backends::new().with_ffmpeg(&ff));
        let (v, p) = video_then_png(&d, &q, &ff);
        r.start();
        wait("the video to run", 30, || {
            matches!(state(&q, v), RowState::Running { .. })
        });
        std::thread::sleep(Duration::from_millis(500));
        assert!(r.cancel(v));
        wait("both rows", 60, || {
            finished(&state(&q, v)) && finished(&state(&q, p))
        });
        assert_eq!(state(&q, v), RowState::Cancelled);
        assert!(matches!(state(&q, p), RowState::Done { .. }));
        assert!(d.no_partials());
        assert!(no_ffmpeg_child());
    }

    #[test]
    fn cancel_a_queued_row() {
        let d = Dir::new("queued");
        let (q, r, _) = setup(Backends::new());
        let ids: Vec<RowId> = q
            .lock()
            .unwrap()
            .add(
                &[d.put("a.png", &PNG_1X1), d.put("b.png", &PNG_1X1)],
                QueueMode::Convert,
            )
            .iter()
            .map(|v| v.id)
            .collect();
        assert!(r.cancel(ids[0]));
        assert_eq!(state(&q, ids[0]), RowState::Cancelled);
        r.start();
        wait("the other row", 30, || finished(&state(&q, ids[1])));
        assert_eq!(state(&q, ids[0]), RowState::Cancelled);
        assert!(!d.0.join("a.jpg").exists());
    }

    // Converts images, so needs the image backend (libvips).
    #[cfg(feature = "vips")]
    #[test]
    fn retry_after_failure() {
        let d = Dir::new("retry");
        let (q, r, _) = setup(Backends::new());
        let a = d.0.join("a.png");
        let id = q
            .lock()
            .unwrap()
            .add(std::slice::from_ref(&a), QueueMode::Convert)[0]
            .id;
        assert!(matches!(state(&q, id), RowState::Failed { .. }));
        std::fs::write(&a, PNG_1X1).unwrap();
        assert!(r.retry(id).is_some());
        wait("the retried row", 30, || finished(&state(&q, id)));
        assert!(
            matches!(state(&q, id), RowState::Done { .. }),
            "{:?}",
            state(&q, id)
        );
    }

    #[test]
    fn a_job_with_no_result_fails() {
        assert!(matches!(outcome_of(vec![], false), RowState::Failed { .. }));
        assert_eq!(outcome_of(vec![], true), RowState::Cancelled);
    }

    #[test]
    fn a_noted_outcome_keeps_its_note() {
        use persimmon_core::error::{Note, NoteKind};
        let note = Note {
            kind: NoteKind::UsedSoftware,
            text: "hardware encoder unavailable — used software".into(),
        };
        let state = outcome_of(
            vec![Ok(Outcome::WrittenNoted("/nowhere/a.mp4".into(), note))],
            false,
        );
        assert_eq!(
            state,
            RowState::Done {
                output: "/nowhere/a.mp4".into(),
                output_size: None,
                note: Some("hardware encoder unavailable — used software".into()),
            }
        );
    }

    #[test]
    fn shutdown_cancels_the_running_row() {
        let Some(ff) = ffmpeg() else { return };
        let _serial = crate::FFMPEG_TESTS
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let d = Dir::new("shutdown");
        let (q, r, _) = setup(Backends::new().with_ffmpeg(&ff));
        let (v, _) = video_then_png(&d, &q, &ff);
        r.start();
        wait("the video to run", 30, || {
            matches!(state(&q, v), RowState::Running { .. })
        });
        std::thread::sleep(Duration::from_millis(500));
        assert!(r.running() > 0);
        r.shutdown();
        assert_eq!(state(&q, v), RowState::Cancelled);
        assert_eq!(r.running(), 0);
        assert!(d.no_partials());
        assert!(no_ffmpeg_child());
    }

    /// A file in a folder nobody can write lands in the fallback, and the
    /// row says so.
    #[cfg(feature = "vips")]
    #[test]
    fn fallback_row_says_where_it_went() {
        use std::os::unix::fs::PermissionsExt;
        let d = Dir::new("fallback");
        let locked = d.0.join("locked");
        std::fs::create_dir(&locked).unwrap();
        let src = locked.join("a.png");
        std::fs::write(&src, PNG_1X1).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let fb = d.0.join("fb");
        let policy = persimmon_core::output::OutputPolicy {
            fallback: Some(fb.clone()),
            ..Default::default()
        };
        let queue = Arc::new(Mutex::new(Queue::new(Registry::builtin(), policy)));
        let r = Runner::new(Arc::clone(&queue), Backends::new(), |_| {});
        let id = queue.lock().unwrap().add(&[src], QueueMode::Convert)[0].id;
        r.start();
        wait("the row", 30, || finished(&state(&queue, id)));
        let v = queue.lock().unwrap().get(id).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(v.state, RowState::Done { .. }), "{:?}", v.state);
        assert_eq!(v.output.as_deref(), Some(fb.join("a.jpg").as_path()));
        assert_eq!(
            v.note.as_deref(),
            Some(
                format!(
                    "Saved to {} — the original's folder isn't writable",
                    crate::queue::tilde(&fb)
                )
                .as_str()
            )
        );
    }

    /// A preset's options reach the engine: Web image caps the long edge
    /// at 2048 px.
    #[cfg(feature = "vips")]
    #[test]
    fn options_reach_the_engine() {
        let d = Dir::new("preset-options");
        let small = d.put("small.png", &PNG_1X1);
        let big = d.0.join("big.png");
        let made = std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "png", "--resampleHeightWidth", "64", "4096"])
            .arg(&small)
            .arg("--out")
            .arg(&big)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        let (q, r, _) = setup(Backends::new());
        let web = crate::presets::builtins()
            .into_iter()
            .find(|p| p.id == "web-image")
            .unwrap();
        let id = q
            .lock()
            .unwrap()
            .add_with(&[big], &crate::queue::Recipe::Preset(web))[0]
            .id;
        r.start();
        wait("the row", 30, || finished(&state(&q, id)));
        let RowState::Done { output, .. } = state(&q, id) else {
            panic!("{:?}", state(&q, id))
        };
        let out = std::process::Command::new("/usr/bin/sips")
            .args(["-g", "pixelWidth"])
            .arg(&output)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("pixelWidth: 2048"), "{text}");
    }

    /// One Start's rows make one batch, recorded once the queue is empty.
    #[cfg(feature = "vips")]
    #[test]
    fn a_batch_is_recorded_when_the_queue_empties() {
        use crate::history::{Batch, ItemOutcome, Source};
        let d = Dir::new("batch");
        let (q, r, _) = setup(Backends::new());
        let seen: Arc<Mutex<Vec<Batch>>> = Arc::default();
        let s2 = Arc::clone(&seen);
        r.set_on_batch(move |b| s2.lock().unwrap().push(b));
        q.lock().unwrap().add(
            &[d.put("a.png", &PNG_1X1), d.put("b.png", &PNG_1X1)],
            QueueMode::Convert,
        );
        r.start_from(Source::Drop);
        wait("the batch", 30, || !seen.lock().unwrap().is_empty());
        let batches = seen.lock().unwrap();
        assert_eq!(batches.len(), 1);
        let b = &batches[0];
        assert_eq!((b.source, b.mode), (Source::Drop, QueueMode::Convert));
        assert_eq!(b.items.len(), 2);
        assert!(b.items.iter().all(|i| i.outcome == ItemOutcome::Done
            && i.output_mtime.is_some()
            && i.output_size.is_some()));
        assert!(b.finished >= b.started);
    }

    mod several {
        use super::*;
        use crate::history::Batch;
        use persimmon_core::backend::{Done, Run};
        use persimmon_core::error::{Error, ErrorCode};
        use persimmon_core::job::{JobOptions, PlannedTask};
        use persimmon_core::pool::Limits;
        use std::sync::Condvar;
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// A stand-in engine. A task whose input's name starts with `hold`
        /// waits until the gate opens (or its row is cancelled, or 10 s
        /// pass); then each task writes its input's bytes. It counts how many
        /// tasks run at once, and how many video tasks do.
        #[derive(Default)]
        struct Gate {
            open: Mutex<bool>,
            opened: Condvar,
            now: AtomicUsize,
            max: AtomicUsize,
            media_now: AtomicUsize,
            media_max: AtomicUsize,
            ran: AtomicUsize,
        }

        impl Gate {
            fn open(&self) {
                *self.open.lock().unwrap() = true;
                self.opened.notify_all();
            }
        }

        impl Run for Gate {
            fn run(
                &self,
                task: &PlannedTask,
                _: &JobOptions,
                temp: &Path,
                cancel: &CancelToken,
                _: &mut dyn FnMut(f32),
            ) -> Result<Done, Error> {
                let input = &task.inputs[0];
                let video = task.backend == persimmon_core::registry::Backend::Ffmpeg;
                self.ran.fetch_add(1, Ordering::SeqCst);
                let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
                self.max.fetch_max(n, Ordering::SeqCst);
                if video {
                    let m = self.media_now.fetch_add(1, Ordering::SeqCst) + 1;
                    self.media_max.fetch_max(m, Ordering::SeqCst);
                }
                let hold = input
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("hold"));
                let end = Instant::now() + Duration::from_secs(10);
                let mut open = self.open.lock().unwrap();
                let mut result = Ok(Done::Written);
                while hold && !*open {
                    if cancel.is_cancelled() {
                        result = Err(Error::new(ErrorCode::Cancelled, "cancelled"));
                        break;
                    }
                    assert!(Instant::now() < end, "the gate never opened");
                    open = self
                        .opened
                        .wait_timeout(open, Duration::from_millis(20))
                        .unwrap()
                        .0;
                }
                drop(open);
                if result.is_ok() {
                    std::fs::write(temp, std::fs::read(input).unwrap()).unwrap();
                }
                if video {
                    self.media_now.fetch_sub(1, Ordering::SeqCst);
                }
                self.now.fetch_sub(1, Ordering::SeqCst);
                result
            }
        }

        fn gated(
            limits: Limits,
            policy: persimmon_core::output::OutputPolicy,
        ) -> (Arc<Mutex<Queue>>, Runner, Arc<Gate>) {
            let queue = Arc::new(Mutex::new(Queue::new(Registry::builtin(), policy)));
            let gate = Arc::new(Gate::default());
            let engine: Arc<dyn Run + Send + Sync> = gate.clone();
            let runner = Runner::with_engine(Arc::clone(&queue), engine, limits, |_| {});
            (queue, runner, gate)
        }

        fn add(q: &Arc<Mutex<Queue>>, paths: &[PathBuf]) -> Vec<RowId> {
            q.lock()
                .unwrap()
                .add(paths, QueueMode::Convert)
                .iter()
                .map(|v| v.id)
                .collect()
        }

        const TWO: Limits = Limits { total: 2, media: 1 };

        #[test]
        fn two_image_rows_run_at_once() {
            let d = Dir::new("several-two");
            let (q, r, gate) = gated(TWO, Default::default());
            let ids = add(
                &q,
                &[d.put("hold-a.png", &PNG_1X1), d.put("hold-b.png", &PNG_1X1)],
            );
            r.start();
            wait("both rows running together", 5, || {
                gate.now.load(Ordering::SeqCst) == 2
            });
            gate.open();
            wait("both rows", 5, || {
                ids.iter().all(|&i| finished(&state(&q, i)))
            });
            assert!(
                ids.iter()
                    .all(|&i| matches!(state(&q, i), RowState::Done { .. }))
            );
        }

        #[test]
        fn rows_into_one_folder_never_share_a_name() {
            let d = Dir::new("several-names");
            for sub in ["a", "b", "out"] {
                std::fs::create_dir_all(d.0.join(sub)).unwrap();
            }
            let a = d.0.join("a/hold.png");
            let b = d.0.join("b/hold.png");
            std::fs::write(&a, PNG_1X1).unwrap();
            std::fs::write(&b, PNG_1X1).unwrap();
            let policy = persimmon_core::output::OutputPolicy {
                dir: Some(d.0.join("out")),
                ..Default::default()
            };
            let (q, r, gate) = gated(TWO, policy);
            let ids = add(&q, &[a, b]);
            r.start();
            wait("both rows running together", 5, || {
                gate.now.load(Ordering::SeqCst) == 2
            });
            gate.open();
            wait("both rows", 5, || {
                ids.iter().all(|&i| finished(&state(&q, i)))
            });
            let outs: Vec<PathBuf> = ids
                .iter()
                .map(|&i| match state(&q, i) {
                    RowState::Done { output, .. } => output,
                    other => panic!("{other:?}"),
                })
                .collect();
            assert_ne!(outs[0], outs[1]);
            assert!(
                outs.iter()
                    .all(|o| o.is_file() && o.parent() == Some(d.0.join("out").as_path()))
            );
        }

        #[test]
        fn cancel_one_of_two_running_rows_leaves_the_other() {
            let d = Dir::new("several-cancel");
            let (q, r, gate) = gated(TWO, Default::default());
            let ids = add(
                &q,
                &[d.put("hold-a.png", &PNG_1X1), d.put("hold-b.png", &PNG_1X1)],
            );
            r.start();
            wait("both rows running together", 5, || {
                gate.now.load(Ordering::SeqCst) == 2
            });
            assert!(r.cancel(ids[0]));
            wait("the cancelled row", 5, || finished(&state(&q, ids[0])));
            gate.open();
            wait("the other row", 5, || finished(&state(&q, ids[1])));
            assert_eq!(state(&q, ids[0]), RowState::Cancelled);
            assert!(matches!(state(&q, ids[1]), RowState::Done { .. }));
            assert!(d.no_partials());
        }

        #[test]
        fn shutdown_cancels_running_and_never_starts_waiting_rows() {
            let d = Dir::new("several-shutdown");
            let (q, r, gate) = gated(Limits::serial(), Default::default());
            let ids = add(
                &q,
                &[
                    d.put("hold-a.png", &PNG_1X1),
                    d.put("hold-b.png", &PNG_1X1),
                    d.put("hold-c.png", &PNG_1X1),
                ],
            );
            r.start();
            wait("the first row", 5, || gate.now.load(Ordering::SeqCst) == 1);
            r.shutdown();
            assert_eq!(state(&q, ids[0]), RowState::Cancelled);
            assert_eq!(gate.ran.load(Ordering::SeqCst), 1);
            assert_eq!(state(&q, ids[1]), RowState::Queued);
            assert_eq!(state(&q, ids[2]), RowState::Queued);
            assert_eq!(r.running(), 0);
            assert!(d.no_partials());
        }

        #[test]
        fn a_row_that_fails_to_plan_frees_its_slot() {
            let d = Dir::new("several-plan-fail");
            let (q, r, gate) = gated(Limits::serial(), Default::default());
            let a = d.put("a.png", &PNG_1X1);
            let ids = add(&q, &[a.clone(), d.put("b.png", &PNG_1X1)]);
            std::fs::remove_file(&a).unwrap();
            gate.open();
            r.start();
            wait("both rows", 5, || {
                ids.iter().all(|&i| finished(&state(&q, i)))
            });
            assert!(matches!(state(&q, ids[0]), RowState::Failed { .. }));
            assert!(matches!(state(&q, ids[1]), RowState::Done { .. }));
            assert_eq!(r.running(), 0);
        }

        /// Panics on inputs named `boom*`; writes the others.
        struct Boom;

        impl Run for Boom {
            fn run(
                &self,
                task: &PlannedTask,
                _: &JobOptions,
                temp: &Path,
                _: &CancelToken,
                _: &mut dyn FnMut(f32),
            ) -> Result<Done, Error> {
                let input = &task.inputs[0];
                if input
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("boom"))
                {
                    panic!("a backend bug");
                }
                std::fs::write(temp, std::fs::read(input).unwrap()).unwrap();
                Ok(Done::Written)
            }
        }

        #[test]
        fn a_row_whose_engine_panics_fails_and_the_queue_goes_on() {
            let d = Dir::new("several-boom");
            let queue = Arc::new(Mutex::new(Queue::new(
                Registry::builtin(),
                Default::default(),
            )));
            let r = Runner::with_engine(Arc::clone(&queue), Arc::new(Boom), TWO, |_| {});
            let batches: Arc<Mutex<Vec<Batch>>> = Arc::default();
            let b2 = Arc::clone(&batches);
            r.set_on_batch(move |b| b2.lock().unwrap().push(b));
            let ids = add(
                &queue,
                &[d.put("boom.png", &PNG_1X1), d.put("fine.png", &PNG_1X1)],
            );
            r.start();
            wait("both rows", 5, || {
                ids.iter().all(|&i| finished(&state(&queue, i)))
            });
            assert!(matches!(state(&queue, ids[0]), RowState::Failed { .. }));
            assert!(matches!(state(&queue, ids[1]), RowState::Done { .. }));
            wait("the batch", 5, || !batches.lock().unwrap().is_empty());
            assert_eq!(batches.lock().unwrap()[0].items.len(), 2);
            assert_eq!(r.running(), 0);
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                r.shutdown();
                let _ = tx.send(());
            });
            rx.recv_timeout(Duration::from_secs(5))
                .expect("shutdown hung after a panic");
        }

        #[test]
        fn queued_rows_go_before_background_work() {
            use persimmon_core::pool::{Lane, Priority};
            let d = Dir::new("several-first");
            let (q, r, gate) = gated(Limits { total: 1, media: 1 }, Default::default());
            let ids = add(
                &q,
                &[d.put("hold-a.png", &PNG_1X1), d.put("b.png", &PNG_1X1)],
            );
            r.start();
            wait("the first row running", 5, || {
                gate.now.load(Ordering::SeqCst) == 1
            });
            let background = {
                let (pool, q) = (r.pool().clone(), Arc::clone(&q));
                let b = ids[1];
                std::thread::spawn(move || {
                    let permit = pool.acquire(Lane::Cpu, Priority::Background, &CancelToken::new());
                    let seen = state(&q, b);
                    drop(permit);
                    seen
                })
            };
            std::thread::sleep(Duration::from_millis(100));
            gate.open();
            wait("both rows", 5, || {
                ids.iter().all(|&i| finished(&state(&q, i)))
            });
            let seen = background.join().unwrap();
            assert!(
                !matches!(seen, RowState::Queued),
                "background work took the slot ahead of a queued row"
            );
        }

        #[test]
        fn a_retried_row_joins_the_running_batch() {
            let d = Dir::new("several-retry");
            let (q, r, gate) = gated(TWO, Default::default());
            let batches: Arc<Mutex<Vec<Batch>>> = Arc::default();
            let b2 = Arc::clone(&batches);
            r.set_on_batch(move |b| b2.lock().unwrap().push(b));
            let a = d.put("a.png", &PNG_1X1);
            let ids = add(&q, &[a.clone(), d.put("hold-b.png", &PNG_1X1)]);
            std::fs::remove_file(&a).unwrap();
            r.start();
            wait("the first row to fail", 5, || {
                matches!(state(&q, ids[0]), RowState::Failed { .. })
            });
            std::fs::write(&a, PNG_1X1).unwrap();
            assert!(r.retry(ids[0]).is_some());
            wait("the retried row", 5, || {
                matches!(state(&q, ids[0]), RowState::Done { .. })
            });
            assert!(
                batches.lock().unwrap().is_empty(),
                "recorded while a row still ran"
            );
            gate.open();
            wait("the batch", 5, || !batches.lock().unwrap().is_empty());
            std::thread::sleep(Duration::from_millis(200));
            let batches = batches.lock().unwrap();
            assert_eq!(batches.len(), 1);
            let inputs: Vec<&Path> = batches[0]
                .items
                .iter()
                .map(|i| i.inputs[0].as_path())
                .collect();
            assert_eq!(inputs, [a.as_path(), d.0.join("hold-b.png").as_path()]);
            assert!(
                batches[0]
                    .items
                    .iter()
                    .all(|i| i.outcome == crate::history::ItemOutcome::Done)
            );
        }

        #[test]
        fn video_rows_run_one_at_a_time() {
            let Some(_ff) = ffmpeg() else { return };
            let d = Dir::new("several-video");
            let src = media_sample("vid-long").unwrap();
            let ext = src.extension().unwrap().to_string_lossy().into_owned();
            let v1 = d.0.join(format!("hold-1.{ext}"));
            let v2 = d.0.join(format!("hold-2.{ext}"));
            std::fs::copy(&src, &v1).unwrap();
            std::fs::copy(&src, &v2).unwrap();
            let (q, r, gate) = gated(Limits { total: 4, media: 1 }, Default::default());
            let ids = add(&q, &[v1, v2, d.put("hold-p.png", &PNG_1X1)]);
            r.start();
            wait("a video and the image together", 5, || {
                gate.now.load(Ordering::SeqCst) == 2
            });
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(
                gate.now.load(Ordering::SeqCst),
                2,
                "the second video started"
            );
            gate.open();
            wait("every row", 10, || {
                ids.iter().all(|&i| finished(&state(&q, i)))
            });
            assert_eq!(gate.media_max.load(Ordering::SeqCst), 1);
            assert!(
                ids.iter()
                    .all(|&i| matches!(state(&q, i), RowState::Done { .. }))
            );
        }
    }
}
