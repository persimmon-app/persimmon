// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The archive backend (DD-37): extracting ZIP, TAR (plain, gz, bz2, xz,
//! zst), 7z and RAR, and packing ZIP and tar.gz. Every reader feeds one
//! [`sink::Sink`], the only code that writes extracted files.

// Until the readers land (Task 3), only tests use the sink.
#[allow(dead_code)]
pub(crate) mod sink;
