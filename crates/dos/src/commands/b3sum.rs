//! B3SUM command.

use blake3::Hasher;

use crate::{
    DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    dos,
    filesystem::{
        self, ReadDirEntrySource, ReadDirectory, fat_dir, fat_file::FatFileCursor, iso9660,
    },
};

pub(crate) struct B3sum;

impl Command for B3sum {
    fn name(&self) -> &'static str {
        "B3SUM"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningB3sum {
            args: args.to_vec(),
            phase: B3sumPhase::Init,
        })
    }
}

const ISO_CHUNK_SIZE: usize = 2048;

struct B3sumState {
    arguments: Vec<ArgumentState>,
    current_argument: usize,
}

struct ArgumentState {
    display_path: Vec<u8>,
    wildcard_prefix: Vec<u8>,
    drive_index: u8,
    directory: ReadDirectory,
    pattern: [u8; 11],
    has_wildcard: bool,
    search_index: u16,
    matched_any: bool,
}

enum HashSourceCursor {
    Fat(FatFileCursor),
    Iso {
        entry: iso9660::IsoDirEntry,
        position: u32,
    },
}

struct FileHashState {
    drive_index: u8,
    cursor: HashSourceCursor,
    display_path: Vec<u8>,
    hasher: Hasher,
}

enum B3sumPhase {
    Init,
    FindNext(B3sumState),
    Hashing(B3sumState, Box<FileHashState>),
}

struct RunningB3sum {
    args: Vec<u8>,
    phase: B3sumPhase,
}

impl RunningB3sum {
    fn step_init(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let args = self.args.trim_ascii();
        if args.is_empty() || is_help_request(args) {
            print_help(io);
            return StepResult::Done(0);
        }

        match init_b3sum_state(state, io, drive, args) {
            Ok(b3sum_state) => {
                self.phase = B3sumPhase::FindNext(b3sum_state);
                StepResult::Continue
            }
            Err(message) => {
                io.print(message);
                StepResult::Done(1)
            }
        }
    }

    fn step_find_next(
        &mut self,
        mut b3sum_state: B3sumState,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if b3sum_state.current_argument >= b3sum_state.arguments.len() {
            return StepResult::Done(0);
        }

        let argument = &mut b3sum_state.arguments[b3sum_state.current_argument];
        if argument.drive_index == 25 {
            io.println(b"Access denied");
            return StepResult::Done(1);
        }

        let result = filesystem::find_matching_read_entry(
            state,
            argument.drive_index,
            &argument.directory,
            &argument.pattern,
            0,
            argument.search_index,
            drive,
        );

        match result {
            Ok(Some((entry, next_index))) => {
                argument.search_index = next_index;

                let display_path = if argument.has_wildcard {
                    let mut display_path = argument.wildcard_prefix.clone();
                    display_path.extend_from_slice(&fat_dir::fcb_to_display_name(&entry.name));
                    display_path
                } else {
                    argument.display_path.clone()
                };

                let drive_index = argument.drive_index;
                let has_wildcard = argument.has_wildcard;
                let hasher = Hasher::new();

                if entry.file_size == 0 {
                    argument.matched_any = true;
                    print_digest(io, &hasher, &display_path);
                    if !has_wildcard {
                        b3sum_state.current_argument += 1;
                    }
                    self.phase = B3sumPhase::FindNext(b3sum_state);
                    return StepResult::Continue;
                }

                let cursor = match entry.source {
                    ReadDirEntrySource::Fat(fat_entry) => {
                        if fat_entry.start_cluster < 2 {
                            io.println(b"Read error");
                            return StepResult::Done(1);
                        }
                        HashSourceCursor::Fat(FatFileCursor::new(&fat_entry))
                    }
                    ReadDirEntrySource::Iso(iso_entry) => HashSourceCursor::Iso {
                        entry: iso_entry,
                        position: 0,
                    },
                };

                self.phase = B3sumPhase::Hashing(
                    b3sum_state,
                    Box::new(FileHashState {
                        drive_index,
                        cursor,
                        display_path,
                        hasher,
                    }),
                );
                StepResult::Continue
            }
            Ok(None) => {
                if !argument.matched_any {
                    io.println(b"File not found");
                    return StepResult::Done(1);
                }

                b3sum_state.current_argument += 1;
                self.phase = B3sumPhase::FindNext(b3sum_state);
                StepResult::Continue
            }
            Err(_) => {
                io.println(b"File not found");
                StepResult::Done(1)
            }
        }
    }

    fn step_hashing(
        &mut self,
        mut b3sum_state: B3sumState,
        mut file_hash_state: Box<FileHashState>,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let chunk_data = match &mut file_hash_state.cursor {
            HashSourceCursor::Fat(cursor) => {
                let volume = match state.fat_volumes[file_hash_state.drive_index as usize].as_ref()
                {
                    Some(volume) => volume,
                    None => {
                        io.println(b"Invalid drive");
                        return StepResult::Done(1);
                    }
                };

                match cursor.read_chunk(volume, drive, volume.bpb.cluster_size() as usize) {
                    Ok(chunk_data) => chunk_data,
                    Err(_) => {
                        io.println(b"Read error");
                        return StepResult::Done(1);
                    }
                }
            }
            HashSourceCursor::Iso { entry, position } => {
                if *position >= entry.file_size {
                    Vec::new()
                } else {
                    match iso9660::read_file_chunk(entry, *position, ISO_CHUNK_SIZE, drive) {
                        Ok(data) => {
                            *position += data.len() as u32;
                            data
                        }
                        Err(_) => {
                            io.println(b"Read error");
                            return StepResult::Done(1);
                        }
                    }
                }
            }
        };

        if chunk_data.is_empty() {
            let current_argument = &mut b3sum_state.arguments[b3sum_state.current_argument];
            current_argument.matched_any = true;
            print_digest(io, &file_hash_state.hasher, &file_hash_state.display_path);
            if !current_argument.has_wildcard {
                b3sum_state.current_argument += 1;
            }
            self.phase = B3sumPhase::FindNext(b3sum_state);
            return StepResult::Continue;
        }

        file_hash_state.hasher.update(&chunk_data);
        self.phase = B3sumPhase::Hashing(b3sum_state, file_hash_state);
        StepResult::Continue
    }
}

impl RunningCommand for RunningB3sum {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let phase = std::mem::replace(&mut self.phase, B3sumPhase::Init);
        match phase {
            B3sumPhase::Init => self.step_init(state, io, drive),
            B3sumPhase::FindNext(b3sum_state) => self.step_find_next(b3sum_state, state, io, drive),
            B3sumPhase::Hashing(b3sum_state, file_hash_state) => {
                self.step_hashing(b3sum_state, file_hash_state, state, io, drive)
            }
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Computes BLAKE3 hashes of files.");
    io.println(b"");
    io.println(b"B3SUM [drive:][path]filename [...]");
    io.println(b"");
    io.println(b"  filename  Specifies one or more files to hash.");
}

fn init_b3sum_state(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    args: &[u8],
) -> Result<B3sumState, &'static [u8]> {
    let mut arguments = Vec::new();

    for part in args.split(|&byte| byte == b' ' || byte == b'\t') {
        if part.is_empty() {
            continue;
        }

        let normalized_path = dos::normalize_path(part);
        let has_wildcard = normalized_path.contains(&b'*') || normalized_path.contains(&b'?');
        let read_path =
            filesystem::resolve_read_file_path(state, &normalized_path, io.memory, drive)
                .map_err(|_| &b"File not found\r\n"[..])?;

        arguments.push(ArgumentState {
            display_path: normalized_path.clone(),
            wildcard_prefix: wildcard_display_prefix(&normalized_path),
            drive_index: read_path.drive_index,
            directory: read_path.directory,
            pattern: read_path.name,
            has_wildcard,
            search_index: 0,
            matched_any: false,
        });
    }

    if arguments.is_empty() {
        return Err(b"Required parameter missing\r\n");
    }

    Ok(B3sumState {
        arguments,
        current_argument: 0,
    })
}

fn wildcard_display_prefix(path: &[u8]) -> Vec<u8> {
    if let Some(position) = path.iter().rposition(|&byte| byte == b'\\') {
        return path[..=position].to_vec();
    }
    if path.len() >= 2 && path[1] == b':' {
        return path[..2].to_vec();
    }
    Vec::new()
}

fn print_digest(io: &mut IoAccess, hasher: &Hasher, display_path: &[u8]) {
    let mut digest = [0u8; 32];
    let mut line = Vec::with_capacity(64 + 2 + display_path.len() + 2);
    hasher.finalize(&mut digest);
    push_digest_hex(&mut line, &digest);
    line.extend_from_slice(b"  ");
    line.extend_from_slice(display_path);
    line.extend_from_slice(b"\r\n");
    io.print(&line);
}

fn push_digest_hex(out: &mut Vec<u8>, digest: &[u8; 32]) {
    for &byte in digest {
        out.push(nibble_to_hex(byte >> 4));
        out.push(nibble_to_hex(byte & 0x0F));
    }
}

fn nibble_to_hex(nibble: u8) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        _ => b'a' + (nibble - 10),
    }
}
