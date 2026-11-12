// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The Persimmon conversion engine.
//!
//! Platform-neutral by rule (DD-22): code that differs per operating system
//! lives only in [`platform`]. Everything else must compile and pass its tests
//! unchanged on macOS, Windows and Linux.

pub mod backend;
pub mod detect;
pub mod error;
pub mod exec;
pub mod format;
pub mod job;
pub mod output;
pub mod platform;
pub mod registry;
#[cfg(feature = "serde")]
mod serde_util;
pub mod update;

/// The engine's version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
