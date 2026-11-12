// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

fn main() {
    #[cfg(target_os = "macos")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Declaring the app's commands makes capabilities gate them too: a
        // window may call only the ones its capability allows.
        let manifest = tauri_build::AppManifest::new().commands(&["engine_version"]);
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
            .expect("tauri-build failed");
    }
}
