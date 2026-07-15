//! SET command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    environment,
};

pub(crate) struct Set;

impl Command for Set {
    fn name(&self) -> &'static str {
        "SET"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningSet {
            args: args.to_vec(),
        })
    }
}

#[derive(Clone)]
/// Serializable state of an executing SET command.
pub(crate) struct RunningSet {
    args: Vec<u8>,
}

state_struct_codec!(RunningSet { args });

impl RunningCommand for RunningSet {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        _drive: &mut dyn DriveIo,
    ) -> StepResult {
        let args = self.args.trim_ascii();

        if is_help_request(&self.args) {
            print_help(io);
            return StepResult::Done(0);
        }

        if args.is_empty() {
            dump_environment(state, io);
            return StepResult::Done(0);
        }

        // SET VAR=VALUE
        if let Some(eq_pos) = args.iter().position(|&b| b == b'=') {
            let var_name = &args[..eq_pos];
            let value = &args[eq_pos + 1..];
            if environment::set_var(state, io.memory, var_name, value, false).is_err() {
                io.println(b"Out of environment space");
                return StepResult::Done(1);
            }
        } else {
            // SET VAR (no =) - print matching vars
            dump_matching_vars(state, io, args);
        }

        StepResult::Done(0)
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Displays, sets, or removes environment variables.");
    io.println(b"");
    io.println(b"SET [variable=[value]]");
    io.println(b"");
    io.println(b"  variable  Specifies the environment variable name.");
    io.println(b"  value     Specifies the value to assign. If omitted, the");
    io.println(b"            variable is removed.");
    io.println(b"");
    io.println(b"Type SET without parameters to display all variables.");
}

fn dump_environment(state: &DosState, io: &mut IoAccess) {
    for entry in environment::entries(state, io.memory) {
        io.print(&entry);
        io.output_byte(b'\r');
        io.output_byte(b'\n');
    }
}

fn dump_matching_vars(state: &DosState, io: &mut IoAccess, prefix: &[u8]) {
    for entry in environment::entries(state, io.memory) {
        if environment::name_starts_with(&entry, prefix) {
            io.print(&entry);
            io.output_byte(b'\r');
            io.output_byte(b'\n');
        }
    }
}
