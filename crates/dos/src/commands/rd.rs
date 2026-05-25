//! RD / RMDIR command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    filesystem,
};

pub(crate) struct Rd;

impl Command for Rd {
    fn name(&self) -> &'static str {
        "RD"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["RMDIR"]
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningRd {
            args: args.to_vec(),
        })
    }
}

struct RunningRd {
    args: Vec<u8>,
}

impl RunningCommand for RunningRd {
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

        match filesystem::remove_directory(state, io.memory, drive, args) {
            Ok(()) => StepResult::Done(0),
            Err(error) => {
                io.print(error_message(error));
                StepResult::Done(1)
            }
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Removes a directory.");
    io.println(b"");
    io.println(b"RD path");
    io.println(b"RMDIR path");
    io.println(b"");
    io.println(b"  path  Specifies the directory to remove. The directory must");
    io.println(b"        be empty before it can be removed.");
}

fn error_message(error: u16) -> &'static [u8] {
    match error {
        0x0003 => b"Invalid path\r\n",
        0x0005 => b"Access denied\r\n",
        0x000F => b"Invalid drive\r\n",
        0x0012 => b"Directory not empty\r\n",
        _ => b"Disk error\r\n",
    }
}
