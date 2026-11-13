// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The runner: one worker thread takes queued rows one at a time (D7) and
//! runs each as its own one-task job with its own cancel token, so one row
//! can be cancelled while the rest wait.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use persimmon_core::backend::Backends;
use persimmon_core::error::ErrorCode;
use persimmon_core::exec::{CancelToken, Event, Outcome, execute};
use persimmon_core::job::{Job, PlanMode};
use persimmon_core::registry::Registry;

use crate::queue::{Queue, RowId, RowState, RowView};
use crate::wording::shown;

/// Progress reaches the window at most this often per row.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

type Sink = Box<dyn Fn(RowView) + Send + Sync>;

struct State {
    running: Option<(RowId, CancelToken)>,
    go: bool,
    stop: bool,
}

struct Inner {
    queue: Arc<Mutex<Queue>>,
    backends: Backends,
    reg: Registry,
    sink: Sink,
    state: Mutex<State>,
    wake: Condvar,
}

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
        let inner = Arc::new(Inner {
            queue,
            backends,
            reg: Registry::builtin(),
            sink: Box::new(sink),
            state: Mutex::new(State {
                running: None,
                go: false,
                stop: false,
            }),
            wake: Condvar::new(),
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

    /// Run every queued row, one at a time.
    pub fn start(&self) {
        lock(&self.inner.state).go = true;
        self.inner.wake.notify_all();
    }

    /// Cancel row `id`: the running one stops mid-file where the backend
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
                true
            }
            Some(RowState::Running { .. }) => {
                let state = lock(&self.inner.state);
                match &state.running {
                    Some((running, token)) if *running == id => {
                        token.cancel();
                        true
                    }
                    _ => false,
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

    pub fn is_busy(&self) -> bool {
        lock(&self.inner.state).running.is_some()
    }

    /// Cancel the running row, stop the worker and wait for it.
    pub fn shutdown(&self) {
        {
            let mut state = lock(&self.inner.state);
            state.stop = true;
            if let Some((_, token)) = &state.running {
                token.cancel();
            }
        }
        self.inner.wake.notify_all();
        if let Some(worker) = lock(&self.worker).take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Inner {
    fn work(&self) {
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
            loop {
                let token = CancelToken::new();
                let next = {
                    let mut queue = lock(&self.queue);
                    let mut state = lock(&self.state);
                    if state.stop {
                        return;
                    }
                    let next = queue.next_queued();
                    if let Some((id, _)) = &next {
                        state.running = Some((*id, token.clone()));
                    }
                    next.map(|(id, job)| (id, job, queue.get(id)))
                };
                let Some((id, job, view)) = next else { break };
                if let Some(v) = view {
                    (self.sink)(v);
                }
                let end = self.run(id, &job, &token);
                self.set(id, end);
                lock(&self.state).running = None;
            }
        }
    }

    /// Plan the row for real, then run it; returns how it ended.
    fn run(&self, id: RowId, job: &Job, token: &CancelToken) -> RowState {
        let plan = job.plan_with(&self.reg, PlanMode::Commit);
        if let Some(e) = plan.errors.first() {
            return RowState::Failed { error: shown(e) };
        }
        let mut last = Instant::now() - PROGRESS_EVERY;
        let results = execute(job, &plan, &self.backends, token, &mut |event| {
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
        match results.into_iter().next() {
            Some(Ok(Outcome::Written(output))) => {
                let size = std::fs::metadata(&output)
                    .ok()
                    .filter(|m| m.is_file())
                    .map(|m| m.len());
                RowState::Done {
                    output,
                    output_size: size,
                }
            }
            Some(Ok(Outcome::NotSmaller)) => RowState::NotSmaller,
            Some(Err(e)) if e.code == ErrorCode::Cancelled => RowState::Cancelled,
            Some(Err(e)) => RowState::Failed { error: shown(&e) },
            None => RowState::Cancelled,
        }
    }

    fn set(&self, id: RowId, state: RowState) {
        let v = lock(&self.queue).update(id, state);
        if let Some(v) = v {
            (self.sink)(v);
        }
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
            } = state(&q, i)
            else {
                panic!("{:?}", state(&q, i))
            };
            assert!(output.is_file());
            assert!(size.unwrap() > 0);
        }
        let seen = seen.lock().unwrap();
        let first_done = seen
            .iter()
            .position(|v| v.id == ids[0] && matches!(v.state, RowState::Done { .. }))
            .unwrap();
        let second_runs = seen
            .iter()
            .position(|v| v.id == ids[1] && matches!(v.state, RowState::Running { .. }))
            .unwrap();
        assert!(
            first_done < second_runs,
            "the second row started before the first finished"
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
        assert!(r.is_busy());
        r.shutdown();
        assert_eq!(state(&q, v), RowState::Cancelled);
        assert!(!r.is_busy());
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
}
