// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! `prsm`, the Persimmon command line.
//!
//! A thin layer over `persimmon-core`: it parses arguments, builds a job,
//! prints the plan or the errors, and maps the first failure to a stable
//! exit code (`docs/exit-codes.md`). Until the backends land, only
//! `formats`, `info` and `--dry-run` do real work.

mod render;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use persimmon_core::detect::detect;
use persimmon_core::error::{Error, ErrorCode};
use persimmon_core::format::FormatId;
use persimmon_core::job::{Job, JobOptions, PageRanges, Quality};
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
    #[arg(long, value_name = "NAME")]
    preset: Option<String>,
    /// The ffmpeg binary to use instead of the one on PATH.
    #[arg(long, value_name = "PATH")]
    ffmpeg: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum QualityArg {
    Low,
    Medium,
    High,
    Max,
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
            print!("{}", render::formats(&reg));
            Ok(())
        }
        Command::Info { file } => info(&reg, file).map_err(fail),
        Command::Convert {
            files,
            to,
            out,
            tune,
        } => parse_target(&reg, &to).map_err(fail).and_then(|target| {
            run(
                &reg,
                Operation::Convert,
                files,
                Some(target),
                out,
                tune.options(),
            )
        }),
        Command::Compress { files, out, tune } => {
            run(&reg, Operation::Compress, files, None, out, tune.options())
        }
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
    fn options(&self) -> JobOptions {
        // `--ffmpeg` is accepted now and used by the FFmpeg backend (weeks 4–5).
        let _ = &self.ffmpeg;
        JobOptions {
            quality: self.quality.map(Quality::from),
            preset: self.preset.clone(),
            pages: None,
        }
    }
}

/// Print an error the way every `prsm` error looks.
fn report(e: &Error) {
    eprintln!("prsm: {e}");
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
        .with_fix("run prsm formats to see every format")
    })
}

fn info(reg: &Registry, file: PathBuf) -> Result<(), Error> {
    let detected = detect(&file, reg.formats())?;
    let bytes = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    print!("{}", render::info(reg, &file, detected, bytes));
    Ok(())
}

/// Plan a job, print the plan (or, without `--dry-run`, try to run it),
/// and return the first failure in input order.
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
    let mut first_failure = None;
    for outcome in plan.outcomes() {
        let failure = match outcome {
            Err(e) => Some(e.clone()),
            Ok(task) if out.dry_run => {
                let from = match op {
                    Operation::Merge => {
                        let n = task.inputs.len();
                        format!("{n} {}", if n == 1 { "file" } else { "files" })
                    }
                    _ => show(&task.inputs[0]),
                };
                println!("{from} → {}", show(&task.output));
                None
            }
            Ok(task) => {
                let e = Error::new(
                    ErrorCode::NotImplemented,
                    format!("the {} backend isn't built yet", task.backend.as_str()),
                )
                .with_fix("use --dry-run to see the plan");
                Some(match op {
                    Operation::Merge => e,
                    _ => e.with_file(&task.inputs[0]),
                })
            }
        };
        if let Some(e) = failure {
            report(&e);
            first_failure.get_or_insert(e.code);
        }
    }
    match first_failure {
        Some(code) => Err(code),
        None => Ok(()),
    }
}
