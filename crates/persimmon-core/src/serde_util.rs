// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Paths serialise lossily: a name that isn't valid UTF-8 comes out with
//! U+FFFD in place of the bad bytes, so serialising never fails.

use std::path::{Path, PathBuf};

use serde::Serializer;

pub fn path<S: Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&p.to_string_lossy())
}

pub fn opt_path<S: Serializer>(p: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
    match p {
        Some(p) => path(p, s),
        None => s.serialize_none(),
    }
}

pub fn paths<S: Serializer>(ps: &[PathBuf], s: S) -> Result<S::Ok, S::Error> {
    s.collect_seq(ps.iter().map(|p| p.to_string_lossy()))
}
