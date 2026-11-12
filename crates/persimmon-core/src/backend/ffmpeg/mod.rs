// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Audio and video through a system FFmpeg (DD-29), always run as a child
//! process (DD-09), never linked.

mod args;
pub(crate) mod child;
mod locate;
mod probe;

pub use locate::Ffmpeg;
pub use probe::{MediaInfo, Stream, StreamKind};

use std::ffi::OsString;
use std::path::Path;

use super::{Done, Run};
use crate::error::{Error, ErrorCode, FixKind};
use crate::exec::CancelToken;
use crate::job::{JobOptions, PlannedTask};

/// Converts and compresses audio and video with one FFmpeg: probe, build
/// the arguments, run the child.
#[derive(Debug, Clone)]
pub struct FfmpegBackend {
    ff: Ffmpeg,
}

impl FfmpegBackend {
    pub fn new(ff: Ffmpeg) -> FfmpegBackend {
        FfmpegBackend { ff }
    }
}

impl Run for FfmpegBackend {
    fn run(
        &self,
        task: &PlannedTask,
        options: &JobOptions,
        temp: &Path,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(f32),
    ) -> Result<Done, Error> {
        let input = &task.inputs[0];
        let info = self.ff.probe_with(input, cancel)?;
        let args = args::build_args(task, &info, options, temp, &|e| self.ff.has_encoder(e))?;
        child::run_ffmpeg(&self.ff, &args, info.duration, cancel, progress).map_err(|f| {
            let why = if f.message.is_empty() {
                "ffmpeg failed".to_string()
            } else {
                f.message
            };
            let e = match f.code {
                ErrorCode::Cancelled => Error::new(f.code, "cancelled"),
                ErrorCode::OutputNotWritable => {
                    Error::new(f.code, format!("can't write the output: {why}")).with_fix_as(
                        FixKind::ChooseOutputFolder,
                        "use --out to write somewhere else",
                    )
                }
                ErrorCode::BackendMissing => Error::new(f.code, why)
                    .with_fix_as(FixKind::InstallFfmpeg, crate::platform::ffmpeg_install()),
                code => Error::new(code, format!("ffmpeg can't convert it: {why}")),
            };
            e.with_file(input)
        })?;
        if task.to.is_none() {
            let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            if size(temp) >= size(input) {
                let _ = std::fs::remove_file(temp);
                return Ok(Done::NotSmaller);
            }
        }
        Ok(Done::Written)
    }
}

/// `path` as an FFmpeg `file:` URL, so a name that starts with `-` or holds
/// a `:` or `|` is never read as an option or a protocol.
pub(crate) fn file_url(path: &Path) -> OsString {
    let mut url = OsString::from("file:");
    url.push(path);
    url
}

/// The last useful line of FFmpeg's stderr, for an error's reason: path
/// and `[demuxer @ 0x…]` prefixes removed, FFmpeg's closing summaries
/// skipped, temp files never shown, at most 160 characters.
pub(crate) fn message(stderr: &str, paths: &[&Path]) -> String {
    let line = stderr
        .lines()
        .rev()
        .filter(|l| {
            let l = l.trim();
            l != "Conversion failed!" && !l.starts_with("Error opening output files")
        })
        .map(|l| {
            let mut l = l.trim();
            if l.starts_with('[')
                && let Some((tag, rest)) = l.split_once("] ")
                && tag.contains(" @ 0x")
            {
                l = rest;
            }
            for p in paths {
                let p = p.display().to_string();
                for prefix in [format!("file:{p}: "), format!("{p}: ")] {
                    l = l.strip_prefix(&prefix).unwrap_or(l);
                }
            }
            l
        })
        .find(|l| !l.is_empty() && !l.contains(".part"))
        .unwrap_or_default();
    line.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn message_drops_stream_prefixes_and_summaries() {
        let stderr = "[out#0/webm @ 0x6000013a8000] Error opening output file x.webm.\nError opening output files: Invalid argument\nConversion failed!\n";
        assert_eq!(message(stderr, &[]), "Error opening output file x.webm.");
        // Nothing useful left: the caller says how FFmpeg exited instead.
        assert_eq!(message("Conversion failed!\n", &[]), "");
    }

    #[test]
    fn message_hides_paths() {
        let p = Path::new("/tmp/x/clip.mp4");
        let t = Path::new("/tmp/x/.prsm-1-0.part");
        let stderr = "[mov,mp4 @ 0x1] moov atom not found\nfile:/tmp/x/clip.mp4: Invalid data found when processing input\n\n";
        assert_eq!(
            message(stderr, &[p]),
            "Invalid data found when processing input"
        );
        assert_eq!(
            message("/tmp/x/clip.mp4: No such file\n", &[p]),
            "No such file"
        );
        assert_eq!(
            message(
                "Error opening output file:/tmp/x/.prsm-1-0.part.\nConversion failed!\n",
                &[p, t]
            ),
            ""
        );
        assert_eq!(message(&"x".repeat(400), &[p]).chars().count(), 160);
        assert_eq!(message("", &[p]), "");
    }

    #[test]
    fn file_url_prefixes() {
        assert_eq!(file_url(Path::new("-a:b|c.mp4")), "file:-a:b|c.mp4");
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;
    use crate::backend::{Backends, Done, Run};
    use crate::detect::detect;
    use crate::error::{Error, ErrorCode};
    use crate::exec::CancelToken;
    use crate::format::FormatId::{self, *};
    use crate::job::{JobOptions, PlannedTask, Preset, Quality};
    use crate::registry::{Backend, Operation, Registry};
    use persimmon_corpus::media::{media_sample, test_ffmpeg};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn ff() -> Option<Ffmpeg> {
        let Some(path) = test_ffmpeg() else {
            eprintln!("skipped: no ffmpeg");
            return None;
        };
        Some(Ffmpeg::locate(Some(&path)).unwrap())
    }

    fn task(input: &Path, to: Option<FormatId>) -> PlannedTask {
        let from = detect(input, Registry::builtin().formats()).unwrap().format;
        PlannedTask {
            inputs: vec![input.to_path_buf()],
            from: vec![from],
            to,
            backend: Backend::Ffmpeg,
            output: PathBuf::from("unused"),
        }
    }

    fn temp() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(".prsm-ffb-{}-{n}.part", std::process::id()))
    }

    /// Convert `input` and return the result and the temp path written.
    fn convert(
        ff: &Ffmpeg,
        input: &Path,
        to: Option<FormatId>,
        o: &JobOptions,
    ) -> (Result<Done, Error>, PathBuf) {
        let t = temp();
        let r = FfmpegBackend::new(ff.clone()).run(
            &task(input, to),
            o,
            &t,
            &CancelToken::new(),
            &mut |_| {},
        );
        (r, t)
    }

    /// Convert a sample, expecting success, and probe the output.
    fn ok(ff: &Ffmpeg, id: &str, to: Option<FormatId>, o: &JobOptions) -> MediaInfo {
        let input = media_sample(id).unwrap();
        let (r, t) = convert(ff, &input, to, o);
        assert_eq!(
            r.as_ref().map_err(|e| e.to_string()),
            Ok(&Done::Written),
            "{id} → {to:?}"
        );
        let info = ff.probe(&t).unwrap();
        let _ = std::fs::remove_file(&t);
        info
    }

    fn q(quality: Quality) -> JobOptions {
        JobOptions {
            quality: Some(quality),
            ..JobOptions::default()
        }
    }

    #[test]
    fn every_media_pair_converts() {
        let Some(ff) = ff() else { return };
        let reg = Registry::builtin();
        let mut count = 0;
        for id in [
            "vid-mov", "vid-mp4", "vid-mkv", "vid-webm", "vid-avi", "aud-mp3", "aud-m4a",
            "aud-wav", "aud-flac", "aud-opus",
        ] {
            let input = media_sample(id).unwrap();
            let from = task(&input, None).from[0];
            for to in reg.targets(Operation::Convert, from) {
                let info = ok(&ff, id, Some(to), &JobOptions::default());
                count += 1;
                let video = match to {
                    Mp4 | Mov => Some("h264"),
                    Webm => Some("vp9"),
                    Gif => Some("gif"),
                    _ => None,
                };
                assert_eq!(
                    info.video().map(|s| s.codec.as_str()),
                    video,
                    "{id} → {to:?}"
                );
                let audio = match to {
                    Mp4 | Mov | M4a => Some("aac"),
                    Webm | Opus => Some("opus"),
                    Mp3 => Some("mp3"),
                    Wav => Some("pcm_s16le"),
                    Flac => Some("flac"),
                    _ => None,
                };
                assert_eq!(
                    info.audio().first().map(|s| s.codec.as_str()),
                    audio,
                    "{id} → {to:?}"
                );
                if to != Gif {
                    let d = info.duration.unwrap();
                    assert!((d - 2.0).abs() < 0.2, "{id} → {to:?}: {d}");
                }
            }
        }
        assert_eq!(count, 62);
    }

    #[test]
    fn default_keeps_hevc_and_ten_bit() {
        let Some(ff) = ff() else { return };
        let info = ok(&ff, "vid-hevc", Some(Mp4), &JobOptions::default());
        let v = info.video().unwrap();
        assert_eq!(v.codec, "hevc");
        assert_eq!(v.tag.as_deref(), Some("hvc1"));
        assert!(v.ten_bit());
    }

    #[test]
    fn ten_bit_hevc_to_compatible_is_8_bit() {
        let Some(ff) = ff() else { return };
        let o = JobOptions {
            preset: Some(Preset::Compatible),
            ..JobOptions::default()
        };
        let info = ok(&ff, "vid-hevc", Some(Mp4), &o);
        let v = info.video().unwrap();
        assert_eq!(v.codec, "h264");
        assert_eq!(v.pix_fmt.as_deref(), Some("yuv420p"));
    }

    #[test]
    fn small_preset_codecs() {
        let Some(ff) = ff() else { return };
        let o = JobOptions {
            preset: Some(Preset::Small),
            ..JobOptions::default()
        };
        let mp4 = ok(&ff, "vid-mp4", Some(Mov), &o);
        assert_eq!(mp4.video().unwrap().codec, "hevc");
        assert_eq!(mp4.video().unwrap().tag.as_deref(), Some("hvc1"));
        let webm = ok(&ff, "vid-mp4", Some(Webm), &o);
        assert_eq!(webm.video().unwrap().codec, "av1");
    }

    #[test]
    fn odd_size_to_h264_is_even() {
        let Some(ff) = ff() else { return };
        let v = ok(&ff, "vid-odd", Some(Mp4), &JobOptions::default());
        let v = v.video().unwrap();
        assert_eq!(
            (v.codec.as_str(), v.width, v.height),
            ("h264", Some(320), Some(240))
        );
    }

    #[test]
    fn rotated_video_reencoded_upright() {
        let Some(ff) = ff() else { return };
        let info = ok(&ff, "vid-rotated", Some(Mp4), &q(Quality::High));
        let v = info.video().unwrap();
        assert_eq!((v.width, v.height), (Some(240), Some(320)));
    }

    #[test]
    fn max_size_shrinks_video() {
        let Some(ff) = ff() else { return };
        let o = JobOptions {
            max_size: Some(160),
            ..JobOptions::default()
        };
        let info = ok(&ff, "vid-mp4", Some(Mov), &o);
        let v = info.video().unwrap();
        assert_eq!((v.width, v.height), (Some(160), Some(120)));
    }

    #[test]
    fn silent_video_to_mp3_is_unsupported_input() {
        let Some(ff) = ff() else { return };
        let input = media_sample("vid-silent").unwrap();
        let (r, t) = convert(&ff, &input, Some(Mp3), &JobOptions::default());
        let e = r.unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedInput);
        assert_eq!(e.reason, "there's no audio track to convert");
        assert_eq!(e.fix_text(), Some("pick a video format, such as --to mp4"));
        assert_eq!(e.file.as_deref(), Some(input.as_path()));
        assert!(!t.exists());
    }

    #[test]
    fn cover_art_survives_mp3_to_m4a() {
        let Some(ff) = ff() else { return };
        let info = ok(&ff, "aud-cover", Some(M4a), &JobOptions::default());
        assert_eq!(info.audio()[0].codec, "aac");
        assert!(info.cover().is_some(), "{info:?}");
        assert!(info.video().is_none());
    }

    #[test]
    fn surround_converts_to_webm_and_opus() {
        let Some(ff) = ff() else { return };
        for to in [Webm, Opus] {
            let info = ok(&ff, "vid-surround", Some(to), &JobOptions::default());
            let a = info.audio()[0];
            assert_eq!((a.codec.as_str(), a.channels), ("opus", Some(6)), "{to:?}");
        }
    }

    #[test]
    fn deep_flac_to_wav_is_24_bit() {
        let Some(ff) = ff() else { return };
        let info = ok(&ff, "aud-flac-24", Some(Wav), &JobOptions::default());
        assert_eq!(info.audio()[0].codec, "pcm_s24le");
    }

    #[test]
    fn truncated_is_corrupt_input() {
        let Some(ff) = ff() else { return };
        let input = media_sample("vid-truncated").unwrap();
        let (r, _) = convert(&ff, &input, Some(Webm), &JobOptions::default());
        let e = r.unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert_eq!(e.file.as_deref(), Some(input.as_path()));
    }

    #[test]
    fn compress_compact_is_not_smaller() {
        let Some(ff) = ff() else { return };
        let input = media_sample("vid-compact").unwrap();
        let (r, t) = convert(&ff, &input, None, &JobOptions::default());
        assert_eq!(r.unwrap(), Done::NotSmaller);
        assert!(!t.exists(), "the larger result was left behind");
    }

    #[test]
    fn compress_long_is_smaller() {
        let Some(ff) = ff() else { return };
        let input = media_sample("vid-long").unwrap();
        let (r, t) = convert(&ff, &input, None, &JobOptions::default());
        assert_eq!(r.unwrap(), Done::Written);
        let (before, after) = (
            std::fs::metadata(&input).unwrap().len(),
            std::fs::metadata(&t).unwrap().len(),
        );
        assert!(after < before, "{after} >= {before}");
        assert_eq!(ff.probe(&t).unwrap().video().unwrap().codec, "h264");
        let _ = std::fs::remove_file(&t);
    }

    #[cfg(unix)]
    #[test]
    fn misreadable_names_convert() {
        let Some(ff) = ff() else { return };
        let dir = std::env::temp_dir().join(format!("prsm-ffb-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let odd = dir.join("-a:b|c.mp4");
        std::fs::copy(media_sample("vid-mp4").unwrap(), &odd).unwrap();
        let (r, t) = convert(&ff, &odd, Some(Webm), &JobOptions::default());
        assert_eq!(r.map_err(|e| e.to_string()), Ok(Done::Written));
        assert_eq!(ff.probe(&t).unwrap().video().unwrap().codec, "vp9");
        let _ = std::fs::remove_file(&t);
    }

    #[test]
    fn progress_reaches_one() {
        let Some(ff) = ff() else { return };
        let input = media_sample("vid-mp4").unwrap();
        let t = temp();
        let mut seen = Vec::new();
        FfmpegBackend::new(ff)
            .run(
                &task(&input, Some(Webm)),
                &JobOptions::default(),
                &t,
                &CancelToken::new(),
                &mut |f| seen.push(f),
            )
            .unwrap();
        assert_eq!(seen.last(), Some(&1.0), "{seen:?}");
        let _ = std::fs::remove_file(&t);
    }

    #[test]
    fn backends_dispatch_video_without_ffmpeg() {
        let dir = std::env::temp_dir().join(format!("prsm-ffb-noff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mov");
        std::fs::write(&clip, b"\x00\x00\x00\x14ftypqt  \x00\x00\x00\x00qt  ").unwrap();
        let o = JobOptions {
            ffmpeg: Some(PathBuf::from("/nonexistent/ffmpeg")),
            ..JobOptions::default()
        };
        let e = Backends::new()
            .run(
                &task(&clip, Some(Mp4)),
                &o,
                &dir.join(".t.part"),
                &CancelToken::new(),
                &mut |_| {},
            )
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::BackendMissing);
        assert_eq!(e.file.as_deref(), Some(clip.as_path()));
        assert_eq!(e.reason, "no ffmpeg at /nonexistent/ffmpeg");
    }

    #[cfg(unix)]
    #[test]
    fn probe_killed_by_ctrl_c_is_cancelled() {
        let Some(real) = ff() else { return };
        let ff = super::child::tests::fake(&real, "ffprobe", "kill -INT $$");
        let input = media_sample("aud-flac").unwrap();
        let cancel = CancelToken::new();
        super::child::tests::cancel_later(&cancel, 100);
        let e = FfmpegBackend::new(ff)
            .run(
                &task(&input, Some(Wav)),
                &JobOptions::default(),
                &temp(),
                &cancel,
                &mut |_| {},
            )
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled, "{e}");
        assert_eq!(e.file.as_deref(), Some(input.as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn probe_crash_has_a_reason() {
        let Some(real) = ff() else { return };
        let ff = super::child::tests::fake(&real, "ffprobe", "kill -SEGV $$");
        let input = media_sample("aud-flac").unwrap();
        let e = ff.probe(&input).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert!(
            e.reason.starts_with("ffprobe stopped unexpectedly"),
            "{}",
            e.reason
        );
    }
}
