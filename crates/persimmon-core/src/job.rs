// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Jobs: inputs → operation → output policy. A job is planned before
//! anything runs, so mistakes surface up front (unknown formats, impossible
//! conversions, clashing outputs), and planning never writes an output.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::detect::detect;
use crate::error::{Error, ErrorCode};
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

/// `--pages`: 1-based, inclusive ranges. `(7, None)` is "7 to the end".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRanges(pub Vec<(u32, Option<u32>)>);

impl PageRanges {
    /// Parse `"1-3,5,7-"`.
    pub fn parse(s: &str) -> Result<PageRanges, Error> {
        let bad = || {
            Error::new(ErrorCode::Usage, format!("\"{s}\" isn't a page range"))
                .with_fix("use page numbers from 1, e.g. --pages 1-3,5,7-")
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
    pub preset: Option<String>,
    pub pages: Option<PageRanges>,
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
                            .with_fix("add --to <format>"),
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
        let dir = self.output.output_dir(input)?;
        let stem = stem_of(input, reg);
        let ow = self.output.overwrite;
        let output = match self.op {
            Operation::Convert => {
                let ext = reg.format(to.expect("convert has a target")).extensions[0];
                free_name(&dir, &stem, ext, ow, taken)
            }
            Operation::Compress => {
                let ext = reg.format(from).extensions[0];
                free_name(&dir, &format!("{stem} (compressed)"), ext, ow, taken)
            }
            _ => free_name(&dir, &stem, "", ow, taken),
        };
        Ok(PlannedTask {
            inputs: vec![input.to_path_buf()],
            from: vec![from],
            to,
            backend: cap.backend,
            output,
        })
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
                        Some(cap) => {
                            backend = cap.backend;
                            from.push(d.format);
                        }
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
                .with_fix("add --pages, e.g. --pages 1-3,5"));
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
fn stem_of(path: &Path, reg: &Registry) -> String {
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
    Error::new(ErrorCode::UnsupportedConversion, reason).with_fix(format!(
        "run prsm formats to see what {} can become",
        from.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use persimmon_corpus::{GIF_1X1, PNG_1X1, ZIP_EMPTY};

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
        let a = put(&dir, "merged.pdf", b"%PDF-1.4\n");
        let b = put(&dir, "b.pdf", b"%PDF-1.4\n");
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
            e.fix.as_deref(),
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
        let p = put(&dir, "doc.pdf", b"%PDF-1.4\n");
        let plan = job(Operation::Split, vec![p], Some(FormatId::Pdf)).plan(&Registry::builtin());
        assert!(plan.tasks.is_empty());
        assert_eq!(plan.errors[0].code, ErrorCode::Usage);
        assert_eq!(
            plan.errors[0].fix.as_deref(),
            Some("add --pages, e.g. --pages 1-3,5")
        );
    }

    #[test]
    fn split_names_the_pages() {
        let dir = scratch("split-pages");
        let p = put(&dir, "doc.pdf", b"%PDF-1.4\n");
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
}
