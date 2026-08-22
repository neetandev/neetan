//! Error types, context traits, and macros for error handling.

use alloc::{
    borrow::{Cow, ToOwned},
    boxed::Box,
    string::String,
};

/// An error that wraps a source error with a context message.
pub struct ContextError {
    source: Box<dyn core::error::Error + Send + Sync>,
    context: Cow<'static, str>,
}

impl ContextError {
    fn new(
        source: impl core::error::Error + Send + Sync + 'static,
        context: Cow<'static, str>,
    ) -> Self {
        Self {
            source: Box::new(source),
            context,
        }
    }
}

impl core::fmt::Display for ContextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if f.alternate() {
            write!(f, "{}: {:#}", self.context, self.source)
        } else {
            write!(f, "{}", self.context)
        }
    }
}

impl core::fmt::Debug for ContextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:#}")
    }
}

impl core::error::Error for ContextError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&*self.source)
    }
}

/// A simple string-based error.
pub struct StringError(pub String);

impl core::fmt::Display for StringError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::fmt::Debug for StringError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self}")
    }
}

impl core::error::Error for StringError {}

/// Extension trait on `Result<T, E>` for adding context to errors.
pub trait Context<T, E> {
    /// Wraps the error with a static context message.
    fn context(self, msg: &'static str) -> Result<T, ContextError>;

    /// Wraps the error with a lazily-evaluated context message.
    fn with_context<F: FnOnce() -> String>(self, f: F) -> Result<T, ContextError>;
}

impl<T, E: core::error::Error + Send + Sync + 'static> Context<T, E> for Result<T, E> {
    fn context(self, msg: &'static str) -> Result<T, ContextError> {
        self.map_err(|e| ContextError::new(e, Cow::Borrowed(msg)))
    }

    fn with_context<F: FnOnce() -> String>(self, f: F) -> Result<T, ContextError> {
        self.map_err(|e| ContextError::new(e, Cow::Owned(f())))
    }
}

/// Extension trait on `Option<T>` for converting `None` into an error with context.
pub trait OptionContext<T> {
    /// Converts `None` into an error with the given message.
    fn context(self, msg: &'static str) -> Result<T, StringError>;
}

impl<T> OptionContext<T> for Option<T> {
    fn context(self, msg: &'static str) -> Result<T, StringError> {
        self.ok_or_else(|| StringError(msg.to_owned()))
    }
}

/// Returns early with an error built from a format string.
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::error::StringError(format!($($arg)*)).into())
    };
}

/// Returns early with an error if the condition is not satisfied.
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            $crate::bail!($($arg)*)
        }
    };
}

/// An error carrying either a context chain or a plain message.
pub enum Error {
    /// An error with context.
    Context(ContextError),
    /// An error with a message.
    Message(StringError),
}

impl core::fmt::Display for Error {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Context(error) => core::fmt::Display::fmt(error, formatter),
            Self::Message(error) => core::fmt::Display::fmt(error, formatter),
        }
    }
}

impl core::fmt::Debug for Error {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Context(error) => core::fmt::Debug::fmt(error, formatter),
            Self::Message(error) => core::fmt::Debug::fmt(error, formatter),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Context(error) => error.source(),
            Self::Message(error) => error.source(),
        }
    }
}

impl From<ContextError> for Error {
    fn from(error: ContextError) -> Self {
        Self::Context(error)
    }
}

impl From<StringError> for Error {
    fn from(error: StringError) -> Self {
        Self::Message(error)
    }
}

#[cfg(feature = "std")]
impl From<std::ffi::NulError> for Error {
    fn from(error: std::ffi::NulError) -> Self {
        use alloc::string::ToString;
        Self::Message(StringError(error.to_string()))
    }
}

/// Result alias defaulting to the shared [`Error`].
pub type Result<T, E = Error> = core::result::Result<T, E>;
