//! VER command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
};

pub(crate) struct Ver;

impl Command for Ver {
    fn name(&self) -> &'static str {
        "VER"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningVer {
            args: args.to_vec(),
        })
    }
}

#[derive(Clone)]
/// Serializable state of an executing VER command.
pub(crate) struct RunningVer {
    args: Vec<u8>,
}

state_struct_codec!(RunningVer { args });

impl RunningCommand for RunningVer {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        _drive: &mut dyn DriveIo,
    ) -> StepResult {
        if is_help_request(&self.args) {
            print_help(io);
            return StepResult::Done(0);
        }

        let (major, minor) = state.version;
        let msg = format!("Neetan DOS {}.{}\r\n\r\n", major, minor);
        for &byte in msg.as_bytes() {
            io.output_byte(byte);
        }
        StepResult::Done(0)
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Displays the operating system version.");
    io.println(b"");
    io.println(b"VER");
}
