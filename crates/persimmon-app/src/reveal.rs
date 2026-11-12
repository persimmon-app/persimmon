// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Showing an output in Finder.

use std::ffi::OsString;
use std::path::Path;

/// Arguments for `/usr/bin/open` that select `path` in a Finder window.
pub fn reveal_args(path: &Path) -> Vec<OsString> {
    vec![OsString::from("-R"), path.as_os_str().to_owned()]
}

/// Select `path` in Finder. False if it doesn't exist or `open` failed.
pub fn reveal(path: &Path) -> bool {
    path.exists()
        && std::process::Command::new("/usr/bin/open")
            .args(reveal_args(path))
            .status()
            .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_args_select_the_file() {
        assert_eq!(
            reveal_args(Path::new("/x/a.jpg")),
            vec![OsString::from("-R"), OsString::from("/x/a.jpg")]
        );
    }
}
