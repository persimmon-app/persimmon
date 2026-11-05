// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! `cargo run -p persimmon-corpus -- <dir> [--large]`: generate the corpus
//! and print one `id<TAB>path` line per case.

use std::path::PathBuf;
use std::process::ExitCode;

use persimmon_corpus::{Options, generate};

fn main() -> ExitCode {
    let mut dir = None;
    let mut opts = Options::default();
    for arg in std::env::args_os().skip(1) {
        if arg == "--large" {
            opts.large = true;
        } else if dir.is_none() {
            dir = Some(PathBuf::from(arg));
        } else {
            eprintln!("usage: persimmon-corpus <dir> [--large]");
            return ExitCode::from(2);
        }
    }
    let Some(dir) = dir else {
        eprintln!("usage: persimmon-corpus <dir> [--large]");
        return ExitCode::from(2);
    };
    match generate(&dir, &opts) {
        Ok(cases) => {
            for case in cases {
                println!("{}\t{}", case.id, case.path.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("persimmon-corpus: {}: {e}", dir.display());
            ExitCode::FAILURE
        }
    }
}
