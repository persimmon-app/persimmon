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

#[test]
fn generate_refuses_a_folder_it_did_not_make() {
    let dir = scratch("foreign");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("precious.txt"), b"keep me").unwrap();
    assert!(generate(&dir, &Options { large: false }).is_err());
    assert_eq!(std::fs::read(dir.join("precious.txt")).unwrap(), b"keep me");
}

#[test]
fn generate_accepts_an_empty_existing_folder() {
    let dir = scratch("empty-existing");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(generate(&dir, &Options { large: false }).is_ok());
}

#[test]
fn image_samples_are_generated() {
    let dir = scratch("images");
    let _ = std::fs::remove_dir_all(&dir);
    let cases = generate(&dir, &Options { large: false }).unwrap();
    let get = |id: &str| {
        cases
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("missing case {id}"))
    };
    for id in [
        "img-jpeg",
        "img-webp",
        "img-heic",
        "img-tiff",
        "img-avif",
        "img-bmp",
        "img-orient-6",
        "img-cmyk",
        "img-p3",
        "img-alpha",
        "img-deep-16",
        "img-anim",
        "img-truncated",
        "brackets",
        "comma",
    ] {
        assert!(get(id).path.is_file(), "{id} not written");
    }
    assert_eq!(get("brackets").path.file_name().unwrap(), "photo [1].png");
    assert_eq!(get("comma").path.file_name().unwrap(), "a,b.png");
    assert_eq!(get("img-heic").expect, Expect::Format("heic"));

    let cmyk = std::fs::read(&get("img-cmyk").path).unwrap();
    assert_eq!(jpeg_components(&cmyk), Some(4));
    let orient = std::fs::read(&get("img-orient-6").path).unwrap();
    assert_eq!(exif_segments(&orient), 1);
    assert_eq!(&orient[2..4], &[0xFF, 0xE0], "JFIF stays first");

    let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
    let total: u64 = std::fs::read_dir(&samples)
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(total < 2_000_000, "samples are {total} bytes");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The number of colour components in a JPEG's frame header.
fn jpeg_components(jpeg: &[u8]) -> Option<u8> {
    let mut i = 2;
    while i + 4 <= jpeg.len() && jpeg[i] == 0xFF {
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        if matches!(jpeg[i + 1], 0xC0..=0xC2) {
            return jpeg.get(i + 9).copied();
        }
        i += 2 + len;
    }
    None
}

/// How many APP1 EXIF segments precede the image data.
fn exif_segments(jpeg: &[u8]) -> usize {
    let (mut n, mut i) = (0, 2);
    while i + 4 <= jpeg.len() && jpeg[i] == 0xFF && jpeg[i + 1] != 0xDA {
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        if jpeg[i + 1] == 0xE1 && jpeg[i + 4..].starts_with(b"Exif\0\0") {
            n += 1;
        }
        i += 2 + len;
    }
    n
}
