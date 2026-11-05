// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The formats Persimmon knows in v0.1 (DD-28), and content sniffing.
//!
//! A format is identified by its bytes first and its name second: when the
//! two disagree, the bytes win.

/// A format Persimmon reads or writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FormatId {
    Jpeg,
    Png,
    Webp,
    Heic,
    Tiff,
    Gif,
    Avif,
    Bmp,
    Mov,
    Mp4,
    Mkv,
    Webm,
    Avi,
    Mp3,
    M4a,
    Wav,
    Flac,
    Opus,
    Pdf,
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    SevenZ,
    Rar,
}

impl FormatId {
    /// The stable lowercase id, as typed after `--to`.
    pub fn as_str(self) -> &'static str {
        use FormatId::*;
        match self {
            Jpeg => "jpeg",
            Png => "png",
            Webp => "webp",
            Heic => "heic",
            Tiff => "tiff",
            Gif => "gif",
            Avif => "avif",
            Bmp => "bmp",
            Mov => "mov",
            Mp4 => "mp4",
            Mkv => "mkv",
            Webm => "webm",
            Avi => "avi",
            Mp3 => "mp3",
            M4a => "m4a",
            Wav => "wav",
            Flac => "flac",
            Opus => "opus",
            Pdf => "pdf",
            Zip => "zip",
            Tar => "tar",
            TarGz => "tar.gz",
            TarBz2 => "tar.bz2",
            TarXz => "tar.xz",
            TarZst => "tar.zst",
            SevenZ => "7z",
            Rar => "rar",
        }
    }
}

/// The broad kind of media a format holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Document,
    Archive,
}

/// One row of the format table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub id: FormatId,
    /// The display name, e.g. `"PNG"`.
    pub name: &'static str,
    pub kind: MediaKind,
    /// File extensions, lowercase and without the dot. `extensions[0]` is
    /// the one written on output.
    pub extensions: &'static [&'static str],
}

const fn f(
    id: FormatId,
    name: &'static str,
    kind: MediaKind,
    extensions: &'static [&'static str],
) -> Format {
    Format {
        id,
        name,
        kind,
        extensions,
    }
}

static FORMATS: [Format; 27] = {
    use FormatId::*;
    use MediaKind::*;
    [
        f(Jpeg, "JPEG", Image, &["jpg", "jpeg", "jpe"]),
        f(Png, "PNG", Image, &["png"]),
        f(Webp, "WebP", Image, &["webp"]),
        f(Heic, "HEIC", Image, &["heic", "heif"]),
        f(Tiff, "TIFF", Image, &["tiff", "tif"]),
        f(Gif, "GIF", Image, &["gif"]),
        f(Avif, "AVIF", Image, &["avif"]),
        f(Bmp, "BMP", Image, &["bmp"]),
        f(Mov, "MOV", Video, &["mov"]),
        f(Mp4, "MP4", Video, &["mp4"]),
        f(Mkv, "MKV", Video, &["mkv"]),
        f(Webm, "WebM", Video, &["webm"]),
        f(Avi, "AVI", Video, &["avi"]),
        f(Mp3, "MP3", Audio, &["mp3"]),
        f(M4a, "M4A", Audio, &["m4a", "aac"]),
        f(Wav, "WAV", Audio, &["wav"]),
        f(Flac, "FLAC", Audio, &["flac"]),
        f(Opus, "Opus", Audio, &["opus"]),
        f(Pdf, "PDF", Document, &["pdf"]),
        f(Zip, "ZIP", Archive, &["zip"]),
        f(Tar, "TAR", Archive, &["tar"]),
        f(TarGz, "TAR.GZ", Archive, &["tar.gz", "tgz"]),
        f(TarBz2, "TAR.BZ2", Archive, &["tar.bz2", "tbz2"]),
        f(TarXz, "TAR.XZ", Archive, &["tar.xz", "txz"]),
        f(TarZst, "TAR.ZST", Archive, &["tar.zst", "tzst"]),
        f(SevenZ, "7z", Archive, &["7z"]),
        f(Rar, "RAR", Archive, &["rar"]),
    ]
};

/// Every format Persimmon knows, in [`FormatId`] order.
pub fn all_formats() -> &'static [Format] {
    &FORMATS
}

/// The format a file name's extension names, if any. Matching ignores case
/// and prefers the longest extension, so `photos.tar.gz` is `tar.gz`, not
/// an unknown `.gz`. A name that is only an extension (`png`) matches nothing.
pub fn format_for_name(name: &str, formats: &[Format]) -> Option<FormatId> {
    let lower = name.to_lowercase();
    formats
        .iter()
        .flat_map(|f| f.extensions.iter().map(move |e| (f.id, *e)))
        .filter(|(_, ext)| {
            lower.len() > ext.len() + 1
                && lower.ends_with(ext)
                && lower.as_bytes()[lower.len() - ext.len() - 1] == b'.'
        })
        .max_by_key(|(_, ext)| ext.len())
        .map(|(id, _)| id)
}

/// Identify a format from the first bytes of a file. Looks at no more than
/// the first 512 bytes.
pub fn sniff(head: &[u8]) -> Option<FormatId> {
    use FormatId::*;
    let starts = |sig: &[u8]| head.starts_with(sig);
    let at = |offset: usize, sig: &[u8]| head.get(offset..offset + sig.len()) == Some(sig);

    if starts(&[0xFF, 0xD8, 0xFF]) {
        return Some(Jpeg);
    }
    if starts(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(Png);
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some(Gif);
    }
    if starts(b"RIFF") {
        if at(8, b"WEBP") {
            return Some(Webp);
        }
        if at(8, b"WAVE") {
            return Some(Wav);
        }
        if at(8, b"AVI ") {
            return Some(Avi);
        }
    }
    if starts(b"BM") {
        return Some(Bmp);
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        return Some(Tiff);
    }
    if starts(b"%PDF-") {
        return Some(Pdf);
    }
    if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") {
        return Some(Zip);
    }
    if starts(b"7z\xBC\xAF\x27\x1C") {
        return Some(SevenZ);
    }
    if starts(b"Rar!\x1A\x07") {
        return Some(Rar);
    }
    if starts(&[0x1F, 0x8B]) {
        return Some(TarGz);
    }
    if starts(b"BZh") {
        return Some(TarBz2);
    }
    if starts(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00]) {
        return Some(TarXz);
    }
    if starts(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return Some(TarZst);
    }
    if at(257, b"ustar") {
        return Some(Tar);
    }
    if starts(b"fLaC") {
        return Some(Flac);
    }
    if starts(b"OggS") {
        return Some(Opus);
    }
    // AAC in ADTS frames (a plain .aac file): sync word FFF, layer 00.
    if head.len() >= 2 && head[0] == 0xFF && matches!(head[1], 0xF0 | 0xF1 | 0xF8 | 0xF9) {
        return Some(M4a);
    }
    if starts(b"ID3")
        || (head.len() >= 2 && head[0] == 0xFF && matches!(head[1], 0xF2 | 0xF3 | 0xFA | 0xFB))
    {
        return Some(Mp3);
    }
    if starts(&[0x1A, 0x45, 0xDF, 0xA3]) {
        let end = head.len().min(4 + 64);
        let doctype = &head[4..end];
        return Some(if doctype.windows(4).any(|w| w == b"webm") {
            Webm
        } else {
            Mkv
        });
    }
    if at(4, b"ftyp") && head.len() >= 12 {
        return Some(match &head[8..12] {
            b"heic" | b"heix" | b"mif1" | b"msf1" | b"hevc" => Heic,
            b"avif" | b"avis" => Avif,
            b"qt  " => Mov,
            b"M4A " => M4a,
            _ => Mp4,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(offset: usize, bytes: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; offset];
        v.extend_from_slice(bytes);
        v
    }

    fn ftyp(brand: &[u8; 4]) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 0x18];
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(brand);
        v.extend_from_slice(&[0; 8]);
        v
    }

    #[test]
    fn sniff_table() {
        use FormatId::*;
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Jpeg));
        assert_eq!(
            sniff(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            Some(Png)
        );
        assert_eq!(sniff(b"GIF87a"), Some(Gif));
        assert_eq!(sniff(b"GIF89a"), Some(Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Webp));
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVEfmt "), Some(Wav));
        assert_eq!(sniff(b"RIFF\0\0\0\0AVI LIST"), Some(Avi));
        assert_eq!(sniff(b"BM\0\0\0\0"), Some(Bmp));
        assert_eq!(sniff(b"II*\0"), Some(Tiff));
        assert_eq!(sniff(b"MM\0*"), Some(Tiff));
        assert_eq!(sniff(b"%PDF-1.7"), Some(Pdf));
        assert_eq!(sniff(b"PK\x03\x04"), Some(Zip));
        assert_eq!(sniff(b"PK\x05\x06"), Some(Zip));
        assert_eq!(sniff(b"7z\xBC\xAF\x27\x1C"), Some(SevenZ));
        assert_eq!(sniff(b"Rar!\x1A\x07\x01\x00"), Some(Rar));
        assert_eq!(sniff(&[0x1F, 0x8B, 0x08]), Some(TarGz));
        assert_eq!(sniff(b"BZh91AY"), Some(TarBz2));
        assert_eq!(sniff(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00]), Some(TarXz));
        assert_eq!(sniff(&[0x28, 0xB5, 0x2F, 0xFD]), Some(TarZst));
        assert_eq!(sniff(&at(257, b"ustar\x0000")), Some(Tar));
        assert_eq!(sniff(b"fLaC\0\0\0\x22"), Some(Flac));
        assert_eq!(sniff(b"OggS\0\x02"), Some(Opus));
        assert_eq!(sniff(b"ID3\x04\0"), Some(Mp3));
        assert_eq!(sniff(&[0xFF, 0xFB, 0x90]), Some(Mp3));
        assert_eq!(sniff(&[0xFF, 0xF3, 0x90]), Some(Mp3));
        assert_eq!(sniff(&[0xFF, 0xF2, 0x90]), Some(Mp3));
        assert_eq!(sniff(&[0xFF, 0xFA, 0x90]), Some(Mp3));
        // AAC in ADTS frames, as in a plain .aac file.
        assert_eq!(sniff(&[0xFF, 0xF1, 0x50, 0x80]), Some(M4a));
        assert_eq!(sniff(&[0xFF, 0xF9, 0x50, 0x80]), Some(M4a));
        let mut mkv = vec![0x1A, 0x45, 0xDF, 0xA3];
        mkv.extend_from_slice(b"\x9f\x42\x86\x81\x01\x42\x82\x88matroska");
        assert_eq!(sniff(&mkv), Some(Mkv));
        let mut webm = vec![0x1A, 0x45, 0xDF, 0xA3];
        webm.extend_from_slice(b"\x9f\x42\x86\x81\x01\x42\x82\x84webm");
        assert_eq!(sniff(&webm), Some(Webm));
        for brand in [b"heic", b"heix", b"mif1", b"msf1", b"hevc"] {
            assert_eq!(sniff(&ftyp(brand)), Some(Heic), "{brand:?}");
        }
        assert_eq!(sniff(&ftyp(b"avif")), Some(Avif));
        assert_eq!(sniff(&ftyp(b"avis")), Some(Avif));
        assert_eq!(sniff(&ftyp(b"qt  ")), Some(Mov));
        assert_eq!(sniff(&ftyp(b"M4A ")), Some(M4a));
        assert_eq!(sniff(&ftyp(b"isom")), Some(Mp4));
    }

    #[test]
    fn sniff_unknown_is_none() {
        assert_eq!(sniff(b"hello"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn ids_and_extensions() {
        assert_eq!(FormatId::TarGz.as_str(), "tar.gz");
        assert_eq!(FormatId::SevenZ.as_str(), "7z");
        assert_eq!(FormatId::Jpeg.as_str(), "jpeg");
        let jpeg = all_formats()
            .iter()
            .find(|f| f.id == FormatId::Jpeg)
            .unwrap();
        assert_eq!(jpeg.extensions, &["jpg", "jpeg", "jpe"]);
        assert_eq!(all_formats().len(), 27);
    }

    #[test]
    fn file_names_map_by_longest_extension() {
        let f = all_formats();
        assert_eq!(format_for_name("photos.tar.gz", f), Some(FormatId::TarGz));
        assert_eq!(format_for_name("PHOTO.JPG", f), Some(FormatId::Jpeg));
        assert_eq!(format_for_name("a.tgz", f), Some(FormatId::TarGz));
        assert_eq!(format_for_name("notes.xyz", f), None);
        assert_eq!(format_for_name("png", f), None);
    }
}
