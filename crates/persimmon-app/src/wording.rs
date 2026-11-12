// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Engine errors in the app's own words: no CLI flags, no `prsm` commands,
//! no install commands (the app bundles its engines).

use std::path::PathBuf;

use persimmon_core::error::{Error, ErrorCode, Fix, FixKind};

/// An error as a row shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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

/// The app's wording for `e`.
pub fn shown(e: &Error) -> Shown {
    Shown {
        code: e.code,
        file: e.file.clone(),
        reason: reason(&e.reason),
        fix: e.fix.as_ref().map(fix),
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
    r.to_string()
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
        assert_eq!(shown(&Error::new(ErrorCode::Usage, "x")).fix, None);
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
    }

    #[test]
    fn shown_serialises_like_the_engine_error() {
        let e = Error::new(ErrorCode::InputNotFound, "no such file").with_file("/a.png");
        assert_eq!(
            serde_json::to_value(shown(&e)).unwrap(),
            serde_json::json!({"code": "INPUT_NOT_FOUND", "file": "/a.png", "reason": "no such file", "fix": null})
        );
    }

    mod corpus {
        use super::super::shown;
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
                        for cli in ["--", "prsm "] {
                            assert!(
                                !text.contains(cli),
                                "{}: {text:?} has {cli:?} (engine: {e})",
                                f.display()
                            );
                        }
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
