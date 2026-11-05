// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use persimmon_corpus::{Case, Options, generate};

const PRSM: &str = env!("CARGO_BIN_EXE_prsm");

struct Corpus {
    dir: PathBuf,
    cases: Vec<Case>,
}

impl Corpus {
    fn new(test: &str) -> Corpus {
        Corpus::with(test, false)
    }

    fn with(test: &str, large: bool) -> Corpus {
        let dir = std::env::temp_dir().join(format!("prsm-cli-{test}-{}", std::process::id()));
        let cases = generate(&dir, &Options { large }).unwrap();
        Corpus { dir, cases }
    }

    fn path(&self, id: &str) -> &Path {
        &self.cases.iter().find(|c| c.id == id).unwrap().path
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn prsm<I, S>(cwd: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new(PRSM)
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).unwrap()
}

/// The lines of one `prsm formats` section.
fn section(text: &str, op: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| *l != op)
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn formats_lists_heic_to_jpeg() {
    let out = prsm(Path::new("."), ["formats"]);
    assert!(out.status.success());
    let text = stdout(&out);
    let heic = section(&text, "convert")
        .into_iter()
        .find(|l| l.starts_with("  heic"))
        .expect("a heic line under convert");
    let (_, targets) = heic.split_once('→').unwrap();
    assert!(targets.contains("jpeg"), "{heic}");
    assert!(!targets.contains(" heic"), "{heic}");
}

#[test]
fn formats_sections_in_order() {
    let text = stdout(&prsm(Path::new("."), ["formats"]));
    let headers: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with(' ') && !l.is_empty())
        .collect();
    assert_eq!(
        headers,
        ["convert", "compress", "extract", "merge", "split"]
    );
    assert!(
        section(&text, "extract")
            .iter()
            .any(|l| l == "  rar     → folder")
    );
    assert!(
        section(&text, "split")
            .iter()
            .any(|l| l == "  pdf     → pdf")
    );
}

#[test]
fn info_reports_mismatch() {
    let c = Corpus::new("info-mismatch");
    let out = prsm(&c.dir, [c.path("png-as-jpg")]);
    // `prsm <file>` alone is a usage error; info is explicit.
    assert_eq!(out.status.code(), Some(2));
    let out = prsm(&c.dir, ["info".as_ref(), c.path("png-as-jpg").as_os_str()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("format   PNG (png)"), "{text}");
    assert!(text.contains("size     67 B (67 bytes)"), "{text}");
    assert!(
        text.contains("note     the extension says .jpg but the content is PNG"),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.starts_with("becomes  ") && l.contains("webp")),
        "{text}"
    );
}

#[test]
fn info_without_mismatch_has_no_note() {
    let c = Corpus::new("info-plain");
    let text = stdout(&prsm(
        &c.dir,
        ["info".as_ref(), c.path("png-1x1").as_os_str()],
    ));
    assert!(text.contains("format   PNG (png)"), "{text}");
    assert!(!text.contains("note"), "{text}");
}

#[test]
fn info_empty_exits_5() {
    let c = Corpus::new("info-empty");
    let out = prsm(&c.dir, ["info".as_ref(), c.path("empty-png").as_os_str()]);
    assert_eq!(out.status.code(), Some(5));
    let err = stderr(&out);
    assert!(err.starts_with("prsm: "), "{err}");
    assert!(err.contains("file is empty"), "{err}");
}

#[test]
fn dry_run_leading_dash_and_emoji() {
    let c = Corpus::new("dash-emoji");
    let out = prsm(
        &c.dir,
        [
            "convert",
            "--dry-run",
            "--to",
            "webp",
            "--",
            "-rf.png",
            "\u{1F351} peach.png",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().filter(|l| l.contains('→')).collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].ends_with("-rf.webp"), "{text}");
    assert!(lines[1].ends_with("\u{1F351} peach.webp"), "{text}");
    assert_eq!(lines[0], "-rf.png → -rf.webp");
}

#[test]
fn dry_run_nfd_name() {
    let c = Corpus::new("nfd");
    let input = c.path("nfd");
    let out = prsm(
        &c.dir,
        [
            "convert".as_ref(),
            "--dry-run".as_ref(),
            "--to".as_ref(),
            "webp".as_ref(),
            input.as_os_str(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let expected = input.with_file_name("cafe\u{301}.webp");
    assert_eq!(
        stdout(&out).trim_end(),
        format!("{} → {}", input.display(), expected.display())
    );
}

#[test]
fn batch_exit_code_is_first_failure() {
    let c = Corpus::new("batch");
    let out = prsm(
        &c.dir,
        [
            "convert",
            "--dry-run",
            "--to",
            "webp",
            "missing.png",
            "empty.png",
        ],
    );
    assert_eq!(out.status.code(), Some(7));
    let err = stderr(&out);
    assert!(err.contains("missing.png"), "{err}");
    assert!(err.contains("empty.png: file is empty"), "{err}");
}

#[test]
fn without_dry_run_is_not_implemented() {
    let c = Corpus::new("not-impl");
    let out = prsm(&c.dir, ["convert", "--to", "webp", "pixel.png"]);
    assert_eq!(out.status.code(), Some(10));
    let err = stderr(&out);
    assert!(err.contains("the image backend isn't built yet"), "{err}");
    assert!(err.contains("fix: use --dry-run to see the plan"), "{err}");
    assert!(!c.dir.join("pixel.webp").exists());
}

#[test]
fn unsupported_conversion_exit_8() {
    let c = Corpus::new("unsupported");
    let out = prsm(
        &c.dir,
        [
            "convert".as_ref(),
            "--dry-run".as_ref(),
            "--to".as_ref(),
            "mp3".as_ref(),
            c.path("png-1x1").as_os_str(),
        ],
    );
    assert_eq!(out.status.code(), Some(8));
    assert!(stderr(&out).contains("fix: run prsm formats to see what png can become"));
}

#[test]
fn merge_dry_run_counts_files() {
    let c = Corpus::new("merge");
    let out = prsm(
        &c.dir,
        [
            "merge",
            "--dry-run",
            "--to",
            "pdf",
            "pixel.png",
            "pixel.gif",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim_end(), "2 files → merged.pdf");
}

#[test]
fn out_dir_dry_run() {
    let c = Corpus::new("out");
    let out = prsm(
        &c.dir,
        ["compress", "--dry-run", "--out", "new/place", "pixel.png"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let expected = Path::new("new/place").join("pixel (compressed).png");
    assert_eq!(
        stdout(&out).trim_end(),
        format!("pixel.png → {}", expected.display())
    );
}

#[test]
fn split_bad_pages_is_usage() {
    let c = Corpus::new("split");
    let out = prsm(&c.dir, ["split", "--dry-run", "--pages", "3-1", "page.pdf"]);
    assert_eq!(out.status.code(), Some(2));
    let err = stderr(&out);
    assert!(err.starts_with("prsm: "), "{err}");
    assert!(err.contains("isn't a page range"), "{err}");
}

#[test]
fn unknown_target_format() {
    let c = Corpus::new("unknown-target");
    let out = prsm(
        &c.dir,
        ["convert", "--dry-run", "--to", "docx", "pixel.png"],
    );
    assert_eq!(out.status.code(), Some(8));
    assert!(
        stderr(&out).contains("fix: run prsm formats"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn large_file_info_is_fast() {
    if std::env::var("PERSIMMON_CORPUS_LARGE").as_deref() != Ok("1") {
        return;
    }
    let c = Corpus::with("large", true);
    let started = Instant::now();
    let out = prsm(&c.dir, ["info".as_ref(), c.path("large-4gb").as_os_str()]);
    let elapsed = started.elapsed();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("4.3 GB"), "{}", stdout(&out));
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}
