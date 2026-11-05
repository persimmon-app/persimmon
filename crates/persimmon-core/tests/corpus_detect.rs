// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Every corpus case, checked against detection.

use std::time::{Duration, Instant};

use persimmon_core::detect::detect;
use persimmon_core::format::all_formats;
use persimmon_corpus::{Expect, Options, generate};

#[test]
fn corpus_matches_detect() {
    let large = std::env::var("PERSIMMON_CORPUS_LARGE").as_deref() == Ok("1");
    let dir = std::env::temp_dir().join(format!("prsm-core-corpus-{}", std::process::id()));
    let cases = generate(&dir, &Options { large }).unwrap();
    assert_eq!(cases.iter().any(|c| c.id == "large-4gb"), large);

    for case in &cases {
        let started = Instant::now();
        let got = detect(&case.path, all_formats());
        let elapsed = started.elapsed();
        match (case.expect, &got) {
            (Expect::Format(id), Ok(d)) => {
                assert_eq!(d.format.as_str(), id, "{}", case.id);
                assert!(!d.extension_mismatch, "{}", case.id);
            }
            (Expect::Mismatch(id), Ok(d)) => {
                assert_eq!(d.format.as_str(), id, "{}", case.id);
                assert!(d.extension_mismatch, "{}", case.id);
            }
            (Expect::Error(code), Err(e)) => {
                assert_eq!(e.code.as_str(), code, "{}", case.id);
                assert_eq!(e.file.as_deref(), Some(case.path.as_path()), "{}", case.id);
            }
            (expect, got) => panic!("{}: expected {expect:?}, got {got:?}", case.id),
        }
        if case.id == "large-4gb" {
            assert!(
                elapsed < Duration::from_secs(1),
                "large-4gb took {elapsed:?}"
            );
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
