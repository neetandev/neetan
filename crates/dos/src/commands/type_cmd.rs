//! TYPE command.

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    filesystem::{self, fat_dir},
};

pub(crate) struct TypeCmd;

impl Command for TypeCmd {
    fn name(&self) -> &'static str {
        "TYPE"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningType {
            args: args.to_vec(),
            phase: TypePhase::Init,
        })
    }
}

struct ReadState {
    data: Vec<u8>,
    offset: usize,
}

enum TypePhase {
    Init,
    Outputting(ReadState),
}

struct RunningType {
    args: Vec<u8>,
    phase: TypePhase,
}

impl RunningType {
    fn do_output(
        &mut self,
        _state: &mut DosState,
        io: &mut IoAccess,
        _drive: &mut dyn DriveIo,
        read: ReadState,
    ) -> StepResult {
        let chunk_end = (read.offset + 4096).min(read.data.len());
        for &byte in &read.data[read.offset..chunk_end] {
            io.output_byte(byte);
        }

        if chunk_end >= read.data.len() {
            return StepResult::Done(0);
        }

        self.phase = TypePhase::Outputting(ReadState {
            data: read.data,
            offset: chunk_end,
        });
        StepResult::Continue
    }
}

impl RunningCommand for RunningType {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let phase = std::mem::replace(&mut self.phase, TypePhase::Init);
        match phase {
            TypePhase::Init => {
                let args = self.args.trim_ascii();
                if is_help_request(&self.args) || args.is_empty() {
                    print_help(io);
                    return StepResult::Done(0);
                }

                match init_type(state, io, drive, args) {
                    Ok(new_phase) => {
                        self.phase = new_phase;
                        StepResult::Continue
                    }
                    Err(msg) => {
                        io.print(msg);
                        StepResult::Done(1)
                    }
                }
            }
            TypePhase::Outputting(read) => self.do_output(state, io, drive, read),
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Displays the contents of a text file.");
    io.println(b"");
    io.println(b"TYPE filename");
    io.println(b"");
    io.println(b"  filename  Specifies the file to display.");
}

fn init_type(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    path: &[u8],
) -> Result<TypePhase, &'static [u8]> {
    let read_path = filesystem::resolve_read_file_path(state, path, io.memory, drive)
        .map_err(|_| &b"File not found\r\n"[..])?;

    if read_path.drive_index == 25 {
        return Err(b"Access denied\r\n");
    }

    let entry = filesystem::find_read_entry(state, &read_path, drive)
        .map_err(|_| &b"File not found\r\n"[..])?
        .ok_or(&b"File not found\r\n"[..])?;

    if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 {
        return Err(b"Access denied\r\n");
    }

    if entry.file_size == 0 {
        return Ok(TypePhase::Init);
    }

    let data = filesystem::read_entry_all(state, read_path.drive_index, &entry, drive)
        .map_err(|_| &b"Read error\r\n"[..])?;

    Ok(TypePhase::Outputting(ReadState { data, offset: 0 }))
}
