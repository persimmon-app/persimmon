// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The app's logic, apart from its window: the words it shows, the queue
//! and the runner. macOS only for now (WD-10).

#[cfg(target_os = "macos")]
pub mod queue;
#[cfg(target_os = "macos")]
pub mod wording;
