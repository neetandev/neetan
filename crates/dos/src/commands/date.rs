//! DATE command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
};

pub(crate) struct Date;

impl Command for Date {
    fn name(&self) -> &'static str {
        "DATE"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningDate {
            args: args.to_vec(),
        })
    }
}

#[derive(Clone)]
/// Serializable state of an executing DATE command.
pub(crate) struct RunningDate {
    args: Vec<u8>,
}

state_struct_codec!(RunningDate { args });

impl RunningCommand for RunningDate {
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

        let (year, month, day, dow) = state.current_date_parts();
        let dow_name = match dow {
            0 => "Sun",
            1 => "Mon",
            2 => "Tue",
            3 => "Wed",
            4 => "Thu",
            5 => "Fri",
            _ => "Sat",
        };

        let msg = format!(
            "Current date is {} {:02}-{:02}-{:04}\r\n",
            dow_name, month, day, year
        );
        for &byte in msg.as_bytes() {
            io.output_byte(byte);
        }
        StepResult::Done(0)
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Displays the date.");
    io.println(b"");
    io.println(b"DATE");
}
