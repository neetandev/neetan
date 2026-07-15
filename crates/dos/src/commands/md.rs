//! MD / MKDIR command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    filesystem,
};

pub(crate) struct Md;

impl Command for Md {
    fn name(&self) -> &'static str {
        "MD"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["MKDIR"]
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningMd {
            args: args.to_vec(),
        })
    }
}

#[derive(Clone)]
/// Serializable state of an executing MD command.
pub(crate) struct RunningMd {
    args: Vec<u8>,
}

state_struct_codec!(RunningMd { args });

impl RunningCommand for RunningMd {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let args = self.args.trim_ascii();
        if is_help_request(&self.args) || args.is_empty() {
            print_help(io);
            return StepResult::Done(0);
        }

        match filesystem::create_directory(state, io.memory, drive, args) {
            Ok(()) => StepResult::Done(0),
            Err(error) => {
                io.print(error_message(error));
                StepResult::Done(1)
            }
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Creates a directory.");
    io.println(b"");
    io.println(b"MD path");
    io.println(b"MKDIR path");
    io.println(b"");
    io.println(b"  path  Specifies the directory to create.");
}

fn error_message(error: u16) -> &'static [u8] {
    match error {
        0x0005 => b"Access denied\r\n",
        0x000Fu16 => b"Invalid drive\r\n",
        _ => b"Unable to create directory\r\n",
    }
}
