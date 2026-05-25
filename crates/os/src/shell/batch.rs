//! Batch file (.BAT) interpreter.

use super::{RedirectSpec, key_available, read_env_var, read_key};
use crate::{
    DriveIo, IoAccess, MemoryAccess, OsState,
    commands::{RunningCommand, StepResult},
    filesystem::{fat, fat_dir, fat_file},
};

struct CallFrame {
    lines: Vec<Vec<u8>>,
    current_line: usize,
    params: [Vec<u8>; 10],
    bat_path: Vec<u8>,
}

pub(crate) struct BatchState {
    lines: Vec<Vec<u8>>,
    current_line: usize,
    params: [Vec<u8>; 10],
    bat_path: Vec<u8>,
    echo_on: bool,
    pub(crate) running_command: Option<Box<dyn RunningCommand>>,
    pub(crate) current_redirect: Option<RedirectSpec>,
    pub(crate) redirect_buffer: Option<Vec<u8>>,
    call_stack: Vec<CallFrame>,
    paused: bool,
}

struct WaitingForChildCommand {
    owner_psp: u16,
}

impl WaitingForChildCommand {
    fn new(owner_psp: u16) -> Self {
        Self { owner_psp }
    }
}

impl RunningCommand for WaitingForChildCommand {
    fn step(
        &mut self,
        state: &mut OsState,
        _io: &mut IoAccess,
        _disk: &mut dyn DriveIo,
    ) -> StepResult {
        if state.current_psp == self.owner_psp {
            StepResult::Done(state.last_return_code)
        } else {
            StepResult::Continue
        }
    }
}

impl BatchState {
    pub(crate) fn new(
        lines: Vec<Vec<u8>>,
        params: [Vec<u8>; 10],
        bat_path: Vec<u8>,
        echo_on: bool,
    ) -> Self {
        Self {
            lines,
            current_line: 0,
            params,
            bat_path,
            echo_on,
            running_command: None,
            current_redirect: None,
            redirect_buffer: None,
            call_stack: Vec::new(),
            paused: false,
        }
    }

    pub(crate) fn substitute_variables(
        &self,
        state: &OsState,
        memory: &dyn MemoryAccess,
        line: &[u8],
    ) -> Vec<u8> {
        let mut result = Vec::with_capacity(line.len());
        let mut i = 0;
        while i < line.len() {
            if line[i] == b'%' {
                i += 1;
                if i >= line.len() {
                    result.push(b'%');
                    break;
                }
                // %% -> literal %
                if line[i] == b'%' {
                    result.push(b'%');
                    i += 1;
                    continue;
                }
                // %0-%9 -> batch parameters
                if line[i].is_ascii_digit() {
                    let idx = (line[i] - b'0') as usize;
                    if idx == 0 {
                        result.extend_from_slice(&self.bat_path);
                    } else {
                        result.extend_from_slice(&self.params[idx]);
                    }
                    i += 1;
                    continue;
                }
                // %VARIABLE% -> environment variable
                let var_start = i;
                while i < line.len() && line[i] != b'%' {
                    i += 1;
                }
                if i < line.len() && line[i] == b'%' {
                    let var_name = &line[var_start..i];
                    if let Some(value) = read_env_var(state, memory, var_name) {
                        result.extend_from_slice(&value);
                    }
                    i += 1; // skip closing %
                } else {
                    // No closing %, emit as-is
                    result.push(b'%');
                    result.extend_from_slice(&line[var_start..]);
                }
            } else {
                result.push(line[i]);
                i += 1;
            }
        }
        result
    }

    fn find_label(&self, label: &[u8]) -> Option<usize> {
        let upper_label: Vec<u8> = label.iter().map(|b| b.to_ascii_uppercase()).collect();
        for (idx, line) in self.lines.iter().enumerate() {
            let trimmed = line.trim_ascii();
            if trimmed.starts_with(b":") {
                let line_label: Vec<u8> = trimmed[1..]
                    .trim_ascii()
                    .iter()
                    .map(|b| b.to_ascii_uppercase())
                    .collect();
                // Compare only the first word of the label line
                let line_label_word =
                    if let Some(pos) = line_label.iter().position(|&b| b == b' ' || b == b'\t') {
                        &line_label[..pos]
                    } else {
                        &line_label
                    };
                if line_label_word == upper_label.as_slice() {
                    return Some(idx);
                }
            }
        }
        None
    }

    fn jump_to_label(&mut self, io: &mut IoAccess, label_args: &[u8]) -> BatchStepResult {
        let label = first_label_word(label_args);
        if let Some(target) = self.find_label(label) {
            self.current_line = target + 1;
            BatchStepResult::Continue
        } else {
            io.println(b"Label not found");
            BatchStepResult::Finished
        }
    }

    fn run_conditional_command(
        &mut self,
        shell: &mut super::Shell,
        state: &mut OsState,
        io: &mut IoAccess,
        disk: &mut dyn DriveIo,
        command: &[u8],
    ) -> BatchStepResult {
        let command = command.trim_ascii();
        let (command_name, command_args) = super::split_command(command);
        let command_upper: Vec<u8> = command_name
            .iter()
            .map(|byte| byte.to_ascii_uppercase())
            .collect();
        if command_upper == b"GOTO" {
            return self.jump_to_label(io, command_args);
        }

        let phase = shell.dispatch_single(state, io, disk, command);
        match phase {
            super::ShellPhase::ExecutingCommand(cmd) => {
                self.running_command = Some(cmd);
                if shell.redirect_buffer.is_some() {
                    self.redirect_buffer = shell.redirect_buffer.take();
                    self.current_redirect = shell.current_redirect.take();
                }
            }
            super::ShellPhase::WaitingForChild => {
                self.running_command = Some(Box::new(WaitingForChildCommand::new(shell.owner_psp)));
            }
            super::ShellPhase::ExecutingBatch(new_batch) => {
                self.lines = new_batch.lines;
                self.current_line = 0;
                self.params = new_batch.params;
                self.bat_path = new_batch.bat_path;
                self.echo_on = new_batch.echo_on;
            }
            _ => {
                self.current_line += 1;
            }
        }
        BatchStepResult::Continue
    }

    pub(crate) fn step_batch(
        &mut self,
        shell: &mut super::Shell,
        state: &mut OsState,
        io: &mut IoAccess,
        disk: &mut dyn DriveIo,
    ) -> BatchStepResult {
        // Handle paused state (PAUSE command)
        if self.paused {
            if key_available(io.memory) {
                read_key(io.memory); // consume the key
                self.paused = false;
                self.current_line += 1;
            }
            return BatchStepResult::Continue;
        }

        // Handle running sub-command
        if let Some(ref mut cmd) = self.running_command {
            // Set up redirect if active
            if self.redirect_buffer.is_some() {
                io.redirect_output = self.redirect_buffer.take();
            }
            match cmd.step(state, io, disk) {
                StepResult::Continue => {
                    self.redirect_buffer = io.redirect_output.take();
                    return BatchStepResult::Continue;
                }
                StepResult::Done(code) => {
                    shell.last_exit_code = code;
                    // Handle redirect output
                    if let Some(data) = io.redirect_output.take()
                        && let Some(spec) = self.current_redirect.take()
                    {
                        super::write_redirect_to_file(state, io, disk, &data, &spec);
                    }
                    self.running_command = None;
                    self.redirect_buffer = None;
                    self.current_redirect = None;
                    self.current_line += 1;
                    return BatchStepResult::Continue;
                }
            }
        }

        // No running command: dispatch next batch line
        if self.current_line >= self.lines.len() {
            // Batch finished - check call stack
            if let Some(frame) = self.call_stack.pop() {
                self.lines = frame.lines;
                self.current_line = frame.current_line;
                self.params = frame.params;
                self.bat_path = frame.bat_path;
                return BatchStepResult::Continue;
            }
            return BatchStepResult::Finished;
        }

        let raw_line = self.lines[self.current_line].clone();
        let substituted = self.substitute_variables(state, io.memory, &raw_line);
        let trimmed = substituted.trim_ascii().to_vec();

        if trimmed.is_empty() {
            self.current_line += 1;
            return BatchStepResult::Continue;
        }

        // Check for @ prefix (suppress echo for this line)
        let (suppress_echo, effective_line) = if trimmed.starts_with(b"@") {
            (true, trimmed[1..].trim_ascii().to_vec())
        } else {
            (false, trimmed)
        };

        if effective_line.is_empty() {
            self.current_line += 1;
            return BatchStepResult::Continue;
        }

        // Echo the line if echo is on and not suppressed
        if self.echo_on && !suppress_echo {
            io.print(&effective_line);
            io.console.process_byte(io.memory, b'\r');
            io.console.process_byte(io.memory, b'\n');
        }

        let upper: Vec<u8> = effective_line
            .iter()
            .map(|b| b.to_ascii_uppercase())
            .collect();

        // Labels: skip
        if effective_line.starts_with(b":") || effective_line.starts_with(b":") {
            self.current_line += 1;
            return BatchStepResult::Continue;
        }

        // REM: skip
        if upper.starts_with(b"REM") && (upper.len() == 3 || upper[3] == b' ' || upper[3] == b'\t')
        {
            self.current_line += 1;
            return BatchStepResult::Continue;
        }

        // ECHO ON/OFF
        if upper == b"ECHO ON" {
            self.echo_on = true;
            self.current_line += 1;
            return BatchStepResult::Continue;
        }
        if upper == b"ECHO OFF" {
            self.echo_on = false;
            self.current_line += 1;
            return BatchStepResult::Continue;
        }

        // PAUSE
        if upper == b"PAUSE" || upper.starts_with(b"PAUSE ") {
            io.println(b"Press any key to continue . . .");
            self.paused = true;
            return BatchStepResult::Continue;
        }

        // GOTO
        if upper.starts_with(b"GOTO ") || upper.starts_with(b"GOTO\t") {
            return self.jump_to_label(io, &effective_line[5..]);
        }

        // IF
        if upper.starts_with(b"IF ") {
            return self.handle_if(shell, state, io, disk, &effective_line[3..]);
        }

        // CALL
        if upper.starts_with(b"CALL ") {
            self.handle_call(state, io, disk, &effective_line[5..]);
            return BatchStepResult::Continue;
        }

        // Regular command: dispatch through shell
        let phase = shell.dispatch_parsed(state, io, disk, &effective_line);
        match phase {
            super::ShellPhase::ExecutingCommand(cmd) => {
                self.running_command = Some(cmd);
                // Transfer redirect state from shell to batch
                if shell.redirect_buffer.is_some() {
                    self.redirect_buffer = shell.redirect_buffer.take();
                    self.current_redirect = shell.current_redirect.take();
                }
                BatchStepResult::Continue
            }
            super::ShellPhase::WaitingForChild => {
                self.running_command = Some(Box::new(WaitingForChildCommand::new(shell.owner_psp)));
                BatchStepResult::Continue
            }
            super::ShellPhase::ExecutingBatch(new_batch) => {
                // In DOS, invoking a batch from another batch without CALL
                // replaces the current batch (no return to the caller).
                self.lines = new_batch.lines;
                self.current_line = 0;
                self.params = new_batch.params;
                self.bat_path = new_batch.bat_path;
                self.echo_on = new_batch.echo_on;
                BatchStepResult::Continue
            }
            _ => {
                self.current_line += 1;
                BatchStepResult::Continue
            }
        }
    }

    fn handle_if(
        &mut self,
        shell: &mut super::Shell,
        state: &mut OsState,
        io: &mut IoAccess,
        disk: &mut dyn DriveIo,
        args: &[u8],
    ) -> BatchStepResult {
        let trimmed = args.trim_ascii();
        let upper: Vec<u8> = trimmed.iter().map(|b| b.to_ascii_uppercase()).collect();

        let (negated, rest) = if upper.starts_with(b"NOT ") {
            (true, trimmed[4..].trim_ascii())
        } else {
            (false, trimmed)
        };

        let rest_upper: Vec<u8> = rest.iter().map(|b| b.to_ascii_uppercase()).collect();

        if rest_upper.starts_with(b"EXIST ") {
            let after_exist = rest[6..].trim_ascii();
            // Split into filename and command
            if let Some(pos) = after_exist.iter().position(|&b| b == b' ' || b == b'\t') {
                let filename = &after_exist[..pos];
                let command = after_exist[pos + 1..].trim_ascii();

                let exists = check_file_exists(state, io, disk, filename);
                let condition = if negated { !exists } else { exists };

                if condition && !command.is_empty() {
                    return self.run_conditional_command(shell, state, io, disk, command);
                }
            }
            self.current_line += 1;
            BatchStepResult::Continue
        } else if let Some((level, command)) = parse_errorlevel_condition(rest) {
            let condition_met = shell.last_exit_code >= level;
            let condition = if negated {
                !condition_met
            } else {
                condition_met
            };

            if condition && !command.is_empty() {
                return self.run_conditional_command(shell, state, io, disk, command);
            }
            self.current_line += 1;
            BatchStepResult::Continue
        } else {
            self.current_line += 1;
            BatchStepResult::Continue
        }
    }

    fn handle_call(
        &mut self,
        state: &mut OsState,
        io: &mut IoAccess,
        disk: &mut dyn DriveIo,
        args: &[u8],
    ) {
        let trimmed = args.trim_ascii();
        if trimmed.is_empty() {
            self.current_line += 1;
            return;
        }

        let (bat_name, bat_args) = super::split_command(trimmed);
        let (full_name, new_lines) = match super::find_batch_file(bat_name, state, io.memory, disk)
        {
            super::BatchSearchResult::Found { path, lines } => (path, lines),
            super::BatchSearchResult::ReadError => {
                io.println(b"Error reading batch file");
                self.current_line += 1;
                return;
            }
            super::BatchSearchResult::NotFound => {
                io.println(b"Batch file not found");
                self.current_line += 1;
                return;
            }
        };

        // Save current state
        let frame = CallFrame {
            lines: std::mem::take(&mut self.lines),
            current_line: self.current_line + 1,
            params: std::mem::take(&mut self.params),
            bat_path: std::mem::take(&mut self.bat_path),
        };
        self.call_stack.push(frame);

        // Set up new batch
        self.lines = new_lines;
        self.current_line = 0;
        self.params = super::parse_bat_params(bat_args);
        self.bat_path = full_name;
    }
}

pub(crate) enum BatchStepResult {
    Continue,
    Finished,
}

fn first_label_word(label_args: &[u8]) -> &[u8] {
    let label = label_args.trim_ascii();
    let label = if label.starts_with(b":") {
        &label[1..]
    } else {
        label
    };
    match label.iter().position(|&byte| byte == b' ' || byte == b'\t') {
        Some(position) => &label[..position],
        None => label,
    }
}

fn parse_errorlevel_condition(rest: &[u8]) -> Option<(u8, &[u8])> {
    const KEYWORD: &[u8] = b"ERRORLEVEL";

    if rest.len() <= KEYWORD.len() || !ascii_eq_ignore_case(&rest[..KEYWORD.len()], KEYWORD) {
        return None;
    }

    let mut index = KEYWORD.len();
    if rest[index].is_ascii_whitespace() {
        while index < rest.len() && rest[index].is_ascii_whitespace() {
            index += 1;
        }
        if rest.get(index..index + 2) == Some(b"==") {
            index += 2;
        }
    } else if rest.get(index..index + 2) == Some(b"==") {
        index += 2;
    } else {
        return None;
    }

    while index < rest.len() && rest[index].is_ascii_whitespace() {
        index += 1;
    }

    let start = index;
    while index < rest.len() && rest[index].is_ascii_digit() {
        index += 1;
    }
    if start == index {
        return None;
    }

    let level = std::str::from_utf8(&rest[start..index])
        .ok()?
        .parse()
        .ok()?;
    Some((level, rest[index..].trim_ascii()))
}

fn ascii_eq_ignore_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left_byte, right_byte)| left_byte.eq_ignore_ascii_case(right_byte))
}

pub(crate) fn load_bat_file(
    vol: &fat::FatVolume,
    entry: &fat_dir::DirEntry,
    disk: &mut dyn DriveIo,
) -> Result<Vec<Vec<u8>>, u16> {
    let data = fat_file::read_all(vol, entry, disk)?;
    Ok(split_bat_lines(&data))
}

/// Splits raw file data into lines on \r\n or \n.
/// Stops at Ctrl-Z (0x1A), the DOS end-of-file marker.
pub(crate) fn split_bat_lines(data: &[u8]) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    for &byte in data {
        if byte == 0x1A {
            break;
        }
        if byte == b'\n' {
            lines.push(current);
            current = Vec::new();
        } else if byte == b'\r' {
            // skip, we split on \n
        } else {
            current.push(byte);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn check_file_exists(
    state: &mut OsState,
    io: &mut IoAccess,
    disk: &mut dyn DriveIo,
    filename: &[u8],
) -> bool {
    let (drive_index, dir_cluster, fcb_name) =
        match crate::filesystem::resolve_file_path(state, filename, io.memory, disk) {
            Ok(r) => r,
            Err(_) => return false,
        };

    if drive_index == 25 {
        return false;
    }

    let vol = match state.fat_volumes[drive_index as usize].as_ref() {
        Some(v) => v,
        None => return false,
    };

    matches!(
        fat_dir::find_entry(vol, dir_cluster, &fcb_name, disk),
        Ok(Some(_))
    )
}
