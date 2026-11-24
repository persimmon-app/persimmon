// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Watch folders (D2): file events become settled, matched inputs, each
//! converted once.

mod patterns;
mod tracker;
mod watcher;

pub use patterns::Patterns;
pub use tracker::Stamp;
pub use watcher::{Change, Timing, WatchSpec, Watcher, files_in};
