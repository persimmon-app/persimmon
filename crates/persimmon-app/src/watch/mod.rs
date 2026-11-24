// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Watch folders in the app (spec D2): the rules, each rule's record of
//! the files it handled, the batches its files make, and the runner.

pub mod burst;
pub mod record;
pub mod rule;
pub mod runner;
pub mod views;
