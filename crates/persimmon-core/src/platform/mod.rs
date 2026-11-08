// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The only place in the core allowed to contain platform-conditional code
//! (DD-22). Keep it thin: each item here is a small, documented difference
//! between operating systems, never a feature. Anything macOS-only that lands
//! above this boundary is recorded in the port notes.

/// The operating system this build targets, as a short lowercase name.
pub const fn os_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "other"
    }
}

/// Folders searched for FFmpeg after `PATH`. On macOS, Homebrew's: apps
/// started from Finder don't inherit a shell's `PATH`.
pub const fn ffmpeg_dirs() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &["/opt/homebrew/bin", "/usr/local/bin"]
    }
    #[cfg(not(target_os = "macos"))]
    {
        &[]
    }
}

/// How to install FFmpeg here, as an error's fix.
pub const fn ffmpeg_install() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "brew install ffmpeg"
    }
    #[cfg(target_os = "windows")]
    {
        "winget install Gyan.FFmpeg"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "install ffmpeg 7 or later with your package manager"
    }
}

#[cfg(test)]
mod tests {
    use super::os_name;

    #[test]
    fn os_name_matches_the_target() {
        assert_eq!(os_name(), std::env::consts::OS);
    }

    #[test]
    fn ffmpeg_hints_match_the_os() {
        let install = super::ffmpeg_install();
        match std::env::consts::OS {
            "macos" => {
                assert_eq!(install, "brew install ffmpeg");
                assert_eq!(
                    super::ffmpeg_dirs(),
                    ["/opt/homebrew/bin", "/usr/local/bin"]
                );
            }
            "windows" => assert_eq!(install, "winget install Gyan.FFmpeg"),
            _ => assert!(install.contains("ffmpeg 7")),
        }
    }
}
