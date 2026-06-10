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
