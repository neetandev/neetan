//! Command trait definitions and command registry.
//!
//! All shell commands implement the unified Command/RunningCommand trait system.
//! Commands are stateless factories (Command) that produce stateful running
//! instances (RunningCommand). The shell calls step() once per INT 21h AH=FFh
//! dispatch - commands must return quickly and never block.

pub mod b3sum;
pub mod cd;
pub mod cls;
pub mod copy;
pub mod date;
pub mod del;
pub mod dir;
pub mod diskcopy;
pub mod dosmock;
pub mod echo;
pub mod editor;
pub mod format;
pub mod md;
pub mod mem;
pub mod more;
pub mod path;
pub mod rd;
pub mod rem;
pub mod ren;
pub mod set;
pub mod time;
pub mod type_cmd;
pub mod ver;
pub mod xcopy;

use crate::{DosState, DriveIo, IoAccess};

pub(crate) enum StepResult {
    /// Command completed with the given exit code.
    Done(u8),
    /// Command yielded; call step() again on the next AH=FFh dispatch.
    Continue,
}

pub(crate) trait Command {
    /// The primary command name (e.g., "DIR", "CD", "CLS").
    fn name(&self) -> &'static str;

    /// Alternative names (e.g., &["ERASE"] for DEL, &["CHDIR"] for CD).
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// Create a running instance of this command with the given arguments.
    /// Arguments are raw bytes (Shift-JIS, as typed by the user).
    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand>;
}

pub(crate) trait RunningCommand {
    /// Execute one step of the command.
    ///
    /// Called once per AH=FFh dispatch while this command is active.
    /// Must return quickly - never block.
    /// Simple commands (CLS, VER) return Done on the first call.
    /// Long commands (COPY, FORMAT) process one chunk and return Continue.
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DriveIo,
    ) -> StepResult;
}

pub(crate) fn is_help_request(args: &[u8]) -> bool {
    args.trim_ascii()
        .split(|&b| b == b' ' || b == b'\t')
        .any(|token| token == b"/?")
}
