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
fn locked_pdf_fails_at_planning() {
    use persimmon_corpus::pdf::write_pdf_sample;
    let c = Corpus::new("locked-pdf");
    write_pdf_sample(&c.dir, "pdf-locked");
    write_pdf_sample(&c.dir, "pdf-text");
    let out = prsm(
        &c.dir,
        [
            "convert",
            "--dry-run",
            "--to",
            "png",
            "locked.pdf",
            "text.pdf",
        ],
    );
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim_end(), "text.pdf → text");
    let err = stderr(&out);
    assert!(err.contains("locked.pdf: it's password-protected"), "{err}");
}

mod pdf {
    use super::*;
    use persimmon_corpus::pdf::write_pdf_sample;

    fn with_pdfs(test: &str, ids: &[&str]) -> Corpus {
        let c = Corpus::new(test);
        for id in ids {
            write_pdf_sample(&c.dir, id);
        }
        c
    }

    /// The `pages` row `prsm info` gives for `file`.
    fn pages(dir: &Path, file: &str) -> String {
        let out = prsm(dir, ["info", file]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        stdout(&out)
            .lines()
            .find_map(|l| l.strip_prefix("pages    "))
            .unwrap_or_else(|| panic!("no pages row: {}", stdout(&out)))
            .to_string()
    }

    #[test]
    fn info_shows_pdf_pages() {
        let c = with_pdfs("info-pdf", &["pdf-text"]);
        let out = prsm(&c.dir, ["info", "text.pdf"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let text = stdout(&out);
        assert!(text.contains("pages    3\n"), "{text}");
        assert!(text.contains("page     210×297 mm (A4)\n"), "{text}");
    }

    #[test]
    fn info_locked_pdf_is_exit_3() {
        let c = with_pdfs("info-locked", &["pdf-locked"]);
        let out = prsm(&c.dir, ["info", "locked.pdf"]);
        assert_eq!(out.status.code(), Some(3));
        assert!(stdout(&out).contains("format   PDF (pdf)"));
        assert!(stderr(&out).contains("locked.pdf: it's password-protected"));
    }

    #[test]
    fn pdf_to_png_writes_a_folder() {
        let c = with_pdfs("pdf-png", &["pdf-text"]);
        let out = prsm(&c.dir, ["convert", "--to", "png", "text.pdf"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert_eq!(stdout(&out).trim_end(), "text.pdf → text");
        let mut names: Vec<String> = std::fs::read_dir(c.dir.join("text"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["text 1.png", "text 2.png", "text 3.png"]);
    }

    #[test]
    fn split_writes_selected_pages() {
        let c = with_pdfs("split-pdf", &["pdf-text"]);
        let out = prsm(&c.dir, ["split", "--pages", "2", "text.pdf"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert_eq!(stdout(&out).trim_end(), "text.pdf → text (pages 2).pdf");
        assert_eq!(pages(&c.dir, "text (pages 2).pdf"), "1");
    }

    #[test]
    fn merge_images_and_pdf() {
        let c = with_pdfs("merge-pdf", &["pdf-text"]);
        let photo = persimmon_corpus::IMAGE_SAMPLES
            .iter()
            .find(|s| s.0 == "img-jpeg")
            .unwrap()
            .2;
        std::fs::write(c.dir.join("photo.jpg"), photo).unwrap();
        let out = prsm(&c.dir, ["merge", "--to", "pdf", "photo.jpg", "text.pdf"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert_eq!(stdout(&out).trim_end(), "2 files → merged.pdf");
        assert_eq!(pages(&c.dir, "merged.pdf"), "4");
    }

    #[test]
    fn convert_pages_flag() {
        let c = with_pdfs("pages-flag", &["pdf-text"]);
        let out = prsm(
            &c.dir,
            ["convert", "--to", "jpeg", "--pages", "2", "text.pdf"],
        );
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert_eq!(stdout(&out).trim_end(), "text.pdf → text (page 2).jpg");
        assert!(c.dir.join("text (page 2).jpg").is_file());
    }

    #[test]
    fn bad_pages_flag_is_usage() {
        let c = with_pdfs("pages-bad", &["pdf-text"]);
        let out = prsm(
            &c.dir,
            ["convert", "--to", "png", "--pages", "x", "text.pdf"],
        );
        assert_eq!(out.status.code(), Some(2));
        assert!(stderr(&out).contains("isn't a page range"));
    }
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

#[cfg(feature = "vips")]
mod images {
    use super::*;
    use persimmon_core::format::{FormatId, sniff};

    /// True when no `.part` temp file is left anywhere in `dir`.
    fn no_parts(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .unwrap()
            .all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part"))
    }

    fn head(p: &Path) -> Vec<u8> {
        let b = std::fs::read(p).unwrap();
        b[..b.len().min(512)].to_vec()
    }

    #[test]
    fn convert_writes_webp() {
        let c = Corpus::new("img-webp");
        let out = prsm(&c.dir, ["convert", "--to", "webp", "pixel.png"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert_eq!(stdout(&out).trim_end(), "pixel.png → pixel.webp");
        assert_eq!(
            sniff(&head(&c.dir.join("pixel.webp"))),
            Some(FormatId::Webp)
        );
        assert!(no_parts(&c.dir));
    }

    #[test]
    fn mismatched_name_is_safe() {
        let c = Corpus::new("img-mismatch");
        let out = prsm(&c.dir, ["convert", "--to", "jpeg", "photo.jpg"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert_eq!(stdout(&out).trim_end(), "photo.jpg → photo (1).jpg");
        assert_eq!(
            sniff(&head(&c.dir.join("photo (1).jpg"))),
            Some(FormatId::Jpeg)
        );
        assert_eq!(
            sniff(&head(&c.dir.join("photo.jpg"))),
            Some(FormatId::Png),
            "the input is untouched"
        );
    }

    #[test]
    fn compress_already_compact() {
        let c = Corpus::new("img-compact");
        let out = prsm(&c.dir, ["compress", "pixel.png"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert_eq!(
            stdout(&out).trim_end(),
            "pixel.png: already compact, nothing written"
        );
        assert!(!c.dir.join("pixel (compressed).png").exists());
    }

    #[test]
    fn max_size_and_strip_flags() {
        let c = Corpus::new("img-flags");
        let out = prsm(
            &c.dir,
            [
                "convert",
                "--to",
                "png",
                "--max-size",
                "100",
                "--strip-metadata",
                "images/photo.jpg",
            ],
        );
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let png = std::fs::read(c.dir.join("images/photo.png")).unwrap();
        let (w, h) = (
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
        );
        assert_eq!((w, h), (100, 75));
    }

    #[test]
    fn batch_mixed() {
        let c = Corpus::new("img-batch");
        let out = prsm(
            &c.dir,
            [
                "convert",
                "--to",
                "webp",
                "pixel.png",
                "empty.png",
                "pixel.gif",
            ],
        );
        assert_eq!(out.status.code(), Some(5));
        let text = stdout(&out);
        assert_eq!(
            text.lines().filter(|l| l.contains('→')).count(),
            2,
            "{text}"
        );
        assert!(stderr(&out).contains("empty.png: file is empty"));
    }

    #[cfg(unix)]
    #[test]
    fn closed_stdout_still_converts_every_file() {
        use std::io::BufRead;
        let c = Corpus::new("img-pipe");
        let work = c.dir.join("pipe");
        std::fs::create_dir(&work).unwrap();
        let jpg = std::fs::read(c.path("img-jpeg")).unwrap();
        let mut args = vec!["convert".to_string(), "--to".into(), "webp".into()];
        for i in 0..6 {
            std::fs::write(work.join(format!("p{i}.jpg")), &jpg).unwrap();
            args.push(format!("p{i}.jpg"));
        }
        let mut child = Command::new(PRSM)
            .current_dir(&work)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // Read one line and close the pipe, as `| head -1` does.
        {
            let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
        }
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        for i in 0..6 {
            assert!(
                work.join(format!("p{i}.webp")).exists(),
                "p{i}.webp missing"
            );
        }
        assert!(no_parts(&work));
    }

    #[cfg(unix)]
    #[test]
    fn ctrl_c_leaves_no_part() {
        use std::io::BufRead;
        let c = Corpus::new("img-ctrl-c");
        let work = c.dir.join("many");
        std::fs::create_dir(&work).unwrap();
        let heic = std::fs::read(c.path("img-heic")).unwrap();
        let mut args = vec![
            "convert".to_string(),
            "--to".into(),
            "avif".into(),
            "--".into(),
        ];
        for i in 0..40 {
            std::fs::write(work.join(format!("p{i:02}.heic")), &heic).unwrap();
            args.push(format!("p{i:02}.heic"));
        }
        let mut child = Command::new(PRSM)
            .current_dir(&work)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // Keep reading stdout to the end: closing the pipe early would make
        // prsm's next line fail.
        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        assert!(first.contains('→'), "{first}");
        Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap();
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut reader, &mut rest).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(130),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let written = std::fs::read_dir(&work)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "avif")
            })
            .count();
        assert!((1..40).contains(&written), "{written} written");
        assert!(no_parts(&work));
        assert!(String::from_utf8_lossy(&out.stderr).contains("cancelled"));
    }
}

#[cfg(not(feature = "vips"))]
#[test]
fn no_vips_is_backend_missing() {
    let c = Corpus::new("no-vips");
    let out = prsm(&c.dir, ["convert", "--to", "webp", "pixel.png"]);
    assert_eq!(out.status.code(), Some(4));
    assert!(stderr(&out).contains("this build has no image backend"));
}

/// Tests that need FFmpeg; each skips without one (see `test_ffmpeg`).
mod media {
    use super::*;
    use persimmon_corpus::media::media_sample;

    /// A test's own folder, removed when the test ends.
    struct TempDir(PathBuf);

    impl std::ops::Deref for TempDir {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl AsRef<Path> for TempDir {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A sample copied into a fresh folder for this test, or `None` (skip).
    fn sample(test: &str, id: &str) -> Option<(TempDir, PathBuf)> {
        let Some(src) = media_sample(id) else {
            eprintln!("skipped: no ffmpeg");
            return None;
        };
        let dir = std::env::temp_dir().join(format!("prsm-cli-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(src.file_name().unwrap());
        std::fs::copy(&src, &path).unwrap();
        Some((TempDir(dir), path))
    }

    /// A real clip padded to 4 GB with a sparse tail, so `info` has media
    /// to probe. The corpus's own 4 GB case is a header and zeros, which
    /// `info` rightly calls unreadable.
    #[test]
    fn large_file_info_is_fast() {
        if std::env::var("PERSIMMON_CORPUS_LARGE").as_deref() != Ok("1") {
            return;
        }
        let Some((dir, path)) = sample("info-large", "vid-mov") else {
            return;
        };
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(4 << 30)
            .unwrap();
        let started = Instant::now();
        let out = prsm(&dir, ["info", "clip.mov"]);
        let elapsed = started.elapsed();
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let text = stdout(&out);
        assert!(text.contains("4.3 GB"), "{text}");
        assert!(text.contains("duration 0:02\n"), "{text}");
        assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
    }

    #[test]
    fn info_shows_media_details() {
        let Some((dir, _)) = sample("info-media", "vid-mov") else {
            return;
        };
        let out = prsm(&dir, ["info", "clip.mov"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let text = stdout(&out);
        assert!(text.contains("duration 0:02\n"), "{text}");
        assert!(text.contains("video    H.264, 320×240, 25 fps\n"), "{text}");
        assert!(text.contains("audio    AAC, stereo, 48 kHz\n"), "{text}");
    }

    #[test]
    fn video_converts_and_piped_stderr_has_no_progress() {
        let Some((dir, _)) = sample("video-convert", "vid-mov") else {
            return;
        };
        let out = prsm(&dir, ["convert", "clip.mov", "--to", "mp4"]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert_eq!(stdout(&out), "clip.mov → clip.mp4\n");
        assert!(!stderr(&out).contains('%'), "{}", stderr(&out));
        assert!(dir.join("clip.mp4").is_file());
    }

    /// The FFmpeg processes a prsm started: its command line names prsm's
    /// temp files, `.prsm-{pid}-…`. (prsm runs with relative paths, so the
    /// folder never appears there.)
    #[cfg(unix)]
    fn ffmpeg_of(prsm: u32) -> String {
        let out = Command::new("pgrep")
            .args(["-fl", &format!("\\.prsm-{prsm}-")])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Start `prsm convert long.mp4 --to webm` in its own process group and
    /// wait until FFmpeg is writing. Returns the child and its folder.
    #[cfg(unix)]
    fn start_long_video(test: &str) -> Option<(std::process::Child, TempDir)> {
        use std::os::unix::process::CommandExt;
        let (dir, _) = sample(test, "vid-long")?;
        let child = Command::new(PRSM)
            .current_dir(&dir)
            .args(["convert", "long.mp4", "--to", "webm"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let start = Instant::now();
        while !has_part(&dir) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "FFmpeg never started writing"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        // The check in assert_cancelled_cleanly must be able to fail.
        assert!(
            ffmpeg_of(child.id()).contains("ffmpeg"),
            "can't see the running ffmpeg"
        );
        Some((child, dir))
    }

    #[cfg(unix)]
    fn has_part(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .unwrap()
            .any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".part"))
    }

    /// After a cancel: exit 130, no `.part`, no output, no ffmpeg left.
    #[cfg(unix)]
    fn assert_cancelled_cleanly(child: std::process::Child, dir: &Path) {
        let pid = child.id();
        let out = child.wait_with_output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(130), "{err}");
        assert!(!has_part(dir), "a .part was left");
        assert!(!dir.join("long.webm").exists());
        assert!(!err.contains("CORRUPT") && !err.contains("can't"), "{err}");
        assert_eq!(ffmpeg_of(pid), "", "ffmpeg still running");
    }

    /// A batch of short files: Ctrl-C often lands while ffprobe runs.
    #[cfg(unix)]
    #[test]
    fn terminal_ctrl_c_cancels_audio_batch() {
        use std::io::BufRead;
        use std::os::unix::process::CommandExt;
        let Some((dir, first)) = sample("audio-batch-ctrl-c", "aud-flac") else {
            return;
        };
        let mut args = vec!["convert".to_string(), "--to".into(), "wav".into()];
        for i in 0..60 {
            let name = format!("t{i:02}.flac");
            std::fs::copy(&first, dir.join(&name)).unwrap();
            args.push(name);
        }
        let mut child = Command::new(PRSM)
            .current_dir(&dir)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        Command::new("kill")
            .args(["-INT", &format!("-{}", child.id())])
            .status()
            .unwrap();
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut reader, &mut rest).unwrap();
        let out = child.wait_with_output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(130), "{err}");
        assert!(!err.contains("can't read"), "{err}");
        assert!(!has_part(&dir), "a .part was left");
    }

    #[cfg(unix)]
    #[test]
    fn terminal_ctrl_c_cancels_video() {
        let Some((child, dir)) = start_long_video("video-ctrl-c") else {
            return;
        };
        // A terminal's Ctrl-C goes to the whole process group: prsm and
        // FFmpeg both get it.
        Command::new("kill")
            .args(["-INT", &format!("-{}", child.id())])
            .status()
            .unwrap();
        assert_cancelled_cleanly(child, &dir);
    }

    #[cfg(unix)]
    #[test]
    fn sigterm_cancels_video() {
        let Some((child, dir)) = start_long_video("video-sigterm") else {
            return;
        };
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap();
        assert_cancelled_cleanly(child, &dir);
    }

    #[test]
    fn info_truncated_is_exit_5() {
        let Some((dir, _)) = sample("info-truncated", "vid-truncated") else {
            return;
        };
        let out = prsm(&dir, ["info", "truncated.mp4"]);
        assert_eq!(out.status.code(), Some(5), "{}", stderr(&out));
        assert!(stdout(&out).contains("format   MP4"), "{}", stdout(&out));
        assert!(
            stderr(&out).contains("ffmpeg can't read it"),
            "{}",
            stderr(&out)
        );
    }
}

#[test]
fn info_without_ffmpeg_hints() {
    let dir = std::env::temp_dir().join(format!("prsm-cli-info-noff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Just an ISO BMFF `ftyp` box: detected as MP4, never decoded.
    std::fs::write(
        dir.join("clip.mp4"),
        b"\x00\x00\x00\x18ftypisom\x00\x00\x02\x00isomiso2",
    )
    .unwrap();
    let out = prsm(
        &dir,
        ["info", "--ffmpeg", "/nonexistent/ffmpeg", "clip.mp4"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("format   MP4"), "{text}");
    assert!(text.lines().any(|l| l.starts_with("details  ")), "{text}");
}

#[test]
fn unknown_preset_is_usage() {
    let c = Corpus::new("preset-unknown");
    let out = prsm(
        &c.dir,
        [
            "convert".as_ref(),
            "--to".as_ref(),
            "mp4".as_ref(),
            "--preset".as_ref(),
            "fast".as_ref(),
            c.path("png-1x1").as_os_str(),
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn preset_dry_run_on_image_exits_2() {
    let c = Corpus::new("preset-image");
    let out = prsm(
        &c.dir,
        [
            "convert".as_ref(),
            "--to".as_ref(),
            "webp".as_ref(),
            "--preset".as_ref(),
            "small".as_ref(),
            "--dry-run".as_ref(),
            c.path("png-1x1").as_os_str(),
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("--preset is for mp4, mov and webm video"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn missing_ffmpeg_is_exit_4() {
    let dir = std::env::temp_dir().join(format!("prsm-cli-noff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("clip.mov"),
        b"\x00\x00\x00\x14ftypqt  \x00\x00\x00\x00qt  ",
    )
    .unwrap();
    let out = prsm(
        &dir,
        [
            "convert",
            "--ffmpeg",
            "/nonexistent/ffmpeg",
            "--to",
            "mp4",
            "clip.mov",
        ],
    );
    assert_eq!(out.status.code(), Some(4), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("clip.mov: no ffmpeg at /nonexistent/ffmpeg"),
        "{err}"
    );
    assert!(err.contains("fix:"), "{err}");
}
