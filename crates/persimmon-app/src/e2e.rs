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
