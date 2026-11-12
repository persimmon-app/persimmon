// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! `prsm`, the Persimmon command line.
//!
//! A thin layer over `persimmon-core`: it parses arguments, builds a job,
//! runs it (or, with `--dry-run`, prints the plan), and maps the first
//! failure to a stable exit code (see "Exit codes" in the README).

mod render;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use persimmon_core::backend::Backends;
use persimmon_core::backend::ffmpeg::Ffmpeg;
use persimmon_core::backend::pdf::pdf_info;
use persimmon_core::detect::detect;
use persimmon_core::error::{Error, ErrorCode, FixKind};
use persimmon_core::exec::{CancelToken, Event, Outcome, execute};
use persimmon_core::format::{FormatId, MediaKind};
use persimmon_core::job::{Job, JobOptions, PageRanges, PlannedTask, Preset, Quality};
use persimmon_core::output::OutputPolicy;
use persimmon_core::registry::{Operation, Registry};

use render::show;

#[derive(Parser)]
#[command(
    name = "prsm",
    version,
    about = "Offline file conversion you can script.",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert files to another format.
    Convert {
        #[arg(required = true, value_name = "FILES")]
        files: Vec<PathBuf>,
        /// The format to convert to, e.g. webp, mp4, pdf.
        #[arg(long, value_name = "FMT")]
        to: String,
        #[command(flatten)]
        out: OutArgs,
        #[command(flatten)]
        tune: TuneArgs,
        /// PDF to images: only these pages, e.g. 1-3,5.
        #[arg(long, value_name = "RANGES")]
        pages: Option<String>,
    },
    /// Make files smaller, keeping their format.
    Compress {
        #[arg(required = true, value_name = "FILES")]
        files: Vec<PathBuf>,
        #[command(flatten)]
        out: OutArgs,
        #[command(flatten)]
        tune: TuneArgs,
    },
    /// Unpack archives into folders.
    Extract {
        #[arg(required = true, value_name = "ARCHIVES")]
        archives: Vec<PathBuf>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Combine files into one PDF, ZIP or tar.gz.
    Merge {
        #[arg(required = true, value_name = "FILES")]
        files: Vec<PathBuf>,
        /// pdf, zip or tar.gz.
        #[arg(long, value_name = "pdf|zip|tar.gz")]
        to: String,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Take pages out of a PDF.
    Split {
        #[arg(value_name = "PDF")]
        pdf: PathBuf,
        /// Pages to keep, e.g. 1-3,5,7-.
        #[arg(long, value_name = "RANGES")]
        pages: String,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Show what a file is and what it can become.
    Info {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// The ffmpeg binary to use instead of the one on PATH.
        #[arg(long, value_name = "PATH")]
        ffmpeg: Option<PathBuf>,
    },
    /// List every format and what it can become.
    Formats,
}

#[derive(Args)]
struct OutArgs {
    /// Write output here instead of next to each source. Created if missing.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
    /// Replace existing files instead of numbering around them.
    #[arg(long)]
    overwrite: bool,
    /// Print what would be written, and write nothing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct TuneArgs {
    #[arg(long, value_enum)]
    quality: Option<QualityArg>,
    /// Video: compatible (H.264 or VP9, plays anywhere) or small (HEVC or AV1, smaller and slower).
    #[arg(long, value_enum)]
    preset: Option<PresetArg>,
    /// The ffmpeg binary to use instead of the one on PATH.
    #[arg(long, value_name = "PATH")]
    ffmpeg: Option<PathBuf>,
    /// Images and video: shrink so the long edge is at most this many pixels.
    #[arg(long, value_name = "PX", value_parser = clap::value_parser!(u32).range(1..=i32::MAX as i64))]
    max_size: Option<u32>,
    /// Remove metadata: EXIF, XMP, tags and colour profiles (images convert to sRGB first).
    #[arg(long)]
    strip_metadata: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum QualityArg {
    Low,
    Medium,
    High,
    Max,
}

#[derive(Clone, Copy, ValueEnum)]
enum PresetArg {
    Compatible,
    Small,
}

impl From<PresetArg> for Preset {
    fn from(p: PresetArg) -> Preset {
        match p {
            PresetArg::Compatible => Preset::Compatible,
            PresetArg::Small => Preset::Small,
        }
    }
}

impl From<QualityArg> for Quality {
    fn from(q: QualityArg) -> Quality {
        match q {
            QualityArg::Low => Quality::Low,
            QualityArg::Medium => Quality::Medium,
            QualityArg::High => Quality::High,
            QualityArg::Max => Quality::Max,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let reg = Registry::builtin();
    let result = match cli.command {
        Command::Formats => {
            say(&render::formats(&reg));
            Ok(())
        }
        Command::Info { file, ffmpeg } => info(&reg, file, ffmpeg).map_err(fail),
        Command::Convert {
            files,
            to,
            out,
            tune,
            pages,
        } => parse_target(&reg, &to)
            .and_then(|target| {
                let mut options = tune.options()?;
                options.pages = pages.as_deref().map(PageRanges::parse).transpose()?;
                Ok((target, options))
            })
            .map_err(fail)
            .and_then(|(target, options)| {
                run(&reg, Operation::Convert, files, Some(target), out, options)
            }),
        Command::Compress { files, out, tune } => tune
            .options()
            .map_err(fail)
            .and_then(|options| run(&reg, Operation::Compress, files, None, out, options)),
        Command::Extract { archives, out } => run(
            &reg,
            Operation::Extract,
            archives,
            None,
            out,
            JobOptions::default(),
        ),
        Command::Merge { files, to, out } => {
            parse_target(&reg, &to).map_err(fail).and_then(|target| {
                run(
                    &reg,
                    Operation::Merge,
                    files,
                    Some(target),
                    out,
                    JobOptions::default(),
                )
            })
        }
        Command::Split { pdf, pages, out } => {
            PageRanges::parse(&pages).map_err(fail).and_then(|pages| {
                let options = JobOptions {
                    pages: Some(pages),
                    ..JobOptions::default()
                };
                run(
                    &reg,
                    Operation::Split,
                    vec![pdf],
                    Some(FormatId::Pdf),
                    out,
                    options,
                )
            })
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => ExitCode::from(code.exit_code()),
    }
}

impl TuneArgs {
    /// The job options these flags ask for.
    fn options(&self) -> Result<JobOptions, Error> {
        Ok(JobOptions {
            quality: self.quality.map(Quality::from),
            preset: self.preset.map(Preset::from),
            ffmpeg: self.ffmpeg.clone(),
            max_size: self.max_size,
            strip_metadata: self.strip_metadata,
            pages: None,
        })
    }
}

/// Write to stdout, ignoring a closed pipe: `prsm … | head -1` must still
/// convert every file and keep its exit code, so output is best-effort.
fn say(text: &str) {
    use std::io::Write;
    let _ = std::io::stdout().lock().write_all(text.as_bytes());
}

/// [`say`] one line.
fn line(text: impl std::fmt::Display) {
    say(&format!("{text}\n"));
}

/// Print an error the way every `prsm` error looks. Like [`say`], a closed
/// stderr never stops the batch.
fn report(e: &Error) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "prsm: {e}");
}

/// Print an error and return its code.
fn fail(e: Error) -> ErrorCode {
    report(&e);
    e.code
}

fn parse_target(reg: &Registry, name: &str) -> Result<FormatId, Error> {
    reg.parse_format(name).ok_or_else(|| {
        Error::new(
            ErrorCode::UnsupportedConversion,
            format!("\"{name}\" isn't a format prsm knows"),
        )
        .with_fix_as(FixKind::SeeFormats, "run prsm formats to see every format")
    })
}

/// `prsm info`. Audio and video also get duration and codecs from
/// ffprobe; without FFmpeg, a `details` row says how to get them. PDFs get
/// their page count and page size.
fn info(reg: &Registry, file: PathBuf, ffmpeg: Option<PathBuf>) -> Result<(), Error> {
    let detected = detect(&file, reg.formats())?;
    let bytes = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    let mut media = Vec::new();
    let mut failure = None;
    if matches!(
        reg.format(detected.format).kind,
        MediaKind::Video | MediaKind::Audio
    ) {
        match Ffmpeg::locate(ffmpeg.as_deref()) {
            Ok(ff) => match ff.probe(&file) {
                Ok(probed) => media = render::media_rows(&probed),
                Err(e) => failure = Some(e),
            },
            Err(e) => {
                let fix = e.fix.map(|f| format!(" ({f})")).unwrap_or_default();
                media.push(("details", format!("{}{fix}", e.reason)));
            }
        }
    }
    if detected.format == FormatId::Pdf {
        match pdf_info(&file) {
            Ok(pdf) => media = render::pdf_rows(&pdf),
            Err(e) => failure = Some(e),
        }
    }
    say(&render::info(reg, &file, detected, bytes, &media));
    failure.map_or(Ok(()), Err)
}

/// How a task's input is named in output lines: the file, or for a merge,
/// how many files.
fn label(op: Operation, task: &PlannedTask) -> String {
    match op {
        Operation::Merge => {
            let n = task.inputs.len();
            format!("{n} {}", if n == 1 { "file" } else { "files" })
        }
        _ => show(&task.inputs[0]),
    }
}

/// Report an error, and remember it if it's the batch's first. Only the
/// first cancelled task is reported: the rest were skipped by the same
/// Ctrl-C.
fn note(e: &Error, first: &mut Option<ErrorCode>) {
    if e.code == ErrorCode::Cancelled && *first == Some(ErrorCode::Cancelled) {
        return;
    }
    report(e);
    if e.code == ErrorCode::Cancelled {
        // A cancelled batch exits 130 whatever failed before it.
        *first = Some(ErrorCode::Cancelled);
    } else {
        first.get_or_insert(e.code);
    }
}

/// The first Ctrl-C, SIGTERM or closed terminal (SIGHUP) cancels: a video
/// being converted stops, an image finishes, the rest are skipped, and no
/// partial file is left. A second one quits at once.
fn cancel_on_ctrl_c() -> CancelToken {
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let _ = ctrlc::set_handler(move || {
        if token.is_cancelled() {
            std::process::exit(i32::from(ErrorCode::Cancelled.exit_code()));
        }
        token.cancel();
    });
    cancel
}

/// A progress line on stderr, `[2/5] clip.mov  43%`, drawn only when
/// stderr is a terminal and cleared before anything else is printed.
struct Progress {
    tty: bool,
    shown: bool,
}

impl Progress {
    fn new() -> Progress {
        use std::io::IsTerminal;
        Progress {
            tty: std::io::stderr().is_terminal(),
            shown: false,
        }
    }

    fn draw(&mut self, index: usize, total: usize, label: &str, fraction: f32) {
        if !self.tty {
            return;
        }
        use std::io::Write;
        let pct = (fraction * 100.0) as u32;
        let _ = write!(
            std::io::stderr().lock(),
            "\r\x1b[2K[{}/{total}] {label}  {pct}%",
            index + 1
        );
        self.shown = true;
    }

    fn clear(&mut self) {
        if self.shown {
            use std::io::Write;
            let _ = write!(std::io::stderr().lock(), "\r\x1b[2K");
            self.shown = false;
        }
    }
}

/// Plan a job, then run it (or, with `--dry-run`, print the plan). Output
/// follows input order, and the first failure in that order is returned.
fn run(
    reg: &Registry,
    op: Operation,
    inputs: Vec<PathBuf>,
    target: Option<FormatId>,
    out: OutArgs,
    options: JobOptions,
) -> Result<(), ErrorCode> {
    let job = Job {
        op,
        inputs,
        target,
        output: OutputPolicy {
            dir: out.out,
            overwrite: out.overwrite,
        },
        options,
    };
    let plan = job.plan(reg);
    // Input order: a planned task (by index) or a planning error.
    let mut next_task = 0;
    let order: Vec<Result<usize, &Error>> = plan
        .outcomes()
        .map(|o| match o {
            Ok(_) => {
                next_task += 1;
                Ok(next_task - 1)
            }
            Err(e) => Err(e),
        })
        .collect();
    let mut first = None;

    if out.dry_run {
        for slot in &order {
            match slot {
                Ok(i) => {
                    let task = &plan.tasks[*i];
                    line(format_args!("{} → {}", label(op, task), show(&task.output)));
                }
                Err(e) => note(e, &mut first),
            }
        }
    } else {
        let cancel = cancel_on_ctrl_c();
        let mut cursor = 0;
        let mut progress = Progress::new();
        execute(&job, &plan, &Backends::new(), &cancel, &mut |event| {
            let (task, result) = match event {
                Event::Started { .. } => return,
                Event::Progress {
                    index,
                    total,
                    task,
                    fraction,
                } => {
                    progress.draw(index, total, &label(op, task), fraction);
                    return;
                }
                Event::Finished { task, result, .. } => (task, result),
            };
            progress.clear();
            // Planning errors that come before this task in input order.
            while let Some(Err(e)) = order.get(cursor) {
                note(e, &mut first);
                cursor += 1;
            }
            cursor += 1;
            match result {
                Ok(Outcome::Written(path)) => {
                    line(format_args!("{} → {}", label(op, task), show(path)))
                }
                Ok(Outcome::NotSmaller) => line(format_args!(
                    "{}: already compact, nothing written",
                    label(op, task)
                )),
                Err(e) => note(e, &mut first),
            }
        });
        for slot in &order[cursor.min(order.len())..] {
            if let Err(e) = slot {
                note(e, &mut first);
            }
        }
        if cancel.is_cancelled() {
            first = Some(ErrorCode::Cancelled);
        }
    }
    match first {
        Some(code) => Err(code),
        None => Ok(()),
    }
}
