// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Persimmon's native macOS layer: what Tauri doesn't wrap.
//!
//! The only crate where `unsafe` may be allowed: each block carries
//! `#[allow(unsafe_code)]` and a `// SAFETY:` comment. Everything it
//! offers the app is a safe function. On other platforms it's empty.

#![cfg(target_os = "macos")]

use objc2_app_kit::NSRunningApplication;

/// Whether this process is the active app: the one whose window is in
/// front and takes the keyboard.
pub fn app_is_active() -> bool {
    NSRunningApplication::currentApplication().isActive()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_is_active_is_false_in_a_test_process() {
        assert!(!app_is_active());
    }
}
