//! Deterministic headless automation frontend for the Neetan emulator.
//!
//! This crate embeds the `r7rs` interpreter and runs R7RS Scheme compatibility
//! tests one script at a time on a dedicated executor thread. It provides the
//! `neetan-auto run` binary, the sandboxed interpreter, strict isolated
//! configuration loading, the typed message protocol, the cooperative deadline
//! watchdog, the console renderer, and the `execution-result` contract.

pub mod capabilities;
pub mod cli;
pub mod config;
pub mod executor;
pub mod input;
pub mod media;
pub mod orchestration;
pub mod protocol;
pub mod render;
pub mod scheme;
pub mod screen;
pub mod session;
pub mod watchdog;

pub use config::CommonConfig;
pub use executor::execute_script;
pub use media::{MediaKind, MediaMount, MediaRequest, media_kind_from_name};
pub use protocol::{
    ExecutionResult, MachineIdentity, MessageProtocol, RunProgress, RunTermination, TestCaseOutcome,
};
pub use session::{AutomationSession, OpError, RunError, SessionBudgets};
pub use watchdog::CancelHandle;
