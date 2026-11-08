// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Audio and video through a system FFmpeg (DD-29), always run as a child
//! process (DD-09), never linked.

// The backend that calls it lands in a later commit.
#[allow(dead_code)]
mod args;
#[allow(dead_code)]
mod child;
mod locate;
mod probe;

pub use locate::Ffmpeg;
pub use probe::{MediaInfo, Stream, StreamKind};

use std::ffi::OsString;
use std::path::Path;

/// `path` as an FFmpeg `file:` URL, so a name that starts with `-` or holds
/// a `:` or `|` is never read as an option or a protocol.
pub(crate) fn file_url(path: &Path) -> OsString {
    let mut url = OsString::from("file:");
    url.push(path);
    url
}

/// The last useful line of FFmpeg's stderr, for an error's reason: path
/// prefixes removed, temp files never shown, at most 160 characters.
pub(crate) fn message(stderr: &str, paths: &[&Path]) -> String {
    let line = stderr
        .lines()
        .rev()
        .map(|l| {
            let mut l = l.trim();
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
            "Conversion failed!"
        );
        assert_eq!(message(&"x".repeat(400), &[p]).chars().count(), 160);
        assert_eq!(message("", &[p]), "");
    }

    #[test]
    fn file_url_prefixes() {
        assert_eq!(file_url(Path::new("-a:b|c.mp4")), "file:-a:b|c.mp4");
    }
}
