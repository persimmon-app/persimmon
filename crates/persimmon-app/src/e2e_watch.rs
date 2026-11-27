// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Watch rules and the queue together, on the real engine (Phase 3 spec,
//! week 7's hardening): one pool and one set of output claims, wired as
//! `app/watching.rs` wires them, under load and at quit.

use crate::e2e::{Rig, Scratch};
use crate::history::Batch;
use crate::presets::Presets;
use crate::queue::{Queue, QueueMode, RowState};
use crate::runner::Runner;
use crate::watch::rule;
use crate::watch::runner::{Deps, Notifier, Running, WatchRunner};
use persimmon_core::backend::Backends;
use persimmon_core::format::FormatId;
use persimmon_core::output::OutputPolicy;
use persimmon_core::pool::Limits;
use persimmon_core::registry::Registry;
use persimmon_core::watch::Timing;
use persimmon_corpus::PNG_1X1;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Notifications nobody sees.
struct Quiet;

impl Notifier for Quiet {
    fn allowed(&self) -> bool {
        false
    }
    fn post(&self, _: &str, _: &str, _: bool, _: Option<PathBuf>) {}
}

/// The queue's rig and a watch runner over its engine, pool and claims.
pub(crate) struct Both {
    pub queue: Rig,
    pub watch: WatchRunner,
    pub batches: Arc<Mutex<Vec<Batch>>>,
    /// The watched folder, inside the scratch folder.
    pub folder: PathBuf,
}

fn fast() -> Timing {
    Timing {
        settle: Duration::from_millis(150),
        tick: Duration::from_millis(30),
        rescan: Duration::from_millis(300),
    }
}

/// A new scratch folder, with `backends` and `policy`.
pub(crate) fn both(name: &str, limits: Limits, backends: Backends, policy: OutputPolicy) -> Both {
    both_at(
        Scratch::new(&format!("watch-{name}")),
        limits,
        backends,
        policy,
    )
}

/// An existing scratch folder, its data folder kept: a restart.
pub(crate) fn both_at(
    dir: Scratch,
    limits: Limits,
    backends: Backends,
    policy: OutputPolicy,
) -> Both {
    let queue = Arc::new(Mutex::new(Queue::new(Registry::builtin(), policy)));
    let runner = Runner::with_engine(Arc::clone(&queue), Arc::new(backends), limits, |_| {});
    let folder = dir.0.join("in");
    std::fs::create_dir_all(&folder).unwrap();
    let batches: Arc<Mutex<Vec<Batch>>> = Arc::default();
    let watch = {
        let b = Arc::clone(&batches);
        WatchRunner::new(Deps {
            reg: Registry::builtin(),
            engine: runner.engine(),
            pool: runner.pool().clone(),
            claims: runner.claims(),
            data: dir.0.join("data"),
            timing: fast(),
            quiet: Duration::from_millis(400),
            on_batch: Box::new(move |batch| b.lock().unwrap().push(batch)),
            on_status: Box::new(|_| {}),
            notifier: Arc::new(Quiet),
        })
    };
    Both {
        queue: Rig { queue, runner, dir },
        watch,
        batches,
        folder,
    }
}

impl Both {
    /// A rule on the watched folder, converting to `to`.
    pub(crate) fn rule(&self, to: FormatId) -> rule::Rule {
        let mut r = rule::draft(&Registry::builtin(), self.folder.clone(), to);
        r.id = "r1".into();
        r
    }

    pub(crate) fn running(&self, r: &rule::Rule) -> Running {
        let presets = Presets::load(&self.queue.dir.0.join("presets"));
        let (mut spec, preset_name) = rule::spec(r, &presets, &Registry::builtin());
        spec.timing = fast();
        Running {
            rule: r.clone(),
            spec,
            preset_name,
        }
    }

    /// The names in the rule's results folder.
    pub(crate) fn results(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.folder.join("converted"))
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| !n.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    pub(crate) fn batches(&self) -> Vec<Batch> {
        self.batches.lock().unwrap().clone()
    }

    /// Every row is done.
    pub(crate) fn rows_done(&self) -> bool {
        self.queue
            .queue
            .lock()
            .unwrap()
            .rows()
            .iter()
            .all(|r| matches!(r.state, RowState::Done { .. }))
    }
}

pub(crate) fn wait(what: &str, secs: u64, done: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !done() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Long enough for several rescans to have handed out anything left.
pub(crate) fn settle_down() {
    std::thread::sleep(Duration::from_millis(1_200));
}

/// The inputs the batches name, each once per time it's named.
fn batch_inputs(batches: &[Batch]) -> Vec<String> {
    let mut v: Vec<String> = batches
        .iter()
        .flat_map(|b| b.items.iter())
        .flat_map(|i| i.inputs.iter())
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn webp(path: &Path) -> bool {
    let head = std::fs::read(path).unwrap_or_default();
    head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP"
}

const POOL: Limits = Limits { total: 4, media: 1 };

#[test]
fn a_burst_of_200_beside_running_rows_converts_each_once() {
    let b = both("burst", POOL, Backends::new(), OutputPolicy::default());
    let rows_dir = b.queue.dir.0.join("rows");
    std::fs::create_dir_all(&rows_dir).unwrap();
    let inputs: Vec<PathBuf> = (0..20)
        .map(|i| {
            let p = rows_dir.join(format!("q{i:02}.png"));
            std::fs::write(&p, PNG_1X1).unwrap();
            p
        })
        .collect();
    {
        let mut q = b.queue.queue.lock().unwrap();
        let ids: Vec<_> = q
            .add(&inputs, QueueMode::Convert)
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(q.set_target(&ids, FormatId::Webp).skipped, 0);
    }
    b.queue.runner.start();
    let rule = b.rule(FormatId::Webp);
    b.watch.apply(vec![b.running(&rule)]);
    for i in 0..200 {
        std::fs::write(b.folder.join(format!("b{i:03}.png")), PNG_1X1).unwrap();
    }
    wait("200 outputs", 60, || b.results().len() == 200);
    wait("every row done", 60, || b.rows_done());
    wait("the burst in History", 30, || {
        batch_inputs(&b.batches()).len() == 200
    });
    let expected: Vec<String> = (0..200).map(|i| format!("b{i:03}.png")).collect();
    assert_eq!(batch_inputs(&b.batches()), expected);
    settle_down();
    assert_eq!(b.results().len(), 200, "something converted twice");
    assert!(
        b.results().iter().all(|n| n.ends_with(".webp")),
        "{:?}",
        b.results()
    );
    b.watch.shutdown();
    b.queue.runner.shutdown();
}

#[test]
fn rows_and_a_rule_writing_one_folder_never_share_a_name() {
    let b = both("names", POOL, Backends::new(), OutputPolicy::default());
    let converted = b.folder.join("converted");
    std::fs::create_dir_all(&converted).unwrap();
    // The queue writes into the rule's results folder.
    {
        let mut q = b.queue.queue.lock().unwrap();
        let _ = q.set_policy(OutputPolicy {
            dir: Some(converted.clone()),
            ..OutputPolicy::default()
        });
    }
    let rows_dir = b.queue.dir.0.join("rows");
    std::fs::create_dir_all(&rows_dir).unwrap();
    let inputs: Vec<PathBuf> = (0..30)
        .map(|i| {
            let p = rows_dir.join(format!("s{i:02}.png"));
            std::fs::write(&p, PNG_1X1).unwrap();
            p
        })
        .collect();
    let ids: Vec<_> = {
        let mut q = b.queue.queue.lock().unwrap();
        let ids: Vec<_> = q
            .add(&inputs, QueueMode::Convert)
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(q.set_target(&ids, FormatId::Webp).skipped, 0);
        ids
    };
    let rule = b.rule(FormatId::Webp);
    b.watch.apply(vec![b.running(&rule)]);
    b.queue.runner.start();
    for i in 0..30 {
        std::fs::write(b.folder.join(format!("s{i:02}.png")), PNG_1X1).unwrap();
    }
    wait("60 outputs", 60, || b.results().len() == 60);
    wait("every row done", 60, || b.rows_done());
    wait("the burst in History", 30, || {
        batch_inputs(&b.batches()).len() == 30
    });
    let mut outputs: Vec<PathBuf> = ids
        .iter()
        .map(
            |id| match b.queue.queue.lock().unwrap().get(*id).unwrap().state {
                RowState::Done { output, .. } => output,
                other => panic!("row {id:?}: {other:?}"),
            },
        )
        .collect();
    outputs.extend(
        b.batches()
            .iter()
            .flat_map(|batch| batch.items.iter())
            .map(|i| i.output.clone().expect("a watch item with no output")),
    );
    assert_eq!(outputs.len(), 60);
    for o in &outputs {
        assert!(o.exists(), "{} is gone", o.display());
        assert!(webp(o), "{} isn't a WebP", o.display());
    }
    let mut unique = outputs.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 60, "two outputs share a path: {outputs:?}");
    settle_down();
    assert_eq!(b.results().len(), 60);
    b.watch.shutdown();
    b.queue.runner.shutdown();
}

/// How many FFmpeg processes this test process has started and not reaped.
fn ffmpeg_children() -> usize {
    let out = std::process::Command::new("/usr/bin/pgrep")
        .args(["-P", &std::process::id().to_string(), "ffmpeg"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().count()
}

/// The long video sample, copied to `to`.
fn long_video(to: &Path) -> PathBuf {
    let src = persimmon_corpus::media::media_sample("vid-long").unwrap();
    std::fs::copy(&src, to).unwrap();
    to.to_path_buf()
}

/// Two FFmpegs at once: the rule's video beside the queue's.
const TWO_VIDEOS: Limits = Limits { total: 4, media: 2 };

fn ffmpeg_backends(ff: &Path) -> Backends {
    Backends::new().with_ffmpeg(ff)
}

#[test]
fn quitting_with_rules_and_rows_running_leaves_nothing() {
    let Some(ff) = crate::e2e::ffmpeg() else {
        return;
    };
    let _serial = crate::FFMPEG_TESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let b = both(
        "quit",
        TWO_VIDEOS,
        ffmpeg_backends(&ff),
        OutputPolicy::default(),
    );
    let row_input = long_video(&b.queue.dir.0.join("row.mp4"));
    let id = crate::e2e::row(
        &b.queue,
        &[row_input],
        QueueMode::Convert,
        Some(FormatId::Webm),
    );
    b.queue.runner.start();
    let rule = b.rule(FormatId::Webm);
    b.watch.apply(vec![b.running(&rule)]);
    long_video(&b.folder.join("clip.mp4"));
    wait("both videos encoding", 60, || {
        ffmpeg_children() == 2
            && matches!(
                crate::e2e::state(&b.queue, id),
                RowState::Running { fraction: Some(_) }
            )
    });
    // The app's order (`quit_now`): watch rules, then the queue.
    b.watch.shutdown();
    b.queue.runner.shutdown();
    assert!(crate::e2e::no_partials(&b.queue.dir.0));
    assert!(crate::e2e::no_ffmpeg_child());
    assert!(matches!(
        crate::e2e::state(&b.queue, id),
        RowState::Cancelled
    ));
    assert!(b.results().is_empty(), "{:?}", b.results());
    // The next launch converts the cut-short file, once.
    let Both { queue, watch, .. } = b;
    drop(watch);
    let Rig { runner, dir, .. } = queue;
    drop(runner);
    let b = both_at(
        dir,
        TWO_VIDEOS,
        ffmpeg_backends(&ff),
        OutputPolicy::default(),
    );
    b.watch.apply(vec![b.running(&rule)]);
    wait("the video converted", 180, || b.results() == ["clip.webm"]);
    settle_down();
    assert_eq!(b.results(), ["clip.webm"]);
    b.watch.shutdown();
}

#[test]
fn cancelling_a_row_leaves_the_rule_converting() {
    let Some(ff) = crate::e2e::ffmpeg() else {
        return;
    };
    let _serial = crate::FFMPEG_TESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let b = both(
        "cancel",
        POOL,
        Backends::new().with_ffmpeg(&ff),
        OutputPolicy::default(),
    );
    let row_input = long_video(&b.queue.dir.0.join("row.mp4"));
    let id = crate::e2e::row(
        &b.queue,
        &[row_input],
        QueueMode::Convert,
        Some(FormatId::Webm),
    );
    b.queue.runner.start();
    let rule = b.rule(FormatId::Webp);
    b.watch.apply(vec![b.running(&rule)]);
    for i in 0..10 {
        std::fs::write(b.folder.join(format!("p{i}.png")), PNG_1X1).unwrap();
    }
    wait("the row encoding", 60, || {
        matches!(
            crate::e2e::state(&b.queue, id),
            RowState::Running { fraction: Some(_) }
        )
    });
    assert!(b.queue.runner.cancel(id));
    wait("the row cancelled", 30, || {
        matches!(crate::e2e::state(&b.queue, id), RowState::Cancelled)
    });
    wait("10 outputs", 60, || b.results().len() == 10);
    assert!(crate::e2e::no_partials(&b.queue.dir.0));
    b.watch.shutdown();
    b.queue.runner.shutdown();
    assert!(crate::e2e::no_ffmpeg_child());
}

/// A rule converting the long video, stopped by `stop` mid-file: no FFmpeg
/// and nothing partial left. Returns the rig and the rule.
fn stopped_mid_file(
    name: &str,
    ff: &Path,
    stop: impl Fn(&Both, &rule::Rule),
) -> (Both, rule::Rule) {
    let b = both(name, POOL, ffmpeg_backends(ff), OutputPolicy::default());
    let rule = b.rule(FormatId::Webm);
    b.watch.apply(vec![b.running(&rule)]);
    long_video(&b.folder.join("clip.mp4"));
    wait("the video encoding", 60, || ffmpeg_children() == 1);
    stop(&b, &rule);
    wait("FFmpeg gone", 10, crate::e2e::no_ffmpeg_child);
    assert!(crate::e2e::no_partials(&b.queue.dir.0));
    (b, rule)
}

#[test]
fn pausing_a_rule_mid_file_leaves_no_part_and_resumes() {
    let Some(ff) = crate::e2e::ffmpeg() else {
        return;
    };
    let _serial = crate::FFMPEG_TESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (b, mut rule) = stopped_mid_file("pause", &ff, |b, rule| {
        let mut paused = rule.clone();
        paused.enabled = false;
        b.watch.apply(vec![b.running(&paused)]);
    });
    assert!(b.results().is_empty(), "{:?}", b.results());
    rule.enabled = true;
    b.watch.apply(vec![b.running(&rule)]);
    wait("the video converted", 180, || b.results() == ["clip.webm"]);
    settle_down();
    assert_eq!(b.results(), ["clip.webm"]);
    b.watch.shutdown();
}

#[test]
fn removing_a_rule_mid_file_leaves_no_part() {
    let Some(ff) = crate::e2e::ffmpeg() else {
        return;
    };
    let _serial = crate::FFMPEG_TESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (b, _) = stopped_mid_file("remove", &ff, |b, _| b.watch.apply(vec![]));
    settle_down();
    assert!(b.results().is_empty(), "{:?}", b.results());
    assert_eq!(b.watch.active(), 0);
}
