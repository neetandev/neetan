//! CD / CHDIR command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    tables,
};

pub(crate) struct Cd;

impl Command for Cd {
    fn name(&self) -> &'static str {
        "CD"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["CHDIR"]
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningCd {
            args: args.to_vec(),
        })
    }
}

#[derive(Clone)]
/// Serializable state of an executing CD command.
pub(crate) struct RunningCd {
    args: Vec<u8>,
}

state_struct_codec!(RunningCd { args });

impl RunningCommand for RunningCd {
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

        if args.is_empty() {
            print_current_directory(state, io);
            return StepResult::Done(0);
        }

        match crate::filesystem::change_directory(state, io.memory, drive, args) {
            Ok(()) => StepResult::Done(0),
            Err(_) => {
                io.println(b"Invalid directory");
                StepResult::Done(1)
            }
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Displays the name of or changes the current directory.");
    io.println(b"");
    io.println(b"CD [path]");
    io.println(b"CHDIR [path]");
    io.println(b"");
    io.println(b"  path  Specifies the directory to change to.");
    io.println(b"");
    io.println(b"Type CD without parameters to display the current directory.");
}

fn print_current_directory(state: &DosState, io: &mut IoAccess) {
    let cds_addr = tables::CDS_BASE + (state.current_drive as u32) * tables::CDS_ENTRY_SIZE;

    let mut path = Vec::new();
    for i in 0..67u32 {
        let byte = io.memory.read_byte(cds_addr + tables::CDS_OFF_PATH + i);
        if byte == 0 {
            break;
        }
        path.push(byte);
    }
    path.push(b'\r');
    path.push(b'\n');

    for &byte in &path {
        io.output_byte(byte);
    }
}
