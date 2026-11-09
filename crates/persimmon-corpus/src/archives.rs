// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Archive samples, written at test time and never committed (except the
//! RAR ones, which nothing free can write). Each is deterministic: fixed
//! times, no owners, and fixed salts where a format would pick random ones.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

/// One item of the standard tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeItem {
    /// Contents and Unix mode.
    File(&'static [u8], u32),
    Dir,
    /// The link's target, as stored.
    Symlink(&'static str),
}

/// The standard tree every `tree.*` archive holds under a top folder
/// `tree/`, in the order it's written. 7z leaves out the symlink.
pub const TREE: &[(&str, TreeItem)] = &[
    ("hello.txt", TreeItem::File(b"hello\n", 0o644)),
    ("run.sh", TreeItem::File(b"#!/bin/sh\necho hi\n", 0o755)),
    ("sub", TreeItem::Dir),
    ("sub/deep.txt", TreeItem::File(b"deep\n", 0o644)),
    ("cafe\u{301}.txt", TreeItem::File(b"unicode\n", 0o644)),
    ("empty", TreeItem::Dir),
    ("link", TreeItem::Symlink("hello.txt")),
];

/// Every entry's modification time: 2026-01-02 03:04:05 UTC.
pub const MTIME: u64 = 1_767_323_045;

/// Every archive sample: `(id, file name)`.
pub const ARCHIVE_SAMPLES: [(&str, &str); 19] = [
    ("arc-zip", "tree.zip"),
    ("arc-tar", "tree.tar"),
    ("arc-tgz", "tree.tar.gz"),
    ("arc-tbz2", "tree.tar.bz2"),
    ("arc-txz", "tree.tar.xz"),
    ("arc-tzst", "tree.tar.zst"),
    ("arc-7z", "tree.7z"),
    ("arc-nested", "photos.zip"),
    ("arc-slip", "slip.zip"),
    ("arc-abs", "abs.tar"),
    ("arc-through", "through.tar"),
    ("arc-hard", "hard.tar"),
    ("arc-hard-out", "hard-out.tar"),
    ("arc-case", "case.zip"),
    ("arc-dup", "dup.tar"),
    ("arc-locked-zip", "locked.zip"),
    ("arc-locked-7z", "locked.7z"),
    ("arc-plain-gz", "notes.gz"),
    ("arc-big", "big.tar.gz"),
];

/// Write the sample `id` into `dir` and return its path. Panics on an
/// unknown id.
pub fn write_archive_sample(dir: &Path, id: &str) -> PathBuf {
    let name = ARCHIVE_SAMPLES
        .iter()
        .find(|(i, _)| *i == id)
        .unwrap_or_else(|| panic!("unknown archive sample {id}"))
        .1;
    let bytes = match id {
        "arc-zip" => tree_zip(),
        "arc-tar" => tree_tar(),
        "arc-tgz" => gzip(&tree_tar()),
        "arc-tbz2" => {
            let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::new(6));
            enc.write_all(&tree_tar()).expect("bzip2");
            enc.finish().expect("bzip2")
        }
        "arc-txz" => {
            let options = lzma_rust2::XzOptions::with_preset(6);
            let mut enc = lzma_rust2::XzWriter::new(Vec::new(), options).expect("xz");
            enc.write_all(&tree_tar()).expect("xz");
            enc.finish().expect("xz")
        }
        "arc-tzst" => {
            let mut out = Vec::new();
            ruzstd::encoding::compress(
                Cursor::new(tree_tar()),
                &mut out,
                ruzstd::encoding::CompressionLevel::Fastest,
            );
            out
        }
        "arc-7z" => tree_7z(None),
        "arc-nested" => zip(&[
            ("photos/", Item::Dir),
            ("photos/a.txt", Item::File(b"a\n", 0o644)),
        ]),
        "arc-slip" => zip(&[
            ("ok.txt", Item::File(b"ok\n", 0o644)),
            ("../evil.txt", Item::File(b"evil\n", 0o644)),
        ]),
        "arc-abs" => tar(&[
            ("ok.txt", Item::File(b"ok\n", 0o644)),
            ("/tmp/prsm-evil-abs.txt", Item::File(b"evil\n", 0o644)),
        ]),
        "arc-through" => tar(&[
            ("link", Item::Symlink("/tmp")),
            ("link/prsm-evil-through.txt", Item::File(b"evil\n", 0o644)),
        ]),
        "arc-hard" => tar(&[
            ("a.txt", Item::File(b"shared\n", 0o644)),
            ("b.txt", Item::Hardlink("a.txt")),
        ]),
        "arc-hard-out" => tar(&[("b.txt", Item::Hardlink("/etc/passwd"))]),
        "arc-case" => zip(&[
            ("A.txt", Item::File(b"upper\n", 0o644)),
            ("a.txt", Item::File(b"lower\n", 0o644)),
        ]),
        "arc-dup" => tar(&[
            ("x.txt", Item::File(b"first\n", 0o644)),
            ("x.txt", Item::File(b"second\n", 0o644)),
        ]),
        "arc-locked-zip" => locked_zip(),
        "arc-locked-7z" => tree_7z(Some("secret")),
        "arc-plain-gz" => gzip(b"notes\n"),
        "arc-big" => gzip(&tar(&[("big.bin", Item::Owned(noise(3 << 20), 0o644))])),
        _ => unreachable!("every id in ARCHIVE_SAMPLES has a generator"),
    };
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    path
}

/// An entry for the writers below.
enum Item<'a> {
    File(&'a [u8], u32),
    Owned(Vec<u8>, u32),
    Dir,
    Symlink(&'a str),
    Hardlink(&'a str),
}

/// The standard tree as writer items, under `tree/`.
fn tree_items(with_link: bool) -> Vec<(String, Item<'static>)> {
    let mut items = vec![("tree/".to_string(), Item::Dir)];
    for (name, item) in TREE {
        let path = format!("tree/{name}");
        items.push(match *item {
            TreeItem::File(data, mode) => (path, Item::File(data, mode)),
            TreeItem::Dir => (format!("{path}/"), Item::Dir),
            TreeItem::Symlink(target) if with_link => (path, Item::Symlink(target)),
            TreeItem::Symlink(_) => continue,
        });
    }
    items
}

fn tree_tar() -> Vec<u8> {
    let items = tree_items(true);
    let refs: Vec<(&str, Item)> = items
        .iter()
        .map(|(n, i)| (n.as_str(), i.borrowed()))
        .collect();
    tar(&refs)
}

fn tree_zip() -> Vec<u8> {
    let items = tree_items(true);
    let refs: Vec<(&str, Item)> = items
        .iter()
        .map(|(n, i)| (n.as_str(), i.borrowed()))
        .collect();
    zip(&refs)
}

impl Item<'_> {
    fn borrowed(&self) -> Item<'_> {
        match self {
            Item::File(d, m) => Item::File(d, *m),
            Item::Owned(d, m) => Item::File(d, *m),
            Item::Dir => Item::Dir,
            Item::Symlink(t) => Item::Symlink(t),
            Item::Hardlink(t) => Item::Hardlink(t),
        }
    }
}

/// A tar archive. Names are written into the header as raw bytes, so
/// hostile names (absolute, `..`) can be made; the `tar` crate's own
/// setters refuse them.
fn tar(items: &[(&str, Item)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, item) in items {
        let mut header = tar::Header::new_ustar();
        header.set_mtime(MTIME);
        header.set_uid(0);
        header.set_gid(0);
        let (kind, mode, data): (tar::EntryType, u32, &[u8]) = match item {
            Item::File(d, m) => (tar::EntryType::Regular, *m, d),
            Item::Owned(d, m) => (tar::EntryType::Regular, *m, d),
            Item::Dir => (tar::EntryType::Directory, 0o755, b""),
            Item::Symlink(_) => (tar::EntryType::Symlink, 0o777, b""),
            Item::Hardlink(_) => (tar::EntryType::Link, 0o644, b""),
        };
        header.set_entry_type(kind);
        header.set_mode(mode);
        header.set_size(data.len() as u64);
        if let Item::Symlink(target) | Item::Hardlink(target) = item {
            raw_field(&mut header.as_old_mut().linkname, target);
        }
        raw_field(&mut header.as_old_mut().name, name);
        header.set_cksum();
        builder.append(&header, data).expect("append to tar");
    }
    builder.into_inner().expect("finish tar")
}

fn raw_field(field: &mut [u8], value: &str) {
    assert!(
        value.len() < field.len(),
        "{value} is too long for a tar field"
    );
    field.fill(0);
    field[..value.len()].copy_from_slice(value.as_bytes());
}

fn zip_time() -> zip::DateTime {
    zip::DateTime::from_date_and_time(2026, 1, 2, 3, 4, 5).expect("a valid date")
}

fn zip(items: &[(&str, Item)]) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let base = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip_time());
    for (name, item) in items {
        match item {
            Item::File(data, mode) => {
                w.start_file(*name, base.unix_permissions(*mode))
                    .expect("zip file");
                w.write_all(data).expect("zip data");
            }
            Item::Owned(data, mode) => {
                w.start_file(*name, base.unix_permissions(*mode))
                    .expect("zip file");
                w.write_all(data).expect("zip data");
            }
            Item::Dir => w
                .add_directory(name.trim_end_matches('/'), base.unix_permissions(0o755))
                .expect("zip dir"),
            Item::Symlink(target) => w
                .add_symlink(*name, *target, base.unix_permissions(0o777))
                .expect("zip symlink"),
            Item::Hardlink(_) => unreachable!("ZIP has no hard links"),
        }
    }
    w.finish().expect("finish zip").into_inner()
}

/// A ZIP whose one entry is marked encrypted: bit 0 of the general-purpose
/// flags, in both the local and the central header. ZipCrypto itself
/// starts each file with random bytes, which would make the sample differ
/// every run; the flag is what readers check.
fn locked_zip() -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip_time());
    w.start_file("secret.txt", options).expect("zip file");
    w.write_all(b"secret\n").expect("zip data");
    let mut bytes = w.finish().expect("finish zip").into_inner();
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    bytes[6] |= 1;
    let central = bytes
        .windows(4)
        .position(|w| w == b"PK\x01\x02")
        .expect("a central directory");
    bytes[central + 8] |= 1;
    bytes
}

/// The standard tree as 7z, without the symlink; with a password, AES-256
/// with a fixed IV and salt.
fn tree_7z(password: Option<&str>) -> Vec<u8> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
    let mut w = ArchiveWriter::new(Cursor::new(Vec::new())).expect("7z writer");
    let mut methods = Vec::new();
    if let Some(password) = password {
        let mut aes = AesEncoderOptions::new(password.into());
        aes.iv = [7; 16];
        aes.salt = [9; 16];
        methods.push(aes.into());
    }
    methods.push(Lzma2Options::from_level(6).into());
    w.set_content_methods(methods);
    // 100 ns ticks since 1601.
    let nt = (MTIME + 11_644_473_600) * 10_000_000;
    let mut push = |name: &str, dir: bool, mode: u32, data: &[u8]| {
        let mut entry = if dir {
            ArchiveEntry::new_directory(name)
        } else {
            ArchiveEntry::new_file(name)
        };
        entry.has_last_modified_date = true;
        entry.last_modified_date = nt.into();
        entry.has_windows_attributes = true;
        // FILE_ATTRIBUTE_UNIX_EXTENSION, with the mode in the high half.
        entry.windows_attributes = 0x8000 | (mode << 16);
        let reader = (!dir).then(|| Cursor::new(data.to_vec()));
        w.push_archive_entry(entry, reader).expect("7z entry");
    };
    push("tree", true, 0o040755, b"");
    for (name, item) in TREE {
        let path = format!("tree/{name}");
        match *item {
            TreeItem::File(data, mode) => push(&path, false, 0o100000 | mode, data),
            TreeItem::Dir => push(&path, true, 0o040755, b""),
            TreeItem::Symlink(_) => {}
        }
    }
    w.finish().expect("finish 7z").into_inner()
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), flate2::Compression::new(6));
    enc.write_all(data).expect("gzip");
    enc.finish().expect("gzip")
}

/// `len` bytes of xorshift noise from a fixed seed: they don't compress.
fn noise(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}
