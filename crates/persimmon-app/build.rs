// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

fn main() {
    #[cfg(target_os = "macos")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Declaring the app's commands makes capabilities gate them too: a
        // window may call only the ones its capability allows.
        let manifest = tauri_build::AppManifest::new().commands(&[
            "engine_version",
            "add_paths",
            "pick_files",
            "set_target",
            "targets",
            "remove_rows",
            "queue_rows",
            "start",
            "cancel_row",
            "retry_row",
            "reveal_row",
            "quit_now",
            "remove_input",
            "reorder_input",
            "get_settings",
            "set_mode",
            "set_output_beside",
            "choose_output_folder",
            "choose_fallback_folder",
            "toggle_drop_window",
            "drop_paths",
            "open_settings",
        ]);
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
            .expect("tauri-build failed");
    }
}
