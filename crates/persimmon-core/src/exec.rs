// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Running a plan: each task writes to a temporary file through
//! [`PendingOutput`] and is committed only when complete, so a failed or
//! cancelled task never leaves a partial file under the final name.
//! Cancellation is co-operative: it is checked before each task, and a
//! backend that can stop mid-file, such as FFmpeg, does; others finish
//! the task they're on.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crate::backend::{Done, Run};
use crate::error::{Error, ErrorCode, Note};
use crate::job::{Job, Plan, PlannedTask};
use crate::output::PendingOutput;
use crate::pool::{Lane, Pool};

/// A shared cancel flag. Clones share it.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// What a finished task produced.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub enum Outcome {
    #[cfg_attr(feature = "serde", serde(serialize_with = "crate::serde_util::path"))]
    Written(PathBuf),
    /// Written, with something to say about how (a fallback it took).
    WrittenNoted(
        #[cfg_attr(feature = "serde", serde(serialize_with = "crate::serde_util::path"))] PathBuf,
        Note,
    ),
    /// Compressing wouldn't make the file smaller; nothing was written.
    NotSmaller,
}

/// Progress: each task starts, may report how far it is, and finishes.
/// `index` counts from 0; `total` is the number of tasks.
#[derive(Debug)]
pub enum Event<'a> {
    Started {
        index: usize,
        total: usize,
        task: &'a PlannedTask,
    },
    /// How much of the task is done, from 0 to 1, never decreasing.
    Progress {
        index: usize,
        total: usize,
        task: &'a PlannedTask,
        fraction: f32,
    },
    Finished {
        index: usize,
        total: usize,
        task: &'a PlannedTask,
        result: &'a Result<Outcome, Error>,
    },
}

/// Run every task in `plan`, in order, and return one result per task.
pub fn execute(
    job: &Job,
    plan: &Plan,
    runner: &dyn Run,
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(Event),
) -> Vec<Result<Outcome, Error>> {
    let total = plan.tasks.len();
    let mut results = Vec::with_capacity(total);
    for (index, task) in plan.tasks.iter().enumerate() {
        let result = if cancel.is_cancelled() {
            let e = Error::new(ErrorCode::Cancelled, "cancelled");
            Err(match task.inputs.as_slice() {
                [one] => e.with_file(one),
                _ => e,
            })
        } else {
            on_event(Event::Started { index, total, task });
            let pending = PendingOutput::new(task.output.clone());
            let mut progress = |fraction| {
                on_event(Event::Progress {
                    index,
                    total,
                    task,
                    fraction,
                })
            };
            match runner.run(
                task,
                &job.options,
                pending.temp_path(),
                cancel,
                &mut progress,
            ) {
                Ok(Done::Written) => pending.commit(job.output.overwrite).map(Outcome::Written),
                Ok(Done::Noted(note)) => pending
                    .commit(job.output.overwrite)
                    .map(|p| Outcome::WrittenNoted(p, note)),
                Ok(Done::NotSmaller) => Ok(Outcome::NotSmaller),
                // Dropping `pending` removes the temp file.
                Err(e) => Err(e),
            }
        };
        on_event(Event::Finished {
            index,
            total,
            task,
            result: &result,
        });
        results.push(result);
    }
    results
}

/// What a worker tells the caller of [`execute_in`].
enum Msg {
    Started(usize),
    Progress(usize, f32),
    Finished(usize, Result<Outcome, Error>),
}

/// The result of a task that never started because the batch was cancelled.
fn cancelled(task: &PlannedTask) -> Error {
    let e = Error::new(ErrorCode::Cancelled, "cancelled");
    match task.inputs.as_slice() {
        [one] => e.with_file(one),
        _ => e,
    }
}

/// The result of a task whose backend panicked.
pub(crate) fn crashed(task: &PlannedTask) -> Error {
    let e = Error::new(ErrorCode::Internal, "the converter stopped unexpectedly");
    match task.inputs.as_slice() {
        [one] => e.with_file(one),
        _ => e,
    }
}

/// Run one task and commit its output.
fn run_task(
    job: &Job,
    task: &PlannedTask,
    runner: &dyn Run,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(f32),
) -> Result<Outcome, Error> {
    let pending = PendingOutput::new(task.output.clone());
    match runner.run(task, &job.options, pending.temp_path(), cancel, progress) {
        Ok(Done::Written) => pending.commit(job.output.overwrite).map(Outcome::Written),
        Ok(Done::Noted(note)) => pending
            .commit(job.output.overwrite)
            .map(|p| Outcome::WrittenNoted(p, note)),
        Ok(Done::NotSmaller) => Ok(Outcome::NotSmaller),
        // Dropping `pending` removes the temp file.
        Err(e) => Err(e),
    }
}

/// Run `plan`'s tasks several at once, as `pool` allows, and return one
/// result per task, in task order. `Started` and `Progress` events arrive as
/// they happen; `Finished` events arrive in task order, each once every
/// earlier task has finished. `on_event` is only ever called on this
/// thread. Once `cancel` fires, tasks not yet started end as `CANCELLED`.
pub fn execute_in(
    job: &Job,
    plan: &Plan,
    runner: &(dyn Run + Sync),
    cancel: &CancelToken,
    pool: &Pool,
    on_event: &mut dyn FnMut(Event),
) -> Vec<Result<Outcome, Error>> {
    let total = plan.tasks.len();
    let mut results: Vec<Option<Result<Outcome, Error>>> = (0..total).map(|_| None).collect();
    let mut waiting: VecDeque<usize> = (0..total).collect();
    let mut reported = 0;
    let (tx, rx) = mpsc::channel::<Msg>();
    std::thread::scope(|scope| {
        loop {
            if cancel.is_cancelled() {
                for i in waiting.drain(..) {
                    results[i] = Some(Err(cancelled(&plan.tasks[i])));
                }
            }
            // Start the earliest waiting tasks whose lane has room.
            let mut k = 0;
            while k < waiting.len() && pool.running() < pool.limits().total {
                let index = waiting[k];
                let task = &plan.tasks[index];
                let Some(permit) = pool.try_acquire(Lane::of(task.backend)) else {
                    k += 1;
                    continue;
                };
                waiting.remove(k);
                let tx = tx.clone();
                scope.spawn(move || {
                    let _ = tx.send(Msg::Started(index));
                    // A backend that panics fails its own task, not the batch;
                    // unwinding drops the temp file.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run_task(job, task, runner, cancel, &mut |f| {
                            let _ = tx.send(Msg::Progress(index, f));
                        })
                    }))
                    .unwrap_or_else(|_| Err(crashed(task)));
                    // Free the slot before saying so, so the next task can start.
                    drop(permit);
                    let _ = tx.send(Msg::Finished(index, result));
                });
            }
            // Report finished tasks in order.
            while let Some(Some(result)) = results.get(reported) {
                on_event(Event::Finished {
                    index: reported,
                    total,
                    task: &plan.tasks[reported],
                    result,
                });
                reported += 1;
            }
            if reported == total {
                break;
            }
            // A pool shared with other jobs frees slots without telling us.
            let Ok(msg) = rx.recv_timeout(Duration::from_millis(20)) else {
                continue;
            };
            match msg {
                Msg::Started(index) => on_event(Event::Started {
                    index,
                    total,
                    task: &plan.tasks[index],
                }),
                Msg::Progress(index, fraction) => on_event(Event::Progress {
                    index,
                    total,
                    task: &plan.tasks[index],
                    fraction,
                }),
                Msg::Finished(index, result) => results[index] = Some(result),
            }
        }
    });
    results
        .into_iter()
        .map(|r| r.expect("every task reported"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Done, Run};
    use crate::error::ErrorCode;
    use crate::format::FormatId;
    use crate::job::{Job, JobOptions, PlannedTask};
    use crate::output::OutputPolicy;
    use crate::registry::{Operation, Registry};
    use persimmon_corpus::PNG_1X1;
    use std::cell::Cell;
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-exec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A job converting `n` PNGs in `dir` to WebP.
    fn job(dir: &Path, n: usize) -> Job {
        let inputs = (0..n)
            .map(|i| {
                let p = dir.join(format!("in{i}.png"));
                std::fs::write(&p, PNG_1X1).unwrap();
                p
            })
            .collect();
        Job {
            op: Operation::Convert,
            inputs,
            target: Some(FormatId::Webp),
            output: OutputPolicy::default(),
            options: JobOptions::default(),
        }
    }

    /// Writes `ok`; fails on `fail_at`; cancels the token while running `cancel_at`.
    struct Fake {
        fail_at: Option<usize>,
        cancel_at: Option<usize>,
        not_smaller: bool,
        /// Reported once as progress while running.
        progress: Option<f32>,
        calls: Cell<usize>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake {
                fail_at: None,
                cancel_at: None,
                not_smaller: false,
                progress: None,
                calls: Cell::new(0),
            }
        }
    }

    impl Run for Fake {
        fn run(
            &self,
            task: &PlannedTask,
            _: &JobOptions,
            temp: &Path,
            cancel: &CancelToken,
            progress: &mut dyn FnMut(f32),
        ) -> Result<Done, crate::error::Error> {
            if let Some(f) = self.progress {
                progress(f);
            }
            let i = self.calls.get();
            self.calls.set(i + 1);
            if self.cancel_at == Some(i) {
                cancel.cancel();
            }
            if self.fail_at == Some(i) {
                std::fs::write(temp, b"half").unwrap();
                return Err(crate::error::Error::new(ErrorCode::CorruptInput, "bad")
                    .with_file(&task.inputs[0]));
            }
            if self.not_smaller {
                return Ok(Done::NotSmaller);
            }
            std::fs::write(temp, b"ok").unwrap();
            Ok(Done::Written)
        }
    }

    fn no_parts(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .unwrap()
            .all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part"))
    }

    fn run(
        j: &Job,
        fake: &Fake,
        cancel: &CancelToken,
    ) -> (Vec<Result<Outcome, crate::error::Error>>, Vec<String>) {
        let plan = j.plan(&Registry::builtin());
        let mut events = Vec::new();
        let results = execute(j, &plan, fake, cancel, &mut |e| {
            events.push(match e {
                Event::Started { index, .. } => format!("Started {index}"),
                Event::Progress {
                    index, fraction, ..
                } => format!("Progress {index} {fraction}"),
                Event::Finished { index, .. } => format!("Finished {index}"),
            })
        });
        (results, events)
    }

    #[test]
    fn executes_and_commits_each_task() {
        let dir = scratch("commit");
        let (results, _) = run(&job(&dir, 2), &Fake::new(), &CancelToken::new());
        assert_eq!(results.len(), 2);
        for (i, r) in results.iter().enumerate() {
            let out = dir.join(format!("in{i}.webp"));
            assert_eq!(r.as_ref().unwrap(), &Outcome::Written(out.clone()));
            assert_eq!(std::fs::read(&out).unwrap(), b"ok");
        }
        assert!(no_parts(&dir));
    }

    #[test]
    fn failure_leaves_no_part_and_continues() {
        let dir = scratch("fail");
        let fake = Fake {
            fail_at: Some(0),
            ..Fake::new()
        };
        let (results, _) = run(&job(&dir, 2), &fake, &CancelToken::new());
        assert_eq!(
            results[0].as_ref().unwrap_err().code,
            ErrorCode::CorruptInput
        );
        assert!(!dir.join("in0.webp").exists());
        assert!(matches!(results[1], Ok(Outcome::Written(_))));
        assert!(no_parts(&dir));
    }

    #[test]
    fn cancel_mid_batch() {
        let dir = scratch("cancel");
        let fake = Fake {
            cancel_at: Some(1),
            ..Fake::new()
        };
        let (results, _) = run(&job(&dir, 3), &fake, &CancelToken::new());
        assert!(matches!(results[0], Ok(Outcome::Written(_))));
        assert!(matches!(results[1], Ok(Outcome::Written(_))));
        let e = results[2].as_ref().unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        assert_eq!(e.reason, "cancelled");
        assert_eq!(e.file.as_deref(), Some(dir.join("in2.png").as_path()));
        assert!(!dir.join("in2.webp").exists());
        assert_eq!(fake.calls.get(), 2, "the cancelled task never runs");
        assert!(no_parts(&dir));
    }

    #[test]
    fn not_smaller_writes_nothing() {
        let dir = scratch("smaller");
        let fake = Fake {
            not_smaller: true,
            ..Fake::new()
        };
        let (results, _) = run(&job(&dir, 1), &fake, &CancelToken::new());
        assert_eq!(results[0].as_ref().unwrap(), &Outcome::NotSmaller);
        assert!(!dir.join("in0.webp").exists());
        assert!(no_parts(&dir));
    }

    #[test]
    fn events_bracket_each_task() {
        let dir = scratch("events");
        let (_, events) = run(&job(&dir, 2), &Fake::new(), &CancelToken::new());
        assert_eq!(
            events,
            ["Started 0", "Finished 0", "Started 1", "Finished 1"]
        );
    }

    #[test]
    fn progress_events_carry_index() {
        let dir = scratch("progress");
        let j = job(&dir, 1);
        let fake = Fake {
            progress: Some(0.5),
            ..Fake::new()
        };
        let (_, events) = run(&j, &fake, &CancelToken::new());
        assert_eq!(events, ["Started 0", "Progress 0 0.5", "Finished 0"]);
    }

    mod parallel {
        use super::*;
        use crate::pool::{Limits, Pool};
        use crate::registry::Backend;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        /// Writes each input's bytes after a delay, counting how many run at once.
        #[derive(Default)]
        struct Slow {
            /// How long the task for input 0 takes; the others take `each`.
            first: Duration,
            each: Duration,
            /// Hold each task until this many run at once (or 2 s pass).
            gather: usize,
            /// Tasks that have started, never decremented, for `gather`.
            arrived: AtomicUsize,
            now: AtomicUsize,
            max: AtomicUsize,
        }

        impl Run for Slow {
            fn run(
                &self,
                task: &PlannedTask,
                _: &JobOptions,
                temp: &Path,
                _: &CancelToken,
                _: &mut dyn FnMut(f32),
            ) -> Result<Done, crate::error::Error> {
                let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
                self.max.fetch_max(n, Ordering::SeqCst);
                self.arrived.fetch_add(1, Ordering::SeqCst);
                let until = Instant::now() + Duration::from_secs(2);
                while self.arrived.load(Ordering::SeqCst) < self.gather && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(5));
                }
                let first = task.inputs[0].file_name().is_some_and(|n| n == "in0.png");
                std::thread::sleep(if first { self.first } else { self.each });
                std::fs::write(temp, std::fs::read(&task.inputs[0]).unwrap()).unwrap();
                self.now.fetch_sub(1, Ordering::SeqCst);
                Ok(Done::Written)
            }
        }

        fn events_of(e: &Event) -> String {
            match e {
                Event::Started { index, .. } => format!("Started {index}"),
                Event::Progress { index, .. } => format!("Progress {index}"),
                Event::Finished { index, .. } => format!("Finished {index}"),
            }
        }

        #[test]
        fn a_thousand_tasks_never_share_an_output_name() {
            let dir = scratch("thousand");
            let out = dir.join("out");
            let mut inputs = Vec::new();
            for f in 0..10 {
                let sub = dir.join(format!("f{f}"));
                std::fs::create_dir_all(&sub).unwrap();
                for i in 0..100 {
                    let p = sub.join(format!("{i:02}.png"));
                    std::fs::write(&p, PNG_1X1).unwrap();
                    inputs.push(p);
                }
            }
            let j = Job {
                op: Operation::Convert,
                inputs,
                target: Some(FormatId::Webp),
                output: OutputPolicy {
                    dir: Some(out.clone()),
                    ..OutputPolicy::default()
                },
                options: JobOptions::default(),
            };
            let plan = j.plan(&Registry::builtin());
            let pool = Pool::new(Limits { total: 8, media: 1 });
            let results = execute_in(
                &j,
                &plan,
                &Slow::default(),
                &CancelToken::new(),
                &pool,
                &mut |_| {},
            );
            let written: std::collections::HashSet<PathBuf> = results
                .iter()
                .map(|r| match r {
                    Ok(Outcome::Written(p)) => p.clone(),
                    other => panic!("{other:?}"),
                })
                .collect();
            assert_eq!(written.len(), 1000);
            assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1000);
            assert!(no_parts(&out));
        }

        /// Panics on input 1; writes the others.
        struct Panics;

        impl Run for Panics {
            fn run(
                &self,
                task: &PlannedTask,
                _: &JobOptions,
                temp: &Path,
                _: &CancelToken,
                _: &mut dyn FnMut(f32),
            ) -> Result<Done, crate::error::Error> {
                if task.inputs[0].file_name().is_some_and(|n| n == "in1.png") {
                    panic!("a backend bug");
                }
                std::fs::write(temp, b"x").unwrap();
                Ok(Done::Written)
            }
        }

        #[test]
        fn a_task_that_panics_fails_alone() {
            let dir = scratch("panics");
            let j = job(&dir, 3);
            let plan = j.plan(&Registry::builtin());
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let pool = Pool::new(Limits { total: 4, media: 1 });
                let results =
                    execute_in(&j, &plan, &Panics, &CancelToken::new(), &pool, &mut |_| {});
                let _ = tx.send(results);
            });
            let results = rx
                .recv_timeout(Duration::from_secs(10))
                .expect("execute_in hung after a panic");
            assert!(results[0].is_ok() && results[2].is_ok(), "{results:?}");
            let e = results[1].as_ref().unwrap_err();
            assert_eq!(e.code, ErrorCode::Internal);
            assert!(no_parts(&dir));
        }

        #[test]
        fn results_and_finished_events_come_in_task_order() {
            let dir = scratch("order");
            let j = job(&dir, 5);
            let plan = j.plan(&Registry::builtin());
            let slow = Slow {
                first: Duration::from_millis(200),
                ..Slow::default()
            };
            let mut finished = Vec::new();
            let results = execute_in(
                &j,
                &plan,
                &slow,
                &CancelToken::new(),
                &Pool::new(Limits { total: 4, media: 1 }),
                &mut |e| {
                    if let Event::Finished { index, .. } = e {
                        finished.push(index);
                    }
                },
            );
            assert_eq!(finished, [0, 1, 2, 3, 4]);
            for (i, r) in results.iter().enumerate() {
                assert_eq!(
                    r.as_ref().unwrap(),
                    &Outcome::Written(dir.join(format!("in{i}.webp")))
                );
            }
        }

        #[test]
        fn media_tasks_run_one_at_a_time() {
            let dir = scratch("media");
            let j = job(&dir, 4);
            let mut plan = j.plan(&Registry::builtin());
            for t in &mut plan.tasks {
                t.backend = Backend::Ffmpeg;
            }
            let slow = Slow {
                each: Duration::from_millis(30),
                ..Slow::default()
            };
            let pool = Pool::new(Limits { total: 4, media: 1 });
            execute_in(&j, &plan, &slow, &CancelToken::new(), &pool, &mut |_| {});
            assert_eq!(slow.max.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn cpu_tasks_run_together() {
            let dir = scratch("together");
            let j = job(&dir, 4);
            let plan = j.plan(&Registry::builtin());
            let slow = Slow {
                gather: 4,
                ..Slow::default()
            };
            let pool = Pool::new(Limits { total: 4, media: 1 });
            let started = Instant::now();
            execute_in(&j, &plan, &slow, &CancelToken::new(), &pool, &mut |_| {});
            assert_eq!(slow.max.load(Ordering::SeqCst), 4);
            assert!(started.elapsed() < Duration::from_secs(2));
        }

        #[test]
        fn cancelling_mid_pool_leaves_no_part_file() {
            let dir = scratch("pool-cancel");
            let j = job(&dir, 20);
            let plan = j.plan(&Registry::builtin());
            let slow = Slow {
                each: Duration::from_millis(50),
                first: Duration::from_millis(50),
                ..Slow::default()
            };
            let cancel = CancelToken::new();
            let token = cancel.clone();
            let results = execute_in(
                &j,
                &plan,
                &slow,
                &cancel,
                &Pool::new(Limits { total: 4, media: 1 }),
                &mut |e| {
                    if let Event::Started { .. } = e {
                        token.cancel();
                    }
                },
            );
            assert_eq!(results.len(), 20);
            let cancelled = results
                .iter()
                .filter(|r| matches!(r, Err(e) if e.code == ErrorCode::Cancelled))
                .count();
            assert!(cancelled >= 10, "{cancelled} cancelled");
            assert!(results.iter().all(|r| match r {
                Ok(Outcome::Written(p)) => p.exists(),
                Err(e) => e.code == ErrorCode::Cancelled,
                other => panic!("{other:?}"),
            }));
            assert!(no_parts(&dir));
        }

        #[test]
        fn a_serial_pool_matches_execute() {
            let dir = scratch("serial");
            let j = job(&dir, 3);
            let plan = j.plan(&Registry::builtin());
            let mut a = Vec::new();
            let ra = execute(&j, &plan, &Slow::default(), &CancelToken::new(), &mut |e| {
                a.push(events_of(&e))
            });
            for t in &plan.tasks {
                let _ = std::fs::remove_file(&t.output);
            }
            let mut b = Vec::new();
            let rb = execute_in(
                &j,
                &plan,
                &Slow::default(),
                &CancelToken::new(),
                &Pool::new(Limits::serial()),
                &mut |e| b.push(events_of(&e)),
            );
            assert_eq!(ra, rb);
            assert_eq!(a, b);
        }
    }
}
