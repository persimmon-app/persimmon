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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// The window may call the app's own commands and nothing else: no
    /// core defaults (tray, menus, paths, devtools), no plugins.
    #[test]
    fn capability_allows_only_the_apps_commands() {
        let cap: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
        assert_eq!(cap["windows"], serde_json::json!(["main"]));
        assert_eq!(
            cap["permissions"],
            serde_json::json!(["allow-engine-version"])
        );
    }
}
