// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The ugly-file corpus (BL-T10), generated at test time instead of
//! committed as binaries. Every case is a file (or folder) plus what the
//! engine is expected to make of it.
//!
//! To generate it by hand: `cargo run -p persimmon-corpus -- <dir>`, adding
//! `--large` for the 4 GB sparse case. Each line printed is `id<TAB>path`.
//!
//! Expectations are strings (format ids, error codes), not engine types:
//! the engine's own tests depend on this crate, so it can't depend back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What the engine should make of a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// Detected as this format id, extension agreeing.
    Format(&'static str),
    /// Detected as this format id, with the extension saying otherwise.
    Mismatch(&'static str),
    /// Rejected with this error code.
    Error(&'static str),
}

/// One generated file or folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub id: &'static str,
    pub path: PathBuf,
    pub expect: Expect,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Also generate the 4 GB sparse video. Off by default: on volumes
    /// without sparse files it costs 4 GB of disk.
    pub large: bool,
}

/// A valid 1×1 transparent PNG, the standard 67-byte file.
pub const PNG_1X1: [u8; 67] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// A valid 1×1 GIF.
pub const GIF_1X1: &[u8] = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\xff\xff\x00\x00\x00!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";

/// An empty ZIP: just the 22-byte end-of-central-directory record.
pub const ZIP_EMPTY: [u8; 22] = [
    0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// A 44-byte WAV header: PCM, mono, 44.1 kHz, 16-bit, no samples.
pub const WAV_HEADER: [u8; 44] = [
    b'R', b'I', b'F', b'F', 36, 0, 0, 0, b'W', b'A', b'V', b'E', b'f', b'm', b't', b' ', 16, 0, 0,
    0, 1, 0, 1, 0, 0x44, 0xAC, 0, 0, 0x88, 0x58, 0x01, 0, 2, 0, 16, 0, b'd', b'a', b't', b'a', 0,
    0, 0, 0,
];

/// The header of a QuickTime file: an `ftyp` box with the `qt  ` brand.
const MOV_HEADER: &[u8] = b"\x00\x00\x00\x14ftypqt  \x00\x00\x00\x00qt  ";

/// The committed image samples: case id, file name, bytes, expected format.
/// `img-truncated` detects as JPEG; converting it must fail.
pub const IMAGE_SAMPLES: [(&str, &str, &[u8], &str); 13] = [
    (
        "img-jpeg",
        "photo.jpg",
        include_bytes!("../samples/photo.jpg"),
        "jpeg",
    ),
    (
        "img-webp",
        "photo.webp",
        include_bytes!("../samples/photo.webp"),
        "webp",
    ),
    (
        "img-heic",
        "photo.heic",
        include_bytes!("../samples/photo.heic"),
        "heic",
    ),
    (
        "img-tiff",
        "photo.tif",
        include_bytes!("../samples/photo.tif"),
        "tiff",
    ),
    (
        "img-avif",
        "photo.avif",
        include_bytes!("../samples/photo.avif"),
        "avif",
    ),
    (
        "img-bmp",
        "photo.bmp",
        include_bytes!("../samples/photo.bmp"),
        "bmp",
    ),
    (
        "img-orient-6",
        "orient-6.jpg",
        include_bytes!("../samples/orient-6.jpg"),
        "jpeg",
    ),
    (
        "img-cmyk",
        "cmyk.jpg",
        include_bytes!("../samples/cmyk.jpg"),
        "jpeg",
    ),
    (
        "img-p3",
        "p3.jpg",
        include_bytes!("../samples/p3.jpg"),
        "jpeg",
    ),
    (
        "img-alpha",
        "alpha.png",
        include_bytes!("../samples/alpha.png"),
        "png",
    ),
    (
        "img-deep-16",
        "deep-16.png",
        include_bytes!("../samples/deep-16.png"),
        "png",
    ),
    (
        "img-anim",
        "anim.gif",
        include_bytes!("../samples/anim.gif"),
        "gif",
    ),
    (
        "img-truncated",
        "truncated.jpg",
        include_bytes!("../samples/truncated.jpg"),
        "jpeg",
    ),
];

/// A minimal one-page PDF with a correct cross-reference table.
pub fn minimal_pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] >>",
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

/// The file that marks a folder as one `generate` made, and may delete.
pub const MARKER: &str = ".persimmon-corpus";

/// Generate the corpus into `dir`, replacing a corpus already there. A
/// folder that isn't empty and wasn't made by `generate` is refused, never
/// deleted: a mistyped path must not cost anyone their files.
pub fn generate(dir: &Path, opts: &Options) -> io::Result<Vec<Case>> {
    if fs::symlink_metadata(dir).is_ok() {
        if dir.join(MARKER).is_file() {
            fs::remove_dir_all(dir)?;
        } else if fs::read_dir(dir)?.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("not a corpus folder (no {MARKER} file), so it won't be replaced"),
            ));
        }
    }
    fs::create_dir_all(dir)?;
    fs::write(
        dir.join(MARKER),
        b"generated by persimmon-corpus; safe to delete\n",
    )?;

    let mut cases = Vec::new();
    let mut file = |id: &'static str, rel: &str, bytes: &[u8], expect: Expect| -> io::Result<()> {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, bytes)?;
        cases.push(Case { id, path, expect });
        Ok(())
    };

    use Expect::*;
    file("png-1x1", "pixel.png", &PNG_1X1, Format("png"))?;
    file("gif-1x1", "pixel.gif", GIF_1X1, Format("gif"))?;
    file("pdf-min", "page.pdf", &minimal_pdf(), Format("pdf"))?;
    file("zip-empty", "empty.zip", &ZIP_EMPTY, Format("zip"))?;
    file("wav-header", "silence.wav", &WAV_HEADER, Format("wav"))?;
    file("empty-png", "empty.png", b"", Error("CORRUPT_INPUT"))?;
    file("png-as-jpg", "photo.jpg", &PNG_1X1, Mismatch("png"))?;
    file(
        "truncated-png",
        "truncated.png",
        &PNG_1X1[..3],
        Error("CORRUPT_INPUT"),
    )?;
    file(
        "unknown-xyz",
        "notes.xyz",
        b"plain text, no known signature\n",
        Error("UNSUPPORTED_INPUT"),
    )?;
    file("emoji", "\u{1F351} peach.png", &PNG_1X1, Format("png"))?;
    // APFS treats the NFC and NFD spellings as one name, so they get
    // separate folders.
    file("nfc", "nfc/caf\u{e9}.png", &PNG_1X1, Format("png"))?;
    file("nfd", "nfd/cafe\u{301}.png", &PNG_1X1, Format("png"))?;
    file(
        "rtl",
        "\u{645}\u{631}\u{62D}\u{628}\u{627}.png",
        &PNG_1X1,
        Format("png"),
    )?;
    file("leading-dash", "-rf.png", &PNG_1X1, Format("png"))?;
    // Small real images for the image backend, made once and committed
    // (`samples/README.md` says how). They live in `images/` so their names
    // can't clash with the cases above.
    for (id, name, bytes, fmt) in IMAGE_SAMPLES {
        file(id, &format!("images/{name}"), bytes, Format(fmt))?;
    }
    // Names libvips would read as options if it parsed them.
    file("brackets", "images/photo [1].png", &PNG_1X1, Format("png"))?;
    file("comma", "images/a,b.png", &PNG_1X1, Format("png"))?;
    let long = format!("{}.png", "a".repeat(236));
    file("long-name", &long, &PNG_1X1, Format("png"))?;
    // `"` is illegal in Windows file names.
    if !cfg!(windows) {
        file("quotes", "it's \"quoted\".png", &PNG_1X1, Format("png"))?;
    }

    let folder = dir.join("album.png");
    fs::create_dir(&folder)?;
    cases.push(Case {
        id: "folder-named-png",
        path: folder,
        expect: Expect::Error("UNSUPPORTED_INPUT"),
    });

    if opts.large {
        let path = dir.join("large.mov");
        let f = fs::File::create(&path)?;
        io::Write::write_all(&mut &f, MOV_HEADER)?;
        f.set_len(4 << 30)?;
        cases.push(Case {
            id: "large-4gb",
            path,
            expect: Expect::Format("mov"),
        });
    }

    Ok(cases)
}
