// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The format wheel (spec D4): which formats it offers for the dragged
//! files, in what order, what its hub says, and where a point lands on it.
//! Pure; the window only mirrors this geometry for hover.

use persimmon_core::format::{FormatId, MediaKind};
use persimmon_core::registry::{Operation, Registry};

/// The wheel's diameter, in points.
pub const SIZE: f64 = 320.0;
/// The hub's radius, in points.
pub const HUB: f64 = 64.0;
/// How many formats the wheel holds.
pub const SEGMENTS: usize = 6;
/// How many recently used formats are kept.
const RECENT: usize = 12;

/// A first video's formats, in the spec's order.
const VIDEO_ORDER: [FormatId; 6] = [
    FormatId::Mp4,
    FormatId::Gif,
    FormatId::Mp3,
    FormatId::Webm,
    FormatId::Mov,
    FormatId::M4a,
];

/// The formats Convert can make from every one of `files`, at most
/// [`SEGMENTS`]: `recent` first (most recent first), then, when the first
/// file is a video, the spec's video order, then the registry's order for
/// the first file. Convert never makes a file's own format.
pub fn formats(reg: &Registry, files: &[FormatId], recent: &[FormatId]) -> Vec<FormatId> {
    let Some(&first) = files.first() else {
        return Vec::new();
    };
    let own = reg.targets(Operation::Convert, first);
    let shared = |t: &FormatId| {
        files
            .iter()
            .all(|&f| reg.targets(Operation::Convert, f).contains(t))
    };
    let video = reg.format(first).kind == MediaKind::Video;
    let preferred = if video { &VIDEO_ORDER[..] } else { &[] };
    let mut out: Vec<FormatId> = Vec::new();
    for t in recent.iter().chain(preferred).chain(&own) {
        if out.len() == SEGMENTS {
            break;
        }
        if !out.contains(t) && shared(t) {
            out.push(*t);
        }
    }
    out
}

/// The hub's text: the files' kind and count, such as `"MOV · 1 FILE"`,
/// `"IMAGES · 3 FILES"` or `"MIXED · 4 FILES"`.
pub fn hub(reg: &Registry, files: &[FormatId]) -> String {
    let count = match files.len() {
        1 => "1 FILE".to_string(),
        n => format!("{n} FILES"),
    };
    let Some(&first) = files.first() else {
        return count;
    };
    let kind = reg.format(first).kind;
    let what = if files.iter().all(|&f| f == first) {
        reg.format(first).name.to_uppercase()
    } else if files.iter().all(|&f| reg.format(f).kind == kind) {
        match kind {
            MediaKind::Image => "IMAGES",
            MediaKind::Video => "VIDEOS",
            MediaKind::Audio => "AUDIO",
            MediaKind::Document => "DOCUMENTS",
            MediaKind::Archive => "ARCHIVES",
        }
        .to_string()
    } else {
        "MIXED".to_string()
    };
    format!("{what} · {count}")
}

/// Where a point lands on the wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Hub,
    /// The segment's index: 0 is centred at 12 o'clock, then clockwise.
    Format(usize),
    Outside,
}

/// Where `(x, y)`, in points from the wheel's top-left, lands.
pub fn hit(x: f64, y: f64) -> Hit {
    let (dx, dy) = (x - SIZE / 2.0, y - SIZE / 2.0);
    let r = dx.hypot(dy);
    if r <= HUB {
        return Hit::Hub;
    }
    if r > SIZE / 2.0 {
        return Hit::Outside;
    }
    // Clockwise from 12 o'clock (screen y grows down), in [0, 360).
    let deg = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
    let step = 360.0 / SEGMENTS as f64;
    Hit::Format(((deg + step / 2.0) / step) as usize % SEGMENTS)
}

/// A segment's hover text: the format's name and what Persimmon writes
/// into it, such as `"MPEG-4 · H.264"`. Images, PDF and archives give
/// their name alone.
pub fn label(id: FormatId) -> String {
    use FormatId::*;
    let (name, codec) = match id {
        Mp4 => ("MPEG-4", "H.264"),
        Mov => ("QuickTime", "H.264"),
        Webm => ("WebM", "VP9"),
        Gif => ("GIF", "animated"),
        Mp3 => ("MP3", "audio"),
        M4a => ("MPEG-4 audio", "AAC"),
        Wav => ("WAV", "PCM"),
        Flac => ("FLAC", "lossless"),
        Opus => ("Opus", "audio"),
        other => return Registry::builtin().format(other).name.to_string(),
    };
    format!("{name} · {codec}")
}

/// Whether a ⇧-drag carries anything the wheel can take.
pub fn should_show(files: usize, promises: usize) -> bool {
    files + promises > 0
}

/// Note that `used` was picked: it moves to the front, and at most
/// twelve are kept.
pub fn remember(recent: &mut Vec<FormatId>, used: FormatId) {
    recent.retain(|&f| f != used);
    recent.insert(0, used);
    recent.truncate(RECENT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use persimmon_core::format::FormatId::{self, *};
    use persimmon_core::registry::Registry;

    #[test]
    fn a_first_video_gets_the_spec_order() {
        let reg = Registry::builtin();
        assert_eq!(formats(&reg, &[Mkv], &[]), [Mp4, Gif, Mp3, Webm, Mov, M4a]);
        // Convert can't make a file's own format: a MOV's wheel skips MOV,
        // and the registry's order fills the sixth segment.
        assert_eq!(formats(&reg, &[Mov], &[]), [Mp4, Gif, Mp3, Webm, M4a, Wav]);
    }

    #[test]
    fn other_kinds_follow_the_registry() {
        let reg = Registry::builtin();
        assert_eq!(
            formats(&reg, &[Png], &[]),
            reg.targets(persimmon_core::registry::Operation::Convert, Png)
        );
    }

    #[test]
    fn recent_formats_come_first() {
        let reg = Registry::builtin();
        // Recent formats the files can't become are skipped.
        assert_eq!(
            formats(&reg, &[Mkv], &[Flac, Jpeg, Webm]),
            [Flac, Webm, Mp4, Gif, Mp3, Mov]
        );
    }

    #[test]
    fn files_with_nothing_in_common_get_no_formats() {
        let reg = Registry::builtin();
        assert_eq!(formats(&reg, &[Pdf, Mp3], &[]), Vec::<FormatId>::new());
        assert_eq!(formats(&reg, &[], &[Mp4]), Vec::<FormatId>::new());
    }

    #[test]
    fn formats_are_shared_by_every_file() {
        let reg = Registry::builtin();
        // A PNG and a PDF can both become JPEG and PNG, but a PNG can't
        // become a PNG.
        assert_eq!(formats(&reg, &[Png, Pdf], &[]), [Jpeg]);
        assert!(formats(&reg, &[Mov, Mp3, Wav], &[]).len() <= SEGMENTS);
    }

    #[test]
    fn the_hub_names_kind_and_count() {
        let reg = Registry::builtin();
        assert_eq!(hub(&reg, &[Mov]), "MOV · 1 FILE");
        assert_eq!(hub(&reg, &[Png, Png, Png]), "PNG · 3 FILES");
        assert_eq!(hub(&reg, &[Webm]), "WEBM · 1 FILE");
        assert_eq!(hub(&reg, &[Png, Jpeg, Heic]), "IMAGES · 3 FILES");
        assert_eq!(hub(&reg, &[Mov, Mp4]), "VIDEOS · 2 FILES");
        assert_eq!(hub(&reg, &[Mov, Png, Mp3, Pdf]), "MIXED · 4 FILES");
    }

    #[test]
    fn hit_finds_hub_segments_and_outside() {
        assert_eq!(hit(160.0, 160.0), Hit::Hub);
        assert_eq!(hit(160.0, 40.0), Hit::Format(0));
        assert_eq!(hit(264.0, 100.0), Hit::Format(1));
        assert_eq!(hit(160.0, 280.0), Hit::Format(3));
        assert_eq!(hit(56.0, 100.0), Hit::Format(5));
        assert_eq!(hit(5.0, 5.0), Hit::Outside);
        // The hub's edge, and the wheel's.
        assert_eq!(hit(160.0, 160.0 - HUB + 1.0), Hit::Hub);
        assert_eq!(hit(160.0, 160.0 - HUB - 1.0), Hit::Format(0));
        assert_eq!(hit(160.0, 1.0), Hit::Format(0));
        assert_eq!(hit(160.0, -1.0), Hit::Outside);
        // Just either side of the boundary between segments 0 and 1, at 30°.
        let at = |deg: f64| {
            let r = 120.0;
            let a = deg.to_radians();
            hit(160.0 + r * a.sin(), 160.0 - r * a.cos())
        };
        assert_eq!(at(29.0), Hit::Format(0));
        assert_eq!(at(31.0), Hit::Format(1));
        assert_eq!(at(331.0), Hit::Format(0));
        assert_eq!(at(329.0), Hit::Format(5));
    }

    #[test]
    fn labels_name_format_and_codec() {
        assert_eq!(label(Mp4), "MPEG-4 · H.264");
        assert_eq!(label(Mov), "QuickTime · H.264");
        assert_eq!(label(Webm), "WebM · VP9");
        assert_eq!(label(Gif), "GIF · animated");
        assert_eq!(label(Mp3), "MP3 · audio");
        assert_eq!(label(M4a), "MPEG-4 audio · AAC");
        assert_eq!(label(Png), "PNG");
        assert_eq!(label(Pdf), "PDF");
    }

    #[test]
    fn the_wheel_shows_only_for_files_or_promises() {
        assert!(!should_show(0, 0));
        assert!(should_show(1, 0));
        assert!(should_show(0, 2));
    }

    #[test]
    fn remember_moves_to_front_and_caps() {
        let mut recent = vec![Mp4, Gif, Webm];
        remember(&mut recent, Webm);
        assert_eq!(recent, [Webm, Mp4, Gif]);
        remember(&mut recent, Png);
        assert_eq!(recent, [Png, Webm, Mp4, Gif]);
        let mut many: Vec<FormatId> = Registry::builtin()
            .formats()
            .iter()
            .map(|f| f.id)
            .take(12)
            .collect();
        remember(&mut many, Rar);
        assert_eq!(many.len(), 12);
        assert_eq!(many[0], Rar);
    }
}
