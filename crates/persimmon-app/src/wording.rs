// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Engine errors in the app's own words: no CLI flags, no `prsm` commands,
//! no install commands (the app bundles its engines).

use std::path::PathBuf;

use persimmon_core::error::{Error, ErrorCode, Fix, FixKind};

/// An error as a row shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub code: ErrorCode,
    #[serde(serialize_with = "lossy")]
    pub file: Option<PathBuf>,
    pub reason: String,
    pub fix: Option<String>,
}

fn lossy<S: serde::Serializer>(p: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
    match p {
        Some(p) => s.serialize_str(&p.to_string_lossy()),
        None => s.serialize_none(),
    }
}

/// CLI words that must never reach a row: flags, the CLI's own name and
/// package managers (the app bundles its engines).
pub const BANNED: [&str; 5] = ["--", "prsm", "brew", "apt", "-dev"];

/// The first CLI word in `text`, if any: `--` anywhere; `prsm`, `brew` and
/// `apt` as whole words (so "adapt" is fine); `-dev` ending a word, as in a
/// package name (so "/dev/null" is fine).
pub fn cli_word(text: &str) -> Option<&'static str> {
    if text.contains(BANNED[0]) {
        return Some(BANNED[0]);
    }
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '-')) {
        if let Some(w) = BANNED[1..4].iter().find(|w| word == **w) {
            return Some(w);
        }
        if word.len() > 4 && word.ends_with(BANNED[4]) {
            return Some(BANNED[4]);
        }
    }
    None
}

/// An error the app itself finds, before the engine sees the file (a
/// Photos preview, a folder outside Combine).
pub fn app_error(
    code: ErrorCode,
    file: &std::path::Path,
    reason: &str,
    fix: Option<&str>,
) -> Shown {
    Shown {
        code,
        file: Some(file.to_path_buf()),
        reason: reason.into(),
        fix: fix.map(Into::into),
    }
}

/// What a row offers when the engine has no fix (an empty or damaged
/// file): every failed row has one (Phase 2 gate).
pub const FALLBACK_FIX: &str = "Remove this file from the queue.";

/// The app's wording for `e`.
pub fn shown(e: &Error) -> Shown {
    Shown {
        code: e.code,
        file: e.file.clone(),
        reason: reason(&e.reason),
        fix: Some(e.fix.as_ref().map_or_else(|| FALLBACK_FIX.into(), fix)),
    }
}

/// The few engine reasons that name a CLI flag or `prsm` itself.
fn reason(r: &str) -> String {
    let rewrites = [
        ("--pages is for PDFs", "Page ranges are for PDFs"),
        (
            "--preset is for mp4, mov and webm video",
            "Presets are for MP4, MOV and WebM video",
        ),
        (
            "--max-size is for images and video",
            "A size limit is for images and video",
        ),
    ];
    for (cli, app) in rewrites {
        if let Some(rest) = r.strip_prefix(cli) {
            return format!("{app}{rest}");
        }
    }
    // "ffmpeg 6.1 is too old; prsm needs 7.0 or later"
    if let Some(rest) = r.strip_prefix("ffmpeg ")
        && rest.contains(" is too old; prsm needs ")
    {
        return format!(
            "FFmpeg {}",
            rest.replace("; prsm needs ", "; Persimmon needs ")
        );
    }
    // The app is Persimmon, not prsm ("it uses encryption prsm can't read").
    r.replace("prsm", "Persimmon")
}

fn fix(f: &Fix) -> String {
    match &f.kind {
        FixKind::Plain => f.text.clone(),
        FixKind::ChooseOutputFolder => "Choose another output folder.".into(),
        FixKind::Overwrite => "Rename or move the existing file, then retry.".into(),
        FixKind::AddTarget => "Pick an output format.".into(),
        FixKind::DropOption { option } => match option.as_str() {
            "pages" => "Page ranges are for PDFs.".into(),
            "preset" => "Pick a preset made for MP4, MOV or WebM.".into(),
            _ => "Remove the size limit for this file.".into(),
        },
        FixKind::PageRange => "Choose pages that exist in this PDF.".into(),
        FixKind::PickAudioTarget => "Pick an audio format, such as M4A.".into(),
        FixKind::PickVideoTarget => "Pick a video format, such as MP4.".into(),
        FixKind::ShrinkOrPickTarget { max } => {
            format!("Set a size limit of {max} px or less, or pick PNG or JPEG.")
        }
        FixKind::UseCompress => {
            "To resize or remove metadata in the same format, use Compress.".into()
        }
        FixKind::SeeFormats => "Pick one of the formats this file can become.".into(),
        FixKind::InstallFfmpeg => {
            "Persimmon's FFmpeg is missing or damaged. Reinstall Persimmon.".into()
        }
        FixKind::InstallLibvips => {
            "Persimmon's image engine is missing or damaged. Reinstall Persimmon.".into()
        }
        FixKind::InstallLibarchive => {
            "Persimmon's RAR reader is missing or damaged. Reinstall Persimmon.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_words_are_whole_words() {
        for text in [
            "use --out",
            "run prsm compress",
            "brew install ffmpeg",
            "sudo apt install x",
            "install libvips-dev",
        ] {
            assert!(cli_word(text).is_some(), "{text}");
        }
        for text in [
            "adapt the chapter",
            "Persimmon can't read it",
            "a brewery",
            "/dev/null is fine",
        ] {
            assert_eq!(cli_word(text), None, "{text}");
        }
    }

    #[test]
    fn an_error_with_no_fix_gets_the_fallback() {
        let s = shown(&Error::new(ErrorCode::CorruptInput, "it's damaged"));
        assert_eq!(s.fix.as_deref(), Some("Remove this file from the queue."));
    }

    fn fix(kind: FixKind, cli: &str) -> Option<String> {
        shown(&Error::new(ErrorCode::Usage, "x").with_fix_as(kind, cli)).fix
    }

    #[test]
    fn every_fix_kind_has_app_words() {
        let plain = Error::new(ErrorCode::CorruptInput, "x")
            .with_fix("extract it with the app that made it");
        assert_eq!(
            shown(&plain).fix.as_deref(),
            Some("extract it with the app that made it")
        );
        let cases = [
            (FixKind::ChooseOutputFolder, "Choose another output folder."),
            (
                FixKind::Overwrite,
                "Rename or move the existing file, then retry.",
            ),
            (FixKind::AddTarget, "Pick an output format."),
            (
                FixKind::DropOption {
                    option: "pages".into(),
                },
                "Page ranges are for PDFs.",
            ),
            (
                FixKind::DropOption {
                    option: "preset".into(),
                },
                "Pick a preset made for MP4, MOV or WebM.",
            ),
            (
                FixKind::DropOption {
                    option: "max-size".into(),
                },
                "Remove the size limit for this file.",
            ),
            (FixKind::PageRange, "Choose pages that exist in this PDF."),
            (
                FixKind::PickAudioTarget,
                "Pick an audio format, such as M4A.",
            ),
            (
                FixKind::PickVideoTarget,
                "Pick a video format, such as MP4.",
            ),
            (
                FixKind::ShrinkOrPickTarget { max: 16383 },
                "Set a size limit of 16383 px or less, or pick PNG or JPEG.",
            ),
            (
                FixKind::UseCompress,
                "To resize or remove metadata in the same format, use Compress.",
            ),
            (
                FixKind::SeeFormats,
                "Pick one of the formats this file can become.",
            ),
            (
                FixKind::InstallFfmpeg,
                "Persimmon's FFmpeg is missing or damaged. Reinstall Persimmon.",
            ),
            (
                FixKind::InstallLibvips,
                "Persimmon's image engine is missing or damaged. Reinstall Persimmon.",
            ),
            (
                FixKind::InstallLibarchive,
                "Persimmon's RAR reader is missing or damaged. Reinstall Persimmon.",
            ),
        ];
        for (kind, want) in cases {
            assert_eq!(
                fix(kind.clone(), "cli words").as_deref(),
                Some(want),
                "{kind:?}"
            );
        }
        assert_eq!(
            shown(&Error::new(ErrorCode::Usage, "x")).fix.as_deref(),
            Some(FALLBACK_FIX)
        );
    }

    #[test]
    fn cli_worded_reasons_are_rewritten() {
        let r = |s: &str| shown(&Error::new(ErrorCode::Usage, s)).reason;
        assert_eq!(r("--pages is for PDFs"), "Page ranges are for PDFs");
        assert_eq!(
            r("--preset is for mp4, mov and webm video"),
            "Presets are for MP4, MOV and WebM video"
        );
        assert_eq!(
            r("--max-size is for images and video"),
            "A size limit is for images and video"
        );
        assert_eq!(
            r("ffmpeg 6.1 is too old; prsm needs 7.0 or later"),
            "FFmpeg 6.1 is too old; Persimmon needs 7.0 or later"
        );
        assert_eq!(r("it has 3 pages"), "it has 3 pages");
        assert_eq!(
            r("it uses encryption prsm can't read"),
            "it uses encryption Persimmon can't read"
        );
    }

    /// Every reason literal in the engine that names a flag or `prsm`
    /// reads cleanly once the app words it: new ones fail here.
    #[test]
    fn no_engine_reason_reaches_the_window_with_cli_words() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../persimmon-core/src");
        let mut files = vec![src];
        let mut checked = 0;
        while let Some(f) = files.pop() {
            if f.is_dir() {
                files.extend(std::fs::read_dir(&f).unwrap().map(|e| e.unwrap().path()));
                continue;
            }
            let text = std::fs::read_to_string(&f).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let Some(at) = line.find("Error::new(") else {
                    continue;
                };
                // The reason is the first string after `Error::new(`.
                let window = lines[i..lines.len().min(i + 3)].join(" ")[at..].to_string();
                let Some(start) = window.find('"') else {
                    continue;
                };
                let Some(len) = window[start + 1..].find('"') else {
                    continue;
                };
                let lit = &window[start + 1..start + 1 + len];
                if lit.contains("--") || lit.contains("prsm") {
                    checked += 1;
                    let shown = reason(lit);
                    assert!(
                        !shown.contains("--") && !shown.contains("prsm"),
                        "{}: {lit:?} shows as {shown:?}",
                        f.display()
                    );
                }
            }
        }
        assert!(checked >= 4, "only {checked} CLI-worded reasons found");
    }

    #[test]
    fn shown_serialises_like_the_engine_error() {
        let e = Error::new(ErrorCode::InputNotFound, "no such file").with_file("/a.png");
        assert_eq!(
            serde_json::to_value(shown(&e)).unwrap(),
            serde_json::json!({"code": "INPUT_NOT_FOUND", "file": "/a.png", "reason": "no such file", "fix": FALLBACK_FIX})
        );
    }

    mod corpus {
        use super::super::{cli_word, shown};
        use persimmon_core::backend::Backends;
        use persimmon_core::detect::detect;
        use persimmon_core::error::Error;
        use persimmon_core::exec::{CancelToken, execute};
        use persimmon_core::format::{FormatId, MediaKind};
        use persimmon_core::job::{Job, JobOptions};
        use persimmon_core::output::OutputPolicy;
        use persimmon_core::registry::{Operation, Registry};
        use persimmon_corpus::archives::{ARCHIVE_SAMPLES, write_archive_sample};
        use persimmon_corpus::media::{MEDIA_SAMPLES, media_sample};
        use persimmon_corpus::pdf::{PDF_SAMPLES, write_pdf_sample};
        use persimmon_corpus::{IMAGE_SAMPLES, Options, RAR_SAMPLES, generate};
        use std::path::{Path, PathBuf};

        /// The natural job for a file, as the CLI's corpus sweep picks it.
        fn natural(reg: &Registry, path: &Path, out: &Path) -> Option<Job> {
            let detected = detect(path, reg.formats()).ok().map(|d| d.format);
            let (op, target) = match detected {
                Some(f) if reg.format(f).kind == MediaKind::Archive => (Operation::Extract, None),
                Some(FormatId::Pdf) | None => (Operation::Convert, Some(FormatId::Png)),
                Some(f) => (
                    Operation::Convert,
                    Some(*reg.targets(Operation::Convert, f).first()?),
                ),
            };
            Some(Job {
                op,
                inputs: vec![path.to_path_buf()],
                target,
                output: OutputPolicy {
                    dir: Some(out.to_path_buf()),
                    overwrite: false,
                    fallback: None,
                },
                options: JobOptions::default(),
            })
        }

        fn errors_of(reg: &Registry, backends: &Backends, path: &Path, out: &Path) -> Vec<Error> {
            let Some(job) = natural(reg, path, out) else {
                return Vec::new();
            };
            let plan = job.plan(reg);
            let mut errors = plan.errors.clone();
            let results = execute(&job, &plan, backends, &CancelToken::new(), &mut |_| {});
            errors.extend(results.into_iter().filter_map(Result::err));
            errors
        }

        #[test]
        fn corpus_failures_read_in_app_words() {
            let _serial = crate::FFMPEG_TESTS
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let reg = Registry::builtin();
            let backends = Backends::new();
            let dir = std::env::temp_dir().join(format!("prsm-app-wording-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let out = dir.join("out");
            let mut files: Vec<PathBuf> = generate(&dir, &Options { large: false })
                .unwrap()
                .into_iter()
                .map(|c| c.path)
                .filter(|p| p.is_file())
                .filter(|p| !["many.pdf", "big-page.pdf"].iter().any(|n| p.ends_with(n)))
                .collect();
            let samples = dir.join("samples");
            std::fs::create_dir_all(&samples).unwrap();
            for (_, name, bytes, _) in IMAGE_SAMPLES {
                std::fs::write(samples.join(name), bytes).unwrap();
                files.push(samples.join(name));
            }
            for (_, name, bytes) in RAR_SAMPLES {
                std::fs::write(samples.join(name), bytes).unwrap();
                files.push(samples.join(name));
            }
            for (id, _, _) in PDF_SAMPLES {
                if !["many", "big-page"].contains(&id) {
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
            let mut seen = 0;
            for f in &files {
                for e in errors_of(&reg, &backends, f, &out) {
                    seen += 1;
                    let s = shown(&e);
                    for text in [Some(&s.reason), s.fix.as_ref()].into_iter().flatten() {
                        assert_eq!(
                            cli_word(text),
                            None,
                            "{}: {text:?} has a CLI word (engine: {e})",
                            f.display()
                        );
                    }
                }
            }
            // 18 errors on 2026-11-12: a floor, so a sweep that stops
            // reaching the failures can't pass by checking nothing.
            assert!(
                seen >= 10,
                "only {seen} errors: the sweep isn't reaching the failures"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
