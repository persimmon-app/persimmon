// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

use std::process::Command;

const PRSM: &str = env!("CARGO_BIN_EXE_prsm");

#[test]
fn prints_its_version() {
    let out = Command::new(PRSM).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        format!("prsm {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn rejects_unknown_arguments_with_exit_code_2() {
    let out = Command::new(PRSM).arg("--frobnicate").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}
