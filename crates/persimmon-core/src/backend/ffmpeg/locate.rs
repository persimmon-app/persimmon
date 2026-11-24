// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Finding FFmpeg: `--ffmpeg`, else `PATH`, else the platform's usual
//! folders. It must be 7.0 or later, with `ffprobe` beside it.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Error, ErrorCode, FixKind};
use crate::platform;

/// A usable FFmpeg: 7.0 or later, with `ffprobe` beside it.
#[derive(Debug, Clone)]
pub struct Ffmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// As `ffmpeg -version` prints it, e.g. `7.1.1`.
    pub version: String,
    encoders: HashSet<String>,
    /// The pixel formats of the encoders whose formats matter, by name:
    /// only HEVC's hardware encoder, whose 10-bit support varies.
    pix_fmts: HashMap<String, Vec<String>>,
}

/// The hardware encoders prsm uses, with the codec each makes.
const HARDWARE: [(&str, &str); 2] = [
    ("h264_videotoolbox", "H.264"),
    ("hevc_videotoolbox", "HEVC"),
];

impl Ffmpeg {
    /// Find FFmpeg: `explicit` (`--ffmpeg`) if given, else the first on
    /// `PATH`, else the platform's usual folders. Every failure is
    /// `BACKEND_MISSING` with the install command as its fix.
    pub fn locate(explicit: Option<&Path>) -> Result<Ffmpeg, Error> {
        let install = platform::ffmpeg_install();
        let missing = |reason: String| Error::new(ErrorCode::BackendMissing, reason);
        let ffmpeg = match explicit {
            Some(path) if path.is_file() => path.to_path_buf(),
            Some(path) => {
                return Err(
                    missing(format!("no ffmpeg at {}", path.display())).with_fix_as(
                        FixKind::InstallFfmpeg,
                        "check the --ffmpeg path, or leave it out to use the one on PATH",
                    ),
                );
            }
            None => find_in(
                "ffmpeg",
                std::env::var_os("PATH").as_deref(),
                platform::ffmpeg_dirs(),
            )
            .ok_or_else(|| {
                missing("ffmpeg isn't installed".into()).with_fix_as(
                    FixKind::InstallFfmpeg,
                    format!("{install}, or pass --ffmpeg <path>"),
                )
            })?,
        };
        let run = |args: &[&str]| -> Result<String, Error> {
            let out = Command::new(&ffmpeg)
                .arg("-hide_banner")
                .args(args)
                .stdin(Stdio::null())
                .output()
                .map_err(|e| {
                    missing(format!("{} didn't run: {e}", ffmpeg.display()))
                        .with_fix_as(FixKind::InstallFfmpeg, install)
                })?;
            if !out.status.success() {
                return Err(
                    missing(format!("{} didn't run: {}", ffmpeg.display(), out.status))
                        .with_fix_as(FixKind::InstallFfmpeg, install),
                );
            }
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        };
        let first = run(&["-version"])?
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        if let Some((major, minor)) = parse_version(&first)
            && major < 7
        {
            return Err(missing(format!(
                "ffmpeg {major}.{minor} is too old; prsm needs 7.0 or later"
            ))
            .with_fix_as(FixKind::InstallFfmpeg, install));
        }
        let ffprobe = ffmpeg.with_file_name(format!("ffprobe{}", std::env::consts::EXE_SUFFIX));
        if !ffprobe.is_file() {
            return Err(
                missing(format!("there's no ffprobe next to {}", ffmpeg.display()))
                    .with_fix_as(FixKind::InstallFfmpeg, install),
            );
        }
        let mut encoders = parse_encoders(&run(&["-encoders"])?);
        keep_usable_hardware(&mut encoders, std::env::consts::ARCH);
        let mut pix_fmts = HashMap::new();
        if encoders.contains("hevc_videotoolbox") {
            let help = run(&["-h", "encoder=hevc_videotoolbox"]).unwrap_or_default();
            pix_fmts.insert("hevc_videotoolbox".to_string(), parse_pix_fmts(&help));
        }
        let version = first
            .split_whitespace()
            .nth(2)
            .unwrap_or_default()
            .to_string();
        Ok(Ffmpeg {
            ffmpeg,
            ffprobe,
            version,
            encoders,
            pix_fmts,
        })
    }

    /// Whether this FFmpeg was built with the encoder `name`, e.g. `libx265`.
    pub fn has_encoder(&self, name: &str) -> bool {
        self.encoders.contains(name)
    }

    /// Whether the encoder `name` takes the pixel format `pix_fmt`, as far
    /// as this FFmpeg says; only known for the encoders in `pix_fmts`.
    pub fn encoder_takes(&self, name: &str, pix_fmt: &str) -> bool {
        self.pix_fmts
            .get(name)
            .is_some_and(|f| f.iter().any(|p| p == pix_fmt))
    }

    /// The codecs this FFmpeg's hardware encoders make, in a fixed order:
    /// `"H.264"` and/or `"HEVC"`.
    pub fn hardware(&self) -> Vec<String> {
        HARDWARE
            .iter()
            .filter(|(enc, _)| self.has_encoder(enc))
            .map(|(_, codec)| codec.to_string())
            .collect()
    }

    /// This FFmpeg, said to have `name` too: tests pair it with a fake
    /// program that refuses or accepts that encoder.
    #[cfg(all(test, unix))]
    pub(crate) fn with_encoder(mut self, name: &str) -> Ffmpeg {
        self.encoders.insert(name.to_string());
        self
    }
}

/// Whether FFmpeg's VideoToolbox encoders work as prsm drives them on
/// this architecture: they take quality as `-q:v` only on Apple silicon,
/// and refuse it elsewhere.
fn videotoolbox_usable(arch: &str) -> bool {
    arch == "aarch64"
}

/// Leave out the hardware encoders prsm can't drive on `arch`, so asking
/// for them there reads "unavailable" rather than failing every video.
fn keep_usable_hardware(encoders: &mut HashSet<String>, arch: &str) {
    if !videotoolbox_usable(arch) {
        for (enc, _) in HARDWARE {
            encoders.remove(enc);
        }
    }
}

/// The pixel formats an encoder lists in `ffmpeg -h encoder=<name>`'s
/// "Supported pixel formats:" line; empty when it lists none.
fn parse_pix_fmts(help: &str) -> Vec<String> {
    help.lines()
        .find_map(|l| l.trim().strip_prefix("Supported pixel formats:"))
        .map(|rest| rest.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

/// The first `name` (plus the platform's executable suffix) in the folders
/// of `path_var`, in order, then in `extra`.
fn find_in(name: &str, path_var: Option<&OsStr>, extra: &[&str]) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    path_var
        .map(|p| std::env::split_paths(p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(extra.iter().map(PathBuf::from))
        .map(|dir| dir.join(&file))
        .find(|p| p.is_file())
}

/// `(major, minor)` from `ffmpeg -version`'s first line. `None` for builds
/// that carry no release number, such as git builds (`N-118000-g…`).
fn parse_version(first_line: &str) -> Option<(u32, u32)> {
    let token = first_line
        .split("version ")
        .nth(1)?
        .split_whitespace()
        .next()?;
    let token = token.strip_prefix('n').unwrap_or(token);
    let mut parts = token.split('.');
    let number = |s: Option<&str>| -> Option<u32> {
        let digits: String = s?.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let major = number(parts.next())?;
    Some((major, number(parts.next()).unwrap_or(0)))
}

/// Encoder names from `ffmpeg -encoders`: the second column of each line
/// after the `------` rule.
fn parse_encoders(listing: &str) -> HashSet<String> {
    listing
        .lines()
        .skip_while(|l| l.trim() != "------")
        .skip(1)
        .filter_map(|l| l.split_whitespace().nth(1))
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-locate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_version_cases() {
        assert_eq!(
            parse_version("ffmpeg version 7.1.1 Copyright (c) 2000-2025"),
            Some((7, 1))
        );
        assert_eq!(
            parse_version("ffmpeg version n7.0.2-static https://johnvansickle.com/ffmpeg/"),
            Some((7, 0))
        );
        assert_eq!(
            parse_version("ffmpeg version 6.1.1-3ubuntu5 Copyright"),
            Some((6, 1))
        );
        assert_eq!(parse_version("ffmpeg version 9 Copyright"), Some((9, 0)));
        assert_eq!(
            parse_version("ffmpeg version N-118000-gabc123 Copyright"),
            None
        );
    }

    #[test]
    fn parse_encoders_skips_the_header() {
        let listing = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n V....D libx264              libx264 H.264\n A....D aac                  AAC (Advanced Audio Coding)\n";
        let enc = parse_encoders(listing);
        assert!(enc.contains("libx264"));
        assert!(enc.contains("aac"));
        assert!(!enc.contains("="));
        assert_eq!(enc.len(), 2);
    }

    /// FFmpeg's VideoToolbox encoders take `-q:v` only on Apple silicon;
    /// elsewhere every hardware encode would fail and be redone.
    #[test]
    fn videotoolbox_needs_apple_silicon() {
        assert!(videotoolbox_usable("aarch64"));
        assert!(!videotoolbox_usable("x86_64"));
        let mut enc: HashSet<String> = ["libx264", "h264_videotoolbox", "hevc_videotoolbox"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        keep_usable_hardware(&mut enc, "x86_64");
        assert_eq!(enc.len(), 1);
        assert!(enc.contains("libx264"));
    }

    #[test]
    fn parse_pix_fmts_reads_the_supported_line() {
        let help = "Encoder hevc_videotoolbox [VideoToolbox H.265 Encoder]:\n    General capabilities: dr1 delay hybrid \n    Supported pixel formats: videotoolbox_vld nv12 yuv420p bgra ayuv p010le p210le\nhevc_videotoolbox AVOptions:\n";
        let fmts = parse_pix_fmts(help);
        assert!(fmts.iter().any(|f| f == "p010le"), "{fmts:?}");
        assert!(fmts.iter().any(|f| f == "nv12"), "{fmts:?}");
        assert!(parse_pix_fmts("Encoder x:\n").is_empty());
    }

    #[test]
    fn find_in_follows_path_order() {
        let dir = scratch("order");
        let (a, b, c) = (dir.join("a"), dir.join("b"), dir.join("c"));
        for d in [&a, &b, &c] {
            std::fs::create_dir(d).unwrap();
        }
        let name = format!("ffmpeg{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(b.join(&name), b"").unwrap();
        std::fs::write(c.join(&name), b"").unwrap();
        let path = std::env::join_paths([&a, &b]).unwrap();
        let c_str = c.to_str().unwrap();
        assert_eq!(
            find_in("ffmpeg", Some(&path), &[c_str]),
            Some(b.join(&name))
        );
        let only_a = std::env::join_paths([&a]).unwrap();
        assert_eq!(
            find_in("ffmpeg", Some(&only_a), &[c_str]),
            Some(c.join(&name))
        );
        assert_eq!(find_in("ffmpeg", Some(&only_a), &[]), None);
    }

    #[test]
    fn explicit_missing_path() {
        let e = Ffmpeg::locate(Some(Path::new("/nonexistent/ffmpeg"))).unwrap_err();
        assert_eq!(e.code, ErrorCode::BackendMissing);
        assert_eq!(e.reason, "no ffmpeg at /nonexistent/ffmpeg");
        assert_eq!(
            e.fix_text(),
            Some("check the --ffmpeg path, or leave it out to use the one on PATH")
        );
    }

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn too_old_is_backend_missing() {
        let dir = scratch("old");
        let ff = script(&dir, "ffmpeg", "echo 'ffmpeg version 6.1.1 Copyright'");
        script(&dir, "ffprobe", "exit 0");
        let e = Ffmpeg::locate(Some(&ff)).unwrap_err();
        assert_eq!(e.code, ErrorCode::BackendMissing);
        assert_eq!(e.reason, "ffmpeg 6.1 is too old; prsm needs 7.0 or later");
        assert_eq!(e.fix_text(), Some(crate::platform::ffmpeg_install()));
    }

    #[cfg(unix)]
    #[test]
    fn no_ffprobe() {
        let dir = scratch("noprobe");
        let ff = script(&dir, "ffmpeg", "echo 'ffmpeg version 7.1 Copyright'");
        let e = Ffmpeg::locate(Some(&ff)).unwrap_err();
        assert_eq!(e.code, ErrorCode::BackendMissing);
        assert!(
            e.reason.starts_with("there's no ffprobe next to"),
            "{}",
            e.reason
        );
    }

    #[test]
    fn real_ffmpeg_is_found() {
        let Some(path) = persimmon_corpus::media::test_ffmpeg() else {
            eprintln!("skipped: no ffmpeg");
            return;
        };
        let ff = Ffmpeg::locate(Some(&path)).unwrap();
        assert!(ff.ffprobe.is_file());
        for enc in ["libx264", "aac", "libopus"] {
            assert!(ff.has_encoder(enc), "{enc}");
        }
        assert!(!ff.has_encoder("no-such-encoder"));
        let on_path = Ffmpeg::locate(None).unwrap();
        assert_eq!(on_path.ffmpeg, path);
    }
}
