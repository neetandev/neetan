//! PATH command.

use crate::{
    DriveIo, IoAccess, OsState,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    environment,
};

pub(crate) struct PathCommand;

impl Command for PathCommand {
    fn name(&self) -> &'static str {
        "PATH"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningPath {
            args: args.to_vec(),
        })
    }
}

struct RunningPath {
    args: Vec<u8>,
}

impl RunningCommand for RunningPath {
    fn step(
        &mut self,
        state: &mut OsState,
        io: &mut IoAccess,
        _disk: &mut dyn DriveIo,
    ) -> StepResult {
        if is_help_request(&self.args) {
            print_help(io);
            return StepResult::Done(0);
        }

        let args = self.args.trim_ascii();
        if args.is_empty() {
            print_path(state, io);
            return StepResult::Done(0);
        }

        let value = path_value_from_args(args);
        if environment::set_var(state, io.memory, b"PATH", value, true).is_err() {
            io.println(b"Out of environment space");
            return StepResult::Done(1);
        }

        StepResult::Done(0)
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Displays or sets a search path for executable files.");
    io.println(b"");
    io.println(b"PATH [[drive:]path[;...]]");
    io.println(b"PATH ;");
    io.println(b"");
    io.println(b"Type PATH without parameters to display the current path.");
    io.println(b"Type PATH ; to clear the search path.");
}

fn print_path(state: &OsState, io: &mut IoAccess) {
    match environment::read_var(state, io.memory, b"PATH") {
        Some(value) if !value.is_empty() => {
            io.print(b"PATH=");
            io.println(&value);
        }
        _ => io.println(b"No Path"),
    }
}

fn path_value_from_args(args: &[u8]) -> &[u8] {
    let value = if args.first() == Some(&b'=') {
        args[1..].trim_ascii()
    } else {
        args
    };

    if value == b";" { b"" } else { value }
}
