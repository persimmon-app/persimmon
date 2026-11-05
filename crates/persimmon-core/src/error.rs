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
    /// Pre-0.1 only: the backend for this task isn't built yet.
    NotImplemented,
    Cancelled,
}

impl ErrorCode {
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
            ErrorCode::NotImplemented => 10,
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
            ErrorCode::NotImplemented => "NOT_IMPLEMENTED",
            ErrorCode::Cancelled => "CANCELLED",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failure, shaped for people and scripts alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: ErrorCode,
    pub file: Option<PathBuf>,
    pub reason: String,
    pub fix: Option<String>,
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

    pub fn with_fix(mut self, fix: impl Into<String>) -> Error {
        self.fix = Some(fix.into());
        self
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
        assert_eq!(ErrorCode::NotImplemented.exit_code(), 10);
        assert_eq!(ErrorCode::Cancelled.exit_code(), 130);
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
    fn display_without_file() {
        assert_eq!(
            Error::new(ErrorCode::Usage, "no files given").to_string(),
            "no files given"
        );
    }
}
