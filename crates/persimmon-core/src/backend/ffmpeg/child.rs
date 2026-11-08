// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Running FFmpeg as a child process (DD-09): its `-progress` stream
//! becomes progress events, and a cancel kills it at once.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use super::{Ffmpeg, message};
use crate::error::ErrorCode;
use crate::exec::CancelToken;

/// Why FFmpeg failed: a code, and its last stderr line (or `cancelled`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub code: ErrorCode,
    pub message: String,
}

/// Run FFmpeg with `args` (which must include `-progress pipe:1`). With a
/// known `duration` in seconds, its progress goes to `progress`, ending at
/// 1.0 on success. Once `cancel` is set the child is killed and the result
/// is `CANCELLED`.
pub(crate) fn run_ffmpeg(
    ff: &Ffmpeg,
    args: &[OsString],
    duration: Option<f64>,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(f32),
) -> Result<(), Failure> {
    let mut child = Command::new(&ff.ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure {
            code: ErrorCode::BackendMissing,
            message: format!("{} didn't run: {e}", ff.ffmpeg.display()),
        })?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(d) = duration
                && let Some(f) = progress_fraction(&line, d)
                && tx.send(f).is_err()
            {
                break;
            }
        }
    });
    // Drained on its own thread, so a chatty FFmpeg never blocks on a
    // full pipe. Only the last lines matter.
    let drain = thread::spawn(move || {
        let mut last = VecDeque::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if last.len() == 20 {
                last.pop_front();
            }
            last.push_back(line);
        }
        Vec::from(last).join("\n")
    });

    let mut done = 0.0_f32;
    let mut forward = |f: f32, progress: &mut dyn FnMut(f32)| {
        if f > done {
            done = f;
            progress(f);
        }
    };
    let mut killed = false;
    let status = loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(f) => forward(f, progress),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => thread::sleep(Duration::from_millis(20)),
        }
        if cancel.is_cancelled() && !killed {
            let _ = child.kill();
            killed = true;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
    };
    let _ = reader.join();
    let stderr = drain.join().unwrap_or_default();
    if !cancel.is_cancelled() {
        while let Ok(f) = rx.try_recv() {
            forward(f, progress);
        }
    }
    if status.is_some_and(|s| s.success()) && !cancel.is_cancelled() {
        if duration.is_some() {
            forward(1.0, progress);
        }
        return Ok(());
    }
    let cancelled = was_cancelled(status.as_ref(), cancel);
    let code = classify(status.and_then(|s| s.code()), cancelled, &stderr);
    let message = if code == ErrorCode::Cancelled {
        "cancelled".to_string()
    } else if status.is_none_or(|s| s.code().is_none()) {
        stopped("ffmpeg", status.as_ref())
    } else {
        // Paths passed as file: URLs are left out of the message.
        let paths: Vec<PathBuf> = args
            .iter()
            .filter_map(|a| a.to_str()?.strip_prefix("file:").map(PathBuf::from))
            .collect();
        let paths: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        let why = message(&stderr, &paths);
        if why.is_empty() {
            format!("ffmpeg exited with {}", describe(status.as_ref()))
        } else {
            why
        }
    };
    Err(Failure { code, message })
}

/// Whether a child that stopped with `status` was cancelled. A terminal's
/// Ctrl-C reaches the child and prsm together, but prsm's handler sets the
/// token on its own thread, so a death by signal or FFmpeg's exit 255 waits
/// a moment for it. Without the token, it's a real failure: a crash, an
/// out-of-memory kill, an error.
pub(crate) fn was_cancelled(status: Option<&ExitStatus>, cancel: &CancelToken) -> bool {
    let interrupted = status.is_none_or(|s| s.code().is_none() || s.code() == Some(255));
    if interrupted {
        let start = Instant::now();
        while !cancel.is_cancelled() && start.elapsed() < Duration::from_millis(250) {
            thread::sleep(Duration::from_millis(10));
        }
    }
    cancel.is_cancelled()
}

/// `"{tool} stopped unexpectedly (signal: 11 (SIGSEGV))"`: a crash.
pub(crate) fn stopped(tool: &str, status: Option<&ExitStatus>) -> String {
    format!("{tool} stopped unexpectedly ({})", describe(status))
}

fn describe(status: Option<&ExitStatus>) -> String {
    status.map_or_else(|| "no exit status".to_string(), |s| s.to_string())
}

/// The fraction done from one `-progress` line, given the input's
/// duration in seconds: `out_time_us` over the duration, and 1.0 at
/// `progress=end`.
pub(crate) fn progress_fraction(line: &str, duration: f64) -> Option<f32> {
    let (key, value) = line.split_once('=')?;
    match key {
        "progress" if value.trim() == "end" => Some(1.0),
        "out_time_us" if duration > 0.0 => {
            let us: f64 = value.trim().parse().ok()?;
            Some((us / 1e6 / duration).clamp(0.0, 1.0) as f32)
        }
        _ => None,
    }
}

/// What a failed FFmpeg run means. `cancelled` comes from
/// [`was_cancelled`]: a cancelled run is `CANCELLED` whatever its exit,
/// never a corrupt input. A crash (no exit code) is `CORRUPT_INPUT`: it was
/// this input that FFmpeg couldn't get through.
pub(crate) fn classify(exit: Option<i32>, cancelled: bool, stderr: &str) -> ErrorCode {
    if cancelled {
        return ErrorCode::Cancelled;
    }
    if exit.is_none() {
        return ErrorCode::CorruptInput;
    }
    let has = |s: &str| stderr.contains(s);
    if has("No space left on device") || has("Permission denied") || has("Read-only file system") {
        ErrorCode::OutputNotWritable
    } else if has("Unknown encoder") || has("Encoder not found") {
        ErrorCode::BackendMissing
    } else {
        ErrorCode::CorruptInput
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::backend::ffmpeg::Ffmpeg;
    use crate::error::ErrorCode;
    use crate::exec::CancelToken;
    use persimmon_corpus::media::test_ffmpeg;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    #[test]
    fn progress_fraction_cases() {
        assert_eq!(progress_fraction("out_time_us=1000000", 2.0), Some(0.5));
        assert_eq!(progress_fraction("out_time_us=3000000", 2.0), Some(1.0));
        assert_eq!(progress_fraction("out_time_us=-5", 2.0), Some(0.0));
        assert_eq!(progress_fraction("progress=end", 2.0), Some(1.0));
        assert_eq!(progress_fraction("progress=continue", 2.0), None);
        assert_eq!(progress_fraction("out_time_us=N/A", 2.0), None);
        assert_eq!(progress_fraction("frame=12", 2.0), None);
    }

    #[test]
    fn classify_cases() {
        assert_eq!(classify(Some(255), true, ""), ErrorCode::Cancelled);
        assert_eq!(classify(None, true, ""), ErrorCode::Cancelled);
        assert_eq!(classify(Some(1), true, "anything"), ErrorCode::Cancelled);
        // Without a cancel, a crash or exit 255 is a real failure.
        assert_eq!(classify(Some(255), false, ""), ErrorCode::CorruptInput);
        assert_eq!(classify(None, false, ""), ErrorCode::CorruptInput);
        assert_eq!(
            classify(
                Some(1),
                false,
                "x\nav_interleaved_write_frame(): No space left on device"
            ),
            ErrorCode::OutputNotWritable
        );
        assert_eq!(
            classify(Some(1), false, "file:/x.part: Permission denied"),
            ErrorCode::OutputNotWritable
        );
        assert_eq!(
            classify(Some(1), false, "Read-only file system"),
            ErrorCode::OutputNotWritable
        );
        assert_eq!(
            classify(Some(8), false, "Unknown encoder 'libx265'"),
            ErrorCode::BackendMissing
        );
        assert_eq!(
            classify(Some(1), false, "Invalid data found when processing input"),
            ErrorCode::CorruptInput
        );
    }

    fn ff() -> Option<Ffmpeg> {
        let Some(path) = test_ffmpeg() else {
            eprintln!("skipped: no ffmpeg");
            return None;
        };
        Some(Ffmpeg::locate(Some(&path)).unwrap())
    }

    fn lavfi(seconds: u32, temp: &Path) -> Vec<OsString> {
        let mut v: Vec<OsString> = format!(
            "-nostdin -hide_banner -loglevel error -nostats -progress pipe:1 -stats_period 0.05 -y -f lavfi -i testsrc2=duration={seconds}:size=640x360:rate=25 -c:v mpeg4 -f mp4"
        )
        .split_whitespace()
        .map(OsString::from)
        .collect();
        v.push(super::super::file_url(temp));
        v
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(".prsm-child-{name}-{}.part", std::process::id()))
    }

    #[test]
    fn run_ffmpeg_reports_progress() {
        let Some(ff) = ff() else { return };
        let t = temp("progress");
        let mut seen = Vec::new();
        run_ffmpeg(
            &ff,
            &lavfi(120, &t),
            Some(120.0),
            &CancelToken::new(),
            &mut |f| seen.push(f),
        )
        .unwrap();
        assert!(seen.len() >= 3, "{seen:?}");
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
        assert_eq!(seen.last(), Some(&1.0));
        assert!(t.is_file());
        std::fs::remove_file(&t).unwrap();
    }

    #[test]
    fn run_ffmpeg_cancel_kills_child() {
        let Some(ff) = ff() else { return };
        let t = temp("cancel");
        let cancel = CancelToken::new();
        let mut cancelled_at = None;
        let result = run_ffmpeg(&ff, &lavfi(600, &t), Some(600.0), &cancel, &mut |f| {
            if f > 0.0 && cancelled_at.is_none() {
                cancel.cancel();
                cancelled_at = Some(Instant::now());
            }
        });
        let at = cancelled_at.expect("progress before the end");
        assert!(at.elapsed() < Duration::from_secs(2), "{:?}", at.elapsed());
        assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
        // Nothing holds the file any more.
        let _ = std::fs::remove_file(&t);
        assert!(!t.exists());
    }

    #[test]
    fn run_ffmpeg_failure_has_a_message() {
        let Some(ff) = ff() else { return };
        let missing = std::env::temp_dir().join("prsm-no-such-input.mov");
        let mut args: Vec<OsString> = ["-nostdin", "-loglevel", "error", "-i"]
            .map(OsString::from)
            .to_vec();
        args.push(super::super::file_url(&missing));
        args.extend(["-f", "mp4"].map(OsString::from));
        args.push(super::super::file_url(&temp("fail")));
        let e = run_ffmpeg(&ff, &args, None, &CancelToken::new(), &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert!(e.message.contains("No such file"), "{}", e.message);
    }

    /// A real FFmpeg whose binary is replaced by a shell script.
    #[cfg(unix)]
    pub(crate) fn fake(ff: &Ffmpeg, which: &str, body: &str) -> Ffmpeg {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "prsm-fake-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(which);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut ff = ff.clone();
        match which {
            "ffmpeg" => ff.ffmpeg = path,
            _ => ff.ffprobe = path,
        }
        ff
    }

    /// Set `cancel` after `ms`, as prsm's Ctrl-C handler does on its own
    /// thread, a moment after the terminal's signal reached the child.
    pub(crate) fn cancel_later(cancel: &CancelToken, ms: u64) {
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            c.cancel();
        });
    }

    #[cfg(unix)]
    #[test]
    fn crash_without_cancel_is_not_cancelled() {
        let Some(ff) = ff() else { return };
        let ff = fake(&ff, "ffmpeg", "kill -SEGV $$");
        let e = run_ffmpeg(&ff, &[], None, &CancelToken::new(), &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert!(
            e.message.starts_with("ffmpeg stopped unexpectedly"),
            "{}",
            e.message
        );
    }

    #[cfg(unix)]
    #[test]
    fn signal_death_with_late_cancel_is_cancelled() {
        let Some(ff) = ff() else { return };
        let ff = fake(&ff, "ffmpeg", "kill -INT $$");
        let cancel = CancelToken::new();
        cancel_later(&cancel, 100);
        let e = run_ffmpeg(&ff, &[], None, &cancel, &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
    }
}
