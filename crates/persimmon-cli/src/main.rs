// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! `prsm`, the Persimmon command line. A placeholder until Phase 1: it only
//! reports its version. Argument parsing gets a real library then.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--version" | "-V"] => {
            println!("prsm {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        [] | ["--help" | "-h"] => {
            println!(
                "prsm {} (persimmon-core {}, {})\n\nNothing to convert yet: Persimmon is in Phase 0.\n\nUsage: prsm --version",
                env!("CARGO_PKG_VERSION"),
                persimmon_core::VERSION,
                persimmon_core::platform::os_name()
            );
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "prsm: unknown arguments: {}\nUsage: prsm --version",
                args.join(" ")
            );
            ExitCode::from(2)
        }
    }
}
