//! PROMPT command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    environment,
};

/// Name of the environment variable that holds the prompt text.
const PROMPT_VARIABLE: &[u8] = b"PROMPT";

/// Factory of the PROMPT command.
pub(crate) struct Prompt;

impl Command for Prompt {
    fn name(&self) -> &'static str {
        "PROMPT"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningPrompt {
            args: args.to_vec(),
        })
    }
}

#[derive(Clone)]
/// Serializable state of an executing PROMPT command.
pub(crate) struct RunningPrompt {
    args: Vec<u8>,
}

state_struct_codec!(RunningPrompt { args });

impl RunningCommand for RunningPrompt {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        _disk: &mut dyn DriveIo,
    ) -> StepResult {
        if is_help_request(&self.args) {
            print_help(io);
            return StepResult::Done(0);
        }

        let value = prompt_value_from_args(&self.args);
        if environment::set_var(state, io.memory, PROMPT_VARIABLE, value, false).is_err() {
            io.println(b"Out of environment space");
            return StepResult::Done(1);
        }
        StepResult::Done(0)
    }
}

/// Prints the usage of the command.
fn print_help(io: &mut IoAccess) {
    io.println(b"Changes the command prompt.");
    io.println(b"");
    io.println(b"PROMPT [text]");
    io.println(b"");
    io.println(b"Type PROMPT without parameters to reset the prompt to the default setting.");
}

/// Returns the prompt text of the arguments. An empty text removes the
/// variable and restores the default prompt.
fn prompt_value_from_args(args: &[u8]) -> &[u8] {
    let text = args.trim_ascii_start();
    let text = text.strip_prefix(b"=").unwrap_or(text);
    text.trim_ascii_end()
}

#[cfg(test)]
mod tests {
    use super::prompt_value_from_args;

    #[test]
    fn value_is_the_text_behind_the_command() {
        assert_eq!(prompt_value_from_args(b" $p$g"), b"$p$g");
        assert_eq!(prompt_value_from_args(b"=$n$g "), b"$n$g");
        assert_eq!(prompt_value_from_args(b"   "), b"");
    }
}
