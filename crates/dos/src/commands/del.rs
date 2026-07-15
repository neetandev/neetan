//! DEL / ERASE command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    filesystem::{self, fat_dir},
};

pub(crate) struct Del;

impl Command for Del {
    fn name(&self) -> &'static str {
        "DEL"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["ERASE"]
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningDel {
            args: args.to_vec(),
            phase: DelPhase::Init,
        })
    }
}

#[derive(Clone)]
/// Authoritative DEL enumeration and confirmation state.
struct DelState {
    drive_index: u8,
    dir_cluster: u16,
    fcb_pattern: [u8; 11],
    start_index: u16,
    prompt: bool,
    deleted_any: bool,
}

state_struct_codec!(DelState {
    drive_index,
    dir_cluster,
    fcb_pattern,
    start_index,
    prompt,
    deleted_any,
});

#[derive(Clone)]
enum DelPhase {
    Init,
    ConfirmAll(DelState),
    DeleteNext(DelState),
    PromptFile(DelState, fat_dir::DirEntry),
}

impl save_state::StateEncode for DelPhase {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Init => save_state::StateEncode::encode_state(&0u8, output),
            Self::ConfirmAll(state) => {
                save_state::StateEncode::encode_state(&1u8, output);
                save_state::StateEncode::encode_state(state, output);
            }
            Self::DeleteNext(state) => {
                save_state::StateEncode::encode_state(&2u8, output);
                save_state::StateEncode::encode_state(state, output);
            }
            Self::PromptFile(state, entry) => {
                save_state::StateEncode::encode_state(&3u8, output);
                save_state::StateEncode::encode_state(state, output);
                save_state::StateEncode::encode_state(entry, output);
            }
        }
    }
}

impl save_state::StateDecode for DelPhase {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Init),
            1 => Ok(Self::ConfirmAll(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            2 => Ok(Self::DeleteNext(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            3 => Ok(Self::PromptFile(
                save_state::StateDecode::decode_state(decoder)?,
                save_state::StateDecode::decode_state(decoder)?,
            )),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

#[derive(Clone)]
/// Serializable state of an executing DEL command.
pub(crate) struct RunningDel {
    args: Vec<u8>,
    phase: DelPhase,
}

state_struct_codec!(RunningDel { args, phase });

const KB_BUF_COUNT: u32 = 0x0528;

impl RunningDel {
    fn step_init(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let args = self.args.trim_ascii().to_vec();
        if is_help_request(&args) || args.is_empty() {
            print_help(io);
            return StepResult::Done(0);
        }

        let (path, has_prompt) = parse_switches(&args);
        let path = path.to_vec();

        let (drive_index, dir_cluster, fcb_pattern) =
            match filesystem::resolve_file_path(state, &path, io.memory, drive) {
                Ok(r) => r,
                Err(_) => {
                    io.println(b"File not found");
                    return StepResult::Done(1);
                }
            };

        if drive_index == 25 {
            io.println(b"Access denied");
            return StepResult::Done(1);
        }

        let del_state = DelState {
            drive_index,
            dir_cluster,
            fcb_pattern,
            start_index: 0,
            prompt: has_prompt,
            deleted_any: false,
        };

        // Check if all-wildcard pattern -> confirm
        let filename_part = path
            .iter()
            .rposition(|&b| b == b'\\')
            .map(|p| &path[p + 1..])
            .unwrap_or(&path);
        let is_all_wildcard = filename_part == b"*.*" || filename_part == b"*";

        if is_all_wildcard && !has_prompt {
            io.print(b"All files in directory will be deleted!\r\nAre you sure (Y/N)?");
            self.phase = DelPhase::ConfirmAll(del_state);
        } else {
            self.phase = DelPhase::DeleteNext(del_state);
        }
        StepResult::Continue
    }

    fn step_confirm_all(&mut self, del_state: DelState, io: &mut IoAccess) -> StepResult {
        if io.memory.read_byte(KB_BUF_COUNT) == 0 {
            self.phase = DelPhase::ConfirmAll(del_state);
            return StepResult::Continue;
        }
        let key = consume_key(io);
        io.output_byte(b'\r');
        io.output_byte(b'\n');
        if key == b'Y' || key == b'y' {
            self.phase = DelPhase::DeleteNext(del_state);
            StepResult::Continue
        } else {
            StepResult::Done(0)
        }
    }

    fn step_delete_next(
        &mut self,
        mut del_state: DelState,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let vol = match state.fat_volumes[del_state.drive_index as usize].as_ref() {
            Some(v) => v,
            None => return StepResult::Done(1),
        };

        let result = fat_dir::find_matching(
            vol,
            del_state.dir_cluster,
            &del_state.fcb_pattern,
            0,
            del_state.start_index,
            drive,
        );

        match result {
            Ok(Some((entry, next_index))) => {
                del_state.start_index = next_index;

                // Skip directories, volume labels, read-only
                if entry.attribute
                    & (fat_dir::ATTR_DIRECTORY | fat_dir::ATTR_VOLUME_ID | fat_dir::ATTR_READ_ONLY)
                    != 0
                {
                    self.phase = DelPhase::DeleteNext(del_state);
                    return StepResult::Continue;
                }

                if del_state.prompt {
                    // /P: show filename and ask
                    let display = fat_dir::fcb_to_display_name(&entry.name);
                    for &b in &display {
                        io.output_byte(b);
                    }
                    io.print(b", Delete (Y/N)?");
                    self.phase = DelPhase::PromptFile(del_state, entry);
                } else {
                    // No prompt: delete immediately
                    if filesystem::delete_file_by_components(
                        state,
                        drive,
                        del_state.drive_index,
                        del_state.dir_cluster,
                        entry.name,
                    )
                    .is_ok()
                    {
                        del_state.deleted_any = true;
                        del_state.start_index = del_state.start_index.saturating_sub(1);
                    }
                    self.phase = DelPhase::DeleteNext(del_state);
                }
                StepResult::Continue
            }
            Ok(None) => {
                if !del_state.deleted_any {
                    io.println(b"File not found");
                    return StepResult::Done(1);
                }
                StepResult::Done(0)
            }
            Err(_) => {
                io.println(b"File not found");
                StepResult::Done(1)
            }
        }
    }

    fn step_prompt_file(
        &mut self,
        mut del_state: DelState,
        entry: fat_dir::DirEntry,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if io.memory.read_byte(KB_BUF_COUNT) == 0 {
            self.phase = DelPhase::PromptFile(del_state, entry);
            return StepResult::Continue;
        }
        let key = consume_key(io);
        io.output_byte(b'\r');
        io.output_byte(b'\n');

        if (key == b'Y' || key == b'y')
            && filesystem::delete_file_by_components(
                state,
                drive,
                del_state.drive_index,
                del_state.dir_cluster,
                entry.name,
            )
            .is_ok()
        {
            del_state.deleted_any = true;
            del_state.start_index = del_state.start_index.saturating_sub(1);
        }

        self.phase = DelPhase::DeleteNext(del_state);
        StepResult::Continue
    }
}

impl RunningCommand for RunningDel {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let phase = std::mem::replace(&mut self.phase, DelPhase::Init);
        match phase {
            DelPhase::Init => self.step_init(state, io, drive),
            DelPhase::ConfirmAll(ds) => self.step_confirm_all(ds, io),
            DelPhase::DeleteNext(ds) => self.step_delete_next(ds, state, io, drive),
            DelPhase::PromptFile(ds, entry) => self.step_prompt_file(ds, entry, state, io, drive),
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Deletes one or more files.");
    io.println(b"");
    io.println(b"DEL [/P] filename");
    io.println(b"ERASE [/P] filename");
    io.println(b"");
    io.println(b"  filename  Specifies the file(s) to delete. Use wildcards to");
    io.println(b"            delete multiple files.");
    io.println(b"  /P        Prompts for confirmation before deleting each file.");
}

fn parse_switches(args: &[u8]) -> (&[u8], bool) {
    let mut i = 0;
    while i < args.len() {
        if args[i] == b'/' && i + 1 < args.len() && args[i + 1].eq_ignore_ascii_case(&b'P') {
            let before = &args[..i];
            let after = if i + 2 < args.len() {
                &args[i + 2..]
            } else {
                &[]
            };
            let trimmed = if !before.is_empty() {
                before.trim_ascii()
            } else {
                after.trim_ascii()
            };
            return (trimmed, true);
        }
        i += 1;
    }
    (args, false)
}

fn consume_key(io: &mut IoAccess) -> u8 {
    let head = io.memory.read_word(0x0524) as u32;
    let ch = io.memory.read_byte(head);
    let mut new_head = head + 2;
    if new_head >= 0x0522 {
        new_head = 0x0502;
    }
    io.memory.write_word(0x0524, new_head as u16);
    let count = io.memory.read_byte(KB_BUF_COUNT);
    if count > 0 {
        io.memory.write_byte(KB_BUF_COUNT, count - 1);
    }
    ch
}
