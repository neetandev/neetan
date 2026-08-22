//! Machine construction shared by the interactive and headless frontends.
//!
//! The `config` module holds the pure-data configuration types and the model
//! resolver. The `machines` module builds fully configured machines from an
//! `EmulatorConfig`.

#[cfg(not(any(
    feature = "pc98",
    feature = "pc88",
    feature = "pc88va",
    feature = "pc60",
    feature = "msx",
    feature = "towns",
    feature = "x1",
    feature = "fm7",
    feature = "x68k",
    feature = "at"
)))]
compile_error!(
    "at least one machine feature must be enabled: pc98, pc88, pc88va, pc60, msx, towns, x1, \
     fm7, x68k or at (or the all-machines feature)"
);

pub mod config;
pub mod machines;

/// Error kind for machine construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitErrorKind {
    /// The requested configuration is invalid.
    BadSpec,
    /// A required ROM directory or ROM file is missing.
    RomMissing,
    /// An I/O or generic construction failure.
    Io,
    /// The requested feature is not supported by the machine.
    Unsupported,
}

/// A machine construction failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitError {
    /// The category of the failure.
    pub kind: InitErrorKind,
    /// A human-readable description of the failure.
    pub message: String,
}

impl InitError {
    /// Builds a ROM-missing error.
    pub fn rom_missing(message: impl Into<String>) -> Self {
        Self {
            kind: InitErrorKind::RomMissing,
            message: message.into(),
        }
    }

    /// Builds a bad-specification error.
    pub fn bad_spec(message: impl Into<String>) -> Self {
        Self {
            kind: InitErrorKind::BadSpec,
            message: message.into(),
        }
    }

    /// Builds an unsupported-feature error.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self {
            kind: InitErrorKind::Unsupported,
            message: message.into(),
        }
    }
}

impl core::fmt::Display for InitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InitError {}

impl From<common::StringError> for InitError {
    fn from(e: common::StringError) -> Self {
        Self {
            kind: InitErrorKind::Io,
            message: e.to_string(),
        }
    }
}

impl From<common::ContextError> for InitError {
    fn from(e: common::ContextError) -> Self {
        Self {
            kind: InitErrorKind::Io,
            message: e.to_string(),
        }
    }
}

impl From<InitError> for common::Error {
    fn from(error: InitError) -> Self {
        Self::Message(common::StringError(error.message))
    }
}
