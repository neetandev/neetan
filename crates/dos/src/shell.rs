//! Shell state machine, command parsing, I/O redirection.

pub mod batch;
pub mod history;

use std::collections::VecDeque;

use history::History;

use crate::{
    DiskIo, DosState, DriveIo, IoAccess, MemoryAccess,
    commands::{self, Command, RunningCommand, StepResult},
    environment, filesystem,
    filesystem::{fat, fat_dir, fat_file},
    process, tables,
};

pub(crate) enum RedirectSpec {
    Overwrite(Vec<u8>),
    Append(Vec<u8>),
}

struct ParsedCommand {
    command: Vec<u8>,
    output_redirect: Option<RedirectSpec>,
    input_file: Option<Vec<u8>>,
}

struct PendingCommand {
    parsed: ParsedCommand,
}

/// An external program (.COM or .EXE) to be EXECed from the shell.
pub(crate) struct PendingExec {
    pub path: Vec<u8>,
    pub args: Vec<u8>,
}

const SCAN_INSERT: u8 = 0x38;
const SCAN_DELETE: u8 = 0x39;
const SCAN_UP: u8 = 0x3A;
const SCAN_LEFT: u8 = 0x3B;
const SCAN_RIGHT: u8 = 0x3C;
const SCAN_DOWN: u8 = 0x3D;
const SCAN_HOME: u8 = 0x3E;
const SCAN_END: u8 = 0x3F;

pub(crate) struct LineEditor {
    buffer: Vec<u8>,
    cursor: usize,
    prompt_col: u8,
    insert_mode: bool,
    saved_line: Option<Vec<u8>>,
}

impl LineEditor {
    fn new(prompt_col: u8) -> Self {
        Self {
            buffer: Vec::with_capacity(128),
            cursor: 0,
            prompt_col,
            insert_mode: true,
            saved_line: None,
        }
    }
}

pub(crate) enum ShellPhase {
    ShowPrompt,
    ReadingInput(LineEditor),
    ExecutingCommand(Box<dyn RunningCommand>),
    WaitingForChild,
    ExecutingBatch(Box<batch::BatchState>),
}

pub(crate) struct Shell {
    pub(crate) phase: ShellPhase,
    history: History,
    commands: Vec<Box<dyn Command>>,
    pub(crate) echo_on: bool,
    pub(crate) last_exit_code: u8,
    boot_banner_shown: bool,
    pending_commands: VecDeque<PendingCommand>,
    current_redirect: Option<RedirectSpec>,
    redirect_buffer: Option<Vec<u8>>,
    pipe_input: Option<Vec<u8>>,
    startup_command: Option<Vec<u8>>,
    terminate_after_command: bool,
    allow_exit: bool,
    /// COMMAND.COM PSP segment, used to detect child process termination.
    pub(crate) owner_psp: u16,
    /// External program pending EXEC (set by dispatch, consumed by int21h_ffh_shell_step).
    pub(crate) pending_exec: Option<PendingExec>,
    /// Child shell termination requested by EXIT or /C completion.
    pub(crate) pending_terminate: Option<u8>,
}

impl Shell {
    fn build_commands() -> Vec<Box<dyn Command>> {
        vec![
            Box::new(commands::b3sum::B3sum),
            Box::new(commands::cls::Cls),
            Box::new(commands::ver::Ver),
            Box::new(commands::echo::Echo),
            Box::new(commands::editor::Editor),
            Box::new(commands::rem::Rem),
            Box::new(commands::cd::Cd),
            Box::new(commands::set::Set),
            Box::new(commands::path::PathCommand),
            Box::new(commands::copy::Copy),
            Box::new(commands::date::Date),
            Box::new(commands::del::Del),
            Box::new(commands::dir::Dir),
            Box::new(commands::diskcopy::Diskcopy),
            Box::new(commands::dosmock::Dosmock),
            Box::new(commands::format::Format),
            Box::new(commands::md::Md),
            Box::new(commands::mem::Mem),
            Box::new(commands::more::More),
            Box::new(commands::rd::Rd),
            Box::new(commands::ren::Ren),
            Box::new(commands::time::Time),
            Box::new(commands::type_cmd::TypeCmd),
            Box::new(commands::xcopy::Xcopy),
        ]
    }

    pub(crate) fn new(command_com_psp: u16) -> Self {
        Self {
            phase: ShellPhase::ShowPrompt,
            history: History::new(),
            commands: Self::build_commands(),
            echo_on: true,
            last_exit_code: 0,
            boot_banner_shown: false,
            pending_commands: VecDeque::new(),
            current_redirect: None,
            redirect_buffer: None,
            pipe_input: None,
            startup_command: None,
            terminate_after_command: false,
            allow_exit: false,
            owner_psp: command_com_psp,
            pending_exec: None,
            pending_terminate: None,
        }
    }

    pub(crate) fn new_with_autoexec(
        command_com_psp: u16,
        lines: Vec<Vec<u8>>,
        bat_path: Vec<u8>,
    ) -> Self {
        let params: [Vec<u8>; 10] = Default::default();
        let bat_state = batch::BatchState::new(lines, params, bat_path, true);
        Self {
            phase: ShellPhase::ExecutingBatch(Box::new(bat_state)),
            history: History::new(),
            commands: Self::build_commands(),
            echo_on: true,
            last_exit_code: 0,
            boot_banner_shown: true,
            pending_commands: VecDeque::new(),
            current_redirect: None,
            redirect_buffer: None,
            pipe_input: None,
            startup_command: None,
            terminate_after_command: false,
            allow_exit: false,
            owner_psp: command_com_psp,
            pending_exec: None,
            pending_terminate: None,
        }
    }

    pub(crate) fn new_child(command_com_psp: u16, command_tail: &[u8]) -> Self {
        let (startup_command, terminate_after_command) = parse_startup_command(command_tail);
        let pending_terminate = if terminate_after_command && startup_command.is_none() {
            Some(0)
        } else {
            None
        };

        Self {
            phase: ShellPhase::ShowPrompt,
            history: History::new(),
            commands: Self::build_commands(),
            echo_on: true,
            last_exit_code: 0,
            boot_banner_shown: true,
            pending_commands: VecDeque::new(),
            current_redirect: None,
            redirect_buffer: None,
            pipe_input: None,
            startup_command,
            terminate_after_command,
            allow_exit: true,
            owner_psp: command_com_psp,
            pending_exec: None,
            pending_terminate,
        }
    }

    pub(crate) fn handle_exec_failure(&mut self, return_code: u8) {
        self.last_exit_code = return_code;
        self.phase = ShellPhase::ShowPrompt;
        if self.terminate_after_command {
            self.pending_terminate = Some(return_code);
        }
    }

    pub(crate) fn step(&mut self, state: &mut DosState, io: &mut IoAccess, disk: &mut dyn DriveIo) {
        let phase = std::mem::replace(&mut self.phase, ShellPhase::ShowPrompt);
        self.phase = match phase {
            ShellPhase::ShowPrompt => {
                if let Some(startup_command) = self.startup_command.take() {
                    let next_phase = self.dispatch_command(state, io, disk, &startup_command);
                    if self.terminate_after_command
                        && matches!(next_phase, ShellPhase::ShowPrompt)
                        && self.pending_terminate.is_none()
                    {
                        self.pending_terminate = Some(self.last_exit_code);
                    }
                    next_phase
                } else {
                    if !self.boot_banner_shown {
                        let (major, minor) = state.version;
                        let msg = format!("Neetan DOS {}.{}\r\n\r\n", major, minor);
                        io.print(msg.as_bytes());
                        self.boot_banner_shown = true;
                    }
                    render_prompt(state, io);
                    let prompt_col = io.console.cursor_col(io.memory);
                    ShellPhase::ReadingInput(LineEditor::new(prompt_col))
                }
            }
            ShellPhase::ReadingInput(mut editor) => {
                if !key_available(io.memory) {
                    ShellPhase::ReadingInput(editor)
                } else {
                    let (scan, ch) = read_key(io.memory);
                    match ch {
                        0x0D => {
                            io.console.process_byte(io.memory, b'\r');
                            io.console.process_byte(io.memory, b'\n');
                            let line = editor.buffer.clone();
                            if !line.trim_ascii().is_empty() {
                                self.history.push(line.clone());
                            }
                            self.history.reset_position();
                            self.dispatch_command(state, io, disk, &line)
                        }
                        0x08 => {
                            if editor.cursor > 0 {
                                editor.cursor -= 1;
                                editor.buffer.remove(editor.cursor);
                                redraw_from_cursor(&editor, io);
                            }
                            ShellPhase::ReadingInput(editor)
                        }
                        0x00 => {
                            match scan {
                                SCAN_LEFT if editor.cursor > 0 => {
                                    editor.cursor -= 1;
                                    let row = io.console.cursor_row(io.memory);
                                    let col = editor.prompt_col + editor.cursor as u8;
                                    io.console.set_cursor(io.memory, row, col);
                                }
                                SCAN_RIGHT if editor.cursor < editor.buffer.len() => {
                                    editor.cursor += 1;
                                    let row = io.console.cursor_row(io.memory);
                                    let col = editor.prompt_col + editor.cursor as u8;
                                    io.console.set_cursor(io.memory, row, col);
                                }
                                SCAN_HOME => {
                                    editor.cursor = 0;
                                    let row = io.console.cursor_row(io.memory);
                                    io.console.set_cursor(io.memory, row, editor.prompt_col);
                                }
                                SCAN_END => {
                                    editor.cursor = editor.buffer.len();
                                    let row = io.console.cursor_row(io.memory);
                                    let col = editor.prompt_col + editor.cursor as u8;
                                    io.console.set_cursor(io.memory, row, col);
                                }
                                SCAN_INSERT => {
                                    editor.insert_mode = !editor.insert_mode;
                                }
                                SCAN_DELETE if editor.cursor < editor.buffer.len() => {
                                    editor.buffer.remove(editor.cursor);
                                    redraw_from_cursor(&editor, io);
                                }
                                SCAN_UP => {
                                    if self.history.at_end() && !self.history.is_empty() {
                                        editor.saved_line = Some(editor.buffer.clone());
                                    }
                                    if let Some(entry) = self.history.navigate_up() {
                                        let entry = entry.to_vec();
                                        replace_line(&mut editor, entry, io);
                                    }
                                }
                                SCAN_DOWN if !self.history.at_end() => {
                                    match self.history.navigate_down() {
                                        Some(entry) => {
                                            let entry = entry.to_vec();
                                            replace_line(&mut editor, entry, io);
                                        }
                                        None => {
                                            let restored =
                                                editor.saved_line.take().unwrap_or_default();
                                            replace_line(&mut editor, restored, io);
                                        }
                                    }
                                }
                                _ => {}
                            }
                            ShellPhase::ReadingInput(editor)
                        }
                        ch if ch >= 0x20 => {
                            if editor.buffer.len() < 127 {
                                if editor.insert_mode || editor.cursor >= editor.buffer.len() {
                                    editor.buffer.insert(editor.cursor, ch);
                                    editor.cursor += 1;
                                    if editor.cursor == editor.buffer.len() {
                                        io.console.process_byte(io.memory, ch);
                                    } else {
                                        redraw_from(&editor, editor.cursor - 1, io);
                                    }
                                } else {
                                    editor.buffer[editor.cursor] = ch;
                                    editor.cursor += 1;
                                    io.console.process_byte(io.memory, ch);
                                }
                            }
                            ShellPhase::ReadingInput(editor)
                        }
                        _ => ShellPhase::ReadingInput(editor),
                    }
                }
            }
            ShellPhase::ExecutingCommand(mut cmd) => {
                if self.redirect_buffer.is_some() {
                    io.redirect_output = self.redirect_buffer.take();
                }
                if self.pipe_input.is_some() {
                    io.redirect_input = self
                        .pipe_input
                        .take()
                        .map(|data| crate::RedirectInput { data, position: 0 });
                }
                match cmd.step(state, io, disk) {
                    StepResult::Continue => {
                        self.redirect_buffer = io.redirect_output.take();
                        self.pipe_input = io.redirect_input.take().map(|ri| {
                            let mut d = ri.data;
                            d.drain(..ri.position);
                            d
                        });
                        ShellPhase::ExecutingCommand(cmd)
                    }
                    StepResult::Done(code) => {
                        self.last_exit_code = code;
                        let output_data = io.redirect_output.take();
                        if let Some(spec) = self.current_redirect.take()
                            && let Some(data) = &output_data
                        {
                            write_redirect_to_file(state, io, disk, data, &spec);
                        }
                        if let Some(next) = self.pending_commands.pop_front() {
                            self.setup_and_dispatch(next, output_data, state, io, disk)
                        } else if self.terminate_after_command {
                            self.pending_terminate = Some(code);
                            ShellPhase::ShowPrompt
                        } else {
                            ShellPhase::ShowPrompt
                        }
                    }
                }
            }
            ShellPhase::WaitingForChild => {
                // The child process runs via CPU execution. When it terminates
                // (INT 21h AH=4Ch), terminate_process() restores COMMAND.COM's
                // PSP and IRET frame. We detect completion by checking if
                // current_psp has returned to COMMAND.COM's PSP.
                if state.current_psp == self.owner_psp {
                    self.last_exit_code = state.last_return_code;
                    if self.terminate_after_command {
                        self.pending_terminate = Some(state.last_return_code);
                    }
                    ShellPhase::ShowPrompt
                } else {
                    ShellPhase::WaitingForChild
                }
            }
            ShellPhase::ExecutingBatch(mut batch) => {
                match batch.step_batch(self, state, io, disk) {
                    batch::BatchStepResult::Continue => ShellPhase::ExecutingBatch(batch),
                    batch::BatchStepResult::Finished => {
                        if self.terminate_after_command {
                            self.pending_terminate = Some(self.last_exit_code);
                        }
                        ShellPhase::ShowPrompt
                    }
                }
            }
        };
    }

    fn dispatch_command(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DriveIo,
        line: &[u8],
    ) -> ShellPhase {
        let trimmed = line.trim_ascii();
        if trimmed.is_empty() {
            return ShellPhase::ShowPrompt;
        }

        // Split on sequence separator (ASCII 0x14) first
        let sequences = split_on_sequence(trimmed);
        if sequences.len() > 1 {
            let mut seq_iter = sequences.into_iter();
            let first = seq_iter.next().unwrap();
            for seg in seq_iter {
                self.pending_commands.push_back(PendingCommand {
                    parsed: ParsedCommand {
                        command: seg,
                        output_redirect: None,
                        input_file: None,
                    },
                });
            }
            return self.dispatch_single(state, io, disk, &first);
        }

        // Split on pipes
        let pipes = split_on_pipes(trimmed);
        if pipes.len() > 1 {
            let mut pipe_iter = pipes.into_iter();
            let first = pipe_iter.next().unwrap();
            for seg in pipe_iter {
                let parsed = parse_redirections(&seg);
                self.pending_commands.push_back(PendingCommand { parsed });
            }
            // First pipe stage: redirect output to buffer
            let parsed = parse_redirections(&first);
            self.current_redirect = None;
            self.redirect_buffer = Some(Vec::new());
            return self.dispatch_parsed(state, io, disk, &parsed.command);
        }

        self.dispatch_single(state, io, disk, trimmed)
    }

    fn dispatch_single(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DriveIo,
        segment: &[u8],
    ) -> ShellPhase {
        let parsed = parse_redirections(segment);

        // Set up output redirection
        if parsed.output_redirect.is_some() {
            self.current_redirect = parsed.output_redirect;
            self.redirect_buffer = Some(Vec::new());
        }

        // Set up input redirection
        if let Some(ref filename) = parsed.input_file {
            match read_file_data(state, io, disk, filename) {
                Ok(data) => {
                    self.pipe_input = Some(data);
                }
                Err(msg) => {
                    io.print(msg);
                    self.current_redirect = None;
                    self.redirect_buffer = None;
                    return ShellPhase::ShowPrompt;
                }
            }
        }

        self.dispatch_parsed(state, io, disk, &parsed.command)
    }

    pub(crate) fn dispatch_parsed(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DriveIo,
        command: &[u8],
    ) -> ShellPhase {
        let trimmed = command.trim_ascii();
        if trimmed.is_empty() {
            return ShellPhase::ShowPrompt;
        }

        let (cmd_name, args) = split_command(trimmed);
        let cmd_upper: Vec<u8> = cmd_name.iter().map(|b| b.to_ascii_uppercase()).collect();

        if trimmed.len() >= 5
            && eq_ignore_ascii_case(&trimmed[..5], b"PATH=")
            && let Some(cmd) = self.find_command(b"PATH")
        {
            let running = cmd.start(&trimmed[4..]);
            return ShellPhase::ExecutingCommand(running);
        }

        if eq_ignore_ascii_case(trimmed, b"PATH;")
            && let Some(cmd) = self.find_command(b"PATH")
        {
            let running = cmd.start(b";");
            return ShellPhase::ExecutingCommand(running);
        }

        // Handle ECHO ON/OFF specially (affects shell state)
        if cmd_upper == b"ECHO" {
            let args_trimmed = args.trim_ascii();
            let args_upper: Vec<u8> = args_trimmed
                .iter()
                .map(|b| b.to_ascii_uppercase())
                .collect();
            if args_upper == b"ON" {
                self.echo_on = true;
                return ShellPhase::ShowPrompt;
            }
            if args_upper == b"OFF" {
                self.echo_on = false;
                return ShellPhase::ShowPrompt;
            }
        }

        // Special case: ECHO. (dot immediately after ECHO, no space)
        if cmd_upper.starts_with(b"ECHO.")
            && let Some(cmd) = self.find_command(b"ECHO")
        {
            let running = cmd.start(b"");
            return ShellPhase::ExecutingCommand(running);
        }

        // Handle drive change: single letter followed by colon (e.g. "A:")
        if cmd_upper.len() == 2 && cmd_upper[1] == b':' && cmd_upper[0].is_ascii_uppercase() {
            let drive_index = cmd_upper[0] - b'A';
            let cds_addr = tables::CDS_BASE + (drive_index as u32) * tables::CDS_ENTRY_SIZE;
            let cds_flags = io.memory.read_word(cds_addr + tables::CDS_OFF_FLAGS);
            if cds_flags == 0 {
                io.println(b"Invalid drive");
                self.last_exit_code = 1;
                return ShellPhase::ShowPrompt;
            }
            if state
                .ensure_readable_drive_ready(drive_index, io.memory, disk)
                .is_err()
            {
                let msg = [
                    b"No media in drive ".as_slice(),
                    &[b'A' + drive_index],
                    b"\r\n",
                ]
                .concat();
                io.print(&msg);
                self.last_exit_code = 1;
                return ShellPhase::ShowPrompt;
            }
            state.current_drive = drive_index;
            return ShellPhase::ShowPrompt;
        }

        if cmd_upper == b"EXIT" && args.trim_ascii().is_empty() {
            if self.allow_exit {
                self.pending_terminate = Some(0);
            }
            return ShellPhase::ShowPrompt;
        }

        // Look up in command registry
        if let Some(cmd) = self.find_command(&cmd_upper) {
            let running = cmd.start(args);
            return ShellPhase::ExecutingCommand(running);
        }

        match find_command_file(cmd_name, &cmd_upper, state, io.memory, disk) {
            Some(FoundCommandFile::External(path)) => {
                self.pending_exec = Some(PendingExec {
                    path,
                    args: args.to_vec(),
                });
                return ShellPhase::WaitingForChild;
            }
            Some(FoundCommandFile::Batch { path, lines }) => {
                let params = parse_bat_params(args);
                let bat_state = batch::BatchState::new(lines, params, path, self.echo_on);
                return ShellPhase::ExecutingBatch(Box::new(bat_state));
            }
            Some(FoundCommandFile::BatchReadError) => {
                io.println(b"Error reading batch file");
                return ShellPhase::ShowPrompt;
            }
            None => {}
        }

        io.println(b"Bad command or file name");
        self.last_exit_code = 1;

        ShellPhase::ShowPrompt
    }

    fn setup_and_dispatch(
        &mut self,
        pending: PendingCommand,
        pipe_data: Option<Vec<u8>>,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DriveIo,
    ) -> ShellPhase {
        // Set up output redirection for this command
        if pending.parsed.output_redirect.is_some() {
            self.current_redirect = pending.parsed.output_redirect;
            self.redirect_buffer = Some(Vec::new());
        } else if !self.pending_commands.is_empty() {
            // More pipe stages follow: capture output
            self.current_redirect = None;
            self.redirect_buffer = Some(Vec::new());
        } else {
            self.current_redirect = None;
            self.redirect_buffer = None;
        }

        // Set up input: pipe data from previous command, or input file redirect
        if let Some(data) = pipe_data {
            self.pipe_input = Some(data);
        } else if let Some(ref filename) = pending.parsed.input_file {
            match read_file_data(state, io, disk, filename) {
                Ok(data) => {
                    self.pipe_input = Some(data);
                }
                Err(msg) => {
                    io.print(msg);
                    self.current_redirect = None;
                    self.redirect_buffer = None;
                    return ShellPhase::ShowPrompt;
                }
            }
        }

        self.dispatch_parsed(state, io, disk, &pending.parsed.command)
    }

    fn find_command(&self, name: &[u8]) -> Option<&dyn Command> {
        for cmd in &self.commands {
            if cmd.name().as_bytes() == name {
                return Some(cmd.as_ref());
            }
            for alias in cmd.aliases() {
                if alias.as_bytes() == name {
                    return Some(cmd.as_ref());
                }
            }
        }
        None
    }
}

fn redraw_from_cursor(editor: &LineEditor, io: &mut IoAccess) {
    redraw_from(editor, editor.cursor, io);
}

fn redraw_from(editor: &LineEditor, from: usize, io: &mut IoAccess) {
    let row = io.console.cursor_row(io.memory);
    io.console
        .set_cursor(io.memory, row, editor.prompt_col + from as u8);
    for &byte in &editor.buffer[from..] {
        io.console.process_byte(io.memory, byte);
    }
    io.console.process_byte(io.memory, b' ');
    io.console
        .set_cursor(io.memory, row, editor.prompt_col + editor.cursor as u8);
}

fn replace_line(editor: &mut LineEditor, new_buffer: Vec<u8>, io: &mut IoAccess) {
    let row = io.console.cursor_row(io.memory);
    io.console.set_cursor(io.memory, row, editor.prompt_col);
    io.console.clear_line_from_cursor(io.memory);
    for &byte in &new_buffer {
        io.console.process_byte(io.memory, byte);
    }
    editor.buffer = new_buffer;
    editor.cursor = editor.buffer.len();
}

fn split_command(line: &[u8]) -> (&[u8], &[u8]) {
    if let Some(pos) = line.iter().position(|&b| b == b' ' || b == b'\t') {
        (&line[..pos], &line[pos + 1..])
    } else {
        (line, &[])
    }
}

fn eq_ignore_ascii_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(&left_byte, &right_byte)| left_byte.eq_ignore_ascii_case(&right_byte))
}

fn trim_outer_quotes(line: &[u8]) -> Vec<u8> {
    if line.len() >= 2 && line[0] == b'"' && line[line.len() - 1] == b'"' {
        line[1..line.len() - 1].to_vec()
    } else {
        line.to_vec()
    }
}

fn parse_startup_command(command_tail: &[u8]) -> (Option<Vec<u8>>, bool) {
    let trimmed = command_tail.trim_ascii();
    if trimmed.is_empty() {
        return (None, false);
    }

    let (switch, rest) = split_command(trimmed);
    let rest = trim_outer_quotes(rest.trim_ascii());
    if eq_ignore_ascii_case(switch, b"/C") {
        let command = (!rest.is_empty()).then_some(rest);
        return (command, true);
    }
    if eq_ignore_ascii_case(switch, b"/K") {
        let command = (!rest.is_empty()).then_some(rest);
        return (command, false);
    }

    (None, false)
}

fn key_available(memory: &dyn MemoryAccess) -> bool {
    tables::key_available(memory)
}

fn read_key(memory: &mut dyn MemoryAccess) -> (u8, u8) {
    tables::read_key(memory)
}

fn render_prompt(state: &DosState, io: &mut IoAccess) {
    let prompt_value = read_env_var(state, io.memory, b"PROMPT");
    let prompt = prompt_value.unwrap_or_else(|| b"$P$G".to_vec());

    let mut i = 0;
    while i < prompt.len() {
        if prompt[i] == b'$' && i + 1 < prompt.len() {
            i += 1;
            match prompt[i].to_ascii_uppercase() {
                b'P' => {
                    let cds_addr =
                        tables::CDS_BASE + (state.current_drive as u32) * tables::CDS_ENTRY_SIZE;
                    for j in 0..67u32 {
                        let byte = io.memory.read_byte(cds_addr + tables::CDS_OFF_PATH + j);
                        if byte == 0 {
                            break;
                        }
                        io.console.process_byte(io.memory, byte);
                    }
                }
                b'G' => {
                    io.console.process_byte(io.memory, b'>');
                }
                b'L' => {
                    io.console.process_byte(io.memory, b'<');
                }
                b'E' => {
                    io.console.process_byte(io.memory, 0x1B);
                }
                b'H' => {
                    io.console.process_byte(io.memory, 0x08);
                }
                b'_' => {
                    io.console.process_byte(io.memory, b'\r');
                    io.console.process_byte(io.memory, b'\n');
                }
                b'$' => {
                    io.console.process_byte(io.memory, b'$');
                }
                b'D' => {
                    let (year, month, day, _dow) = state.current_date_parts();
                    let msg = format!("{:04}-{:02}-{:02}", year, month, day);
                    for &byte in msg.as_bytes() {
                        io.console.process_byte(io.memory, byte);
                    }
                }
                b'T' => {
                    let (hour, minute, second) = state.current_time_parts();
                    let msg = format!("{:02}:{:02}:{:02}", hour, minute, second);
                    for &byte in msg.as_bytes() {
                        io.console.process_byte(io.memory, byte);
                    }
                }
                b'N' => {
                    io.console
                        .process_byte(io.memory, b'A' + state.current_drive);
                }
                b'V' => {
                    let (major, minor) = state.version;
                    let msg = format!("Neetan DOS {}.{}", major, minor);
                    for &byte in msg.as_bytes() {
                        io.console.process_byte(io.memory, byte);
                    }
                }
                _ => {
                    io.console.process_byte(io.memory, b'$');
                    io.console.process_byte(io.memory, prompt[i]);
                }
            }
        } else {
            io.console.process_byte(io.memory, prompt[i]);
        }
        i += 1;
    }
}

pub(crate) fn read_env_var(
    state: &DosState,
    memory: &dyn MemoryAccess,
    var_name: &[u8],
) -> Option<Vec<u8>> {
    environment::read_var(state, memory, var_name)
}

fn split_on_sequence(line: &[u8]) -> Vec<Vec<u8>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    for &byte in line {
        if byte == 0x14 {
            if !current.is_empty() {
                segments.push(current);
                current = Vec::new();
            }
        } else {
            current.push(byte);
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn split_on_pipes(line: &[u8]) -> Vec<Vec<u8>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    for &byte in line {
        if byte == b'|' {
            segments.push(current);
            current = Vec::new();
        } else {
            current.push(byte);
        }
    }
    segments.push(current);
    segments
}

fn parse_redirections(segment: &[u8]) -> ParsedCommand {
    let mut command = Vec::new();
    let mut output_redirect = None;
    let mut input_file = None;

    let mut i = 0;
    while i < segment.len() {
        if segment[i] == b'>' {
            i += 1;
            let append = i < segment.len() && segment[i] == b'>';
            if append {
                i += 1;
            }
            // Skip whitespace after >
            while i < segment.len() && (segment[i] == b' ' || segment[i] == b'\t') {
                i += 1;
            }
            // Read filename
            let mut filename = Vec::new();
            while i < segment.len()
                && segment[i] != b' '
                && segment[i] != b'\t'
                && segment[i] != b'>'
                && segment[i] != b'<'
                && segment[i] != b'|'
            {
                filename.push(segment[i]);
                i += 1;
            }
            if !filename.is_empty() {
                output_redirect = if append {
                    Some(RedirectSpec::Append(filename))
                } else {
                    Some(RedirectSpec::Overwrite(filename))
                };
            }
        } else if segment[i] == b'<' {
            i += 1;
            while i < segment.len() && (segment[i] == b' ' || segment[i] == b'\t') {
                i += 1;
            }
            let mut filename = Vec::new();
            while i < segment.len()
                && segment[i] != b' '
                && segment[i] != b'\t'
                && segment[i] != b'>'
                && segment[i] != b'<'
                && segment[i] != b'|'
            {
                filename.push(segment[i]);
                i += 1;
            }
            if !filename.is_empty() {
                input_file = Some(filename);
            }
        } else {
            command.push(segment[i]);
            i += 1;
        }
    }

    ParsedCommand {
        command,
        output_redirect,
        input_file,
    }
}

enum FoundCommandFile {
    External(Vec<u8>),
    Batch { path: Vec<u8>, lines: Vec<Vec<u8>> },
    BatchReadError,
}

pub(crate) enum BatchSearchResult {
    Found { path: Vec<u8>, lines: Vec<Vec<u8>> },
    ReadError,
    NotFound,
}

#[derive(Clone, Copy)]
enum CommandFileExtension {
    Com,
    Exe,
    Bat,
}

fn find_command_file(
    original_cmd: &[u8],
    cmd_upper: &[u8],
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DriveIo,
) -> Option<FoundCommandFile> {
    let extension = command_file_extension(cmd_upper);

    if command_has_path(original_cmd) {
        return try_find_command_file(original_cmd, extension, state, memory, disk);
    }

    if let Some(found) = try_find_command_file(original_cmd, extension, state, memory, disk) {
        return Some(found);
    }

    let path_value = read_env_var(state, memory, b"PATH")?;
    for dir in path_value.split(|&b| b == b';') {
        if let Some(candidate) = path_candidate(dir, original_cmd)
            && let Some(found) = try_find_command_file(&candidate, extension, state, memory, disk)
        {
            return Some(found);
        }
    }

    None
}

pub(crate) fn find_batch_file(
    original_name: &[u8],
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DriveIo,
) -> BatchSearchResult {
    let upper_name: Vec<u8> = original_name
        .iter()
        .map(|byte| byte.to_ascii_uppercase())
        .collect();
    let extension = command_file_extension(&upper_name);

    if matches!(
        extension,
        Some(CommandFileExtension::Com | CommandFileExtension::Exe)
    ) {
        return BatchSearchResult::NotFound;
    }

    if command_has_path(original_name) {
        return try_find_batch_candidate(original_name, extension, state, memory, disk);
    }

    match try_find_batch_candidate(original_name, extension, state, memory, disk) {
        BatchSearchResult::NotFound => {}
        result => return result,
    }

    let Some(path_value) = read_env_var(state, memory, b"PATH") else {
        return BatchSearchResult::NotFound;
    };
    for dir in path_value.split(|&byte| byte == b';') {
        let Some(candidate) = path_candidate(dir, original_name) else {
            continue;
        };
        match try_find_batch_candidate(&candidate, extension, state, memory, disk) {
            BatchSearchResult::NotFound => {}
            result => return result,
        }
    }

    BatchSearchResult::NotFound
}

fn try_find_command_file(
    path: &[u8],
    extension: Option<CommandFileExtension>,
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DriveIo,
) -> Option<FoundCommandFile> {
    match extension {
        Some(CommandFileExtension::Com | CommandFileExtension::Exe) => {
            try_find_external_program(path, state, memory, disk).map(FoundCommandFile::External)
        }
        Some(CommandFileExtension::Bat) => {
            match try_find_batch_candidate(path, extension, state, memory, disk) {
                BatchSearchResult::Found { path, lines } => {
                    Some(FoundCommandFile::Batch { path, lines })
                }
                BatchSearchResult::ReadError => Some(FoundCommandFile::BatchReadError),
                BatchSearchResult::NotFound => None,
            }
        }
        None => {
            let mut com_path = path.to_vec();
            com_path.extend_from_slice(b".COM");
            if let Some(found) = try_find_external_program(&com_path, state, memory, disk) {
                return Some(FoundCommandFile::External(found));
            }

            let mut exe_path = path.to_vec();
            exe_path.extend_from_slice(b".EXE");
            if let Some(found) = try_find_external_program(&exe_path, state, memory, disk) {
                return Some(FoundCommandFile::External(found));
            }

            let mut bat_path = path.to_vec();
            bat_path.extend_from_slice(b".BAT");
            match try_find_batch_candidate(
                &bat_path,
                Some(CommandFileExtension::Bat),
                state,
                memory,
                disk,
            ) {
                BatchSearchResult::Found { path, lines } => {
                    Some(FoundCommandFile::Batch { path, lines })
                }
                BatchSearchResult::ReadError => Some(FoundCommandFile::BatchReadError),
                BatchSearchResult::NotFound => None,
            }
        }
    }
}

fn try_find_external_program(
    path: &[u8],
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DriveIo,
) -> Option<Vec<u8>> {
    if process::is_command_processor_path(path) || file_exists_on_disk(path, state, memory, disk) {
        Some(path.to_vec())
    } else {
        None
    }
}

fn try_find_batch_candidate(
    path: &[u8],
    extension: Option<CommandFileExtension>,
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DriveIo,
) -> BatchSearchResult {
    let mut bat_path = path.to_vec();
    if !matches!(extension, Some(CommandFileExtension::Bat)) {
        bat_path.extend_from_slice(b".BAT");
    }

    let (drive_index, dir_cluster, fcb_name) =
        match crate::filesystem::resolve_file_path(state, &bat_path, memory, disk) {
            Ok(path) => path,
            Err(_) => return BatchSearchResult::NotFound,
        };

    if drive_index == 25 {
        return BatchSearchResult::NotFound;
    }

    let Some(vol) = state.fat_volumes[drive_index as usize].as_ref() else {
        return BatchSearchResult::NotFound;
    };

    let entry = match fat_dir::find_entry(vol, dir_cluster, &fcb_name, disk) {
        Ok(Some(entry)) if entry.attribute & fat_dir::ATTR_DIRECTORY == 0 => entry,
        _ => return BatchSearchResult::NotFound,
    };

    match batch::load_bat_file(vol, &entry, disk) {
        Ok(lines) => BatchSearchResult::Found {
            path: bat_path,
            lines,
        },
        Err(_) => BatchSearchResult::ReadError,
    }
}

fn command_file_extension(cmd_upper: &[u8]) -> Option<CommandFileExtension> {
    if cmd_upper.len() <= 4 {
        return None;
    }
    if cmd_upper.ends_with(b".COM") {
        Some(CommandFileExtension::Com)
    } else if cmd_upper.ends_with(b".EXE") {
        Some(CommandFileExtension::Exe)
    } else if cmd_upper.ends_with(b".BAT") {
        Some(CommandFileExtension::Bat)
    } else {
        None
    }
}

fn command_has_path(command: &[u8]) -> bool {
    command.contains(&b'\\') || command.contains(&b'/') || command.len() >= 2 && command[1] == b':'
}

fn path_candidate(dir: &[u8], command: &[u8]) -> Option<Vec<u8>> {
    let dir = dir.trim_ascii();
    if dir.is_empty() {
        return None;
    }

    let mut path = dir.to_vec();
    if !path.ends_with(b"\\") && !path.ends_with(b"/") {
        path.push(b'\\');
    }
    path.extend_from_slice(command);
    Some(path)
}

/// Checks if a file exists on a DOS drive, including the virtual Z: drive.
fn file_exists_on_disk(
    path: &[u8],
    state: &mut DosState,
    memory: &dyn MemoryAccess,
    disk: &mut dyn DriveIo,
) -> bool {
    let read_path = match crate::filesystem::resolve_read_file_path(state, path, memory, disk) {
        Ok(path) => path,
        Err(_) => return false,
    };
    if read_path.drive_index == 25 {
        let (_, _, fcb_name) = match crate::filesystem::resolve_file_path(state, path, memory, disk)
        {
            Ok(parts) => parts,
            Err(_) => return false,
        };
        return state.virtual_drive.find_entry(&fcb_name).is_some();
    }
    let entry = match filesystem::find_read_entry(state, &read_path, disk) {
        Ok(Some(entry)) => entry,
        _ => return false,
    };
    entry.attribute & fat_dir::ATTR_DIRECTORY == 0
}

fn parse_bat_params(args: &[u8]) -> [Vec<u8>; 10] {
    let mut params: [Vec<u8>; 10] = Default::default();
    let mut idx = 1usize; // %1 is first argument, %0 is filled by caller
    let trimmed = args.trim_ascii();
    if trimmed.is_empty() {
        return params;
    }
    let mut i = 0;
    while i < trimmed.len() && idx < 10 {
        // Skip whitespace
        while i < trimmed.len() && (trimmed[i] == b' ' || trimmed[i] == b'\t') {
            i += 1;
        }
        if i >= trimmed.len() {
            break;
        }
        let start = i;
        while i < trimmed.len() && trimmed[i] != b' ' && trimmed[i] != b'\t' {
            i += 1;
        }
        params[idx] = trimmed[start..i].to_vec();
        idx += 1;
    }
    params
}

fn write_redirect_to_file(
    state: &mut DosState,
    io: &mut IoAccess,
    disk: &mut dyn DiskIo,
    data: &[u8],
    spec: &RedirectSpec,
) {
    let filename = match spec {
        RedirectSpec::Overwrite(f) | RedirectSpec::Append(f) => f,
    };
    let is_append = matches!(spec, RedirectSpec::Append(_));

    let (drive_index, dir_cluster, fcb_name) =
        match crate::filesystem::resolve_file_path(state, filename, io.memory, disk) {
            Ok(r) => r,
            Err(_) => {
                io.console.process_byte(io.memory, b'\r');
                io.console.process_byte(io.memory, b'\n');
                for &byte in b"File creation error" {
                    io.console.process_byte(io.memory, byte);
                }
                io.console.process_byte(io.memory, b'\r');
                io.console.process_byte(io.memory, b'\n');
                return;
            }
        };

    if drive_index == 25 {
        return; // Z: is read-only
    }

    let (time, date) = state.dos_timestamp_now();

    let vol = match state.fat_volumes[drive_index as usize].as_mut() {
        Some(v) => v,
        None => return,
    };

    if is_append {
        // Append mode: find existing file, walk to end of chain, append data
        if let Ok(Some(existing)) = fat_dir::find_entry(vol, dir_cluster, &fcb_name, disk) {
            append_to_existing_file(vol, &existing, data, disk);
        } else {
            let _ = fat_file::create_or_replace_file(
                vol,
                dir_cluster,
                &fcb_name,
                data,
                fat_file::FileCreateOptions {
                    attributes: fat_dir::ATTR_ARCHIVE,
                    time,
                    date,
                },
                disk,
            );
        }
    } else {
        let _ = fat_file::create_or_replace_file(
            vol,
            dir_cluster,
            &fcb_name,
            data,
            fat_file::FileCreateOptions {
                attributes: fat_dir::ATTR_ARCHIVE,
                time,
                date,
            },
            disk,
        );
    }

    let _ = vol.flush_fat(disk);
}

fn append_to_existing_file(
    vol: &mut fat::FatVolume,
    entry: &fat_dir::DirEntry,
    data: &[u8],
    disk: &mut dyn DiskIo,
) {
    let mut writer = fat_file::FatFileWriter::new(entry.start_cluster, entry.file_size);
    if writer.write_chunk(vol, disk, data).is_err() {
        return;
    }

    let mut updated = entry.clone();
    updated.start_cluster = writer.start_cluster();
    updated.file_size = writer.position();
    let _ = fat_dir::update_entry(vol, &updated, disk);
}

fn read_file_data(
    state: &mut DosState,
    io: &mut IoAccess,
    disk: &mut dyn DriveIo,
    filename: &[u8],
) -> Result<Vec<u8>, &'static [u8]> {
    let read_path = crate::filesystem::resolve_read_file_path(state, filename, io.memory, disk)
        .map_err(|_| &b"File not found\r\n"[..])?;
    if read_path.drive_index == 25 {
        return Err(b"Access denied\r\n");
    }

    let entry = filesystem::find_read_entry(state, &read_path, disk)
        .map_err(|_| &b"File not found\r\n"[..])?
        .ok_or(&b"File not found\r\n"[..])?;

    filesystem::read_entry_all(state, read_path.drive_index, &entry, disk)
        .map_err(|_| &b"Read error\r\n"[..])
}
