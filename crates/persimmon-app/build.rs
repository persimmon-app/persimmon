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
            "presets",
            "use_preset",
            "apply_preset",
            "save_preset",
            "delete_preset",
            "history",
            "undo_batch",
            "undo_last",
            "clear_history",
            "reveal_path",
            "engines",
            "set_drop_window_at_launch",
            "open_licences",
            "open_sources",
            "retry_item",
            "watch_rules",
            "watch_targets",
            "watch_patterns",
            "pick_watch_folder",
            "new_rule",
            "rule_command",
            "save_rule",
            "toggle_rule",
            "remove_rule",
            "scan_rule",
            "copy_command",
            "set_start_at_login",
            "notifications",
            "open_notification_settings",
        ]);
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
            .expect("tauri-build failed");
        // The packaged app carries libvips and the rest in
        // Contents/Frameworks, named @rpath/<name>. Development builds link
        // Homebrew's libraries by absolute name, so this changes nothing there.
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path/../Frameworks");
    }
}
