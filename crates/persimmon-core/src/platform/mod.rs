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

#[cfg(test)]
mod tests {
    use super::os_name;

    #[test]
    fn os_name_matches_the_target() {
        assert_eq!(os_name(), std::env::consts::OS);
    }
}
