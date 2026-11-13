// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The app's command layer end to end, on the corpus (Phase 2 spec,
//! "Testing"): the queue and the runner the Tauri commands drive, with no
//! window. Every DD-28 pair, every failure the corpus provokes, and cancel,
//! quit, retry, undo and the fallback folder.

use crate::queue::{Queue, QueueMode, RowId, RowState, RowView};
use crate::runner::Runner;
use persimmon_core::backend::Backends;
use persimmon_core::format::{FormatId, sniff};
use persimmon_core::output::OutputPolicy;
use persimmon_core::registry::{Operation, Registry};
use persimmon_corpus::archives::write_archive_sample;
use persimmon_corpus::media::{media_sample, test_ffmpeg};
use persimmon_corpus::pdf::write_pdf_sample;
use persimmon_corpus::{GIF_1X1, IMAGE_SAMPLES, PNG_1X1, RAR_SAMPLES};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A temp folder, removed on drop.
pub(crate) struct Scratch(pub PathBuf);

impl Scratch {
    pub(crate) fn new(name: &str) -> Scratch {
        let d = std::env::temp_dir().join(format!("prsm-app-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Scratch(d)
    }

    pub(crate) fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// A copy of `src` under `name`, so one sample can be several rows.
    pub(crate) fn copy(&self, src: &Path, name: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::copy(src, &p).unwrap();
        p
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The queue and runner as `app.rs` wires them, over a scratch folder.
pub(crate) struct Rig {
    pub queue: Arc<Mutex<Queue>>,
    pub runner: Runner,
    pub dir: Scratch,
}

pub(crate) fn rig(name: &str, policy: OutputPolicy, backends: Backends) -> Rig {
    let queue = Arc::new(Mutex::new(Queue::new(Registry::builtin(), policy)));
    let runner = Runner::new(Arc::clone(&queue), backends, |_| {});
    Rig {
        queue,
        runner,
        dir: Scratch::new(name),
    }
}

fn finished(s: &RowState) -> bool {
    !matches!(s, RowState::Queued | RowState::Running { .. })
}

/// Start, then wait until no row is queued or running.
pub(crate) fn run_all(rig: &Rig, secs: u64) {
    rig.runner.start();
    let end = Instant::now() + Duration::from_secs(secs);
    while !rig
        .queue
        .lock()
        .unwrap()
        .rows()
        .iter()
        .all(|r| finished(&r.state))
    {
        assert!(Instant::now() < end, "timed out after {secs}s");
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub(crate) fn state(rig: &Rig, id: RowId) -> RowState {
    rig.queue.lock().unwrap().get(id).unwrap().state
}

/// FFmpeg for the media cases; None skips them, unless CI requires it.
pub(crate) fn ffmpeg() -> Option<PathBuf> {
    let found = test_ffmpeg();
    if found.is_none() && std::env::var_os("PERSIMMON_REQUIRE_FFMPEG").is_some() {
        panic!("PERSIMMON_REQUIRE_FFMPEG is set but no FFmpeg was found");
    }
    found
}

/// Add `paths` in `mode` and point the new row at `to`.
fn row(rig: &Rig, paths: &[PathBuf], mode: QueueMode, to: Option<FormatId>) -> RowId {
    let mut q = rig.queue.lock().unwrap();
    let added = q.add(paths, mode);
    let id = added
        .last()
        .unwrap_or_else(|| panic!("{paths:?} wasn't added"))
        .id;
    if let Some(to) = to {
        let set = q.set_target(&[id], to);
        assert_eq!(set.skipped, 0, "{paths:?} can't become {to:?}");
    }
    id
}

fn done_output(rig: &Rig, id: RowId) -> PathBuf {
    let view: RowView = rig.queue.lock().unwrap().get(id).unwrap();
    match &view.state {
        RowState::Done { output, .. } => {
            assert!(
                output.exists(),
                "{}: {} is missing",
                view.name,
                output.display()
            );
            output.clone()
        }
        other => panic!("{} → {:?}: {other:?}", view.name, view.to),
    }
}

fn head(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0; 4096];
    let n = std::fs::File::open(path).unwrap().read(&mut buf).unwrap();
    buf.truncate(n);
    buf
}

/// The image inputs the core's own pair test uses.
fn image_inputs(dir: &Scratch) -> Vec<(String, PathBuf)> {
    let mut out = vec![
        ("png-1x1".to_string(), dir.put("src-png-1x1.png", &PNG_1X1)),
        ("gif-1x1".to_string(), dir.put("src-gif-1x1.gif", GIF_1X1)),
    ];
    for id in [
        "img-jpeg", "img-webp", "img-heic", "img-tiff", "img-avif", "img-bmp",
    ] {
        let (_, name, bytes, _) = IMAGE_SAMPLES.iter().find(|s| s.0 == id).unwrap();
        out.push((id.to_string(), dir.put(&format!("src-{name}"), bytes)));
    }
    out
}

fn ext(path: &Path) -> String {
    let name = path.file_name().unwrap().to_string_lossy();
    if let Some(i) = name.find(".tar.") {
        return name[i + 1..].to_string();
    }
    path.extension().unwrap().to_string_lossy().into_owned()
}

#[test]
fn every_image_pair_converts_through_the_queue() {
    let rig = rig("images", OutputPolicy::default(), Backends::new());
    let reg = Registry::builtin();
    let mut pairs = Vec::new();
    for (id, src) in image_inputs(&rig.dir) {
        let from = persimmon_core::detect::detect(&src, reg.formats())
            .unwrap()
            .format;
        for to in reg.targets(Operation::Convert, from) {
            let input = rig.dir.copy(&src, &format!("{id}-to-{to:?}.{}", ext(&src)));
            pairs.push((row(&rig, &[input], QueueMode::Convert, Some(to)), to));
        }
    }
    assert!(!pairs.is_empty());
    run_all(&rig, 300);
    for (id, to) in &pairs {
        let out = done_output(&rig, *id);
        if *to != FormatId::Pdf {
            assert_eq!(sniff(&head(&out)), Some(*to), "{}", out.display());
        }
    }
}

#[test]
fn every_media_pair_converts_through_the_queue() {
    let Some(ff) = ffmpeg() else { return };
    let _serial = crate::FFMPEG_TESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rig = rig(
        "media",
        OutputPolicy::default(),
        Backends::new().with_ffmpeg(&ff),
    );
    let reg = Registry::builtin();
    let mut ids = Vec::new();
    for id in [
        "vid-mov", "vid-mp4", "vid-mkv", "vid-webm", "vid-avi", "aud-mp3", "aud-m4a", "aud-wav",
        "aud-flac", "aud-opus",
    ] {
        let src = media_sample(id).unwrap();
        let from = persimmon_core::detect::detect(&src, reg.formats())
            .unwrap()
            .format;
        for to in reg.targets(Operation::Convert, from) {
            let input = rig.dir.copy(&src, &format!("{id}-to-{to:?}.{}", ext(&src)));
            ids.push(row(&rig, &[input], QueueMode::Convert, Some(to)));
        }
    }
    assert_eq!(ids.len(), 62, "the core's every_media_pair_converts has 62");
    run_all(&rig, 900);
    for id in ids {
        let out = done_output(&rig, id);
        assert!(
            std::fs::metadata(&out).unwrap().len() > 0,
            "{}",
            out.display()
        );
    }
}

/// The pages a converted PDF folder holds.
fn pages(dir: &Path) -> usize {
    std::fs::read_dir(dir).unwrap().count()
}

#[test]
fn every_pdf_job_runs_through_the_queue() {
    let rig = rig("pdf", OutputPolicy::default(), Backends::new());
    let text = write_pdf_sample(&rig.dir.0, "pdf-text");
    let to_png = row(
        &rig,
        &[rig.dir.copy(&text, "to-png.pdf")],
        QueueMode::Convert,
        Some(FormatId::Png),
    );
    let to_jpeg = row(
        &rig,
        &[rig.dir.copy(&text, "to-jpeg.pdf")],
        QueueMode::Convert,
        Some(FormatId::Jpeg),
    );
    let images: Vec<(String, PathBuf)> = image_inputs(&rig.dir);
    let to_pdf: Vec<RowId> = images
        .iter()
        .map(|(id, src)| {
            let input = rig.dir.copy(src, &format!("pdf-{id}.{}", ext(src)));
            row(&rig, &[input], QueueMode::Convert, Some(FormatId::Pdf))
        })
        .collect();
    let a = rig.dir.copy(&images[2].1, "combine-a.jpg");
    let b = rig.dir.copy(&images[3].1, "combine-b.webp");
    let c = rig.dir.copy(&text, "combine-c.pdf");
    let merged = row(&rig, &[a, b, c], QueueMode::Combine, Some(FormatId::Pdf));
    run_all(&rig, 300);
    assert_eq!(pages(&done_output(&rig, to_png)), 3);
    assert_eq!(pages(&done_output(&rig, to_jpeg)), 3);
    for id in to_pdf {
        assert_eq!(sniff(&head(&done_output(&rig, id))), Some(FormatId::Pdf));
    }
    // Two images and a three-page PDF: five pages, counted by converting
    // the merged PDF back through the queue.
    let merged = done_output(&rig, merged);
    let check = row(&rig, &[merged], QueueMode::Convert, Some(FormatId::Png));
    run_all(&rig, 120);
    assert_eq!(pages(&done_output(&rig, check)), 5);
}

#[test]
fn every_archive_extracts_through_the_queue() {
    let rig = rig("archives", OutputPolicy::default(), Backends::new());
    let mut ids = Vec::new();
    for id in [
        "arc-zip", "arc-tar", "arc-tgz", "arc-tbz2", "arc-txz", "arc-tzst", "arc-7z",
    ] {
        let input = write_archive_sample(&rig.dir.0, id);
        ids.push(row(&rig, &[input], QueueMode::Extract, None));
    }
    for (id, name, bytes) in RAR_SAMPLES {
        if id != "rar4-locked" {
            ids.push(row(
                &rig,
                &[rig.dir.put(name, bytes)],
                QueueMode::Extract,
                None,
            ));
        }
    }
    run_all(&rig, 300);
    for id in ids {
        let out = done_output(&rig, id);
        assert!(
            out.is_dir() && pages(&out) > 0,
            "{} is empty",
            out.display()
        );
    }
}

#[test]
fn combine_and_compress_run_through_the_queue() {
    let Some(ff) = ffmpeg() else { return };
    let _serial = crate::FFMPEG_TESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rig = rig(
        "combine",
        OutputPolicy::default(),
        Backends::new().with_ffmpeg(&ff),
    );
    let images = image_inputs(&rig.dir);
    let files = |tag: &str| -> Vec<PathBuf> {
        (2..5)
            .map(|i| {
                rig.dir.copy(
                    &images[i].1,
                    &format!("{tag}-{}.{}", images[i].0, ext(&images[i].1)),
                )
            })
            .collect()
    };
    // Combine collects files into the one Combine row still queued, so the
    // ZIP runs before the tar.gz's files are added.
    let zip = row(&rig, &files("zip"), QueueMode::Combine, Some(FormatId::Zip));
    run_all(&rig, 120);
    let tgz = row(
        &rig,
        &files("tgz"),
        QueueMode::Combine,
        Some(FormatId::TarGz),
    );
    let mut compress = vec![
        row(
            &rig,
            &[rig.dir.copy(&images[2].1, "small.jpg")],
            QueueMode::Compress,
            None,
        ),
        row(
            &rig,
            &[rig.dir.copy(&images[0].1, "small.png")],
            QueueMode::Compress,
            None,
        ),
    ];
    let mp4 = media_sample("vid-mp4").unwrap();
    compress.push(row(
        &rig,
        &[rig.dir.copy(&mp4, "small.mp4")],
        QueueMode::Compress,
        None,
    ));
    run_all(&rig, 300);
    assert_eq!(sniff(&head(&done_output(&rig, zip))), Some(FormatId::Zip));
    assert_ne!(zip, tgz, "the tar.gz files made a row of their own");
    assert_eq!(sniff(&head(&done_output(&rig, tgz))), Some(FormatId::TarGz));
    for id in compress {
        let s = state(&rig, id);
        assert!(
            matches!(s, RowState::Done { .. } | RowState::NotSmaller),
            "compress: {s:?}"
        );
    }
}

/// Every corpus file, as week 2's wording sweep gathers them.
fn corpus_files(dir: &Path) -> Vec<PathBuf> {
    use persimmon_corpus::archives::ARCHIVE_SAMPLES;
    use persimmon_corpus::media::MEDIA_SAMPLES;
    use persimmon_corpus::pdf::PDF_SAMPLES;
    use persimmon_corpus::{Options, generate};
    let mut files: Vec<PathBuf> = generate(&dir.join("generated"), &Options { large: false })
        .unwrap()
        .into_iter()
        .map(|c| c.path)
        .filter(|p| p.is_file())
        .filter(|p| !["many.pdf", "big-page.pdf"].iter().any(|n| p.ends_with(n)))
        .collect();
    let samples = dir.join("samples");
    std::fs::create_dir_all(&samples).unwrap();
    for (_, name, bytes, _) in IMAGE_SAMPLES {
        let p = samples.join(name);
        std::fs::write(&p, bytes).unwrap();
        files.push(p);
    }
    for (_, name, bytes) in RAR_SAMPLES {
        let p = samples.join(name);
        std::fs::write(&p, bytes).unwrap();
        files.push(p);
    }
    for (id, _, _) in PDF_SAMPLES {
        if !["pdf-many", "pdf-big-page"].contains(&id) {
            files.push(write_pdf_sample(&samples, id));
        }
    }
    for (id, _) in ARCHIVE_SAMPLES {
        files.push(write_archive_sample(&samples, id));
    }
    for (id, _) in MEDIA_SAMPLES {
        if id != "vid-long"
            && let Some(src) = media_sample(id)
        {
            let to = samples.join(src.file_name().unwrap());
            std::fs::copy(&src, &to).unwrap();
            files.push(to);
        }
    }
    files
}

/// Gate item 3: every error a corpus input provokes shows in its row with
/// the file, the reason and a fix in the app's words, from planning or
/// from running.
#[test]
fn every_corpus_failure_shows_file_reason_and_fix() {
    use persimmon_core::detect::detect;
    use persimmon_core::format::MediaKind;
    let Some(ff) = ffmpeg() else { return };
    let _serial = crate::FFMPEG_TESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let out = Scratch::new("failures-out");
    let rig = rig(
        "failures",
        OutputPolicy {
            dir: Some(out.0.clone()),
            ..OutputPolicy::default()
        },
        Backends::new().with_ffmpeg(&ff),
    );
    let reg = Registry::builtin();
    for f in corpus_files(&rig.dir.0) {
        let detected = detect(&f, reg.formats()).ok().map(|d| d.format);
        let mut q = rig.queue.lock().unwrap();
        match detected {
            Some(k) if reg.format(k).kind == MediaKind::Archive => {
                q.add(std::slice::from_ref(&f), QueueMode::Extract);
            }
            Some(FormatId::Pdf) | None => {
                if let Some(r) = q.add(std::slice::from_ref(&f), QueueMode::Convert).first() {
                    q.set_target(&[r.id], FormatId::Png);
                }
            }
            Some(_) => {
                q.add(std::slice::from_ref(&f), QueueMode::Convert);
            }
        }
    }
    run_all(&rig, 600);
    let mut failed = 0;
    for r in rig.queue.lock().unwrap().rows() {
        let RowState::Failed { error } = &r.state else {
            continue;
        };
        failed += 1;
        let at = r.path.display();
        assert!(error.file.is_some(), "{at}: no file in {error:?}");
        assert!(!error.reason.is_empty(), "{at}: no reason");
        let fix = error
            .fix
            .as_deref()
            .unwrap_or_else(|| panic!("{at}: no fix for {error:?}"));
        for text in [error.reason.as_str(), fix] {
            if let Some(word) = crate::wording::cli_word(text) {
                panic!("{at}: {text:?} has the CLI word {word:?}");
            }
        }
    }
    // The corpus is full of bad files on purpose; a sweep that stops
    // reaching them can't pass by checking nothing.
    assert!(failed >= 10, "only {failed} rows failed");
}

/// No `.part` or `.prsm-*` file anywhere under `dir`.
fn no_partials(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().all(|e| {
        let e = e.unwrap();
        let n = e.file_name().to_string_lossy().into_owned();
        let ok = !n.ends_with(".part") && !n.starts_with(".prsm-");
        ok && (!e.file_type().unwrap().is_dir() || no_partials(&e.path()))
    })
}

fn no_ffmpeg_child() -> bool {
    !std::process::Command::new("/usr/bin/pgrep")
        .args(["-P", &std::process::id().to_string(), "ffmpeg"])
        .status()
        .unwrap()
        .success()
}

fn wait_until(what: &str, secs: u64, done: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !done() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A tar of 1,000 files of 256 KB: long enough to extract that a cancel
/// lands in the middle.
fn many_files_tar(dir: &Scratch) -> PathBuf {
    let src = dir.0.join("many-src");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..1000 {
        std::fs::write(src.join(format!("f{i:04}.bin")), vec![i as u8; 256 << 10]).unwrap();
    }
    let tar = dir.0.join("many.tar");
    let ok = std::process::Command::new("/usr/bin/tar")
        .arg("-cf")
        .arg(&tar)
        .arg("-C")
        .arg(&src)
        .arg(".")
        .status()
        .unwrap()
        .success();
    assert!(ok);
    std::fs::remove_dir_all(&src).unwrap();
    tar
}

/// One long input per backend: FFmpeg, PDF pages, an archive, each with
/// the mode and target its row takes.
fn long_inputs(rig: &Rig) -> Vec<(&'static str, PathBuf, QueueMode, Option<FormatId>)> {
    let video = rig.dir.copy(&media_sample("vid-long").unwrap(), "long.mov");
    let pdf = write_pdf_sample(&rig.dir.0, "pdf-many");
    let tar = many_files_tar(&rig.dir);
    vec![
        ("video", video, QueueMode::Convert, Some(FormatId::Webm)),
        ("pdf", pdf, QueueMode::Convert, Some(FormatId::Png)),
        ("archive", tar, QueueMode::Extract, None),
    ]
}

fn planned_output(rig: &Rig, id: RowId) -> PathBuf {
    rig.queue.lock().unwrap().get(id).unwrap().output.unwrap()
}

/// Gate item 2, per backend: a cancelled row leaves no partial output and
/// no FFmpeg.
#[test]
fn cancel_leaves_nothing_behind_on_every_backend() {
    let Some(ff) = ffmpeg() else { return };
    let _serial = crate::FFMPEG_TESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rig = rig(
        "cancel",
        OutputPolicy::default(),
        Backends::new().with_ffmpeg(&ff),
    );
    // One row at a time, so the check never sees the next row's work.
    for (what, input, mode, to) in long_inputs(&rig) {
        let id = row(&rig, &[input], mode, to);
        let output = planned_output(&rig, id);
        rig.runner.start();
        wait_until(what, 30, || {
            matches!(state(&rig, id), RowState::Running { .. })
        });
        assert!(rig.runner.cancel(id), "{what}: cancel refused");
        wait_until(what, 30, || finished(&state(&rig, id)));
        assert_eq!(state(&rig, id), RowState::Cancelled, "{what}");
        assert!(!output.exists(), "{what}: {} was left", output.display());
        assert!(no_partials(&rig.dir.0), "{what}: a partial file was left");
        assert!(no_ffmpeg_child(), "{what}: FFmpeg still runs");
    }
}

/// Gate item 2, quitting: shutdown with rows running and queued.
#[test]
fn shutdown_leaves_nothing_behind() {
    let Some(ff) = ffmpeg() else { return };
    let _serial = crate::FFMPEG_TESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rig = rig(
        "shutdown",
        OutputPolicy::default(),
        Backends::new().with_ffmpeg(&ff),
    );
    let rows: Vec<(&str, RowId)> = long_inputs(&rig)
        .into_iter()
        .map(|(what, input, mode, to)| (what, row(&rig, &[input], mode, to)))
        .collect();
    let outputs: Vec<PathBuf> = rows
        .iter()
        .map(|(_, id)| planned_output(&rig, *id))
        .collect();
    rig.runner.start();
    wait_until("the video", 30, || {
        matches!(state(&rig, rows[0].1), RowState::Running { .. })
    });
    rig.runner.shutdown();
    for ((what, id), output) in rows.iter().zip(&outputs) {
        let s = state(&rig, *id);
        assert!(
            !matches!(s, RowState::Running { .. }),
            "{what} still running"
        );
        assert!(!output.exists(), "{what}: {} was left", output.display());
    }
    assert!(no_partials(&rig.dir.0));
    assert!(no_ffmpeg_child());
}

/// A cancel that races the row's own finish: whole output or none.
#[test]
fn cancel_racing_the_finish_leaves_a_whole_or_no_output() {
    let rig = rig("race", OutputPolicy::default(), Backends::new());
    for i in 0..20 {
        let input = rig.dir.put(&format!("race-{i}.png"), &PNG_1X1);
        let id = row(&rig, &[input], QueueMode::Convert, Some(FormatId::Jpeg));
        let output = planned_output(&rig, id);
        rig.runner.start();
        rig.runner.cancel(id);
        wait_until("the row", 30, || finished(&state(&rig, id)));
        match state(&rig, id) {
            RowState::Done { output: out, .. } => {
                assert_eq!(sniff(&head(&out)), Some(FormatId::Jpeg), "run {i}")
            }
            RowState::Cancelled => assert!(!output.exists(), "run {i}: output left"),
            other => panic!("run {i}: {other:?}"),
        }
        assert!(no_partials(&rig.dir.0), "run {i}");
    }
}

/// Makes `dir` read-only until dropped, even if an assertion fails.
struct ReadOnly(PathBuf);
impl ReadOnly {
    fn new(dir: &Path) -> ReadOnly {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        ReadOnly(dir.to_path_buf())
    }
    fn restore(&self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}
impl Drop for ReadOnly {
    fn drop(&mut self) {
        self.restore();
    }
}

#[test]
fn retry_after_the_cause_is_fixed_succeeds() {
    let out = Scratch::new("retry-out");
    let locked = ReadOnly::new(&out.0.join("locked"));
    let rig = rig(
        "retry",
        OutputPolicy {
            dir: Some(locked.0.clone()),
            fallback: None,
            ..OutputPolicy::default()
        },
        Backends::new(),
    );
    let id = row(
        &rig,
        &[rig.dir.put("a.png", &PNG_1X1)],
        QueueMode::Convert,
        Some(FormatId::Jpeg),
    );
    run_all(&rig, 60);
    match state(&rig, id) {
        RowState::Failed { error } => {
            assert_eq!(
                error.code,
                persimmon_core::error::ErrorCode::OutputNotWritable
            )
        }
        other => panic!("{other:?}"),
    }
    locked.restore();
    assert!(rig.runner.retry(id).is_some(), "retry refused");
    run_all(&rig, 60);
    assert!(done_output(&rig, id).starts_with(&locked.0));
}

/// The fallback folder is for a source folder that can't be written (the
/// Photos library, Mail, a read-only disk), with outputs beside originals.
#[test]
fn fallback_catches_an_unwritable_folder() {
    use std::os::unix::fs::PermissionsExt;
    let out = Scratch::new("fallback-out");
    let fallback = out.0.join("fallback");
    let rig = rig(
        "fallback",
        OutputPolicy {
            fallback: Some(fallback.clone()),
            ..OutputPolicy::default()
        },
        Backends::new(),
    );
    let source = rig.dir.0.join("source");
    std::fs::create_dir_all(&source).unwrap();
    let input = source.join("a.png");
    std::fs::write(&input, PNG_1X1).unwrap();
    std::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o644)).unwrap();
    let locked = ReadOnly::new(&source);
    let id = row(&rig, &[input], QueueMode::Convert, Some(FormatId::Jpeg));
    run_all(&rig, 60);
    assert!(done_output(&rig, id).starts_with(&fallback));
    assert!(
        rig.queue.lock().unwrap().get(id).unwrap().note.is_some(),
        "no note"
    );
    drop(locked);
}

struct FakeTrash(PathBuf);
impl crate::history::Trasher for FakeTrash {
    fn trash(&self, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.0)?;
        std::fs::rename(path, self.0.join(path.file_name().unwrap()))
    }
}

#[test]
fn undo_trashes_what_a_run_wrote() {
    use crate::history::History;
    let rig = rig("undo", OutputPolicy::default(), Backends::new());
    let history = Arc::new(Mutex::new(History::open(&rig.dir.0.join("history.jsonl"))));
    let h = Arc::clone(&history);
    rig.runner.set_on_batch(move |b| {
        h.lock().unwrap().append(b).unwrap();
    });
    let inputs = [
        rig.dir.put("a.png", &PNG_1X1),
        rig.dir.put("b.png", &PNG_1X1),
    ];
    let ids: Vec<RowId> = inputs
        .iter()
        .map(|p| {
            row(
                &rig,
                std::slice::from_ref(p),
                QueueMode::Convert,
                Some(FormatId::Jpeg),
            )
        })
        .collect();
    run_all(&rig, 60);
    let outputs: Vec<PathBuf> = ids.iter().map(|id| done_output(&rig, *id)).collect();
    wait_until("the batch", 10, || {
        !history.lock().unwrap().batches().is_empty()
    });
    let mut h = history.lock().unwrap();
    let batch = h.batches()[0].id.clone();
    let report = h.undo(&batch, &FakeTrash(rig.dir.0.join("trash")));
    assert_eq!(report.trashed, 2, "{report:?}");
    assert!(outputs.iter().all(|o| !o.exists()));
    assert!(inputs.iter().all(|i| i.exists()));
}
