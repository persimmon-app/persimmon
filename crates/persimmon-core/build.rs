// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! rs-vips links `-lvips` without saying where libvips lives. With the
//! `vips` feature, ask pkg-config for the library folder; without
//! pkg-config, fall back to the linker's default search path.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_VIPS").is_none() {
        return;
    }
    let Ok(out) = std::process::Command::new("pkg-config")
        .args(["--libs-only-L", "vips"])
        .output()
    else {
        return;
    };
    for flag in String::from_utf8_lossy(&out.stdout).split_whitespace() {
        if let Some(dir) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        }
    }
}
