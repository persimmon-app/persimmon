// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Backends: the engine components that turn one planned task into one
//! output. The executor ([`crate::exec`]) handles naming, temp files and
//! commits; a backend only writes the finished output to the temp path it
//! is given.

#[cfg(feature = "vips")]
pub mod image;

pub mod archive;
pub mod ffmpeg;
pub mod pdf;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::error::Error;
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
///
/// FFmpeg is looked up on the first audio or video task: the task's own
/// `--ffmpeg` if it has one, else the one given to [`Backends::with_ffmpeg`],
/// else the one on `PATH`. A successful lookup is kept for later tasks that
/// ask for the same FFmpeg; a failed one is tried again on the next task,
/// so installing FFmpeg needs no restart.
#[derive(Debug, Default)]
pub struct Backends {
    ffmpeg_path: Option<PathBuf>,
    ffmpeg: Mutex<Option<(Option<PathBuf>, Arc<ffmpeg::FfmpegBackend>)>>,
}

impl Backends {
    pub fn new() -> Backends {
        Backends::default()
    }

    /// Use this FFmpeg (with its `ffprobe` beside it) unless a task names
    /// its own; for example, the copy an app bundles.
    pub fn with_ffmpeg(mut self, path: impl Into<PathBuf>) -> Backends {
        self.ffmpeg_path = Some(path.into());
        self
    }

    /// The FFmpeg backend for a task whose `--ffmpeg` is `explicit`.
    pub(crate) fn ffmpeg_for(
        &self,
        explicit: Option<&Path>,
    ) -> Result<Arc<ffmpeg::FfmpegBackend>, Error> {
        let want = explicit.or(self.ffmpeg_path.as_deref());
        let mut cache = self.ffmpeg.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((key, backend)) = cache.as_ref()
            && key.as_deref() == want
        {
            return Ok(Arc::clone(backend));
        }
        let backend = Arc::new(ffmpeg::FfmpegBackend::new(ffmpeg::Ffmpeg::locate(want)?));
        *cache = Some((want.map(Path::to_path_buf), Arc::clone(&backend)));
        Ok(backend)
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
                Err(Error::new(
                    crate::error::ErrorCode::BackendMissing,
                    "this build has no image backend",
                )
                .with_file(&task.inputs[0])
                .with_fix_as(
                    crate::error::FixKind::InstallLibvips,
                    "install a build of prsm made with libvips",
                ))
            }
            Backend::Ffmpeg => match self.ffmpeg_for(options.ffmpeg.as_deref()) {
                Ok(backend) => backend.run(task, options, temp, cancel, progress),
                Err(e) => Err(e.with_file(&task.inputs[0])),
            },
            Backend::Pdf => pdf::PdfBackend.run(task, options, temp, cancel, progress),
            Backend::Archive => archive::ArchiveBackend.run(task, options, temp, cancel, progress),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::error::{ErrorCode, FixKind};

    #[test]
    fn backends_are_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Backends>();
    }

    /// A fresh folder in the system temp folder, removed when dropped.
    #[cfg(unix)]
    struct Scratch(PathBuf);

    #[cfg(unix)]
    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir =
                std::env::temp_dir().join(format!("prsm-backends-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn script(&self, name: &str, body: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let p = self.0.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
    }

    #[cfg(unix)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    const VERSION_7: &str = "echo 'ffmpeg version 7.1 Copyright'";

    #[cfg(unix)]
    #[test]
    fn failed_ffmpeg_lookup_is_retried() {
        let dir = Scratch::new("retry");
        let backends = Backends::new().with_ffmpeg(dir.0.join("ffmpeg"));
        let e = backends.ffmpeg_for(None).unwrap_err();
        assert_eq!(e.code, ErrorCode::BackendMissing);
        assert_eq!(e.fix.as_ref().unwrap().kind, FixKind::InstallFfmpeg);
        dir.script("ffmpeg", VERSION_7);
        dir.script("ffprobe", VERSION_7);
        assert!(backends.ffmpeg_for(None).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn configured_ffmpeg_that_is_a_folder_is_missing() {
        let dir = Scratch::new("folder");
        let backends = Backends::new().with_ffmpeg(&dir.0);
        let e = backends.ffmpeg_for(None).unwrap_err();
        assert_eq!(e.code, ErrorCode::BackendMissing);
        assert_eq!(e.fix.unwrap().kind, FixKind::InstallFfmpeg);
    }

    #[cfg(unix)]
    #[test]
    fn job_ffmpeg_beats_the_configured_one() {
        let dir = Scratch::new("beats");
        let backends = Backends::new().with_ffmpeg(dir.0.join("missing/ffmpeg"));
        let ff = dir.script("ffmpeg", VERSION_7);
        dir.script("ffprobe", VERSION_7);
        assert!(backends.ffmpeg_for(Some(&ff)).is_ok());
        assert!(backends.ffmpeg_for(None).is_err());
    }
}
