// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Watch folders (D2): file events become settled, matched inputs, each
//! converted once.

mod patterns;
// The watcher (next) is the tracker's only caller.
#[allow(dead_code)]
mod tracker;

pub use patterns::Patterns;
pub use tracker::Stamp;
