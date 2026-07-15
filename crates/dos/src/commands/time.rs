//! TIME command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
};

pub(crate) struct Time;

impl Command for Time {
    fn name(&self) -> &'static str {
        "TIME"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningTime {
            args: args.to_vec(),
        })
    }
}

#[derive(Clone)]
/// Serializable state of an executing TIME command.
pub(crate) struct RunningTime {
    args: Vec<u8>,
}

state_struct_codec!(RunningTime { args });

impl RunningCommand for RunningTime {
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

        let (hour, minute, second) = state.current_time_parts();

        let msg = format!(
            "Current time is {:02}:{:02}:{:02}.00\r\n",
            hour, minute, second
        );
        for &byte in msg.as_bytes() {
            io.output_byte(byte);
        }
        StepResult::Done(0)
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Displays the time.");
    io.println(b"");
    io.println(b"TIME");
}
