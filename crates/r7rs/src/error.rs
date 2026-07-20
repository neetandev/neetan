use std::{error, fmt};

use crate::Span;

/// Broad stage in which an error occurred.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorPhase {
    /// Engine configuration or contract validation.
    Configuration,
    /// Source reading and lexical analysis.
    Read,
    /// Syntax expansion.
    Expand,
    /// Bytecode compilation.
    Compile,
    /// Scheme execution.
    Runtime,
    /// A configured resource limit was exceeded.
    Limit,
    /// An injected host capability failed or was unavailable.
    Host,
}

/// Stable classification for a structured error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Engine configuration is internally inconsistent.
    InvalidConfiguration,
    /// A source exceeded the configured size limit.
    SourceTooLarge,
    /// A span has invalid ordering or does not address a source boundary.
    InvalidSpan,
    /// A source identifier is unknown to this engine.
    UnknownSource,
    /// Source loading was attempted without an installed capability.
    SourceLoadingDenied,
    /// An installed source loader returned an error.
    SourceLoadFailed,
    /// A host capability required by a Scheme operation is unavailable.
    CapabilityDenied,
    /// A requested library is not registered with this engine.
    LibraryNotFound,
    /// A library declaration or import/export specification is invalid.
    LibraryError,
    /// Library dependencies form a cycle.
    LibraryCycle,
    /// One canonical source identity was returned with conflicting contents.
    ConflictingSourceIdentity,
    /// No further source identifiers can be represented.
    SourceIdExhausted,
    /// Input bytes were not valid UTF-8.
    InvalidUtf8,
    /// A lexical token was malformed.
    InvalidToken,
    /// A string, identifier, character, or comment was not terminated.
    UnexpectedEof,
    /// A datum had invalid structure.
    InvalidDatum,
    /// A datum label was invalid, duplicated, or unresolved.
    InvalidDatumLabel,
    /// A numeric literal was malformed or not representable.
    InvalidNumber,
    /// A reader resource limit was exceeded.
    ReaderLimitExceeded,
    /// Heap allocation exceeded a configured resource limit.
    HeapLimitExceeded,
    /// A core program could not be compiled.
    CompileError,
    /// Source syntax or a macro transformer was malformed.
    ExpandError,
    /// A syntactic form this engine does not implement.
    UnsupportedSyntax,
    /// Macro expansion exceeded a configured resource limit.
    ExpansionLimitExceeded,
    /// Generated or supplied bytecode violated a VM invariant.
    InvalidBytecode,
    /// Execution attempted an invalid operation.
    RuntimeError,
    /// A Scheme textual reader failed while consuming a datum.
    ReadError,
    /// A Scheme file or host-backed port operation failed.
    FileError,
    /// A procedure received the wrong number of arguments.
    ArityError,
    /// A procedure received an argument of the wrong type.
    TypeError,
    /// A sequence operation received an invalid index or range.
    RangeError,
    /// An operation cannot be represented by the currently supported runtime.
    ImplementationRestriction,
    /// A host root belongs to a different engine.
    WrongEngine,
    /// A registered native callback panicked.
    NativePanic,
    /// Execution exhausted a configured resource limit.
    ExecutionLimitExceeded,
}

impl ErrorKind {
    /// Returns a stable machine-readable code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "configuration.invalid",
            Self::SourceTooLarge => "limit.source-too-large",
            Self::InvalidSpan => "source.invalid-span",
            Self::UnknownSource => "source.unknown",
            Self::SourceLoadingDenied => "host.source-loading-denied",
            Self::SourceLoadFailed => "host.source-load-failed",
            Self::CapabilityDenied => "host.capability-denied",
            Self::LibraryNotFound => "library.not-found",
            Self::LibraryError => "library.invalid",
            Self::LibraryCycle => "library.cycle",
            Self::ConflictingSourceIdentity => "source.conflicting-identity",
            Self::SourceIdExhausted => "limit.source-id-exhausted",
            Self::InvalidUtf8 => "read.invalid-utf8",
            Self::InvalidToken => "read.invalid-token",
            Self::UnexpectedEof => "read.unexpected-eof",
            Self::InvalidDatum => "read.invalid-datum",
            Self::InvalidDatumLabel => "read.invalid-datum-label",
            Self::InvalidNumber => "read.invalid-number",
            Self::ReaderLimitExceeded => "limit.reader",
            Self::HeapLimitExceeded => "limit.heap",
            Self::CompileError => "compile.invalid-core",
            Self::ExpandError => "expand.invalid-syntax",
            Self::UnsupportedSyntax => "expand.unsupported-syntax",
            Self::ExpansionLimitExceeded => "limit.expansion",
            Self::InvalidBytecode => "compile.invalid-bytecode",
            Self::RuntimeError => "runtime.error",
            Self::ReadError => "runtime.read-error",
            Self::FileError => "runtime.file-error",
            Self::ArityError => "runtime.arity",
            Self::TypeError => "runtime.type",
            Self::RangeError => "runtime.range",
            Self::ImplementationRestriction => "runtime.implementation-restriction",
            Self::WrongEngine => "host.wrong-engine",
            Self::NativePanic => "host.native-panic",
            Self::ExecutionLimitExceeded => "limit.execution",
        }
    }

    pub(crate) const fn phase(self) -> ErrorPhase {
        match self {
            Self::InvalidConfiguration | Self::InvalidSpan | Self::UnknownSource => {
                ErrorPhase::Configuration
            }
            Self::SourceTooLarge | Self::SourceIdExhausted => ErrorPhase::Limit,
            Self::SourceLoadingDenied | Self::SourceLoadFailed | Self::CapabilityDenied => {
                ErrorPhase::Host
            }
            Self::LibraryNotFound | Self::LibraryError | Self::LibraryCycle => ErrorPhase::Expand,
            Self::ConflictingSourceIdentity => ErrorPhase::Read,
            Self::InvalidUtf8
            | Self::InvalidToken
            | Self::UnexpectedEof
            | Self::InvalidDatum
            | Self::InvalidDatumLabel
            | Self::InvalidNumber => ErrorPhase::Read,
            Self::ReaderLimitExceeded => ErrorPhase::Limit,
            Self::ExpansionLimitExceeded => ErrorPhase::Limit,
            Self::HeapLimitExceeded => ErrorPhase::Limit,
            Self::ExpandError | Self::UnsupportedSyntax => ErrorPhase::Expand,
            Self::CompileError | Self::InvalidBytecode => ErrorPhase::Compile,
            Self::RuntimeError
            | Self::ReadError
            | Self::FileError
            | Self::ArityError
            | Self::TypeError
            | Self::RangeError
            | Self::ImplementationRestriction => ErrorPhase::Runtime,
            Self::WrongEngine | Self::NativePanic => ErrorPhase::Host,
            Self::ExecutionLimitExceeded => ErrorPhase::Limit,
        }
    }
}

/// Visual role of a diagnostic source label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelStyle {
    /// The principal location responsible for the error.
    Primary,
    /// Additional context related to the error.
    Secondary,
}

/// A labeled source span in a diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticLabel {
    span: Span,
    style: LabelStyle,
    message: String,
}

impl DiagnosticLabel {
    /// Returns the labeled span.
    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    /// Returns the label's visual role.
    #[must_use]
    pub const fn style(&self) -> LabelStyle {
        self.style
    }

    /// Returns the explanatory label text.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Structured diagnostic information independent of presentation format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    phase: ErrorPhase,
    kind: ErrorKind,
    message: String,
    labels: Vec<DiagnosticLabel>,
    notes: Vec<String>,
    suggestion: Option<String>,
    cause: Option<String>,
    stack: Vec<SchemeStackFrame>,
}

/// One Scheme activation recorded in a runtime diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemeStackFrame {
    procedure: Option<String>,
    span: Option<Span>,
}

impl SchemeStackFrame {
    /// Returns the procedure's stable or inferred name, when known.
    #[must_use]
    pub fn procedure(&self) -> Option<&str> {
        self.procedure.as_deref()
    }

    /// Returns the call-site source span, when retained by the compiler.
    #[must_use]
    pub const fn span(&self) -> Option<Span> {
        self.span
    }
}

impl Diagnostic {
    /// Returns the processing phase.
    #[must_use]
    pub const fn phase(&self) -> ErrorPhase {
        self.phase
    }

    /// Returns the stable error classification.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the main human-readable message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns source labels in deterministic display order.
    #[must_use]
    pub fn labels(&self) -> &[DiagnosticLabel] {
        &self.labels
    }

    /// Returns supplementary notes.
    #[must_use]
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Returns an optional suggested correction.
    #[must_use]
    pub fn suggestion(&self) -> Option<&str> {
        self.suggestion.as_deref()
    }

    /// Returns text supplied by a failing host capability.
    #[must_use]
    pub fn cause(&self) -> Option<&str> {
        self.cause.as_deref()
    }

    /// Returns Scheme activations from innermost to outermost.
    #[must_use]
    pub fn stack(&self) -> &[SchemeStackFrame] {
        &self.stack
    }

    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            phase: kind.phase(),
            kind,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            suggestion: None,
            cause: None,
            stack: Vec::new(),
        }
    }

    pub(crate) fn with_label(
        mut self,
        span: Span,
        style: LabelStyle,
        message: impl Into<String>,
    ) -> Self {
        self.labels.push(DiagnosticLabel {
            span,
            style,
            message: message.into(),
        });
        self
    }

    pub(crate) fn with_cause(mut self, cause: impl Into<String>) -> Self {
        self.cause = Some(cause.into());
        self
    }
}

/// Top-level error returned by all fallible embedding operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    diagnostic: Box<Diagnostic>,
}

impl Error {
    /// Builds an error with the given classification and message.
    ///
    /// A native procedure returns `Err(..)` to raise a Scheme condition. The
    /// [`ErrorKind`] selects the condition class the handler sees, so
    /// [`ErrorKind::FileError`] produces a `file-error?` condition and the
    /// read-family kinds produce a `read-error?` condition. Any other kind
    /// produces a plain error object whose `error-object-message` is `message`.
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self::plain(kind, message)
    }

    /// Builds a runtime error with the given message.
    ///
    /// Shortcut for `Error::new(ErrorKind::RuntimeError, message)`, the common
    /// case for a native procedure raising a custom-message condition.
    #[must_use]
    pub fn runtime(message: impl Into<String>) -> Self {
        Self::plain(ErrorKind::RuntimeError, message)
    }

    pub(crate) fn plain(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            diagnostic: Box::new(Diagnostic::new(kind, message)),
        }
    }

    pub(crate) fn from_diagnostic(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostic: Box::new(diagnostic),
        }
    }

    /// Returns the structured diagnostic.
    #[must_use]
    pub const fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    /// Returns the stable error classification.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.diagnostic.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}",
            self.diagnostic.kind.code(),
            self.diagnostic.message
        )
    }
}

impl error::Error for Error {}
