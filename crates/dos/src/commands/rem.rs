//! REM command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
};

pub(crate) struct Rem;

impl Command for Rem {
    fn name(&self) -> &'static str {
        "REM"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningRem {
            args: args.to_vec(),
        })
    }
}

struct RunningRem {
    args: Vec<u8>,
}

impl RunningCommand for RunningRem {
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
        StepResult::Done(0)
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Records comments in a batch file.");
    io.println(b"");
    io.println(b"REM [comment]");
}
