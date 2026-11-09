// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Running a plan: each task writes to a temporary file through
//! [`PendingOutput`] and is committed only when complete, so a failed or
//! cancelled task never leaves a partial file under the final name.
//! Cancellation is co-operative: it is checked before each task, and a
//! backend that can stop mid-file, such as FFmpeg, does; others finish
//! the task they're on.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::backend::{Done, Run};
use crate::error::{Error, ErrorCode};
use crate::job::{Job, Plan, PlannedTask};
use crate::output::PendingOutput;

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
pub enum Outcome {
    Written(PathBuf),
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
}
