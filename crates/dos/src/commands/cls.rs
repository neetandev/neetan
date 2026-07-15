//! CLS command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
};

pub(crate) struct Cls;

impl Command for Cls {
    fn name(&self) -> &'static str {
        "CLS"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningCls {
            args: args.to_vec(),
        })
    }
}

#[derive(Clone)]
/// Serializable state of an executing CLS command.
pub(crate) struct RunningCls {
    args: Vec<u8>,
}

state_struct_codec!(RunningCls { args });

impl RunningCommand for RunningCls {
    fn step(
        &mut self,
        _state: &mut DosState,
        io: &mut IoAccess,
        _drive: &mut dyn DriveIo,
    ) -> StepResult {
        if is_help_request(&self.args) {
            print_help(io);
            return StepResult::Done(0);
        }
        io.console.clear_screen(io.memory);
        StepResult::Done(0)
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Clears the screen.");
    io.println(b"");
    io.println(b"CLS");
}
