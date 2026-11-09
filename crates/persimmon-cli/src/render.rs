// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! How `prsm` prints things: sizes, paths, `formats` and `info`.

use std::path::Path;

use persimmon_core::backend::ffmpeg::{MediaInfo, Stream};
use persimmon_core::backend::pdf::PdfInfo;
use persimmon_core::detect::Detected;
use persimmon_core::format::FormatId;
use persimmon_core::registry::{Operation, Registry};

/// A byte count in decimal units, as Finder shows it: `999 B`, `1.5 KB`,
/// `4.3 GB`.
pub fn human_size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    // 999.95 would print as "1000.0": move up a unit instead.
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// A path as people expect to read it: without a leading `./`.
pub fn show(path: &Path) -> String {
    path.strip_prefix(".").unwrap_or(path).display().to_string()
}

/// `prsm formats`: one section per operation, one line per source format.
pub fn formats(reg: &Registry) -> String {
    let mut out = String::new();
    for op in [
        Operation::Convert,
        Operation::Compress,
        Operation::Extract,
        Operation::Merge,
        Operation::Split,
    ] {
        out.push_str(op.as_str());
        out.push('\n');
        for f in reg.formats() {
            if !reg
                .capabilities()
                .iter()
                .any(|c| c.op == op && c.from == f.id)
            {
                continue;
            }
            let targets = match op {
                Operation::Extract => "folder".to_string(),
                // Compress keeps the format.
                Operation::Compress => f.id.as_str().to_string(),
                _ => ids(&reg.targets(op, f.id)),
            };
            out.push_str(&format!("  {:<8}→ {targets}\n", f.id.as_str()));
        }
    }
    out
}

/// `prsm info`: label/value rows. `media` rows (from [`media_rows`]) go
/// after the size.
pub fn info(
    reg: &Registry,
    path: &Path,
    detected: Detected,
    bytes: u64,
    media: &[(&'static str, String)],
) -> String {
    let format = reg.format(detected.format);
    let mut becomes = ids(&reg.targets(Operation::Convert, detected.format));
    if becomes.is_empty() {
        becomes = if reg
            .find(Operation::Extract, detected.format, None)
            .is_some()
        {
            "a folder (prsm extract)".to_string()
        } else {
            "—".to_string()
        };
    }
    let mut rows = vec![
        ("file", show(path)),
        (
            "format",
            format!("{} ({})", format.name, format.id.as_str()),
        ),
        ("size", format!("{} ({bytes} bytes)", human_size(bytes))),
    ];
    rows.extend(media.iter().cloned());
    rows.push(("becomes", becomes));
    if detected.extension_mismatch {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        rows.push((
            "note",
            format!(
                "the extension says .{ext} but the content is {}",
                format.name
            ),
        ));
    }
    rows.iter()
        .map(|(label, value)| format!("{label:<9}{value}\n"))
        .collect()
}

/// Paper sizes `info` names, portrait, in points.
const PAPER: [(&str, f32, f32); 5] = [
    ("A3", 842.0, 1191.0),
    ("A4", 595.0, 842.0),
    ("A5", 420.0, 595.0),
    ("Letter", 612.0, 792.0),
    ("Legal", 612.0, 1008.0),
];

/// `info`'s rows for a PDF: the page count and page 1's size, in
/// millimetres, named when it's a common paper size.
pub fn pdf_rows(info: &PdfInfo) -> Vec<(&'static str, String)> {
    let (w, h) = info.first_page;
    let mm = |pt: f32| (pt * 25.4 / 72.0).round() as u32;
    let mut size = format!("{}×{} mm", mm(w), mm(h));
    let near = |a: f32, b: f32| (a - b).abs() <= 3.0;
    if let Some((name, ..)) = PAPER
        .iter()
        .find(|(_, pw, ph)| (near(w, *pw) && near(h, *ph)) || (near(w, *ph) && near(h, *pw)))
    {
        let landscape = if w > h { " landscape" } else { "" };
        size.push_str(&format!(" ({name}{landscape})"));
    }
    vec![("pages", info.pages.to_string()), ("page", size)]
}

/// `info`'s rows for a probed audio or video file: duration, the video
/// stream, the audio streams and any cover art.
pub fn media_rows(info: &MediaInfo) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let Some(d) = info.duration {
        rows.push(("duration", fmt_duration(d)));
    }
    if let Some(v) = info.video() {
        let mut parts = vec![codec_label(&v.codec)];
        parts.extend(size(v));
        parts.extend(v.frame_rate.map(|r| format!("{} fps", trim(r))));
        rows.push(("video", parts.join(", ")));
    }
    let audio = info.audio();
    if let Some(a) = audio.first() {
        let mut parts = vec![codec_label(&a.codec)];
        parts.extend(a.channels.map(channels_label));
        parts.extend(
            a.sample_rate
                .map(|r| format!("{} kHz", trim(f64::from(r) / 1000.0))),
        );
        let mut value = parts.join(", ");
        if audio.len() > 1 {
            value.push_str(&format!(" (+{} more)", audio.len() - 1));
        }
        rows.push(("audio", value));
    }
    if let Some(c) = info.cover() {
        let label = match c.codec.as_str() {
            "mjpeg" => "JPEG".to_string(),
            other => codec_label(other),
        };
        let mut parts = vec![label];
        parts.extend(size(c));
        rows.push(("cover", parts.join(", ")));
    }
    rows
}

fn size(s: &Stream) -> Option<String> {
    Some(format!("{}×{}", s.width?, s.height?))
}

/// At most two decimals, trailing zeros trimmed: `25`, `29.97`, `44.1`.
fn trim(x: f64) -> String {
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Seconds as `m:ss`, or `h:mm:ss` from an hour, rounded to the second.
pub fn fmt_duration(secs: f64) -> String {
    let total = secs.round() as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A codec as people know it: `h264` → `H.264`, `pcm_s24le` → `PCM 24-bit`.
pub fn codec_label(codec: &str) -> String {
    let known = match codec {
        "h264" => "H.264",
        "hevc" => "HEVC",
        "vp9" => "VP9",
        "vp8" => "VP8",
        "av1" => "AV1",
        "prores" => "ProRes",
        "mpeg4" => "MPEG-4",
        "mjpeg" => "Motion JPEG",
        "ffv1" => "FFV1",
        "png" => "PNG",
        "gif" => "GIF",
        "aac" => "AAC",
        "mp3" => "MP3",
        "opus" => "Opus",
        "vorbis" => "Vorbis",
        "flac" => "FLAC",
        "alac" => "ALAC",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    if let Some(rest) = codec.strip_prefix("pcm_") {
        let bits: String = rest.chars().filter(char::is_ascii_digit).collect();
        if !bits.is_empty() {
            return format!("PCM {bits}-bit");
        }
    }
    codec.to_string()
}

/// `1` → `mono`, `2` → `stereo`, `6` → `5.1`, `8` → `7.1`.
pub fn channels_label(n: u32) -> String {
    match n {
        1 => "mono".into(),
        2 => "stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n} channels"),
    }
}

fn ids(formats: &[FormatId]) -> String {
    formats
        .iter()
        .map(|f| f.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use persimmon_core::backend::ffmpeg::StreamKind;

    #[test]
    fn pdf_rows_name_paper_sizes() {
        let rows = |w, h| {
            pdf_rows(&PdfInfo {
                pages: 3,
                first_page: (w, h),
            })
        };
        assert_eq!(
            rows(595.0, 842.0),
            vec![
                ("pages", "3".to_string()),
                ("page", "210×297 mm (A4)".to_string())
            ]
        );
        assert_eq!(rows(792.0, 612.0)[1].1, "279×216 mm (Letter landscape)");
        assert_eq!(rows(100.0, 100.0)[1].1, "35×35 mm");
    }

    #[test]
    fn human_size_values() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1000), "1.0 KB");
        assert_eq!(human_size(1500), "1.5 KB");
        assert_eq!(human_size(999_960), "1.0 MB");
        assert_eq!(human_size(4_294_967_296), "4.3 GB");
    }

    #[test]
    fn show_drops_leading_dot() {
        assert_eq!(show(Path::new("./a.webp")), "a.webp");
        assert_eq!(show(Path::new("../a.webp")), "../a.webp");
    }

    #[test]
    fn fmt_duration_values() {
        assert_eq!(fmt_duration(2.0), "0:02");
        assert_eq!(fmt_duration(1.6), "0:02");
        assert_eq!(fmt_duration(65.4), "1:05");
        assert_eq!(fmt_duration(3725.0), "1:02:05");
        assert_eq!(fmt_duration(0.024), "0:00");
    }

    fn stream(kind: StreamKind, codec: &str) -> Stream {
        Stream {
            kind,
            codec: codec.into(),
            ..Stream::default()
        }
    }

    #[test]
    fn media_rows_for_a_clip() {
        let info = MediaInfo {
            duration: Some(2.0),
            streams: vec![
                Stream {
                    width: Some(320),
                    height: Some(240),
                    frame_rate: Some(25.0),
                    ..stream(StreamKind::Video, "h264")
                },
                Stream {
                    channels: Some(2),
                    sample_rate: Some(48000),
                    ..stream(StreamKind::Audio, "aac")
                },
            ],
        };
        assert_eq!(
            media_rows(&info),
            vec![
                ("duration", "0:02".to_string()),
                ("video", "H.264, 320×240, 25 fps".to_string()),
                ("audio", "AAC, stereo, 48 kHz".to_string()),
            ]
        );
    }

    #[test]
    fn media_rows_details() {
        let info = MediaInfo {
            duration: None,
            streams: vec![
                Stream {
                    channels: Some(6),
                    sample_rate: Some(44100),
                    ..stream(StreamKind::Audio, "pcm_s24le")
                },
                Stream {
                    channels: Some(1),
                    ..stream(StreamKind::Audio, "opus")
                },
                Stream {
                    width: Some(64),
                    height: Some(64),
                    attached_pic: true,
                    ..stream(StreamKind::Video, "mjpeg")
                },
                Stream {
                    frame_rate: Some(30000.0 / 1001.0),
                    ..stream(StreamKind::Video, "prores")
                },
            ],
        };
        assert_eq!(
            media_rows(&info),
            vec![
                ("video", "ProRes, 29.97 fps".to_string()),
                ("audio", "PCM 24-bit, 5.1, 44.1 kHz (+1 more)".to_string()),
                ("cover", "JPEG, 64×64".to_string()),
            ]
        );
        assert_eq!(codec_label("weird_codec"), "weird_codec");
        assert_eq!(channels_label(8), "7.1");
        assert_eq!(channels_label(3), "3 channels");
    }
}
