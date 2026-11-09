// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Backends: the engine components that turn one planned task into one
//! output. The executor ([`crate::exec`]) handles naming, temp files and
//! commits; a backend only writes the finished output to the temp path it
//! is given.

#[cfg(feature = "vips")]
pub mod image;

pub mod ffmpeg;
pub mod pdf;

use std::path::Path;
use std::sync::OnceLock;

use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;
use crate::job::{JobOptions, PlannedTask};
use crate::registry::Backend;

/// What a backend did with a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Done {
    /// The output is complete at the temp path.
    Written,
    /// Compressing wouldn't make the file smaller; nothing was written.
    NotSmaller,
}

/// Something that can carry out planned tasks.
pub trait Run {
    /// Write `task`'s output to `temp`. On error, anything at `temp` is
    /// removed by the caller. `progress` takes fractions of the task done,
    /// from 0 to 1, never decreasing; a backend that can't tell never
    /// calls it. A backend that can stop mid-task returns `CANCELLED` once
    /// `cancel` is set.
    fn run(
        &self,
        task: &PlannedTask,
        options: &JobOptions,
        temp: &Path,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(f32),
    ) -> Result<Done, Error>;
}

/// Every backend this build has, dispatched by [`PlannedTask::backend`].
/// FFmpeg is found once, on the first audio or video task, using that
/// task's `--ffmpeg`.
#[derive(Debug, Default)]
pub struct Backends {
    ffmpeg: OnceLock<Result<ffmpeg::FfmpegBackend, Error>>,
}

impl Backends {
    pub fn new() -> Backends {
        Backends::default()
    }
}

impl Run for Backends {
    fn run(
        &self,
        task: &PlannedTask,
        options: &JobOptions,
        temp: &Path,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(f32),
    ) -> Result<Done, Error> {
        match task.backend {
            #[cfg(feature = "vips")]
            Backend::Image => image::ImageBackend::new().run(task, options, temp, cancel, progress),
            #[cfg(not(feature = "vips"))]
            Backend::Image => {
                let _ = (options, temp, cancel, progress);
                Err(
                    Error::new(ErrorCode::BackendMissing, "this build has no image backend")
                        .with_file(&task.inputs[0])
                        .with_fix("install a build of prsm made with libvips"),
                )
            }
            Backend::Ffmpeg => {
                let found = self.ffmpeg.get_or_init(|| {
                    ffmpeg::Ffmpeg::locate(options.ffmpeg.as_deref())
                        .map(ffmpeg::FfmpegBackend::new)
                });
                match found {
                    Ok(backend) => backend.run(task, options, temp, cancel, progress),
                    Err(e) => Err(e.clone().with_file(&task.inputs[0])),
                }
            }
            Backend::Pdf => pdf::PdfBackend.run(task, options, temp, cancel, progress),
            other => {
                let e = Error::new(
                    ErrorCode::NotImplemented,
                    format!("the {} backend isn't built yet", other.as_str()),
                )
                .with_fix("use --dry-run to see the plan");
                // Many-to-one tasks have no single file to name.
                Err(match task.inputs.as_slice() {
                    [one] => e.with_file(one),
                    _ => e,
                })
            }
        }
    }
}
