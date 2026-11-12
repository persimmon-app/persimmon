// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The app's logic, apart from its window: the words it shows, the queue
//! and the runner. macOS only for now (WD-10).

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
pub mod queue;
#[cfg(target_os = "macos")]
pub mod runner;
#[cfg(target_os = "macos")]
pub use app::run;
#[cfg(target_os = "macos")]
pub mod wording;

/// Tests that start FFmpeg hold this, so a test that checks no FFmpeg child
/// is left can't see another test's.
#[cfg(all(test, target_os = "macos"))]
pub(crate) static FFMPEG_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(target_os = "macos")]
pub mod reveal;
#[cfg(target_os = "macos")]
pub mod settings;
