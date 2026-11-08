// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! What's inside a media file, from `ffprobe -of flat`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use super::{Ffmpeg, child, file_url, message};
use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;

/// The entries `ffprobe` is asked for.
const ENTRIES: &str = "format=duration:stream=index,codec_type,codec_name,codec_tag_string,width,height,pix_fmt,avg_frame_rate,channels,sample_rate,bits_per_raw_sample:stream_disposition=attached_pic";

/// A media file's duration and streams.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaInfo {
    /// Seconds, when the container knows.
    pub duration: Option<f64>,
    pub streams: Vec<Stream>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    #[default]
    Other,
}

/// One stream, as `ffprobe` reports it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stream {
    pub index: u32,
    pub kind: StreamKind,
    /// FFmpeg's codec name, e.g. `h264`, `aac`, `pcm_s24le`.
    pub codec: String,
    /// The container's codec tag, e.g. `hvc1`; `None` when there is none.
    pub tag: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f64>,
    pub pix_fmt: Option<String>,
    pub channels: Option<u32>,
    pub sample_rate: Option<u32>,
    /// Bits per sample, where the codec records it.
    pub bits: Option<u32>,
    /// Cover art: a still image stored as a video stream.
    pub attached_pic: bool,
}

impl MediaInfo {
    /// The first video stream that isn't cover art.
    pub fn video(&self) -> Option<&Stream> {
        self.streams
            .iter()
            .find(|s| s.kind == StreamKind::Video && !s.attached_pic)
    }

    /// Every audio stream, in order.
    pub fn audio(&self) -> Vec<&Stream> {
        self.streams
            .iter()
            .filter(|s| s.kind == StreamKind::Audio)
            .collect()
    }

    /// The first cover art stream.
    pub fn cover(&self) -> Option<&Stream> {
        self.streams
            .iter()
            .find(|s| s.kind == StreamKind::Video && s.attached_pic)
    }
}

impl Stream {
    /// Whether the pixels have more than 8 bits per component (10 or 12).
    pub fn ten_bit(&self) -> bool {
        self.pix_fmt.as_deref().is_some_and(|f| {
            ["10le", "10be", "12le", "12be"]
                .iter()
                .any(|s| f.ends_with(s))
        })
    }
}

impl Ffmpeg {
    /// Probe `path`. A file `ffprobe` can't read is `CORRUPT_INPUT`.
    pub fn probe(&self, path: &Path) -> Result<MediaInfo, Error> {
        self.probe_with(path, &CancelToken::new())
    }

    /// [`Ffmpeg::probe`], where an ffprobe stopped by a cancel (a
    /// terminal's Ctrl-C reaches it too) is `CANCELLED`.
    pub fn probe_with(&self, path: &Path, cancel: &CancelToken) -> Result<MediaInfo, Error> {
        let out = Command::new(&self.ffprobe)
            .args(["-v", "error", "-show_entries", ENTRIES, "-of", "flat"])
            .arg(file_url(path))
            .stdin(Stdio::null())
            .output()
            .map_err(|e| {
                Error::new(
                    ErrorCode::BackendMissing,
                    format!("{} didn't run: {e}", self.ffprobe.display()),
                )
                .with_file(path)
                .with_fix(crate::platform::ffmpeg_install())
            })?;
        if !out.status.success() {
            if child::was_cancelled(Some(&out.status), cancel) {
                return Err(Error::new(ErrorCode::Cancelled, "cancelled").with_file(path));
            }
            let reason = if out.status.code().is_none() {
                child::stopped("ffprobe", Some(&out.status))
            } else {
                let why = message(&String::from_utf8_lossy(&out.stderr), &[path]);
                if why.is_empty() {
                    format!("ffmpeg can't read it: ffprobe exited with {}", out.status)
                } else {
                    format!("ffmpeg can't read it: {why}")
                }
            };
            return Err(Error::new(ErrorCode::CorruptInput, reason).with_file(path));
        }
        Ok(parse_flat(&String::from_utf8_lossy(&out.stdout)))
    }
}

/// Parse `ffprobe -of flat` output. `N/A`, `0/0` and a zero bit depth read
/// as unknown.
pub fn parse_flat(text: &str) -> MediaInfo {
    let mut duration = None;
    let mut streams: BTreeMap<u32, Stream> = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        let known = (value != "N/A" && !value.is_empty()).then_some(value);
        if key == "format.duration" {
            duration = known.and_then(|v| v.parse().ok());
            continue;
        }
        let Some(rest) = key.strip_prefix("streams.stream.") else {
            continue;
        };
        let Some((n, field)) = rest.split_once('.') else {
            continue;
        };
        let Ok(n) = n.parse::<u32>() else {
            continue;
        };
        let s = streams.entry(n).or_default();
        let number = || known.and_then(|v| v.parse::<u32>().ok());
        match field {
            "index" => s.index = number().unwrap_or(n),
            "codec_name" => s.codec = value.to_string(),
            "codec_type" => {
                s.kind = match value {
                    "video" => StreamKind::Video,
                    "audio" => StreamKind::Audio,
                    _ => StreamKind::Other,
                }
            }
            "codec_tag_string" => s.tag = known.filter(|t| !t.starts_with("[0]")).map(String::from),
            "width" => s.width = number(),
            "height" => s.height = number(),
            "pix_fmt" => s.pix_fmt = known.map(String::from),
            "avg_frame_rate" => s.frame_rate = known.and_then(rate),
            "channels" => s.channels = number(),
            "sample_rate" => s.sample_rate = number(),
            "bits_per_raw_sample" => s.bits = number().filter(|&b| b > 0),
            "disposition.attached_pic" => s.attached_pic = value == "1",
            _ => {}
        }
    }
    MediaInfo {
        duration,
        streams: streams.into_values().collect(),
    }
}

/// `"30000/1001"` → 29.97; `"0/0"` → `None`.
fn rate(v: &str) -> Option<f64> {
    let (a, b) = v.split_once('/').unwrap_or((v, "1"));
    let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
    (a > 0.0 && b > 0.0).then(|| a / b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use persimmon_corpus::media::{media_sample, test_ffmpeg};

    const FLAT: &str = r#"streams.stream.0.index=0
streams.stream.0.codec_name="h264"
streams.stream.0.codec_type="video"
streams.stream.0.codec_tag_string="avc1"
streams.stream.0.width=320
streams.stream.0.height=240
streams.stream.0.pix_fmt="yuv420p"
streams.stream.0.avg_frame_rate="25/1"
streams.stream.0.bits_per_raw_sample="8"
streams.stream.0.disposition.attached_pic=0
streams.stream.1.index=1
streams.stream.1.codec_name="aac"
streams.stream.1.codec_type="audio"
streams.stream.1.codec_tag_string="mp4a"
streams.stream.1.sample_rate="48000"
streams.stream.1.channels=2
streams.stream.1.avg_frame_rate="0/0"
streams.stream.1.bits_per_raw_sample="N/A"
streams.stream.1.disposition.attached_pic=0
streams.stream.2.index=2
streams.stream.2.codec_name="png"
streams.stream.2.codec_type="video"
streams.stream.2.codec_tag_string="[0][0][0][0]"
streams.stream.2.width=64
streams.stream.2.height=64
streams.stream.2.avg_frame_rate="0/0"
streams.stream.2.disposition.attached_pic=1
format.duration="2.000000"
"#;

    #[test]
    fn parse_flat_reads_streams() {
        let info = parse_flat(FLAT);
        assert_eq!(info.duration, Some(2.0));
        assert_eq!(info.streams.len(), 3);
        let v = info.video().unwrap();
        assert_eq!(
            (v.index, v.kind, v.codec.as_str()),
            (0, StreamKind::Video, "h264")
        );
        assert_eq!((v.width, v.height), (Some(320), Some(240)));
        assert_eq!(v.frame_rate, Some(25.0));
        assert_eq!(v.tag.as_deref(), Some("avc1"));
        assert_eq!(v.bits, Some(8));
        let a = info.audio();
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].channels, a[0].sample_rate), (Some(2), Some(48000)));
        assert_eq!(a[0].frame_rate, None);
        assert_eq!(a[0].bits, None);
        let cover = info.cover().unwrap();
        assert_eq!((cover.index, cover.codec.as_str()), (2, "png"));
        assert_eq!(cover.tag, None);
        assert!(!v.ten_bit());
    }

    #[test]
    fn ten_bit_formats() {
        for (fmt, ten) in [
            ("yuv420p10le", true),
            ("yuv422p10be", true),
            ("yuv420p12le", true),
            ("yuv420p", false),
            ("p010le", true),
            ("nv12", false),
        ] {
            let s = Stream {
                pix_fmt: Some(fmt.into()),
                ..Stream::default()
            };
            assert_eq!(s.ten_bit(), ten, "{fmt}");
        }
    }

    #[test]
    fn probe_samples() {
        let Some(ff) = test_ffmpeg() else {
            eprintln!("skipped: no ffmpeg");
            return;
        };
        let ff = Ffmpeg::locate(Some(&ff)).unwrap();
        let hevc = ff.probe(&media_sample("vid-hevc").unwrap()).unwrap();
        let v = hevc.video().unwrap();
        assert_eq!(v.codec, "hevc");
        assert_eq!(v.tag.as_deref(), Some("hvc1"));
        assert!(v.ten_bit());
        let deep = ff.probe(&media_sample("aud-flac-24").unwrap()).unwrap();
        assert_eq!(deep.audio()[0].bits, Some(24));
        let cover = ff.probe(&media_sample("aud-cover").unwrap()).unwrap();
        assert!(cover.cover().is_some());
        assert!(cover.video().is_none());
        let truncated = media_sample("vid-truncated").unwrap();
        let e = ff.probe(&truncated).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert_eq!(e.file.as_deref(), Some(truncated.as_path()));
        assert!(
            e.reason.starts_with("ffmpeg can't read it: "),
            "{}",
            e.reason
        );
        assert!(!e.reason.contains("prsm-media"), "{}", e.reason);
    }
}
