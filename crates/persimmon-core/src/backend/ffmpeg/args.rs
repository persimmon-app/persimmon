// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The FFmpeg command line for one task. Pure: every choice about codecs,
//! quality, streams and metadata (DD-35) is made here, from the probed
//! input, and tested without FFmpeg.

use std::ffi::OsString;
use std::path::Path;

use super::{MediaInfo, Stream, file_url};
use crate::error::{Error, ErrorCode};
use crate::format::FormatId;
use crate::job::{JobOptions, PlannedTask, Preset, Quality};

/// A video codec prsm encodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Video {
    H264,
    Hevc,
    Vp9,
    Av1,
}

impl Video {
    /// FFmpeg's decoder name for streams already in this codec.
    fn codec(self) -> &'static str {
        match self {
            Video::H264 => "h264",
            Video::Hevc => "hevc",
            Video::Vp9 => "vp9",
            Video::Av1 => "av1",
        }
    }

    /// The CRF for each quality level, low to max (DD-35).
    fn crf(self, q: Quality) -> u32 {
        let table = match self {
            Video::H264 => [28, 25, 22, 18],
            Video::Hevc => [30, 28, 25, 21],
            Video::Vp9 => [40, 36, 32, 24],
            Video::Av1 => [40, 35, 30, 24],
        };
        table[level(q)]
    }

    /// 10-bit sources stay 10-bit only in HEVC and AV1; H.264 and VP9
    /// are always 8-bit 4:2:0, which plays anywhere.
    fn keeps_ten_bit(self) -> bool {
        matches!(self, Video::Hevc | Video::Av1)
    }
}

fn level(q: Quality) -> usize {
    match q {
        Quality::Low => 0,
        Quality::Medium => 1,
        Quality::High => 2,
        Quality::Max => 3,
    }
}

/// Builds an argument list, checking encoders as they're chosen.
struct Args<'a> {
    v: Vec<OsString>,
    input: &'a Path,
    has_encoder: &'a dyn Fn(&str) -> bool,
}

impl Args<'_> {
    fn push(&mut self, words: &str) {
        self.v.extend(words.split_whitespace().map(OsString::from));
    }

    fn arg(&mut self, word: impl Into<OsString>) {
        self.v.push(word.into());
    }

    /// Fail with `BACKEND_MISSING` unless this FFmpeg has `encoder`.
    fn need(&self, encoder: &str, label: &str) -> Result<(), Error> {
        if (self.has_encoder)(encoder) {
            return Ok(());
        }
        Err(Error::new(
            ErrorCode::BackendMissing,
            format!("this ffmpeg can't encode {label} (no {encoder})"),
        )
        .with_file(self.input)
        .with_fix(crate::platform::ffmpeg_install()))
    }
}

/// The FFmpeg arguments that turn `task`'s input (probed as `info`) into
/// its output at `temp`. Compress is `task.to == None`.
pub(crate) fn build_args(
    task: &PlannedTask,
    info: &MediaInfo,
    options: &JobOptions,
    temp: &Path,
    has_encoder: &dyn Fn(&str) -> bool,
) -> Result<Vec<OsString>, Error> {
    use FormatId::*;
    let input = task.inputs[0].as_path();
    let out = task.to.unwrap_or(task.from[0]);
    let mut a = Args {
        v: Vec::new(),
        input,
        has_encoder,
    };
    a.push("-nostdin -hide_banner -loglevel error -nostats -progress pipe:1 -y -i");
    a.arg(file_url(input));
    match out {
        Mp4 | Mov | Webm => video_target(&mut a, task, info, options, out)?,
        Gif => gif_target(&mut a, info, options)?,
        _ => audio_target(&mut a, info, options, out)?,
    }
    if options.strip_metadata {
        a.push("-map_metadata -1 -map_chapters -1");
    } else {
        a.push("-map_metadata 0");
    }
    if matches!(out, Mp4 | Mov | M4a) {
        a.push("-movflags +faststart");
    }
    let muxer = match out {
        Mp4 => "mp4",
        Mov => "mov",
        Webm => "webm",
        Gif => "gif",
        Mp3 => "mp3",
        M4a => "ipod",
        Wav => "wav",
        Flac => "flac",
        Opus => "opus",
        other => unreachable!("{other:?} isn't an FFmpeg output"),
    };
    a.push("-f");
    a.arg(muxer);
    a.arg(file_url(temp));
    Ok(a.v)
}

fn no_video(input: &Path) -> Error {
    Error::new(ErrorCode::UnsupportedInput, "there's no video track")
        .with_file(input)
        .with_fix("convert it to an audio format, such as --to m4a")
}

fn video_target(
    a: &mut Args,
    task: &PlannedTask,
    info: &MediaInfo,
    options: &JobOptions,
    out: FormatId,
) -> Result<(), Error> {
    let v = info.video().ok_or_else(|| no_video(a.input))?;
    let webm = out == FormatId::Webm;
    let takes = |codec: &str| {
        if webm {
            matches!(codec, "vp9" | "av1")
        } else {
            matches!(codec, "h264" | "hevc")
        }
    };
    let compress = task.to.is_none();
    let reencode = compress
        || options.preset.is_some()
        || options.quality.is_some()
        || options.max_size.is_some();
    a.push(&format!("-map 0:{}", v.index));
    for s in info.audio() {
        a.push(&format!("-map 0:{}", s.index));
    }
    if !reencode && takes(&v.codec) {
        a.push("-c:v copy");
        if v.codec == "hevc" {
            a.push("-tag:v hvc1");
        }
    } else {
        let codec = match (options.preset, webm) {
            (Some(Preset::Small), false) => Video::Hevc,
            (Some(Preset::Small), true) => Video::Av1,
            (Some(Preset::Compatible), false) => Video::H264,
            (Some(Preset::Compatible), true) => Video::Vp9,
            // No preset: keep the source's codec where the container
            // takes it, else the compatible one.
            (None, _) => [Video::H264, Video::Hevc, Video::Vp9, Video::Av1]
                .into_iter()
                .find(|c| c.codec() == v.codec && takes(c.codec()))
                .unwrap_or(if webm { Video::Vp9 } else { Video::H264 }),
        };
        encode_video(a, v, codec, options)?;
    }
    let (target, encoder, label) = if webm {
        ("opus", "libopus", "Opus")
    } else {
        ("aac", "aac", "AAC")
    };
    let q = level(options.quality.unwrap_or(Quality::High));
    let bitrate = if webm {
        ["64k", "96k", "128k", "192k"][q]
    } else {
        ["96k", "128k", "192k", "256k"][q]
    };
    for (i, s) in info.audio().iter().enumerate() {
        if s.codec == target || (webm && s.codec == "vorbis") {
            a.push(&format!("-c:a:{i} copy"));
        } else {
            a.need(encoder, label)?;
            a.push(&format!("-c:a:{i} {encoder} -b:a:{i} {bitrate}"));
            if webm {
                a.push(&format!("-filter:a:{i} {OPUS_LAYOUTS}"));
            }
        }
    }
    Ok(())
}

fn encode_video(a: &mut Args, v: &Stream, codec: Video, options: &JobOptions) -> Result<(), Error> {
    let crf = codec.crf(options.quality.unwrap_or(Quality::High));
    match codec {
        Video::H264 => {
            a.need("libx264", "H.264")?;
            a.push(&format!("-c:v libx264 -crf {crf} -preset medium"));
        }
        Video::Hevc => {
            a.need("libx265", "HEVC")?;
            a.push(&format!(
                "-c:v libx265 -crf {crf} -preset medium -x265-params log-level=error -tag:v hvc1"
            ));
        }
        Video::Vp9 => {
            a.need("libvpx-vp9", "VP9")?;
            a.push(&format!(
                "-c:v libvpx-vp9 -crf {crf} -b:v 0 -row-mt 1 -cpu-used 4"
            ));
        }
        Video::Av1 => {
            if (a.has_encoder)("libsvtav1") {
                a.push(&format!("-c:v libsvtav1 -crf {crf} -preset 8"));
            } else if (a.has_encoder)("libaom-av1") {
                a.push(&format!("-c:v libaom-av1 -crf {crf} -cpu-used 6 -row-mt 1"));
            } else {
                a.need("libsvtav1", "AV1")?;
            }
        }
    }
    let mut chain = Vec::new();
    if let Some(n) = options.max_size {
        chain.push(format!(
            "scale='if(gte(iw,ih),min(iw,{n}),-2)':'if(gte(iw,ih),-2,min(ih,{n}))'"
        ));
    }
    // 4:2:0 needs even dimensions.
    chain.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".into());
    let pix = if codec.keeps_ten_bit() && v.ten_bit() {
        "yuv420p10le"
    } else {
        "yuv420p"
    };
    chain.push(format!("format={pix}"));
    a.push("-vf");
    a.arg(chain.join(","));
    Ok(())
}

/// A palette per frame (`stats_mode=single`, `new=1`), so memory stays flat
/// however long the video: one palette for the whole clip would hold every
/// frame until the end.
fn gif_target(a: &mut Args, info: &MediaInfo, options: &JobOptions) -> Result<(), Error> {
    let v = info.video().ok_or_else(|| no_video(a.input))?;
    a.need("gif", "GIF")?;
    let q = level(options.quality.unwrap_or(Quality::High));
    let fps = [10, 12, 15, 20][q];
    let cap = options.max_size.unwrap_or([320, 480, 640, 960][q]);
    a.push("-filter_complex");
    a.arg(format!(
        "[0:{}]fps={fps},scale='if(gte(iw,ih),min(iw,{cap}),-1)':'if(gte(iw,ih),-1,min(ih,{cap}))':flags=lanczos,split[a][b];[a]palettegen=stats_mode=single[p];[b][p]paletteuse=new=1:dither=bayer:bayer_scale=5[g]",
        v.index
    ));
    a.push("-map [g] -loop 0");
    Ok(())
}

/// libopus takes 5.1 and 7.1 but not 5.1(side), the layout AC-3, E-AC-3
/// and DTS surround decode to; this lets FFmpeg relabel such audio.
const OPUS_LAYOUTS: &str = "aformat=channel_layouts=7.1|5.1|stereo|mono";

/// Whether a codec loses nothing, so its bit depth is worth keeping.
fn lossless(codec: &str) -> bool {
    codec.starts_with("pcm_")
        || matches!(
            codec,
            "flac" | "alac" | "wavpack" | "ape" | "tta" | "truehd" | "mlp"
        )
}

fn audio_target(
    a: &mut Args,
    info: &MediaInfo,
    options: &JobOptions,
    out: FormatId,
) -> Result<(), Error> {
    use FormatId::*;
    let Some(s) = info.audio().first().copied() else {
        return Err(Error::new(
            ErrorCode::UnsupportedInput,
            "there's no audio track to convert",
        )
        .with_file(a.input)
        .with_fix("pick a video format, such as --to mp4"));
    };
    a.push(&format!("-map 0:{}", s.index));
    if let Some(c) = info.cover()
        && matches!(out, Mp3 | M4a | Flac)
        && !options.strip_metadata
    {
        a.push(&format!(
            "-map 0:{} -c:v copy -disposition:v:0 attached_pic",
            c.index
        ));
    }
    let q = options.quality.map(level);
    let same = |codec: &str| s.codec == codec && q.is_none();
    match out {
        Mp3 if same("mp3") => a.push("-c:a copy"),
        Mp3 => {
            a.need("libmp3lame", "MP3")?;
            a.push(&format!(
                "-c:a libmp3lame -q:a {}",
                ["7", "5", "2", "0"][q.unwrap_or(2)]
            ));
        }
        M4a if same("aac") => a.push("-c:a copy"),
        M4a => {
            a.need("aac", "AAC")?;
            a.push(&format!(
                "-c:a aac -b:a {}",
                ["96k", "128k", "192k", "256k"][q.unwrap_or(2)]
            ));
        }
        Opus if same("opus") => a.push("-c:a copy"),
        Opus => {
            a.need("libopus", "Opus")?;
            a.push(&format!(
                "-c:a libopus -b:a {}",
                ["64k", "96k", "128k", "192k"][q.unwrap_or(2)]
            ));
            a.push(&format!("-af {OPUS_LAYOUTS}"));
        }
        Flac if s.codec == "flac" => a.push("-c:a copy"),
        Flac => {
            a.need("flac", "FLAC")?;
            a.push("-c:a flac");
        }
        Wav => {
            let bits = s.bits.or_else(|| {
                let digits: String = s
                    .codec
                    .strip_prefix("pcm_")?
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect();
                digits.parse().ok()
            });
            let pcm = if lossless(&s.codec) && bits.is_some_and(|b| b > 16) {
                "pcm_s24le"
            } else {
                "pcm_s16le"
            };
            a.need(pcm, "WAV")?;
            a.push(&format!("-c:a {pcm}"));
        }
        other => unreachable!("{other:?} isn't an audio target"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ffmpeg::{MediaInfo, Stream, StreamKind};
    use crate::error::ErrorCode;
    use crate::format::FormatId::{self, *};
    use crate::job::{JobOptions, PlannedTask, Preset, Quality};
    use crate::registry::Backend;
    use std::path::{Path, PathBuf};

    fn video(index: u32, codec: &str, pix: &str) -> Stream {
        Stream {
            index,
            kind: StreamKind::Video,
            codec: codec.into(),
            width: Some(320),
            height: Some(240),
            pix_fmt: Some(pix.into()),
            ..Stream::default()
        }
    }

    fn audio(index: u32, codec: &str) -> Stream {
        Stream {
            index,
            kind: StreamKind::Audio,
            codec: codec.into(),
            channels: Some(2),
            sample_rate: Some(48000),
            ..Stream::default()
        }
    }

    fn clip(vcodec: &str, acodec: &str) -> MediaInfo {
        MediaInfo {
            duration: Some(2.0),
            streams: vec![video(0, vcodec, "yuv420p"), audio(1, acodec)],
        }
    }

    fn tone(codec: &str) -> MediaInfo {
        MediaInfo {
            duration: Some(2.0),
            streams: vec![audio(0, codec)],
        }
    }

    fn task(from: FormatId, to: Option<FormatId>) -> PlannedTask {
        PlannedTask {
            inputs: vec![PathBuf::from("/tmp/in.x")],
            from: vec![from],
            to,
            backend: Backend::Ffmpeg,
            output: PathBuf::from("/tmp/out.x"),
        }
    }

    const TEMP: &str = "/tmp/.prsm-1-0.part";

    fn build(t: &PlannedTask, info: &MediaInfo, o: &JobOptions) -> Result<String, Error> {
        build_with(t, info, o, &|_| true)
    }

    fn build_with(
        t: &PlannedTask,
        info: &MediaInfo,
        o: &JobOptions,
        has: &dyn Fn(&str) -> bool,
    ) -> Result<String, Error> {
        build_args(t, info, o, Path::new(TEMP), has).map(|v| {
            v.iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        })
    }

    fn args(from: FormatId, to: Option<FormatId>, info: &MediaInfo, o: &JobOptions) -> String {
        build(&task(from, to), info, o).unwrap()
    }

    fn q(quality: Quality) -> JobOptions {
        JobOptions {
            quality: Some(quality),
            ..JobOptions::default()
        }
    }

    fn preset(p: Preset) -> JobOptions {
        JobOptions {
            preset: Some(p),
            ..JobOptions::default()
        }
    }

    #[test]
    fn common_prefix_and_output() {
        let a = args(
            Mkv,
            Some(Mp4),
            &clip("h264", "opus"),
            &JobOptions::default(),
        );
        assert!(
            a.starts_with("-nostdin -hide_banner -loglevel error -nostats -progress pipe:1 -y -i file:/tmp/in.x "),
            "{a}"
        );
        assert!(a.ends_with(&format!("-f mp4 file:{TEMP}")), "{a}");
    }

    #[test]
    fn h264_mkv_to_mp4_copies_video() {
        let a = args(
            Mkv,
            Some(Mp4),
            &clip("h264", "opus"),
            &JobOptions::default(),
        );
        assert!(a.contains("-map 0:0 -map 0:1"), "{a}");
        assert!(a.contains("-c:v copy"), "{a}");
        assert!(!a.contains("-vf"), "{a}");
        assert!(a.contains("-c:a:0 aac -b:a:0 192k"), "{a}");
        assert!(!a.contains("libopus"), "{a}");
        assert!(a.contains("-map_metadata 0"), "{a}");
        assert!(a.contains("-movflags +faststart"), "{a}");
    }

    #[test]
    fn matching_audio_is_copied() {
        let a = args(Mov, Some(Mp4), &clip("h264", "aac"), &JobOptions::default());
        assert!(a.contains("-c:a:0 copy"), "{a}");
        let a = args(
            Mkv,
            Some(Webm),
            &clip("vp9", "opus"),
            &JobOptions::default(),
        );
        assert!(a.contains("-c:v copy"), "{a}");
        assert!(a.contains("-c:a:0 copy"), "{a}");
        assert!(!a.contains("faststart"), "{a}");
    }

    #[test]
    fn every_audio_track_is_kept_for_video() {
        let mut info = clip("h264", "aac");
        info.streams.push(audio(2, "ac3"));
        let a = args(Mkv, Some(Mp4), &info, &JobOptions::default());
        assert!(a.contains("-map 0:0 -map 0:1 -map 0:2"), "{a}");
        assert!(a.contains("-c:a:0 copy -c:a:1 aac -b:a:1 192k"), "{a}");
    }

    #[test]
    fn quality_forces_reencode() {
        let a = args(Mov, Some(Mp4), &clip("h264", "aac"), &q(Quality::High));
        assert!(a.contains("-c:v libx264 -crf 22 -preset medium"), "{a}");
        assert!(a.contains("format=yuv420p"), "{a}");
        assert!(!a.contains("-c:v copy"), "{a}");
    }

    #[test]
    fn crf_table() {
        let levels = [Quality::Low, Quality::Medium, Quality::High, Quality::Max];
        let cases = [
            ("h264", Mp4, "libx264 -crf", ["28", "25", "22", "18"]),
            ("hevc", Mp4, "libx265 -crf", ["30", "28", "25", "21"]),
            ("vp9", Webm, "libvpx-vp9 -crf", ["40", "36", "32", "24"]),
            ("av1", Webm, "libsvtav1 -crf", ["40", "35", "30", "24"]),
        ];
        for (codec, to, enc, crfs) in cases {
            for (level, crf) in levels.iter().zip(crfs) {
                let a = args(Mkv, Some(to), &clip(codec, "aac"), &q(*level));
                assert!(
                    a.contains(&format!("{enc} {crf} ")),
                    "{codec} {level:?}: {a}"
                );
            }
        }
        let audio_cases: [(FormatId, &str, [&str; 4]); 3] = [
            (M4a, "-c:a aac -b:a", ["96k", "128k", "192k", "256k"]),
            (Opus, "-c:a libopus -b:a", ["64k", "96k", "128k", "192k"]),
            (Mp3, "-c:a libmp3lame -q:a", ["7", "5", "2", "0"]),
        ];
        for (to, enc, values) in audio_cases {
            for (level, v) in levels.iter().zip(values) {
                let a = args(Wav, Some(to), &tone("pcm_s16le"), &q(*level));
                assert!(a.contains(&format!("{enc} {v} ")), "{to:?} {level:?}: {a}");
            }
        }
    }

    #[test]
    fn ten_bit_compatible_is_yuv420p() {
        let mut info = clip("hevc", "aac");
        info.streams[0].pix_fmt = Some("yuv420p10le".into());
        let a = args(Mov, Some(Mp4), &info, &preset(Preset::Compatible));
        assert!(a.contains("-c:v libx264"), "{a}");
        assert!(a.contains("format=yuv420p "), "{a}");
    }

    #[test]
    fn ten_bit_small_stays_ten_bit() {
        let mut info = clip("hevc", "aac");
        info.streams[0].pix_fmt = Some("yuv420p10le".into());
        let a = args(Mov, Some(Mp4), &info, &preset(Preset::Small));
        assert!(a.contains("-c:v libx265 -crf 25 -preset medium"), "{a}");
        assert!(a.contains("format=yuv420p10le"), "{a}");
        assert!(a.contains("-tag:v hvc1"), "{a}");
    }

    #[test]
    fn hevc_copy_is_tagged_hvc1() {
        let a = args(Mov, Some(Mp4), &clip("hevc", "aac"), &JobOptions::default());
        assert!(a.contains("-c:v copy -tag:v hvc1"), "{a}");
    }

    #[test]
    fn odd_dims_get_even_scale() {
        let a = args(
            Mkv,
            Some(Mp4),
            &clip("ffv1", "flac"),
            &JobOptions::default(),
        );
        assert!(a.contains("-c:v libx264"), "{a}");
        assert!(
            a.contains("-vf scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p"),
            "{a}"
        );
    }

    #[test]
    fn webm_default_is_vp9_with_opus() {
        let a = args(
            Mov,
            Some(Webm),
            &clip("h264", "aac"),
            &JobOptions::default(),
        );
        assert!(
            a.contains("-c:v libvpx-vp9 -crf 32 -b:v 0 -row-mt 1 -cpu-used 4"),
            "{a}"
        );
        assert!(a.contains("-c:a:0 libopus -b:a:0 128k"), "{a}");
        assert!(a.contains("-f webm"), "{a}");
    }

    #[test]
    fn surround_to_opus_gets_a_layout_libopus_takes() {
        let mut info = clip("h264", "ac3");
        info.streams[1].channels = Some(6);
        let a = args(Mkv, Some(Webm), &info, &JobOptions::default());
        assert!(
            a.contains(
                "-c:a:0 libopus -b:a:0 128k -filter:a:0 aformat=channel_layouts=7.1|5.1|stereo|mono"
            ),
            "{a}"
        );
        let a = args(Mkv, Some(Mp4), &info, &JobOptions::default());
        assert!(!a.contains("aformat"), "{a}");
    }

    #[test]
    fn small_webm_is_av1_with_libaom_fallback() {
        let t = task(Mov, Some(Webm));
        let o = preset(Preset::Small);
        let a = build(&t, &clip("h264", "aac"), &o).unwrap();
        assert!(a.contains("-c:v libsvtav1 -crf 30 -preset 8"), "{a}");
        let a = build_with(&t, &clip("h264", "aac"), &o, &|e| e != "libsvtav1").unwrap();
        assert!(
            a.contains("-c:v libaom-av1 -crf 30 -cpu-used 6 -row-mt 1"),
            "{a}"
        );
        let e = build_with(&t, &clip("h264", "aac"), &o, &|e| !e.contains("av1")).unwrap_err();
        assert_eq!(e.code, ErrorCode::BackendMissing);
        assert_eq!(e.reason, "this ffmpeg can't encode AV1 (no libsvtav1)");
    }

    #[test]
    fn audio_targets() {
        let d = JobOptions::default();
        let a = args(Wav, Some(Mp3), &tone("pcm_s16le"), &d);
        assert!(a.contains("-map 0:0 -c:a libmp3lame -q:a 2 "), "{a}");
        assert!(a.contains("-f mp3"), "{a}");
        let a = args(Mp4, Some(M4a), &clip("h264", "aac"), &d);
        assert!(a.contains("-map 0:1 -c:a copy"), "{a}");
        assert!(!a.contains("-map 0:0"), "{a}");
        assert!(a.contains("-movflags +faststart -f ipod"), "{a}");
        let a = args(Mp3, Some(M4a), &tone("mp3"), &q(Quality::High));
        assert!(a.contains("-c:a aac -b:a 192k"), "{a}");
        let a = args(Mp3, Some(Wav), &tone("mp3"), &d);
        assert!(a.contains("-c:a pcm_s16le -map_metadata 0 -f wav"), "{a}");
        let mut deep = tone("flac");
        deep.streams[0].bits = Some(24);
        let a = args(Flac, Some(Wav), &deep, &d);
        assert!(a.contains("-c:a pcm_s24le"), "{a}");
        let a = args(Wav, Some(Flac), &tone("pcm_s24le"), &d);
        assert!(a.contains("-c:a flac -map_metadata 0 -f flac"), "{a}");
        let a = args(Flac, Some(Wav), &tone("pcm_s24le"), &d);
        assert!(a.contains("-c:a pcm_s24le"), "{a}");
        let a = args(M4a, Some(Wav), &tone("aac"), &d);
        assert!(a.contains("-c:a pcm_s16le"), "{a}");
        let a = args(Wav, Some(Opus), &tone("pcm_s16le"), &d);
        assert!(
            a.contains("-c:a libopus -b:a 128k -af aformat=channel_layouts=7.1|5.1|stereo|mono -map_metadata 0 -f opus"),
            "{a}"
        );
        let a = args(Webm, Some(Opus), &clip("vp9", "opus"), &d);
        assert!(a.contains("-c:a copy"), "{a}");
    }

    fn with_cover(codec: &str) -> MediaInfo {
        let mut info = tone(codec);
        info.streams.push(Stream {
            attached_pic: true,
            ..video(1, "png", "rgb24")
        });
        info
    }

    #[test]
    fn cover_kept_for_m4a() {
        let a = args(Mp3, Some(M4a), &with_cover("mp3"), &JobOptions::default());
        assert!(
            a.contains("-map 0:0 -map 0:1 -c:v copy -disposition:v:0 attached_pic"),
            "{a}"
        );
        for to in [Mp3, Flac] {
            let a = args(M4a, Some(to), &with_cover("aac"), &JobOptions::default());
            assert!(a.contains("-map 0:1"), "{to:?}: {a}");
        }
    }

    #[test]
    fn cover_dropped_for_opus_and_wav() {
        for to in [Opus, Wav] {
            let a = args(Mp3, Some(to), &with_cover("mp3"), &JobOptions::default());
            assert!(!a.contains("-map 0:1"), "{to:?}: {a}");
            assert!(!a.contains("attached_pic"), "{to:?}: {a}");
        }
    }

    #[test]
    fn strip_drops_cover_and_metadata() {
        let o = JobOptions {
            strip_metadata: true,
            ..JobOptions::default()
        };
        let a = args(Mp3, Some(M4a), &with_cover("mp3"), &o);
        assert!(!a.contains("-map 0:1"), "{a}");
        assert!(a.contains("-map_metadata -1 -map_chapters -1"), "{a}");
        assert!(!a.contains("-map_metadata 0"), "{a}");
        let a = args(Mov, Some(Mp4), &clip("h264", "aac"), &o);
        assert!(a.contains("-map_metadata -1 -map_chapters -1"), "{a}");
    }

    #[test]
    fn gif_graph() {
        let a = args(Mov, Some(Gif), &clip("h264", "aac"), &JobOptions::default());
        assert!(
            a.contains("-filter_complex [0:0]fps=15,scale='if(gte(iw,ih),min(iw,640),-1)':'if(gte(iw,ih),-1,min(ih,640))':flags=lanczos,split[a][b];[a]palettegen=stats_mode=single[p];[b][p]paletteuse=new=1:dither=bayer:bayer_scale=5[g] -map [g]"),
            "{a}"
        );
        assert!(a.contains("-loop 0"), "{a}");
        assert!(!a.contains("-map 0:1"), "{a}");
        assert!(a.contains("-f gif"), "{a}");
        let o = JobOptions {
            max_size: Some(200),
            quality: Some(Quality::Low),
            ..JobOptions::default()
        };
        let a = args(Mov, Some(Gif), &clip("h264", "aac"), &o);
        assert!(a.contains("fps=10,"), "{a}");
        assert!(a.contains("min(iw,200)"), "{a}");
    }

    #[test]
    fn max_size_forces_reencode() {
        let o = JobOptions {
            max_size: Some(160),
            ..JobOptions::default()
        };
        let a = args(Mov, Some(Mp4), &clip("h264", "aac"), &o);
        assert!(a.contains("-c:v libx264"), "{a}");
        assert!(
            a.contains("-vf scale='if(gte(iw,ih),min(iw,160),-2)':'if(gte(iw,ih),-2,min(ih,160))',scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p"),
            "{a}"
        );
    }

    #[test]
    fn default_reencode_keeps_the_source_codec() {
        let a = args(Mov, Some(Mp4), &clip("hevc", "aac"), &q(Quality::Medium));
        assert!(a.contains("-c:v libx265 -crf 28"), "{a}");
        let a = args(
            Webm,
            Some(Mp4),
            &clip("vp9", "opus"),
            &JobOptions::default(),
        );
        assert!(a.contains("-c:v libx264 -crf 22"), "{a}");
    }

    #[test]
    fn compress_always_reencodes() {
        let a = args(Mp4, None, &clip("h264", "aac"), &JobOptions::default());
        assert!(a.contains("-c:v libx264 -crf 22"), "{a}");
        assert!(a.contains("-c:a:0 copy"), "{a}");
        assert!(a.contains("-f mp4"), "{a}");
        let a = args(Mov, None, &clip("hevc", "aac"), &JobOptions::default());
        assert!(a.contains("-c:v libx265"), "{a}");
        assert!(a.contains("-f mov"), "{a}");
        let a = args(Webm, None, &clip("vp9", "opus"), &JobOptions::default());
        assert!(a.contains("-c:v libvpx-vp9"), "{a}");
        let a = args(Webm, None, &clip("vp9", "opus"), &preset(Preset::Small));
        assert!(a.contains("-c:v libsvtav1"), "{a}");
    }

    #[test]
    fn no_video_track() {
        let e = build(&task(Mp4, Some(Webm)), &tone("aac"), &JobOptions::default()).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput);
        assert_eq!(e.reason, "there's no video track");
        assert_eq!(
            e.fix.as_deref(),
            Some("convert it to an audio format, such as --to m4a")
        );
        assert_eq!(e.file.as_deref(), Some(Path::new("/tmp/in.x")));
        let e = build(
            &task(Mp3, Some(Gif)),
            &with_cover("mp3"),
            &JobOptions::default(),
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput);
    }

    #[test]
    fn no_audio_track() {
        let silent = MediaInfo {
            duration: Some(2.0),
            streams: vec![video(0, "h264", "yuv420p")],
        };
        let e = build(&task(Mp4, Some(Mp3)), &silent, &JobOptions::default()).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput);
        assert_eq!(e.reason, "there's no audio track to convert");
        assert_eq!(
            e.fix.as_deref(),
            Some("pick a video format, such as --to mp4")
        );
        let a = args(Mp4, Some(Webm), &silent, &JobOptions::default());
        assert!(!a.contains("-c:a"), "{a}");
    }

    #[test]
    fn missing_encoder() {
        let e = build_with(
            &task(Mov, Some(Mp4)),
            &clip("h264", "aac"),
            &preset(Preset::Small),
            &|e| e != "libx265",
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::BackendMissing);
        assert_eq!(e.reason, "this ffmpeg can't encode HEVC (no libx265)");
        assert_eq!(e.fix.as_deref(), Some(crate::platform::ffmpeg_install()));
        let e = build_with(
            &task(Wav, Some(Mp3)),
            &tone("pcm_s16le"),
            &JobOptions::default(),
            &|e| e != "libmp3lame",
        )
        .unwrap_err();
        assert_eq!(e.reason, "this ffmpeg can't encode MP3 (no libmp3lame)");
        // A copied stream needs no encoder.
        build_with(
            &task(Mkv, Some(Mp4)),
            &clip("h264", "aac"),
            &JobOptions::default(),
            &|e| e != "libx264",
        )
        .unwrap();
    }

    #[test]
    fn paths_are_file_urls() {
        let mut t = task(Mov, Some(Webm));
        t.inputs = vec![PathBuf::from("/tmp/-a:b|c.mov")];
        let a = build(&t, &clip("h264", "aac"), &JobOptions::default()).unwrap();
        assert!(a.contains("-i file:/tmp/-a:b|c.mov "), "{a}");
    }
}
