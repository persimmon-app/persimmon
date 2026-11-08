// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Audio and video samples, made at test time from FFmpeg's own test
//! sources (`testsrc2`, `sine`) and never committed. They need an `ffmpeg`
//! on `PATH`. Without one, [`media_sample`] returns `None` and tests skip,
//! unless `PERSIMMON_REQUIRE_FFMPEG=1` (as in CI) turns that into a failure.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Every media sample: `(id, file name)`.
pub const MEDIA_SAMPLES: [(&str, &str); 19] = [
    ("vid-mov", "clip.mov"),
    ("vid-mp4", "clip.mp4"),
    ("vid-mkv", "clip.mkv"),
    ("vid-webm", "clip.webm"),
    ("vid-avi", "clip.avi"),
    ("vid-hevc", "hevc.mov"),
    ("vid-silent", "silent.mp4"),
    ("vid-rotated", "rotated.mp4"),
    ("vid-odd", "odd.mkv"),
    ("vid-long", "long.mp4"),
    ("vid-compact", "compact.mp4"),
    ("vid-truncated", "truncated.mp4"),
    ("aud-mp3", "tone.mp3"),
    ("aud-m4a", "tone.m4a"),
    ("aud-wav", "tone.wav"),
    ("aud-flac", "tone.flac"),
    ("aud-opus", "tone.opus"),
    ("aud-flac-24", "deep.flac"),
    ("aud-cover", "cover.mp3"),
];

/// The `ffmpeg` tests should use: the first on `PATH`. Panics when there is
/// none and `PERSIMMON_REQUIRE_FFMPEG=1`.
pub fn test_ffmpeg() -> Option<PathBuf> {
    let name = format!("ffmpeg{}", std::env::consts::EXE_SUFFIX);
    let found = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(&name))
            .find(|p| p.is_file())
    });
    if found.is_none() && std::env::var_os("PERSIMMON_REQUIRE_FFMPEG").is_some_and(|v| v == "1") {
        panic!("PERSIMMON_REQUIRE_FFMPEG is set but there's no ffmpeg on PATH");
    }
    found
}

/// The path of the sample `id`, generating every sample once per process.
/// `None` when there's no ffmpeg (see [`test_ffmpeg`]).
pub fn media_sample(id: &str) -> Option<PathBuf> {
    let name = MEDIA_SAMPLES
        .iter()
        .find(|(i, _)| *i == id)
        .unwrap_or_else(|| panic!("unknown media sample {id}"))
        .1;
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    let dir = DIR
        .get_or_init(|| {
            let ffmpeg = test_ffmpeg()?;
            let dir = std::env::temp_dir().join(format!("prsm-media-{}", std::process::id()));
            generate(&ffmpeg, &dir);
            Some(dir)
        })
        .as_ref()?;
    Some(dir.join(name))
}

const VIDEO: &str = "testsrc2=size=320x240:rate=25:duration=2";
const TONE: &str = "sine=frequency=440:sample_rate=48000:duration=2";

fn generate(ffmpeg: &Path, dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).expect("create the media sample folder");
    let run = |args: &[&str], out: &str| {
        let output = Command::new(ffmpeg)
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
            .args(args)
            .arg(dir.join(out))
            .output()
            .expect("run ffmpeg");
        assert!(
            output.status.success(),
            "making {out} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let av = |video: &'static str, audio: &'static str| -> Vec<&'static str> {
        let mut v = vec!["-f", "lavfi", "-i", VIDEO, "-f", "lavfi", "-i", TONE];
        v.extend(video.split_whitespace());
        v.extend(audio.split_whitespace());
        v.extend(["-ac", "2", "-shortest"]);
        v
    };
    let go = |args: Vec<&str>, out: &str| run(&args, out);

    let h264 = "-c:v libx264 -pix_fmt yuv420p";
    go(av(h264, "-c:a aac"), "clip.mov");
    go(av(h264, "-c:a aac"), "clip.mp4");
    go(av(h264, "-c:a libopus"), "clip.mkv");
    go(
        av(
            "-c:v libvpx-vp9 -b:v 0 -crf 40 -deadline realtime -cpu-used 8",
            "-c:a libopus",
        ),
        "clip.webm",
    );
    go(av("-c:v mjpeg -q:v 5", "-c:a pcm_s16le"), "clip.avi");
    go(
        av(
            "-c:v libx265 -pix_fmt yuv420p10le -tag:v hvc1 -x265-params log-level=error",
            "-c:a aac",
        ),
        "hevc.mov",
    );
    run(
        &[
            "-f", "lavfi", "-i", VIDEO, "-c:v", "libx264", "-pix_fmt", "yuv420p",
        ],
        "silent.mp4",
    );
    let clip = dir.join("clip.mp4");
    let clip = clip.to_str().expect("a UTF-8 temp path");
    run(
        &["-display_rotation", "90", "-i", clip, "-c", "copy"],
        "rotated.mp4",
    );
    run(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=322x242:rate=25:duration=2",
            "-f",
            "lavfi",
            "-i",
            TONE,
            // testsrc2 rounds odd sizes down, and so does cropping 4:2:0;
            // cropping 4:4:4 keeps them odd.
            "-vf",
            "format=yuv444p,crop=321:241:0:0",
            "-c:v",
            "ffv1",
            "-pix_fmt",
            "yuv444p",
            "-c:a",
            "flac",
            "-shortest",
        ],
        "odd.mkv",
    );
    run(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=25:duration=60",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=60",
            "-c:v",
            "mpeg4",
            "-q:v",
            "3",
            "-c:a",
            "aac",
            "-shortest",
        ],
        "long.mp4",
    );
    go(
        av("-c:v libx264 -pix_fmt yuv420p -crf 45", "-c:a aac -b:a 32k"),
        "compact.mp4",
    );
    let bytes = std::fs::read(dir.join("clip.mp4")).expect("read clip.mp4");
    std::fs::write(dir.join("truncated.mp4"), &bytes[..bytes.len() * 6 / 10])
        .expect("write truncated.mp4");

    let tone = |codec: &str, out: &str| {
        let mut args = vec!["-f", "lavfi", "-i", TONE, "-ac", "2"];
        args.extend(codec.split_whitespace());
        run(&args, out);
    };
    tone("-c:a libmp3lame", "tone.mp3");
    tone("-c:a aac", "tone.m4a");
    tone("-c:a pcm_s16le", "tone.wav");
    tone("-c:a flac", "tone.flac");
    tone("-c:a libopus", "tone.opus");
    run(
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=96000:duration=2",
            "-af",
            "aformat=sample_fmts=s32",
            "-c:a",
            "flac",
            "-bits_per_raw_sample",
            "24",
        ],
        "deep.flac",
    );
    // The cover is made first: a still image input ends after one frame
    // without cutting the audio short, as -frames:v 1 would.
    run(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=64x64",
            "-frames:v",
            "1",
        ],
        "cover.png",
    );
    let cover = dir.join("cover.png");
    let cover = cover.to_str().expect("a UTF-8 temp path");
    run(
        &[
            "-f",
            "lavfi",
            "-i",
            TONE,
            "-i",
            cover,
            "-map",
            "0:a",
            "-map",
            "1:v",
            "-ac",
            "2",
            "-c:a",
            "libmp3lame",
            "-c:v",
            "copy",
            "-disposition:v",
            "attached_pic",
            "-id3v2_version",
            "3",
        ],
        "cover.mp3",
    );
}
