//! Command trait definitions and command registry.
//!
//! All shell commands implement the unified Command/RunningCommand trait system.
//! Commands are stateless factories (Command) that produce stateful running
//! instances (RunningCommand). The shell calls step() once per INT 21h AH=FFh
//! dispatch - commands must return quickly and never block.

use std::any::Any;

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
    /// Command completed without changing the previous exit code.
    DonePreserve,
    /// Command yielded; call step() again on the next AH=FFh dispatch.
    Continue,
}

pub(crate) trait Command: Send {
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

pub(crate) trait RunningCommandObject {
    /// Returns this command as a concrete runtime object.
    fn as_any(&self) -> &dyn Any;

    /// Returns this command as a mutable concrete runtime object.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Clones this command without erasing its concrete state.
    fn clone_command(&self) -> Box<dyn RunningCommand>;
}

impl<CommandState> RunningCommandObject for CommandState
where
    CommandState: RunningCommand + Clone + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_command(&self) -> Box<dyn RunningCommand> {
        Box::new(self.clone())
    }
}

pub(crate) trait RunningCommand: RunningCommandObject + Send {
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

impl Clone for Box<dyn RunningCommand> {
    fn clone(&self) -> Self {
        self.clone_command()
    }
}

/// Returns the registered name of a running command.
pub(crate) fn running_command_name(command: &dyn RunningCommand) -> &'static str {
    let command = command.as_any();
    if command.is::<b3sum::RunningB3sum>() {
        "B3SUM"
    } else if command.is::<cd::RunningCd>() {
        "CD"
    } else if command.is::<cls::RunningCls>() {
        "CLS"
    } else if command.is::<copy::RunningCopy>() {
        "COPY"
    } else if command.is::<date::RunningDate>() {
        "DATE"
    } else if command.is::<del::RunningDel>() {
        "DEL"
    } else if command.is::<dir::RunningDir>() {
        "DIR"
    } else if command.is::<diskcopy::RunningDiskcopy>() {
        "DISKCOPY"
    } else if command.is::<dosmock::RunningDosmock>() {
        "DOSMOCK"
    } else if command.is::<echo::RunningEcho>() {
        "ECHO"
    } else if command.is::<editor::RunningEditor>() {
        "EDIT"
    } else if command.is::<format::RunningFormat>() {
        "FORMAT"
    } else if command.is::<md::RunningMd>() {
        "MD"
    } else if command.is::<mem::RunningMem>() {
        "MEM"
    } else if command.is::<more::RunningMore>() {
        "MORE"
    } else if command.is::<path::RunningPath>() {
        "PATH"
    } else if command.is::<rd::RunningRd>() {
        "RD"
    } else if command.is::<rem::RunningRem>() {
        "REM"
    } else if command.is::<ren::RunningRen>() {
        "REN"
    } else if command.is::<set::RunningSet>() {
        "SET"
    } else if command.is::<time::RunningTime>() {
        "TIME"
    } else if command.is::<type_cmd::RunningType>() {
        "TYPE"
    } else if command.is::<ver::RunningVer>() {
        "VER"
    } else if command.is::<xcopy::RunningXcopy>() {
        "XCOPY"
    } else if command.is::<crate::shell::batch::WaitingForChildCommand>() {
        "WAITING_FOR_CHILD"
    } else {
        unreachable!("unregistered HLE DOS running command")
    }
}

impl save_state::StateEncode for Box<dyn RunningCommand> {
    fn encode_state(&self, output: &mut Vec<u8>) {
        let command = self.as_any();
        if encode_command::<b3sum::RunningB3sum>(command, 0, output)
            || encode_command::<cd::RunningCd>(command, 1, output)
            || encode_command::<cls::RunningCls>(command, 2, output)
            || encode_command::<copy::RunningCopy>(command, 3, output)
            || encode_command::<date::RunningDate>(command, 4, output)
            || encode_command::<del::RunningDel>(command, 5, output)
            || encode_command::<dir::RunningDir>(command, 6, output)
            || encode_command::<diskcopy::RunningDiskcopy>(command, 7, output)
            || encode_command::<dosmock::RunningDosmock>(command, 8, output)
            || encode_command::<echo::RunningEcho>(command, 9, output)
            || encode_command::<editor::RunningEditor>(command, 10, output)
            || encode_command::<format::RunningFormat>(command, 11, output)
            || encode_command::<md::RunningMd>(command, 12, output)
            || encode_command::<mem::RunningMem>(command, 13, output)
            || encode_command::<more::RunningMore>(command, 14, output)
            || encode_command::<path::RunningPath>(command, 15, output)
            || encode_command::<rd::RunningRd>(command, 16, output)
            || encode_command::<rem::RunningRem>(command, 17, output)
            || encode_command::<ren::RunningRen>(command, 18, output)
            || encode_command::<set::RunningSet>(command, 19, output)
            || encode_command::<time::RunningTime>(command, 20, output)
            || encode_command::<type_cmd::RunningType>(command, 21, output)
            || encode_command::<ver::RunningVer>(command, 22, output)
            || encode_command::<xcopy::RunningXcopy>(command, 23, output)
            || encode_command::<crate::shell::batch::WaitingForChildCommand>(command, 24, output)
        {
            return;
        }
        unreachable!("unregistered HLE DOS running command")
    }
}

fn encode_command<CommandState: save_state::StateEncode + 'static>(
    command: &dyn Any,
    tag: u8,
    output: &mut Vec<u8>,
) -> bool {
    let Some(command) = command.downcast_ref::<CommandState>() else {
        return false;
    };
    save_state::StateEncode::encode_state(&tag, output);
    save_state::StateEncode::encode_state(command, output);
    true
}

impl save_state::StateDecode for Box<dyn RunningCommand> {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        // Reconstructs one concrete command behind the shell's trait object.
        macro_rules! decode_command {
            ($command_type:ty) => {
                Ok(Box::new(
                    <$command_type as save_state::StateDecode>::decode_state(decoder)?,
                ))
            };
        }

        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => decode_command!(b3sum::RunningB3sum),
            1 => decode_command!(cd::RunningCd),
            2 => decode_command!(cls::RunningCls),
            3 => decode_command!(copy::RunningCopy),
            4 => decode_command!(date::RunningDate),
            5 => decode_command!(del::RunningDel),
            6 => decode_command!(dir::RunningDir),
            7 => decode_command!(diskcopy::RunningDiskcopy),
            8 => decode_command!(dosmock::RunningDosmock),
            9 => decode_command!(echo::RunningEcho),
            10 => decode_command!(editor::RunningEditor),
            11 => decode_command!(format::RunningFormat),
            12 => decode_command!(md::RunningMd),
            13 => decode_command!(mem::RunningMem),
            14 => decode_command!(more::RunningMore),
            15 => decode_command!(path::RunningPath),
            16 => decode_command!(rd::RunningRd),
            17 => decode_command!(rem::RunningRem),
            18 => decode_command!(ren::RunningRen),
            19 => decode_command!(set::RunningSet),
            20 => decode_command!(time::RunningTime),
            21 => decode_command!(type_cmd::RunningType),
            22 => decode_command!(ver::RunningVer),
            23 => decode_command!(xcopy::RunningXcopy),
            24 => decode_command!(crate::shell::batch::WaitingForChildCommand),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

/// Prepares retained host resources for one restored running command.
pub(crate) fn prepare_restore(
    command: &mut Box<dyn RunningCommand>,
) -> Result<(), save_state::StateValidationError> {
    if let Some(copy) = command.as_any_mut().downcast_mut::<copy::RunningCopy>() {
        copy.prepare_restore()?;
    }
    Ok(())
}

pub(crate) fn is_help_request(args: &[u8]) -> bool {
    args.trim_ascii()
        .split(|&b| b == b' ' || b == b'\t')
        .any(|token| token == b"/?")
}
