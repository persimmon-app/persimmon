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
use persimmon_core::exec::{CancelToken, Event, Outcome, execute, execute_in};
use persimmon_core::format::{FormatId, MediaKind};
use persimmon_core::job::{Job, JobOptions, PageRanges, PlannedTask, Preset, Quality};
use persimmon_core::original::move_original;
use persimmon_core::output::OutputPolicy;
use persimmon_core::pool::{Jobs, Limits, Pool};
use persimmon_core::registry::{Operation, Registry};
use persimmon_core::watch::{Change, Patterns, WatchSpec, Watcher};

use render::{show, tilde};

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
        /// With a folder: also convert the files in its subfolders.
        #[arg(long)]
        recursive: bool,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Make files smaller, keeping their format.
    Compress {
        #[arg(required = true, value_name = "FILES")]
        files: Vec<PathBuf>,
        #[command(flatten)]
        out: OutArgs,
        #[command(flatten)]
        tune: TuneArgs,
        /// With a folder: also compress the files in its subfolders.
        #[arg(long)]
        recursive: bool,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Unpack archives into folders.
    Extract {
        #[arg(required = true, value_name = "ARCHIVES")]
        archives: Vec<PathBuf>,
        #[command(flatten)]
        out: OutArgs,
        #[command(flatten)]
        run: RunArgs,
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
    /// Convert files as they land in a folder, until ctrl-c.
    Watch {
        #[arg(value_name = "FOLDER")]
        folder: PathBuf,
        /// The format to convert to, e.g. webp, mp4, pdf.
        #[arg(long, value_name = "FMT")]
        to: String,
        /// Which files to convert, e.g. "*.jpg,*.png". Default: every kind that can become the target.
        #[arg(long = "match", value_name = "PATTERNS")]
        patterns: Option<String>,
        /// Where results go. Default: converted, inside the folder. Never watched.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// After a file converts, move the original here. Never watched.
        #[arg(long, value_name = "DIR")]
        then_move: Option<PathBuf>,
        /// Also watch the folder's subfolders; results mirror them.
        #[arg(long)]
        recursive: bool,
        /// Also convert the files already in the folder.
        #[arg(long)]
        existing: bool,
        /// Convert a file as soon as it appears, without waiting for it to stop growing.
        #[arg(long)]
        no_wait: bool,
        #[command(flatten)]
        tune: TuneArgs,
    },
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
struct RunArgs {
    /// How many files to work on at once: a number, or auto for one per performance core. Video and audio still go one at a time.
    #[arg(long, value_name = "N|auto", default_value = "1", value_parser = parse_jobs)]
    jobs: Jobs,
}

fn parse_jobs(s: &str) -> Result<Jobs, String> {
    s.parse()
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
    /// Use the hardware video encoder (VideoToolbox) for H.264 and HEVC where available; falls back to software.
    #[arg(long)]
    hardware: bool,
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

/// One file at a time: merge and split, and the default.
const SERIAL: Jobs = Jobs::Fixed(std::num::NonZeroUsize::MIN);

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
            recursive,
            run: r,
        } => parse_target(&reg, &to)
            .and_then(|target| {
                let mut options = tune.options()?;
                options.pages = pages.as_deref().map(PageRanges::parse).transpose()?;
                options.recursive = recursive;
                Ok((target, options))
            })
            .map_err(fail)
            .and_then(|(target, options)| {
                run(
                    &reg,
                    Operation::Convert,
                    files,
                    Some(target),
                    out,
                    options,
                    r.jobs,
                )
            }),
        Command::Compress {
            files,
            out,
            tune,
            recursive,
            run: r,
        } => tune
            .options()
            .map(|options| JobOptions {
                recursive,
                ..options
            })
            .map_err(fail)
            .and_then(|options| run(&reg, Operation::Compress, files, None, out, options, r.jobs)),
        Command::Extract {
            archives,
            out,
            run: r,
        } => run(
            &reg,
            Operation::Extract,
            archives,
            None,
            out,
            JobOptions::default(),
            r.jobs,
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
                    SERIAL,
                )
            })
        }
        Command::Watch {
            folder,
            to,
            patterns,
            out,
            then_move,
            recursive,
            existing,
            no_wait,
            tune,
        } => parse_target(&reg, &to)
            .and_then(|target| {
                let mut spec = WatchSpec::new(&reg, folder, target);
                spec.options = tune.options()?;
                if let Some(p) = patterns {
                    spec.patterns = Patterns::parse(&p)?;
                }
                if let Some(out) = out {
                    spec.results = out;
                }
                spec.done = then_move;
                spec.recursive = recursive;
                spec.existing = existing;
                spec.settle = !no_wait;
                Ok(spec)
            })
            .map_err(fail)
            .and_then(|spec| watch(&reg, &spec)),
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
                    SERIAL,
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
            recursive: false,
            hardware: self.hardware,
        })
    }
}

/// `text`, then its notes in one pair of parentheses, joined by `"; "`.
fn with_notes(text: &str, notes: &[String]) -> String {
    if notes.is_empty() {
        text.to_string()
    } else {
        format!("{text} ({})", notes.join("; "))
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
/// The line after a batch whose folders held files it left alone:
/// `1 file skipped: can't become WebP`, `3 files skipped: can't be compressed`.
fn skip_line(n: usize, op: Operation, target: Option<&str>) -> String {
    let files = if n == 1 { "file" } else { "files" };
    let why = match (op, target) {
        (Operation::Compress, _) => "be compressed".to_string(),
        (_, Some(name)) => format!("become {name}"),
        (_, None) => "be converted".to_string(),
    };
    format!("{n} {files} skipped: can't {why}")
}

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

/// `prsm watch`: convert each settled file that lands in the folder, one
/// at a time, until Ctrl-C (which exits 0). A file that fails prints its
/// error and watching goes on; its original is never moved.
fn watch(reg: &Registry, spec: &WatchSpec) -> Result<(), ErrorCode> {
    let mut watcher = Watcher::start(spec).map_err(fail)?;
    line(format_args!(
        "watching {} → {} (ctrl-c to stop)",
        tilde(&spec.folder),
        tilde(&spec.results)
    ));
    let cancel = cancel_on_ctrl_c();
    let backends = Backends::new();
    let mut progress = Progress::new();
    while let Some(change) = watcher.next(&cancel) {
        let path = match change {
            Change::Missing => {
                line(format_args!(
                    "waiting for {}, which isn't there now",
                    tilde(&spec.folder)
                ));
                continue;
            }
            Change::Back => {
                line(format_args!(
                    "{} is back, watching again",
                    tilde(&spec.folder)
                ));
                continue;
            }
            Change::Ready(path) => path,
        };
        let root = watcher.root();
        let name = show(path.strip_prefix(root).unwrap_or(&path));
        let job = spec.job_for(root, &path);
        let plan = job.plan(reg);
        for e in &plan.errors {
            report(e);
        }
        let results = execute(&job, &plan, &backends, &cancel, &mut |event| {
            if let Event::Progress {
                index,
                total,
                fraction,
                ..
            } = event
            {
                progress.draw(index, total, &name, fraction);
            }
        });
        progress.clear();
        for result in results {
            match result {
                Ok(outcome @ (Outcome::Written(_) | Outcome::WrittenNoted(..))) => {
                    let (out, note) = match outcome {
                        Outcome::WrittenNoted(out, note) => (out, Some(note.text)),
                        Outcome::Written(out) => (out, None),
                        Outcome::NotSmaller => unreachable!(),
                    };
                    let text = format!("{name} → {}", tilde(&out));
                    let moved = spec
                        .done
                        .as_ref()
                        .map(|done| move_original(&path, root, done));
                    let mut extras: Vec<String> = note.into_iter().collect();
                    match moved {
                        Some(Ok(m)) => extras.push(format!("original moved to {}", tilde(&m.to))),
                        Some(Err(e)) => {
                            line(with_notes(&text, &extras));
                            report(&e);
                            continue;
                        }
                        None => {}
                    }
                    line(with_notes(&text, &extras));
                }
                Ok(Outcome::NotSmaller) => {
                    line(format_args!("{name}: already compact, nothing written"))
                }
                // Stopping: the file stays as it was. Said, since the next
                // run leaves it alone unless given --existing.
                Err(e) if e.code == ErrorCode::Cancelled => {
                    line(format_args!("{name}: stopped, not converted"))
                }
                Err(e) => report(&e),
            }
        }
    }
    Ok(())
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
    jobs: Jobs,
) -> Result<(), ErrorCode> {
    let job = Job {
        op,
        inputs,
        target,
        output: OutputPolicy {
            dir: out.out,
            overwrite: out.overwrite,
            fallback: None,
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
        let backends = Backends::new();
        let mut on_event = |event: Event| {
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
                Ok(Outcome::WrittenNoted(path, n)) => {
                    line(format_args!("{} → {} ({n})", label(op, task), show(path)))
                }
                Ok(Outcome::NotSmaller) => line(format_args!(
                    "{}: already compact, nothing written",
                    label(op, task)
                )),
                Err(e) => note(e, &mut first),
            }
        };
        if jobs == SERIAL {
            execute(&job, &plan, &backends, &cancel, &mut on_event);
        } else {
            let pool = Pool::new(Limits::of(jobs));
            execute_in(&job, &plan, &backends, &cancel, &pool, &mut on_event);
        }
        for slot in &order[cursor.min(order.len())..] {
            if let Err(e) = slot {
                note(e, &mut first);
            }
        }
        if cancel.is_cancelled() {
            first = Some(ErrorCode::Cancelled);
        }
    }
    if !plan.skipped.is_empty() {
        let name = target.map(|t| reg.format(t).name);
        line(skip_line(plan.skipped.len(), op, name));
    }
    match first {
        Some(code) => Err(code),
        None => Ok(()),
    }
}
