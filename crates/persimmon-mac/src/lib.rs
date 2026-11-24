// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Persimmon's native macOS layer: what Tauri doesn't wrap.
//!
//! The only crate where `unsafe` may be allowed: each block carries
//! `#[allow(unsafe_code)]` and a `// SAFETY:` comment. Everything it
//! offers the app is a safe function. On other platforms it's empty.

#![cfg(target_os = "macos")]

use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSRunningApplication};
use objc2_foundation::NSString;

pub mod gesture;
pub mod login;
pub mod notes;
pub mod panel;
pub mod promises;
pub mod services;

/// Whether this process is the active app: the one whose window is in
/// front and takes the keyboard.
pub fn app_is_active() -> bool {
    NSRunningApplication::currentApplication().isActive()
}

/// Put `text` on the clipboard, as plain text.
pub fn copy_text(text: &str) -> bool {
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    #[allow(unsafe_code)]
    // SAFETY: NSPasteboardTypeString is an immutable string constant that
    // AppKit defines for the process's whole life.
    let kind = unsafe { NSPasteboardTypeString };
    board.setString_forType(&NSString::from_str(text), kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_text_has_a_safe_signature() {
        // Not run against the real pasteboard: that's the person's clipboard.
        let _: fn(&str) -> bool = copy_text;
    }

    #[test]
    fn app_is_active_is_false_in_a_test_process() {
        assert!(!app_is_active());
    }
}
