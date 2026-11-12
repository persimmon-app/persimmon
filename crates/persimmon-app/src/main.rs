// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Persimmon.app: a window over the conversion engine. macOS only for now;
//! elsewhere this builds a stub that says so.

#[cfg(target_os = "macos")]
fn main() {
    persimmon_app::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Persimmon's app is macOS-only for now");
}
