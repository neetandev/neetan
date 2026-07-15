//! ECHO command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult},
};

pub(crate) struct Echo;

impl Command for Echo {
    fn name(&self) -> &'static str {
        "ECHO"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningEcho {
            text: args.to_vec(),
        })
    }
}

#[derive(Clone)]
/// Serializable state of an executing ECHO command.
pub(crate) struct RunningEcho {
    text: Vec<u8>,
}

state_struct_codec!(RunningEcho { text });

fn print_help(io: &mut IoAccess) {
    io.println(b"Displays messages.");
    io.println(b"");
    io.println(b"ECHO [message]");
    io.println(b"");
    io.println(b"  message  Specifies the text to display.");
    io.println(b"");
    io.println(b"Type ECHO without parameters to display a blank line.");
}

impl RunningCommand for RunningEcho {
    fn step(
        &mut self,
        _state: &mut DosState,
        io: &mut IoAccess,
        _drive: &mut dyn DriveIo,
    ) -> StepResult {
        if self.text.trim_ascii() == b"/?" {
            print_help(io);
            return StepResult::DonePreserve;
        }
        if self.text.is_empty() {
            // ECHO. (bare dot) prints a blank line
            io.output_byte(b'\r');
            io.output_byte(b'\n');
        } else {
            for &byte in &self.text {
                io.output_byte(byte);
            }
            io.output_byte(b'\r');
            io.output_byte(b'\n');
        }
        StepResult::DonePreserve
    }
}
