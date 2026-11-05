// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The capability graph: which operation turns which format into which,
//! and which backend does it. `prsm formats` and `prsm info` read this;
//! nothing else hard-codes "MOV can become MP4".

use crate::format::{Format, FormatId, all_formats, format_for_name};

/// What a job does to its inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    Convert,
    Compress,
    Extract,
    Merge,
    Split,
}

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Operation::Convert => "convert",
            Operation::Compress => "compress",
            Operation::Extract => "extract",
            Operation::Merge => "merge",
            Operation::Split => "split",
        }
    }
}

/// The engine component that carries out a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    Image,
    Ffmpeg,
    Pdf,
    Archive,
}

impl Backend {
    /// The lowercase name used in messages, e.g. `"ffmpeg"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Image => "image",
            Backend::Ffmpeg => "ffmpeg",
            Backend::Pdf => "pdf",
            Backend::Archive => "archive",
        }
    }
}

/// One edge of the graph. `to` is `None` for operations whose output isn't
/// a different format: compress (same format) and extract (a folder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub op: Operation,
    pub from: FormatId,
    pub to: Option<FormatId>,
    pub backend: Backend,
}

/// The formats and the capabilities between them.
#[derive(Debug, Clone)]
pub struct Registry {
    formats: &'static [Format],
    caps: Vec<Capability>,
}

impl Registry {
    /// The v0.1 graph (DD-28).
    pub fn builtin() -> Registry {
        use Backend as B;
        use FormatId::*;
        use Operation as Op;

        const IMAGE_READ: &[FormatId] = &[Jpeg, Png, Webp, Heic, Tiff, Gif, Avif, Bmp];
        const IMAGE_WRITE: &[FormatId] = &[Jpeg, Png, Webp, Avif, Tiff];
        const VIDEO: &[FormatId] = &[Mov, Mp4, Mkv, Webm, Avi];
        const AUDIO: &[FormatId] = &[Mp3, M4a, Wav, Flac, Opus];
        const VIDEO_OUT: &[FormatId] = &[Mp4, Webm, Mov, Gif, Mp3, M4a, Wav, Flac, Opus];
        const ARCHIVES: &[FormatId] = &[Zip, Tar, TarGz, TarBz2, TarXz, TarZst, SevenZ, Rar];

        let mut caps = Vec::new();
        let mut edges = |op: Operation, from: &[FormatId], to: &[Option<FormatId>], backend| {
            for &f in from {
                for &t in to {
                    if op == Op::Convert && t == Some(f) {
                        continue;
                    }
                    caps.push(Capability {
                        op,
                        from: f,
                        to: t,
                        backend,
                    });
                }
            }
        };
        let some = |ids: &[FormatId]| ids.iter().map(|&id| Some(id)).collect::<Vec<_>>();

        edges(Op::Convert, IMAGE_READ, &some(IMAGE_WRITE), B::Image);
        edges(Op::Convert, IMAGE_READ, &[Some(Pdf)], B::Pdf);
        edges(Op::Convert, &[Pdf], &[Some(Png), Some(Jpeg)], B::Pdf);
        edges(Op::Convert, VIDEO, &some(VIDEO_OUT), B::Ffmpeg);
        edges(Op::Convert, AUDIO, &some(AUDIO), B::Ffmpeg);

        edges(Op::Compress, IMAGE_WRITE, &[None], B::Image);
        edges(Op::Compress, &[Mp4, Webm, Mov], &[None], B::Ffmpeg);

        edges(Op::Extract, ARCHIVES, &[None], B::Archive);

        let mut pdf_merge = IMAGE_READ.to_vec();
        pdf_merge.push(Pdf);
        edges(Op::Merge, &pdf_merge, &[Some(Pdf)], B::Pdf);
        let every: Vec<FormatId> = all_formats().iter().map(|f| f.id).collect();
        edges(Op::Merge, &every, &[Some(Zip), Some(TarGz)], B::Archive);

        edges(Op::Split, &[Pdf], &[Some(Pdf)], B::Pdf);

        Registry {
            formats: all_formats(),
            caps,
        }
    }

    /// The table row for a format.
    pub fn format(&self, id: FormatId) -> &Format {
        self.formats
            .iter()
            .find(|f| f.id == id)
            .expect("every FormatId has a row in the format table")
    }

    /// Parse a user-typed format: an id or any listed extension, ignoring
    /// case and a leading dot (`jpg`, `.TIF`, `tgz`).
    pub fn parse_format(&self, name: &str) -> Option<FormatId> {
        let name = name.trim().trim_start_matches('.').to_lowercase();
        if let Some(f) = self.formats.iter().find(|f| f.id.as_str() == name) {
            return Some(f.id);
        }
        self.formats
            .iter()
            .find(|f| f.extensions.contains(&name.as_str()))
            .map(|f| f.id)
            .or_else(|| format_for_name(&format!("x.{name}"), self.formats))
    }

    /// The capability for this operation on this format, if there is one.
    pub fn find(&self, op: Operation, from: FormatId, to: Option<FormatId>) -> Option<&Capability> {
        self.caps
            .iter()
            .find(|c| c.op == op && c.from == from && c.to == to)
    }

    /// What `op` can turn `from` into, sorted, without duplicates.
    pub fn targets(&self, op: Operation, from: FormatId) -> Vec<FormatId> {
        let mut t: Vec<FormatId> = self
            .caps
            .iter()
            .filter(|c| c.op == op && c.from == from)
            .filter_map(|c| c.to)
            .collect();
        t.sort();
        t.dedup();
        t
    }

    pub fn capabilities(&self) -> &[Capability] {
        &self.caps
    }

    pub fn formats(&self) -> &[Format] {
        self.formats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FormatId::*;

    #[test]
    fn heic_converts_to_jpeg_not_heic() {
        let t = Registry::builtin().targets(Operation::Convert, Heic);
        assert!(t.contains(&Jpeg));
        assert!(!t.contains(&Heic));
    }

    #[test]
    fn convert_never_maps_a_format_to_itself() {
        let reg = Registry::builtin();
        for f in reg.formats() {
            assert!(
                !reg.targets(Operation::Convert, f.id).contains(&f.id),
                "{:?}",
                f.id
            );
        }
    }

    #[test]
    fn mov_converts_to_audio() {
        assert!(
            Registry::builtin()
                .targets(Operation::Convert, Mov)
                .contains(&Mp3)
        );
    }

    #[test]
    fn pdf_splits() {
        let reg = Registry::builtin();
        let cap = reg.find(Operation::Split, Pdf, Some(Pdf)).unwrap();
        assert_eq!(cap.backend, Backend::Pdf);
    }

    #[test]
    fn rar_extracts_with_archive_backend() {
        let reg = Registry::builtin();
        assert_eq!(
            reg.find(Operation::Extract, Rar, None).unwrap().backend,
            Backend::Archive
        );
    }

    #[test]
    fn gif_compress_is_none() {
        assert!(
            Registry::builtin()
                .find(Operation::Compress, Gif, None)
                .is_none()
        );
    }

    #[test]
    fn targets_are_sorted_and_unique() {
        let t = Registry::builtin().targets(Operation::Merge, Png);
        assert_eq!(t, vec![Pdf, Zip, TarGz]);
    }

    #[test]
    fn parse_aliases() {
        let reg = Registry::builtin();
        assert_eq!(reg.parse_format("jpg"), Some(Jpeg));
        assert_eq!(reg.parse_format("JPEG"), Some(Jpeg));
        assert_eq!(reg.parse_format(".TIF"), Some(Tiff));
        assert_eq!(reg.parse_format("tgz"), Some(TarGz));
        assert_eq!(reg.parse_format("tar.gz"), Some(TarGz));
        assert_eq!(reg.parse_format("heif"), Some(Heic));
        assert_eq!(reg.parse_format("aac"), Some(M4a));
        assert_eq!(reg.parse_format("7z"), Some(SevenZ));
        assert_eq!(reg.parse_format("docx"), None);
    }

    #[test]
    fn every_format_has_a_capability() {
        let reg = Registry::builtin();
        for f in reg.formats() {
            assert!(
                reg.capabilities().iter().any(|c| c.from == f.id),
                "{:?}",
                f.id
            );
        }
    }

    #[test]
    fn format_lookup() {
        assert_eq!(Registry::builtin().format(Png).name, "PNG");
    }
}
