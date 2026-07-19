//! An embeddable implementation of Scheme R7RS-small.

#![deny(unsafe_code)]
#![forbid(missing_docs)]

mod bytecode;
mod compile;
mod config;
mod core;
mod datum;
mod embed;
mod error;
mod expand;
mod frontend;
mod global;
mod hash;
mod heap;
#[cfg(feature = "host-capabilities")]
mod host;
mod library;
mod native;
mod number;
mod port;
mod printer;
mod random;
mod reader;
mod slab;
mod source;
mod value;
mod vm;

pub use core::CoreExpr;

pub use bytecode::CompiledModule;
pub use config::{
    EngineConfig, FeatureSet, InterruptToken, LimitBehavior, Limits, SourceRetention,
};
pub use datum::{Datum, DatumKind, DatumRef, ExactRational};
pub use embed::{Engine, Extension};
pub use error::{
    Diagnostic, DiagnosticLabel, Error, ErrorKind, ErrorPhase, LabelStyle, SchemeStackFrame,
};
#[cfg(feature = "host-capabilities")]
pub use host::{
    StdClock, StdFileSystem, StdProcessContext, StdSourceLoader, StdStandardError,
    StdStandardInput, StdStandardOutput,
};
pub use library::{LibraryName, LibraryNameComponent};
pub use native::{IntoNativeValues, NativeContext, NativeValues};
pub use number::{Number, Real};
pub use port::{Clock, ExitStatus, FileSystem, HostIoError, PortResource, ProcessContext};
pub use reader::Reader;
pub use source::{
    LoadedSource, SourceId, SourceLoader, SourceLoaderError, SourceLocation, SourceRequest, Span,
};
pub use value::{EvalOutcome, Root, Value, ValueKind, Values};
