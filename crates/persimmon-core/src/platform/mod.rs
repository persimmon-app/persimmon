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

use std::path::Path;

/// Create a symlink at `link` pointing at `target`, as stored. `Ok(false)`
/// means this platform refused and the link was skipped: Windows only
/// creates symlinks with Developer Mode or elevation.
pub fn symlink(target: &Path, link: &Path) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).map(|()| true)
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_file(target, link) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(false),
            Err(e) if e.raw_os_error() == Some(1314) => Ok(false), // ERROR_PRIVILEGE_NOT_HELD
            Err(e) => Err(e),
        }
    }
}

/// Make `path` 0755 when `executable`, else 0644. Windows has no such
/// bits; it's a no-op there.
pub fn set_mode(path: &Path, executable: bool) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, executable);
        Ok(())
    }
}

/// The mode to store for a file packed into an archive: its own read,
/// write and execute bits (a private 0600 file stays private), without
/// set-uid, set-gid or sticky. Windows has no such bits: 0644.
pub fn file_mode(meta: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0o644
    }
}

/// Whether this platform can hold a file called `name` (one path
/// component, already free of `/`, `\` and NUL). Windows refuses
/// `<>:"|?*`, control characters, a trailing dot or space, and device
/// names such as `CON` or `com1.txt`.
pub fn file_name_ok(name: &str) -> bool {
    #[cfg(windows)]
    {
        if name.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c)) || name.ends_with(['.', ' ']) {
            return false;
        }
        let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        let device = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && base.as_bytes()[3].is_ascii_digit());
        !device
    }
    #[cfg(not(windows))]
    {
        !name.is_empty()
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
