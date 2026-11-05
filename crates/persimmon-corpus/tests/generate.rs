// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

use std::collections::HashSet;

use persimmon_corpus::{Expect, Options, generate};

fn scratch(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("prsm-corpus-{name}-{}", std::process::id()))
}

#[test]
fn generates_every_case() {
    let dir = scratch("every");
    let cases = generate(&dir, &Options { large: false }).unwrap();
    let ids: HashSet<&str> = cases.iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), cases.len(), "ids are unique");
    for case in &cases {
        assert!(
            std::fs::symlink_metadata(&case.path).is_ok(),
            "{} missing at {}",
            case.id,
            case.path.display()
        );
        assert!(case.path.starts_with(&dir));
    }
    for id in [
        "png-1x1",
        "gif-1x1",
        "pdf-min",
        "zip-empty",
        "wav-header",
        "empty-png",
        "png-as-jpg",
        "truncated-png",
        "unknown-xyz",
        "folder-named-png",
        "emoji",
        "nfc",
        "nfd",
        "rtl",
        "leading-dash",
        "long-name",
    ] {
        assert!(ids.contains(id), "missing case {id}");
    }
    assert_eq!(ids.contains("quotes"), !cfg!(windows));
    assert!(!ids.contains("large-4gb"));
}

#[test]
fn generate_is_idempotent() {
    let dir = scratch("idempotent");
    let first = generate(&dir, &Options { large: false }).unwrap().len();
    std::fs::write(dir.join("stray.txt"), b"left over").unwrap();
    let second = generate(&dir, &Options { large: false }).unwrap().len();
    assert_eq!(first, second);
    assert!(!dir.join("stray.txt").exists());
}

#[test]
fn cases_hold_what_they_claim() {
    let dir = scratch("claims");
    let cases = generate(&dir, &Options { large: false }).unwrap();
    let get = |id: &str| cases.iter().find(|c| c.id == id).unwrap();
    assert_eq!(std::fs::metadata(&get("png-1x1").path).unwrap().len(), 67);
    assert_eq!(
        std::fs::metadata(&get("wav-header").path).unwrap().len(),
        44
    );
    assert_eq!(std::fs::metadata(&get("zip-empty").path).unwrap().len(), 22);
    assert_eq!(std::fs::metadata(&get("empty-png").path).unwrap().len(), 0);
    assert!(get("folder-named-png").path.is_dir());
    assert_eq!(get("long-name").path.file_name().unwrap().len(), 240);
    let nfc = get("nfc")
        .path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let nfd = get("nfd")
        .path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(nfc, "caf\u{e9}.png");
    assert_eq!(nfd, "cafe\u{301}.png");
    assert_eq!(get("png-as-jpg").expect, Expect::Mismatch("png"));
    assert_eq!(get("empty-png").expect, Expect::Error("CORRUPT_INPUT"));
}

#[test]
fn large_case_is_sparse_and_4gb() {
    if std::env::var("PERSIMMON_CORPUS_LARGE").as_deref() != Ok("1") {
        return;
    }
    let dir = scratch("large");
    let cases = generate(&dir, &Options { large: true }).unwrap();
    let large = cases.iter().find(|c| c.id == "large-4gb").unwrap();
    assert_eq!(std::fs::metadata(&large.path).unwrap().len(), 4 << 30);
    assert_eq!(large.expect, Expect::Format("mov"));
    std::fs::remove_dir_all(&dir).unwrap();
}
