// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Structured errors (DD-17). Every failure carries a machine code, a
//! reason, the file it concerns where there is one, and a fix where one
//! exists. Each code maps to a stable process exit code; the table is the
//! public contract in the README's "Exit codes" section.

use std::fmt;
use std::path::PathBuf;

/// What kind of failure an [`Error`] is. The exit codes never change once
/// released: scripts depend on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    Internal,
    Usage,
    UnsupportedInput,
    BackendMissing,
    CorruptInput,
    OutputNotWritable,
    InputNotFound,
    UnsupportedConversion,
    OutputExists,
    Cancelled,
}

impl ErrorCode {
    /// Every code. Exit 10, `NOT_IMPLEMENTED` before v0.1.0, is retired
    /// and never reused.
    pub const ALL: [ErrorCode; 10] = [
        ErrorCode::Internal,
        ErrorCode::Usage,
        ErrorCode::UnsupportedInput,
        ErrorCode::BackendMissing,
        ErrorCode::CorruptInput,
        ErrorCode::OutputNotWritable,
        ErrorCode::InputNotFound,
        ErrorCode::UnsupportedConversion,
        ErrorCode::OutputExists,
        ErrorCode::Cancelled,
    ];

    /// The process exit code for this failure.
    pub fn exit_code(self) -> u8 {
        match self {
            ErrorCode::Internal => 1,
            ErrorCode::Usage => 2,
            ErrorCode::UnsupportedInput => 3,
            ErrorCode::BackendMissing => 4,
            ErrorCode::CorruptInput => 5,
            ErrorCode::OutputNotWritable => 6,
            ErrorCode::InputNotFound => 7,
            ErrorCode::UnsupportedConversion => 8,
            ErrorCode::OutputExists => 9,
            ErrorCode::Cancelled => 130,
        }
    }

    /// The machine-readable name, e.g. `"BACKEND_MISSING"`.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Internal => "INTERNAL",
            ErrorCode::Usage => "USAGE",
            ErrorCode::UnsupportedInput => "UNSUPPORTED_INPUT",
            ErrorCode::BackendMissing => "BACKEND_MISSING",
            ErrorCode::CorruptInput => "CORRUPT_INPUT",
            ErrorCode::OutputNotWritable => "OUTPUT_NOT_WRITABLE",
            ErrorCode::InputNotFound => "INPUT_NOT_FOUND",
            ErrorCode::UnsupportedConversion => "UNSUPPORTED_CONVERSION",
            ErrorCode::OutputExists => "OUTPUT_EXISTS",
            ErrorCode::Cancelled => "CANCELLED",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a fix asks the user to do, so a client that isn't the CLI can say
/// it in its own words. `Plain` fixes read the same anywhere; every other
/// kind's text names a `prsm` flag, command or install step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixKind {
    Plain,
    /// Write the output somewhere else (`--out`).
    ChooseOutputFolder,
    /// Replace the existing file (`--overwrite`).
    Overwrite,
    /// Say what to convert to (`--to`).
    AddTarget,
    /// Leave out an option this job doesn't take (`"pages"`, `"preset"`,
    /// `"max-size"`).
    DropOption {
        option: String,
    },
    /// Choose valid pages (`--pages`).
    PageRange,
    /// Pick an audio target for a file with no video.
    PickAudioTarget,
    /// Pick a video target for a file with video.
    PickVideoTarget,
    /// The image is too large for the target: shrink it to at most `max`
    /// pixels, or pick another format.
    ShrinkOrPickTarget {
        max: u32,
    },
    /// Use compress instead of converting to the same format.
    UseCompress,
    /// Look up what the format can become.
    SeeFormats,
    /// FFmpeg is missing, too old, or not where it was said to be.
    InstallFfmpeg,
    /// The image backend (libvips) is missing or too old.
    InstallLibvips,
    /// RAR needs libarchive.
    InstallLibarchive,
}

/// How to get past a failure: its kind, and the CLI's wording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub kind: FixKind,
    /// The wording `prsm` prints.
    pub text: String,
}

impl fmt::Display for Fix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// A failure, shaped for people and scripts alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: ErrorCode,
    pub file: Option<PathBuf>,
    pub reason: String,
    pub fix: Option<Fix>,
}

impl Error {
    pub fn new(code: ErrorCode, reason: impl Into<String>) -> Error {
        Error {
            code,
            file: None,
            reason: reason.into(),
            fix: None,
        }
    }

    pub fn with_file(mut self, file: impl Into<PathBuf>) -> Error {
        self.file = Some(file.into());
        self
    }

    /// A fix whose wording works in any client (kind `Plain`). A fix that
    /// names a flag, a `prsm` command or an install step needs a kind:
    /// use [`Error::with_fix_as`].
    pub fn with_fix(self, fix: impl Into<String>) -> Error {
        let text = fix.into();
        debug_assert!(
            !["--", "prsm ", "brew ", "winget "]
                .iter()
                .any(|cli| text.contains(cli)),
            "a plain fix names a CLI flag or command: {text}"
        );
        self.with_fix_as(FixKind::Plain, text)
    }

    pub fn with_fix_as(mut self, kind: FixKind, text: impl Into<String>) -> Error {
        self.fix = Some(Fix {
            kind,
            text: text.into(),
        });
        self
    }

    /// The fix as `prsm` words it.
    pub fn fix_text(&self) -> Option<&str> {
        self.fix.as_ref().map(|f| f.text.as_str())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{}: ", file.display())?;
        }
        f.write_str(&self.reason)?;
        if let Some(fix) = &self.fix {
            write!(f, "\n  fix: {fix}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(ErrorCode::Internal.exit_code(), 1);
        assert_eq!(ErrorCode::Usage.exit_code(), 2);
        assert_eq!(ErrorCode::UnsupportedInput.exit_code(), 3);
        assert_eq!(ErrorCode::BackendMissing.exit_code(), 4);
        assert_eq!(ErrorCode::CorruptInput.exit_code(), 5);
        assert_eq!(ErrorCode::OutputNotWritable.exit_code(), 6);
        assert_eq!(ErrorCode::InputNotFound.exit_code(), 7);
        assert_eq!(ErrorCode::UnsupportedConversion.exit_code(), 8);
        assert_eq!(ErrorCode::OutputExists.exit_code(), 9);
        assert_eq!(ErrorCode::Cancelled.exit_code(), 130);
    }

    #[test]
    fn no_code_exits_10() {
        // NOT_IMPLEMENTED (exit 10) was retired with the last backend;
        // 10 is reserved and never reused.
        let mut exits: Vec<u8> = ErrorCode::ALL.iter().map(|c| c.exit_code()).collect();
        assert!(!exits.contains(&10));
        assert!(
            ErrorCode::ALL
                .iter()
                .all(|c| c.as_str() != "NOT_IMPLEMENTED")
        );
        exits.sort_unstable();
        exits.dedup();
        assert_eq!(exits.len(), ErrorCode::ALL.len(), "exit codes are distinct");
    }

    #[test]
    fn codes_are_screaming_snake() {
        assert_eq!(ErrorCode::OutputNotWritable.as_str(), "OUTPUT_NOT_WRITABLE");
        assert_eq!(ErrorCode::BackendMissing.as_str(), "BACKEND_MISSING");
        assert_eq!(
            ErrorCode::UnsupportedConversion.as_str(),
            "UNSUPPORTED_CONVERSION"
        );
    }

    #[test]
    fn display_with_file_and_fix() {
        let e = Error::new(ErrorCode::BackendMissing, "libreoffice not found")
            .with_file("notes.docx")
            .with_fix("install libreoffice and retry");
        assert_eq!(
            e.to_string(),
            "notes.docx: libreoffice not found\n  fix: install libreoffice and retry"
        );
    }

    #[test]
    fn fix_kind_keeps_the_cli_text() {
        let e = Error::new(ErrorCode::OutputNotWritable, "can't write to this folder")
            .with_file("/x")
            .with_fix_as(
                FixKind::ChooseOutputFolder,
                "choose another folder with --out <dir>",
            );
        assert_eq!(
            e.to_string(),
            "/x: can't write to this folder\n  fix: choose another folder with --out <dir>"
        );
        assert_eq!(e.fix.as_ref().unwrap().kind, FixKind::ChooseOutputFolder);
        assert_eq!(e.fix_text(), Some("choose another folder with --out <dir>"));
    }

    #[test]
    fn with_fix_is_plain() {
        let e = Error::new(ErrorCode::CorruptInput, "x")
            .with_fix("try again once nothing is writing to it");
        assert_eq!(e.fix.unwrap().kind, FixKind::Plain);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a plain fix names a CLI flag or command")]
    fn plain_fix_with_a_flag_panics_in_debug() {
        let _ = Error::new(ErrorCode::Usage, "x").with_fix("add --to <format>");
    }

    #[test]
    fn display_without_file() {
        assert_eq!(
            Error::new(ErrorCode::Usage, "no files given").to_string(),
            "no files given"
        );
    }
}
