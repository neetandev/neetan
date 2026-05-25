//! DOSMOCK command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    filesystem,
};

pub(crate) struct Dosmock;

impl Command for Dosmock {
    fn name(&self) -> &'static str {
        "DOSMOCK"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningDosmock {
            args: args.to_vec(),
        })
    }
}

struct RunningDosmock {
    args: Vec<u8>,
}

impl RunningCommand for RunningDosmock {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let args = self.args.trim_ascii();
        if is_help_request(&self.args) {
            print_help(io);
            return StepResult::Done(0);
        }

        let Some(drive_index) = parse_drive_arg(args) else {
            print_help(io);
            return StepResult::Done(1);
        };

        match filesystem::create_dos_mock_files(state, io.memory, drive, drive_index) {
            Ok(()) => {
                io.print(b"DOS mock files created\r\n");
                StepResult::Done(0)
            }
            Err(error) => {
                io.print(error_message(error));
                StepResult::Done(1)
            }
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Creates minimal DOS marker files on a drive root.");
    io.println(b"");
    io.println(b"DOSMOCK drive:");
    io.println(b"");
    io.println(b"  drive:  Specifies the target drive root (for example A:).");
}

fn parse_drive_arg(args: &[u8]) -> Option<u8> {
    if args.len() != 2 && args.len() != 3 {
        return None;
    }
    if args[1] != b':' {
        return None;
    }
    if args.len() == 3 && args[2] != b'\\' && args[2] != b'/' {
        return None;
    }
    let drive_letter = args[0].to_ascii_uppercase();
    if !drive_letter.is_ascii_uppercase() {
        return None;
    }
    Some(drive_letter - b'A')
}

fn error_message(error: u16) -> &'static [u8] {
    match error {
        0x0005 => b"Target drive is not writable\r\n",
        0x0050 => b"DOS mock files already exist\r\n",
        0x001F => b"Unable to create DOS mock files\r\n",
        _ => b"Unable to access target drive\r\n",
    }
}
