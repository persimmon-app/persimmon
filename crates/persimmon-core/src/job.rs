// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Jobs: inputs → operation → output policy. A job is planned before
//! anything runs, so mistakes surface up front (unknown formats, impossible
//! conversions, clashing outputs), and planning never writes an output.

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::backend::pdf::pdf_info;
use crate::detect::detect;
use crate::error::{Error, ErrorCode, FixKind};
use crate::format::{FormatId, format_for_name};
use crate::output::{OutputPolicy, free_name, reserve};
use crate::registry::{Backend, Operation, Registry};

/// `--quality`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Low,
    Medium,
    High,
    Max,
}

impl Quality {
    /// The JPEG quality (1–100) for this level, shared by every backend
    /// that writes JPEG.
    pub fn jpeg(self) -> u8 {
        match self {
            Quality::Low => 60,
            Quality::Medium => 75,
            Quality::High => 85,
            Quality::Max => 95,
        }
    }
}

/// `--preset`: how video is encoded, per container (DD-35).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// H.264 (MP4, MOV) or VP9 (WebM), 8-bit 4:2:0: plays anywhere.
    Compatible,
    /// HEVC (MP4, MOV) or AV1 (WebM): smaller files, slower to make.
    Small,
}

/// `--pages`: 1-based, inclusive ranges. `(7, None)` is "7 to the end".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRanges(pub Vec<(u32, Option<u32>)>);

impl PageRanges {
    /// Parse `"1-3,5,7-"`.
    pub fn parse(s: &str) -> Result<PageRanges, Error> {
        let bad = || {
            Error::new(ErrorCode::Usage, format!("\"{s}\" isn't a page range")).with_fix_as(
                FixKind::PageRange,
                "use page numbers from 1, e.g. --pages 1-3,5,7-",
            )
        };
        let page = |t: &str| t.trim().parse::<u32>().ok().filter(|&n| n >= 1);
        let mut ranges = Vec::new();
        for part in s.split(',') {
            let range = match part.split_once('-') {
                Some((a, b)) => {
                    let a = page(a).ok_or_else(bad)?;
                    if b.trim().is_empty() {
                        (a, None)
                    } else {
                        let b = page(b).ok_or_else(bad)?;
                        if b < a {
                            return Err(bad());
                        }
                        (a, Some(b))
                    }
                }
                None => {
                    let n = page(part).ok_or_else(bad)?;
                    (n, Some(n))
                }
            };
            ranges.push(range);
        }
        Ok(PageRanges(ranges))
    }
}

/// Which pages of a `count`-page PDF `--pages` picks: all of them without
/// ranges, else the pages named, in document order and without repeats. A
/// page past the end is a `USAGE` error; the caller attaches the file.
pub fn select_pages(ranges: Option<&PageRanges>, count: u32) -> Result<Vec<u32>, Error> {
    let Some(ranges) = ranges else {
        return Ok((1..=count).collect());
    };
    let mut pages = BTreeSet::new();
    for &(first, last) in &ranges.0 {
        let last = last.unwrap_or(count);
        if first > count || last > count {
            let noun = if count == 1 { "page" } else { "pages" };
            return Err(
                Error::new(ErrorCode::Usage, format!("it has {count} {noun}")).with_fix_as(
                    FixKind::PageRange,
                    format!("use --pages between 1 and {count}"),
                ),
            );
        }
        pages.extend(first..=last);
    }
    Ok(pages.into_iter().collect())
}

impl fmt::Display for PageRanges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, &(a, b)) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            match b {
                Some(b) if b == a => write!(f, "{a}")?,
                Some(b) => write!(f, "{a}-{b}")?,
                None => write!(f, "{a}-")?,
            }
        }
        Ok(())
    }
}

/// Options that tune a job without changing what it does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobOptions {
    pub quality: Option<Quality>,
    pub preset: Option<Preset>,
    pub pages: Option<PageRanges>,
    /// `--max-size`: shrink so the long edge is at most this many pixels.
    pub max_size: Option<u32>,
    /// `--strip-metadata`: drop EXIF, XMP, tags and colour profiles
    /// (images convert to sRGB first).
    pub strip_metadata: bool,
    /// `--ffmpeg`: the FFmpeg to use instead of the one on `PATH`.
    pub ffmpeg: Option<PathBuf>,
}

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub op: Operation,
    pub inputs: Vec<PathBuf>,
    pub target: Option<FormatId>,
    pub output: OutputPolicy,
    pub options: JobOptions,
}

/// One unit of work, ready for a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTask {
    pub inputs: Vec<PathBuf>,
    /// The detected format of each input, in order. Empty for merging into
    /// an archive, which takes files and folders without detection.
    pub from: Vec<FormatId>,
    pub to: Option<FormatId>,
    pub backend: Backend,
    pub output: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Task(usize),
    Error(usize),
}

/// The outcome of planning: the tasks that can run, and the errors that
/// stopped the others.
#[derive(Debug, Clone)]
pub struct Plan {
    pub tasks: Vec<PlannedTask>,
    pub errors: Vec<Error>,
    order: Vec<Slot>,
}

impl Plan {
    fn new() -> Plan {
        Plan {
            tasks: Vec::new(),
            errors: Vec::new(),
            order: Vec::new(),
        }
    }

    fn task(&mut self, t: PlannedTask) {
        self.order.push(Slot::Task(self.tasks.len()));
        self.tasks.push(t);
    }

    fn error(&mut self, e: Error) {
        self.order.push(Slot::Error(self.errors.len()));
        self.errors.push(e);
    }

    /// Tasks and errors interleaved in input order: what a batch's output
    /// and exit code follow.
    pub fn outcomes(&self) -> impl Iterator<Item = Result<&PlannedTask, &Error>> {
        self.order.iter().map(|slot| match *slot {
            Slot::Task(i) => Ok(&self.tasks[i]),
            Slot::Error(i) => Err(&self.errors[i]),
        })
    }
}

impl Job {
    /// Plan the job: detect every input, check the capability graph, and
    /// choose output names. Writes nothing but the output folder (`--out`
    /// is created if missing) and the writability probe.
    pub fn plan(&self, reg: &Registry) -> Plan {
        let inputs = dedupe(&self.inputs);
        let mut taken = HashSet::new();
        for input in &inputs {
            reserve(&mut taken, input);
        }
        let mut plan = Plan::new();
        match self.op {
            Operation::Convert | Operation::Compress | Operation::Extract => {
                if self.op == Operation::Convert && self.target.is_none() {
                    plan.error(
                        Error::new(ErrorCode::Usage, "no target format")
                            .with_fix_as(FixKind::AddTarget, "add --to <format>"),
                    );
                    return plan;
                }
                for input in &inputs {
                    match self.plan_one(reg, input, &mut taken) {
                        Ok(task) => plan.task(task),
                        Err(e) => plan.error(e),
                    }
                }
            }
            Operation::Merge => match self.plan_merge(reg, &inputs, &mut taken) {
                Ok(task) => plan.task(task),
                Err(errors) => errors.into_iter().for_each(|e| plan.error(e)),
            },
            Operation::Split => match self.plan_split(reg, &inputs, &mut taken) {
                Ok(task) => plan.task(task),
                Err(e) => plan.error(e),
            },
        }
        plan
    }

    fn plan_one(
        &self,
        reg: &Registry,
        input: &Path,
        taken: &mut HashSet<PathBuf>,
    ) -> Result<PlannedTask, Error> {
        let from = detect(input, reg.formats())?.format;
        let to = match self.op {
            Operation::Convert => self.target,
            _ => None,
        };
        let cap = reg
            .find(self.op, from, to)
            .ok_or_else(|| unsupported(reg, self.op, from, to).with_file(input))?;
        self.check_options(cap.backend, from, to.unwrap_or(from))
            .map_err(|e| e.with_file(input))?;
        // Rendering a PDF writes one file per page: planning counts them.
        let render_pages = match (self.op, from) {
            (Operation::Convert, FormatId::Pdf) => {
                let info = pdf_info(input)?;
                Some(
                    select_pages(self.options.pages.as_ref(), info.pages)
                        .map_err(|e| e.with_file(input))?,
                )
            }
            _ => None,
        };
        let dir = self.output.output_dir(input)?;
        let stem = stem_of(input, reg);
        let ow = self.output.overwrite;
        let output = match self.op {
            Operation::Convert if let Some(pages) = render_pages => {
                let ext = reg.format(to.expect("convert has a target")).extensions[0];
                let ranges = self.options.pages.as_ref();
                match (pages.as_slice(), ranges) {
                    ([_], None) => free_name(&dir, &stem, ext, ow, taken),
                    ([n], Some(_)) => {
                        free_name(&dir, &format!("{stem} (page {n})"), ext, ow, taken)
                    }
                    // Several pages make a folder, which is never replaced.
                    (_, None) => free_name(&dir, &stem, "", false, taken),
                    (_, Some(r)) => {
                        free_name(&dir, &format!("{stem} (pages {r})"), "", false, taken)
                    }
                }
            }
            Operation::Convert => {
                let ext = reg.format(to.expect("convert has a target")).extensions[0];
                free_name(&dir, &stem, ext, ow, taken)
            }
            Operation::Compress => {
                // Keep the source's own spelling: `scan.tif` stays `.tif`.
                let exts = reg.format(from).extensions;
                let own = input
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(|e| exts.iter().find(|x| x.eq_ignore_ascii_case(e)));
                let ext = own.copied().unwrap_or(exts[0]);
                free_name(&dir, &format!("{stem} (compressed)"), ext, ow, taken)
            }
            // An extracted folder is never replaced (DD-37).
            _ => free_name(&dir, &stem, "", false, taken),
        };
        Ok(PlannedTask {
            inputs: vec![input.to_path_buf()],
            from: vec![from],
            to,
            backend: cap.backend,
            output,
        })
    }

    /// Options that make no sense for this task's output are usage errors,
    /// found before anything runs.
    fn check_options(&self, backend: Backend, from: FormatId, out: FormatId) -> Result<(), Error> {
        use FormatId::*;
        let renders_pdf = self.op == Operation::Convert && from == Pdf && matches!(out, Png | Jpeg);
        if self.options.pages.is_some() && !renders_pdf && self.op != Operation::Split {
            return Err(
                Error::new(ErrorCode::Usage, "--pages is for PDFs").with_fix_as(
                    FixKind::DropOption {
                        option: "pages".into(),
                    },
                    "drop --pages",
                ),
            );
        }
        let video = backend == Backend::Ffmpeg && matches!(out, Mp4 | Mov | Webm);
        if self.options.preset.is_some() && !video {
            return Err(
                Error::new(ErrorCode::Usage, "--preset is for mp4, mov and webm video")
                    .with_fix_as(
                        FixKind::DropOption {
                            option: "preset".into(),
                        },
                        "drop --preset",
                    ),
            );
        }
        let audio = backend == Backend::Ffmpeg && matches!(out, Mp3 | M4a | Wav | Flac | Opus);
        if self.options.max_size.is_some() && audio {
            return Err(
                Error::new(ErrorCode::Usage, "--max-size is for images and video").with_fix_as(
                    FixKind::DropOption {
                        option: "max-size".into(),
                    },
                    "drop --max-size",
                ),
            );
        }
        Ok(())
    }

    fn plan_merge(
        &self,
        reg: &Registry,
        inputs: &[PathBuf],
        taken: &mut HashSet<PathBuf>,
    ) -> Result<PlannedTask, Vec<Error>> {
        let target = match self.target {
            Some(t @ (FormatId::Pdf | FormatId::Zip | FormatId::TarGz)) => t,
            other => {
                let name = other.map_or("that format", |t| reg.format(t).name);
                return Err(vec![
                    Error::new(
                        ErrorCode::UnsupportedConversion,
                        format!("can't merge files into {name}"),
                    )
                    .with_fix("merge makes pdf, zip or tar.gz"),
                ]);
            }
        };
        let mut errors = Vec::new();
        let mut from = Vec::new();
        let mut backend = Backend::Archive;
        for input in inputs {
            if target == FormatId::Pdf {
                match detect(input, reg.formats()) {
                    Ok(d) => match reg.find(Operation::Merge, d.format, Some(target)) {
                        // A PDF is opened now, so a locked or broken one
                        // fails before anything runs.
                        Some(cap) => match (d.format == FormatId::Pdf).then(|| pdf_info(input)) {
                            Some(Err(e)) => errors.push(e),
                            _ => {
                                backend = cap.backend;
                                from.push(d.format);
                            }
                        },
                        None => errors.push(
                            unsupported(reg, Operation::Merge, d.format, Some(target))
                                .with_file(input),
                        ),
                    },
                    Err(e) => errors.push(e),
                }
            } else if std::fs::symlink_metadata(input).is_err() {
                errors.push(Error::new(ErrorCode::InputNotFound, "no such file").with_file(input));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let Some(first) = inputs.first() else {
            return Err(vec![Error::new(ErrorCode::Usage, "no files given")]);
        };
        let dir = self.output.output_dir(first).map_err(|e| vec![e])?;
        let ext = reg.format(target).extensions[0];
        Ok(PlannedTask {
            inputs: inputs.to_vec(),
            from,
            to: Some(target),
            backend,
            output: free_name(&dir, "merged", ext, self.output.overwrite, taken),
        })
    }

    fn plan_split(
        &self,
        reg: &Registry,
        inputs: &[PathBuf],
        taken: &mut HashSet<PathBuf>,
    ) -> Result<PlannedTask, Error> {
        let Some(pages) = &self.options.pages else {
            return Err(Error::new(ErrorCode::Usage, "no pages given")
                .with_fix_as(FixKind::PageRange, "add --pages, e.g. --pages 1-3,5"));
        };
        let [input] = inputs else {
            return Err(Error::new(ErrorCode::Usage, "split takes exactly one PDF"));
        };
        let from = detect(input, reg.formats())?.format;
        let cap = reg
            .find(Operation::Split, from, Some(FormatId::Pdf))
            .ok_or_else(|| {
                unsupported(reg, Operation::Split, from, Some(FormatId::Pdf)).with_file(input)
            })?;
        let info = pdf_info(input)?;
        select_pages(Some(pages), info.pages).map_err(|e| e.with_file(input))?;
        let dir = self.output.output_dir(input)?;
        let stem = format!("{} (pages {pages})", stem_of(input, reg));
        Ok(PlannedTask {
            inputs: vec![input.clone()],
            from: vec![from],
            to: Some(FormatId::Pdf),
            backend: cap.backend,
            output: free_name(&dir, &stem, "pdf", self.output.overwrite, taken),
        })
    }
}

/// Drop repeated inputs, comparing canonical paths where they resolve, and
/// keep first-seen order.
fn dedupe(inputs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    inputs
        .iter()
        .filter(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| (*p).clone())))
        .cloned()
        .collect()
}

/// The file name without its format extension: `photos.tar.gz` → `photos`,
/// `v1.2.zip` → `v1.2`. Unknown extensions lose only the last one.
pub(crate) fn stem_of(path: &Path, reg: &Registry) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(id) = format_for_name(&name, reg.formats()) {
        let lower = name.to_lowercase();
        if let Some(ext) = reg
            .format(id)
            .extensions
            .iter()
            .filter(|e| lower.ends_with(&format!(".{e}")))
            .max_by_key(|e| e.len())
        {
            return name[..name.len() - ext.len() - 1].to_string();
        }
    }
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(name)
}

fn unsupported(reg: &Registry, op: Operation, from: FormatId, to: Option<FormatId>) -> Error {
    let from_name = reg.format(from).name;
    let reason = match (op, to) {
        (Operation::Convert, Some(to)) => {
            format!("can't convert {from_name} to {}", reg.format(to).name)
        }
        (Operation::Merge, Some(to)) => {
            format!("can't merge {from_name} into {}", reg.format(to).name)
        }
        (op, _) => format!("can't {} {from_name} files", op.as_str()),
    };
    // Point at compress only where compress takes the format.
    let (kind, fix) = if op == Operation::Convert
        && to == Some(from)
        && reg.find(Operation::Compress, from, None).is_some()
    {
        (
            FixKind::UseCompress,
            "to resize or strip metadata in the same format, use prsm compress".to_string(),
        )
    } else {
        (
            FixKind::SeeFormats,
            format!("run prsm formats to see what {} can become", from.as_str()),
        )
    };
    Error::new(ErrorCode::UnsupportedConversion, reason).with_fix_as(kind, fix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use persimmon_corpus::pdf::write_pdf_sample;
    use persimmon_corpus::{GIF_1X1, PNG_1X1, WAV_HEADER, ZIP_EMPTY, minimal_pdf};

    const MOV_HEAD: &[u8] = b"\x00\x00\x00\x14ftypqt  \x00\x00\x00\x00qt  ";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-job-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn job(op: Operation, inputs: Vec<PathBuf>, target: Option<FormatId>) -> Job {
        Job {
            op,
            inputs,
            target,
            output: OutputPolicy::default(),
            options: JobOptions::default(),
        }
    }

    fn outputs(plan: &Plan) -> Vec<PathBuf> {
        plan.tasks.iter().map(|t| t.output.clone()).collect()
    }

    #[test]
    fn convert_plans_next_to_source() {
        let dir = scratch("convert");
        let a = put(&dir, "a.png", &PNG_1X1);
        let plan = job(Operation::Convert, vec![a.clone()], Some(FormatId::Webp))
            .plan(&Registry::builtin());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        let t = &plan.tasks[0];
        assert_eq!(t.inputs, vec![a]);
        assert_eq!(t.from, vec![FormatId::Png]);
        assert_eq!(t.to, Some(FormatId::Webp));
        assert_eq!(t.backend, Backend::Image);
        assert_eq!(t.output, dir.join("a.webp"));
    }

    #[test]
    fn batch_clash_gets_counter() {
        let dir = scratch("clash");
        let a = put(&dir, "a.png", &PNG_1X1);
        let b = put(&dir, "a.gif", GIF_1X1);
        let plan =
            job(Operation::Convert, vec![a, b], Some(FormatId::Webp)).plan(&Registry::builtin());
        assert_eq!(
            outputs(&plan),
            vec![dir.join("a.webp"), dir.join("a (1).webp")]
        );
    }

    #[test]
    fn batch_clash_ignores_case() {
        let dir = scratch("clash-case");
        let a = put(&dir, "Snap.png", &PNG_1X1);
        let b = put(&dir, "snap.gif", GIF_1X1);
        let plan =
            job(Operation::Convert, vec![a, b], Some(FormatId::Webp)).plan(&Registry::builtin());
        assert_eq!(
            outputs(&plan),
            vec![dir.join("Snap.webp"), dir.join("snap (1).webp")]
        );
    }

    #[test]
    fn output_never_replaces_an_input() {
        let dir = scratch("self");
        let p = put(&dir, "photo.jpg", &PNG_1X1);
        let mut j = job(Operation::Convert, vec![p], Some(FormatId::Jpeg));
        j.output.overwrite = true;
        assert_eq!(
            outputs(&j.plan(&Registry::builtin())),
            vec![dir.join("photo (1).jpg")]
        );
    }

    #[test]
    fn merge_output_never_replaces_an_input() {
        let dir = scratch("merge-self");
        let a = put(&dir, "merged.pdf", &minimal_pdf());
        let b = put(&dir, "b.pdf", &minimal_pdf());
        let mut j = job(Operation::Merge, vec![a, b], Some(FormatId::Pdf));
        j.output.overwrite = true;
        assert_eq!(
            outputs(&j.plan(&Registry::builtin())),
            vec![dir.join("merged (1).pdf")]
        );
    }

    #[test]
    fn duplicate_input_planned_once() {
        let dir = scratch("dupe");
        let a = put(&dir, "a.png", &PNG_1X1);
        let again = dir.join(".").join("a.png");
        let plan = job(
            Operation::Convert,
            vec![a.clone(), a, again],
            Some(FormatId::Jpeg),
        )
        .plan(&Registry::builtin());
        assert_eq!(plan.tasks.len(), 1);
        assert!(plan.errors.is_empty());
    }

    #[test]
    fn bad_input_does_not_block_others() {
        let dir = scratch("bad");
        let good = put(&dir, "good.png", &PNG_1X1);
        let empty = put(&dir, "empty.png", b"");
        let plan = job(Operation::Convert, vec![good, empty], Some(FormatId::Jpeg))
            .plan(&Registry::builtin());
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.errors.len(), 1);
        assert_eq!(plan.errors[0].code, ErrorCode::CorruptInput);
    }

    #[test]
    fn outcomes_follow_input_order() {
        let dir = scratch("order");
        let empty = put(&dir, "empty.png", b"");
        let good = put(&dir, "good.png", &PNG_1X1);
        let missing = dir.join("missing.png");
        let plan = job(
            Operation::Convert,
            vec![good, empty, missing],
            Some(FormatId::Jpeg),
        )
        .plan(&Registry::builtin());
        let kinds: Vec<Result<(), ErrorCode>> = plan
            .outcomes()
            .map(|o| o.map(|_| ()).map_err(|e| e.code))
            .collect();
        assert_eq!(
            kinds,
            vec![
                Ok(()),
                Err(ErrorCode::CorruptInput),
                Err(ErrorCode::InputNotFound)
            ]
        );
    }

    #[test]
    fn unsupported_conversion_names_fix() {
        let dir = scratch("unsupported");
        let a = put(&dir, "a.png", &PNG_1X1);
        let plan = job(Operation::Convert, vec![a], Some(FormatId::Mp3)).plan(&Registry::builtin());
        let e = &plan.errors[0];
        assert_eq!(e.code, ErrorCode::UnsupportedConversion);
        assert_eq!(e.reason, "can't convert PNG to MP3");
        assert_eq!(
            e.fix_text(),
            Some("run prsm formats to see what png can become")
        );
    }

    #[test]
    fn compress_suffix() {
        let dir = scratch("compress");
        let p = put(&dir, "photo.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]);
        let plan = job(Operation::Compress, vec![p], None).plan(&Registry::builtin());
        assert_eq!(outputs(&plan), vec![dir.join("photo (compressed).jpg")]);
    }

    #[test]
    fn compress_keeps_tif_extension() {
        let dir = scratch("compress-tif");
        let tif = put(&dir, "scan.tif", b"II*\0");
        let tiff = put(&dir, "other.tiff", b"II*\0");
        let plan = job(Operation::Compress, vec![tif, tiff], None).plan(&Registry::builtin());
        assert_eq!(
            outputs(&plan),
            vec![
                dir.join("scan (compressed).tif"),
                dir.join("other (compressed).tiff")
            ]
        );
    }

    #[test]
    fn same_format_convert_points_at_compress() {
        let dir = scratch("same-format");
        let p = put(&dir, "photo.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]);
        let plan =
            job(Operation::Convert, vec![p], Some(FormatId::Jpeg)).plan(&Registry::builtin());
        let e = &plan.errors[0];
        assert_eq!(e.code, ErrorCode::UnsupportedConversion);
        assert_eq!(e.reason, "can't convert JPEG to JPEG");
        assert_eq!(
            e.fix_text(),
            Some("to resize or strip metadata in the same format, use prsm compress")
        );
        // Compress can't take a PDF: no pointing the user at another error.
        let pdf = put(&dir, "doc.pdf", &minimal_pdf());
        let plan =
            job(Operation::Convert, vec![pdf], Some(FormatId::Pdf)).plan(&Registry::builtin());
        assert_eq!(
            plan.errors[0].fix_text(),
            Some("run prsm formats to see what pdf can become")
        );
    }

    #[test]
    fn extract_strips_double_ext() {
        let dir = scratch("extract");
        let p = put(&dir, "photos.tar.gz", &[0x1F, 0x8B, 0x08, 0x00]);
        let plan = job(Operation::Extract, vec![p], None).plan(&Registry::builtin());
        assert_eq!(outputs(&plan), vec![dir.join("photos")]);
        assert_eq!(plan.tasks[0].backend, Backend::Archive);
    }

    #[test]
    fn extract_keeps_dots_in_the_stem() {
        let dir = scratch("extract-dots");
        let p = put(&dir, "v1.2.zip", &ZIP_EMPTY);
        let plan = job(Operation::Extract, vec![p], None).plan(&Registry::builtin());
        assert_eq!(outputs(&plan), vec![dir.join("v1.2")]);
    }

    #[test]
    fn merge_to_pdf_is_one_task() {
        let dir = scratch("merge");
        let a = put(&dir, "a.png", &PNG_1X1);
        let b = put(&dir, "b.gif", GIF_1X1);
        let plan =
            job(Operation::Merge, vec![a, b], Some(FormatId::Pdf)).plan(&Registry::builtin());
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].from, vec![FormatId::Png, FormatId::Gif]);
        assert_eq!(plan.tasks[0].output, dir.join("merged.pdf"));
    }

    #[test]
    fn merge_any_error_plans_nothing() {
        let dir = scratch("merge-bad");
        let a = put(&dir, "a.png", &PNG_1X1);
        let empty = put(&dir, "empty.png", b"");
        let plan =
            job(Operation::Merge, vec![a, empty], Some(FormatId::Pdf)).plan(&Registry::builtin());
        assert!(plan.tasks.is_empty());
        assert_eq!(plan.errors.len(), 1);
    }

    #[test]
    fn merge_to_zip_takes_folders_without_detection() {
        let dir = scratch("merge-zip");
        let folder = dir.join("album");
        std::fs::create_dir(&folder).unwrap();
        let notes = put(&dir, "notes.xyz", b"text");
        let plan = job(Operation::Merge, vec![folder, notes], Some(FormatId::TarGz))
            .plan(&Registry::builtin());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        assert_eq!(plan.tasks[0].backend, Backend::Archive);
        assert!(plan.tasks[0].from.is_empty());
        assert_eq!(plan.tasks[0].output, dir.join("merged.tar.gz"));
    }

    #[test]
    fn split_needs_pages() {
        let dir = scratch("split");
        let p = put(&dir, "doc.pdf", &minimal_pdf());
        let plan = job(Operation::Split, vec![p], Some(FormatId::Pdf)).plan(&Registry::builtin());
        assert!(plan.tasks.is_empty());
        assert_eq!(plan.errors[0].code, ErrorCode::Usage);
        assert_eq!(
            plan.errors[0].fix_text(),
            Some("add --pages, e.g. --pages 1-3,5")
        );
    }

    #[test]
    fn split_names_the_pages() {
        let dir = scratch("split-pages");
        let many = write_pdf_sample(&dir, "pdf-many");
        let p = dir.join("doc.pdf");
        std::fs::rename(many, &p).unwrap();
        let mut j = job(Operation::Split, vec![p], Some(FormatId::Pdf));
        j.options.pages = Some(PageRanges::parse("1-3,5,7-").unwrap());
        let plan = j.plan(&Registry::builtin());
        assert_eq!(outputs(&plan), vec![dir.join("doc (pages 1-3,5,7-).pdf")]);
    }

    #[test]
    fn page_ranges_parse() {
        assert_eq!(
            PageRanges::parse("1-3,5,7-").unwrap(),
            PageRanges(vec![(1, Some(3)), (5, Some(5)), (7, None)])
        );
        assert_eq!(
            PageRanges::parse(" 2 - 4 ").unwrap(),
            PageRanges(vec![(2, Some(4))])
        );
        for bad in ["3-1", "0", "a", "", "1,,2", "-3"] {
            assert_eq!(
                PageRanges::parse(bad).unwrap_err().code,
                ErrorCode::Usage,
                "{bad:?}"
            );
        }
    }

    fn with_options(j: Job, options: JobOptions) -> Job {
        Job { options, ..j }
    }

    #[test]
    fn preset_on_image_is_usage() {
        let dir = scratch("preset-image");
        let a = put(&dir, "a.png", &PNG_1X1);
        let j = with_options(
            job(Operation::Convert, vec![a.clone()], Some(FormatId::Webp)),
            JobOptions {
                preset: Some(Preset::Small),
                ..JobOptions::default()
            },
        );
        let plan = j.plan(&Registry::builtin());
        assert!(plan.tasks.is_empty());
        let e = &plan.errors[0];
        assert_eq!(e.code, ErrorCode::Usage);
        assert_eq!(e.reason, "--preset is for mp4, mov and webm video");
        assert_eq!(e.fix_text(), Some("drop --preset"));
        assert_eq!(e.file.as_deref(), Some(a.as_path()));
    }

    #[test]
    fn preset_on_audio_target_is_usage() {
        let dir = scratch("preset-audio");
        let w = put(&dir, "a.wav", &WAV_HEADER);
        let j = with_options(
            job(Operation::Convert, vec![w], Some(FormatId::Mp3)),
            JobOptions {
                preset: Some(Preset::Compatible),
                ..JobOptions::default()
            },
        );
        let plan = j.plan(&Registry::builtin());
        assert_eq!(plan.errors[0].code, ErrorCode::Usage);
        assert!(plan.tasks.is_empty());
    }

    #[test]
    fn preset_on_video_plans() {
        let dir = scratch("preset-video");
        let m = put(&dir, "clip.mov", MOV_HEAD);
        for (op, to) in [
            (Operation::Convert, Some(FormatId::Mp4)),
            (Operation::Convert, Some(FormatId::Webm)),
            (Operation::Compress, None),
        ] {
            let j = with_options(
                job(op, vec![m.clone()], to),
                JobOptions {
                    preset: Some(Preset::Small),
                    ..JobOptions::default()
                },
            );
            let plan = j.plan(&Registry::builtin());
            assert!(plan.errors.is_empty(), "{op:?} {to:?}: {:?}", plan.errors);
            assert_eq!(plan.tasks[0].backend, Backend::Ffmpeg);
        }
    }

    #[test]
    fn preset_on_video_to_gif_or_audio_is_usage() {
        let dir = scratch("preset-gif");
        let m = put(&dir, "clip.mov", MOV_HEAD);
        for to in [FormatId::Gif, FormatId::M4a] {
            let j = with_options(
                job(Operation::Convert, vec![m.clone()], Some(to)),
                JobOptions {
                    preset: Some(Preset::Compatible),
                    ..JobOptions::default()
                },
            );
            assert_eq!(
                j.plan(&Registry::builtin()).errors[0].code,
                ErrorCode::Usage,
                "{to:?}"
            );
        }
    }

    #[test]
    fn max_size_on_audio_target_is_usage() {
        let dir = scratch("max-size-audio");
        let m = put(&dir, "clip.mov", MOV_HEAD);
        let opts = JobOptions {
            max_size: Some(640),
            ..JobOptions::default()
        };
        let plan = with_options(
            job(Operation::Convert, vec![m.clone()], Some(FormatId::Mp3)),
            opts.clone(),
        )
        .plan(&Registry::builtin());
        let e = &plan.errors[0];
        assert_eq!(e.code, ErrorCode::Usage);
        assert_eq!(e.reason, "--max-size is for images and video");
        assert_eq!(e.fix_text(), Some("drop --max-size"));
        let plan = with_options(job(Operation::Convert, vec![m], Some(FormatId::Mp4)), opts)
            .plan(&Registry::builtin());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
    }

    #[test]
    fn out_dir_shared_across_batch() {
        let dir = scratch("outdir");
        let src = dir.join("src");
        std::fs::create_dir(&src).unwrap();
        let a = put(&src, "a.png", &PNG_1X1);
        let other = src.join("other");
        std::fs::create_dir(&other).unwrap();
        let b = put(&other, "a.png", &PNG_1X1);
        let mut j = job(Operation::Convert, vec![a, b], Some(FormatId::Jpeg));
        j.output.dir = Some(dir.join("out"));
        let plan = j.plan(&Registry::builtin());
        assert_eq!(
            outputs(&plan),
            vec![dir.join("out/a.jpg"), dir.join("out/a (1).jpg")]
        );
    }

    fn pages(s: &str) -> Option<PageRanges> {
        Some(PageRanges::parse(s).unwrap())
    }

    #[test]
    fn select_pages_picks_in_document_order() {
        assert_eq!(select_pages(None, 3).unwrap(), vec![1, 2, 3]);
        assert_eq!(
            select_pages(pages("3,1-2,2").as_ref(), 3).unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            select_pages(pages("7-").as_ref(), 12).unwrap(),
            (7..=12).collect::<Vec<_>>()
        );
        let e = select_pages(pages("4").as_ref(), 3).unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage);
        assert_eq!(e.reason, "it has 3 pages");
        assert_eq!(e.fix_text(), Some("use --pages between 1 and 3"));
        assert_eq!(e.file, None);
        let e = select_pages(pages("2").as_ref(), 1).unwrap_err();
        assert_eq!(e.reason, "it has 1 page");
    }

    #[test]
    fn pdf_to_png_one_page_is_a_file() {
        let dir = scratch("pdf-one");
        let p = put(&dir, "page.pdf", &minimal_pdf());
        let plan = job(Operation::Convert, vec![p], Some(FormatId::Png)).plan(&Registry::builtin());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        assert_eq!(plan.tasks[0].backend, Backend::Pdf);
        assert_eq!(outputs(&plan), vec![dir.join("page.png")]);
    }

    #[test]
    fn pdf_to_png_many_pages_is_a_folder() {
        let dir = scratch("pdf-many");
        let p = write_pdf_sample(&dir, "pdf-text");
        let plan = job(Operation::Convert, vec![p.clone()], Some(FormatId::Png))
            .plan(&Registry::builtin());
        assert_eq!(outputs(&plan), vec![dir.join("text")]);
        // A folder is never replaced, even with --overwrite.
        std::fs::create_dir(dir.join("text")).unwrap();
        let mut j = job(Operation::Convert, vec![p], Some(FormatId::Png));
        j.output.overwrite = true;
        assert_eq!(
            outputs(&j.plan(&Registry::builtin())),
            vec![dir.join("text (1)")]
        );
    }

    #[test]
    fn pdf_pages_name_the_output() {
        let dir = scratch("pdf-pages");
        let p = write_pdf_sample(&dir, "pdf-text");
        for (r, out) in [("2", "text (page 2).png"), ("1-2", "text (pages 1-2)")] {
            let mut j = job(Operation::Convert, vec![p.clone()], Some(FormatId::Png));
            j.options.pages = pages(r);
            assert_eq!(
                outputs(&j.plan(&Registry::builtin())),
                vec![dir.join(out)],
                "{r}"
            );
        }
    }

    #[test]
    fn pages_past_the_end_is_usage_at_planning() {
        let dir = scratch("pdf-past");
        let p = write_pdf_sample(&dir, "pdf-text");
        let mut j = job(Operation::Split, vec![p.clone()], Some(FormatId::Pdf));
        j.options.pages = pages("9");
        let plan = j.plan(&Registry::builtin());
        assert!(plan.tasks.is_empty());
        let e = &plan.errors[0];
        assert_eq!(e.code, ErrorCode::Usage);
        assert_eq!(e.reason, "it has 3 pages");
        assert_eq!(e.file.as_deref(), Some(p.as_path()));
    }

    #[test]
    fn pages_on_an_image_is_usage() {
        let dir = scratch("pages-image");
        let a = put(&dir, "a.png", &PNG_1X1);
        let mut j = job(Operation::Convert, vec![a], Some(FormatId::Webp));
        j.options.pages = pages("1");
        let plan = j.plan(&Registry::builtin());
        assert!(plan.tasks.is_empty());
        let e = &plan.errors[0];
        assert_eq!(e.code, ErrorCode::Usage);
        assert_eq!(e.reason, "--pages is for PDFs");
        assert_eq!(e.fix_text(), Some("drop --pages"));
    }

    #[test]
    fn bad_pdfs_fail_at_planning() {
        let dir = scratch("pdf-bad");
        for (id, code, reason) in [
            (
                "pdf-locked",
                ErrorCode::UnsupportedInput,
                "it's password-protected",
            ),
            (
                "pdf-garbage",
                ErrorCode::CorruptInput,
                "can't read this PDF",
            ),
            (
                "pdf-no-pages",
                ErrorCode::CorruptInput,
                "this PDF has no pages",
            ),
        ] {
            let p = write_pdf_sample(&dir, id);
            let plan = job(Operation::Convert, vec![p.clone()], Some(FormatId::Png))
                .plan(&Registry::builtin());
            assert!(plan.tasks.is_empty(), "{id}");
            let e = &plan.errors[0];
            assert_eq!((e.code, e.reason.as_str()), (code, reason), "{id}");
            assert_eq!(e.file.as_deref(), Some(p.as_path()), "{id}");
        }
        let locked = &job(
            Operation::Convert,
            vec![write_pdf_sample(&dir, "pdf-locked")],
            Some(FormatId::Png),
        )
        .plan(&Registry::builtin())
        .errors[0];
        assert_eq!(
            locked.fix_text(),
            Some("remove the password in another app, then try again")
        );
        let owner = write_pdf_sample(&dir, "pdf-owner");
        let plan =
            job(Operation::Convert, vec![owner], Some(FormatId::Png)).plan(&Registry::builtin());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        assert_eq!(outputs(&plan), vec![dir.join("owner")]);
    }

    #[test]
    fn merge_reports_every_bad_pdf() {
        let dir = scratch("merge-bad-pdf");
        let inputs = ["pdf-locked", "pdf-text", "pdf-garbage"]
            .map(|id| write_pdf_sample(&dir, id))
            .to_vec();
        let plan = job(Operation::Merge, inputs, Some(FormatId::Pdf)).plan(&Registry::builtin());
        assert!(plan.tasks.is_empty());
        assert_eq!(plan.errors.len(), 2, "{:?}", plan.errors);
    }
}
