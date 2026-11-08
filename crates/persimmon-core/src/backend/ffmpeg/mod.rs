// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Audio and video through a system FFmpeg (DD-29), always run as a child
//! process (DD-09), never linked.

mod locate;

pub use locate::Ffmpeg;
