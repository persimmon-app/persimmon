// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Persimmon.app: a window over the conversion engine. macOS only for now;
//! elsewhere this builds a stub that says so.

#[cfg(target_os = "macos")]
#[tauri::command]
fn engine_version() -> String {
    persimmon_core::VERSION.to_string()
}

#[cfg(target_os = "macos")]
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![engine_version])
        .run(tauri::generate_context!())
        .expect("Persimmon couldn't start");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Persimmon's app is macOS-only for now");
}
