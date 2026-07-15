//! XCOPY command.

use crate::{
    DiskIo, DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    filesystem::{
        self, PendingFatFile, ReadDirEntry, ReadDirEntrySource, ReadDirectory, fat_dir,
        fat_file::FatFileCursor, iso9660,
    },
};

pub(crate) struct Xcopy;

impl Command for Xcopy {
    fn name(&self) -> &'static str {
        "XCOPY"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningXcopy {
            args: args.to_vec(),
            phase: XcopyPhase::Init,
        })
    }
}

const KB_BUF_COUNT: u32 = 0x0528;

#[derive(Clone)]
/// Authoritative XCOPY traversal and prompt state.
struct XcopyState {
    src_drive: u8,
    src_directory: ReadDirectory,
    src_pattern: [u8; 11],
    src_search_index: u16,
    dst_drive: u8,
    dst_dir_cluster: u16,
    files_copied: u32,
    recursive: bool,
    copy_empty_dirs: bool,
    prompt_each: bool,
    dir_stack: Vec<(ReadDirectory, u16)>,
}

state_struct_codec!(XcopyState {
    src_drive,
    src_directory,
    src_pattern,
    src_search_index,
    dst_drive,
    dst_dir_cluster,
    files_copied,
    recursive,
    copy_empty_dirs,
    prompt_each,
    dir_stack,
});

#[derive(Clone)]
enum SourceFileCursor {
    Fat(FatFileCursor),
    Iso {
        entry: iso9660::IsoDirEntry,
        position: u32,
    },
}

impl save_state::StateEncode for SourceFileCursor {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Fat(cursor) => {
                save_state::StateEncode::encode_state(&0u8, output);
                save_state::StateEncode::encode_state(cursor, output);
            }
            Self::Iso { entry, position } => {
                save_state::StateEncode::encode_state(&1u8, output);
                save_state::StateEncode::encode_state(entry, output);
                save_state::StateEncode::encode_state(position, output);
            }
        }
    }
}

impl save_state::StateDecode for SourceFileCursor {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Fat(save_state::StateDecode::decode_state(decoder)?)),
            1 => Ok(Self::Iso {
                entry: save_state::StateDecode::decode_state(decoder)?,
                position: save_state::StateDecode::decode_state(decoder)?,
            }),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

#[derive(Clone)]
/// Authoritative progress of one XCOPY file transfer.
struct FileCopyState {
    src_drive: u8,
    src_cursor: SourceFileCursor,
    src_entry: ReadDirEntry,
    dst_file: PendingFatFile,
}

state_struct_codec!(FileCopyState {
    src_drive,
    src_cursor,
    src_entry,
    dst_file,
});

#[derive(Clone)]
enum XcopyPhase {
    Init,
    FindNext(XcopyState),
    PromptFile(XcopyState, ReadDirEntry),
    ReadChunk(XcopyState, FileCopyState),
    WriteChunk(XcopyState, FileCopyState, Vec<u8>),
    FinishFile(XcopyState, FileCopyState),
    ScanSubdirs(XcopyState),
    Summary(u32),
}

impl save_state::StateEncode for XcopyPhase {
    fn encode_state(&self, output: &mut Vec<u8>) {
        match self {
            Self::Init => save_state::StateEncode::encode_state(&0u8, output),
            Self::FindNext(state) => encode_phase(1, state, output),
            Self::PromptFile(state, entry) => {
                encode_phase(2, state, output);
                save_state::StateEncode::encode_state(entry, output);
            }
            Self::ReadChunk(state, file) => {
                encode_phase(3, state, output);
                save_state::StateEncode::encode_state(file, output);
            }
            Self::WriteChunk(state, file, data) => {
                encode_phase(4, state, output);
                save_state::StateEncode::encode_state(file, output);
                save_state::StateEncode::encode_state(data, output);
            }
            Self::FinishFile(state, file) => {
                encode_phase(5, state, output);
                save_state::StateEncode::encode_state(file, output);
            }
            Self::ScanSubdirs(state) => encode_phase(6, state, output),
            Self::Summary(count) => {
                save_state::StateEncode::encode_state(&7u8, output);
                save_state::StateEncode::encode_state(count, output);
            }
        }
    }
}

fn encode_phase(tag: u8, state: &XcopyState, output: &mut Vec<u8>) {
    save_state::StateEncode::encode_state(&tag, output);
    save_state::StateEncode::encode_state(state, output);
}

impl save_state::StateDecode for XcopyPhase {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Init),
            1 => Ok(Self::FindNext(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            2 => Ok(Self::PromptFile(
                save_state::StateDecode::decode_state(decoder)?,
                save_state::StateDecode::decode_state(decoder)?,
            )),
            3 => Ok(Self::ReadChunk(
                save_state::StateDecode::decode_state(decoder)?,
                save_state::StateDecode::decode_state(decoder)?,
            )),
            4 => Ok(Self::WriteChunk(
                save_state::StateDecode::decode_state(decoder)?,
                save_state::StateDecode::decode_state(decoder)?,
                save_state::StateDecode::decode_state(decoder)?,
            )),
            5 => Ok(Self::FinishFile(
                save_state::StateDecode::decode_state(decoder)?,
                save_state::StateDecode::decode_state(decoder)?,
            )),
            6 => Ok(Self::ScanSubdirs(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            7 => Ok(Self::Summary(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

#[derive(Clone)]
/// Serializable state of an executing XCOPY command.
pub(crate) struct RunningXcopy {
    args: Vec<u8>,
    phase: XcopyPhase,
}

state_struct_codec!(RunningXcopy { args, phase });

impl RunningXcopy {
    fn step_init(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if is_help_request(&self.args) || self.args.trim_ascii().is_empty() {
            print_help(io);
            return StepResult::Done(0);
        }
        match init_xcopy(state, io, drive, &self.args) {
            Ok(xcopy_state) => {
                self.phase = XcopyPhase::FindNext(xcopy_state);
                StepResult::Continue
            }
            Err(msg) => {
                io.print(msg);
                StepResult::Done(1)
            }
        }
    }

    fn step_find_next(
        &mut self,
        mut xcopy_state: XcopyState,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let result = filesystem::find_matching_read_entry(
            state,
            xcopy_state.src_drive,
            &xcopy_state.src_directory,
            &xcopy_state.src_pattern,
            0,
            xcopy_state.src_search_index,
            drive,
        );

        match result {
            Ok(Some((entry, next_index))) => {
                if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 {
                    xcopy_state.src_search_index = next_index;
                    self.phase = XcopyPhase::FindNext(xcopy_state);
                    return StepResult::Continue;
                }

                xcopy_state.src_search_index = next_index;

                if xcopy_state.prompt_each {
                    let display_name = fat_dir::fcb_to_display_name(&entry.name);
                    for &byte in &display_name {
                        io.output_byte(byte);
                    }
                    io.print(b" (Y/N)?");
                    self.phase = XcopyPhase::PromptFile(xcopy_state, entry);
                } else {
                    let display_name = fat_dir::fcb_to_display_name(&entry.name);
                    for &byte in &display_name {
                        io.output_byte(byte);
                    }
                    io.println(b"");

                    self.start_file_copy(&mut xcopy_state, entry);
                }
                StepResult::Continue
            }
            Ok(None) => {
                if xcopy_state.recursive {
                    self.phase = XcopyPhase::ScanSubdirs(xcopy_state);
                } else {
                    self.phase = XcopyPhase::Summary(xcopy_state.files_copied);
                }
                StepResult::Continue
            }
            Err(_) => {
                io.println(b"File not found");
                StepResult::Done(1)
            }
        }
    }

    fn step_prompt_file(
        &mut self,
        mut xcopy_state: XcopyState,
        entry: ReadDirEntry,
        io: &mut IoAccess,
    ) -> StepResult {
        if io.memory.read_byte(KB_BUF_COUNT) == 0 {
            self.phase = XcopyPhase::PromptFile(xcopy_state, entry);
            return StepResult::Continue;
        }
        let key = consume_key(io);
        io.output_byte(b'\r');
        io.output_byte(b'\n');

        if key == b'Y' || key == b'y' {
            self.start_file_copy(&mut xcopy_state, entry);
        } else {
            self.phase = XcopyPhase::FindNext(xcopy_state);
        }
        StepResult::Continue
    }

    fn step_read_chunk(
        &mut self,
        xcopy_state: XcopyState,
        mut file_state: FileCopyState,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let dst_cluster_size =
            match state.fat_volumes[file_state.dst_file.drive_index as usize].as_ref() {
                Some(v) => v.bpb.cluster_size() as usize,
                None => return StepResult::Done(1),
            };
        let write_data = match &mut file_state.src_cursor {
            SourceFileCursor::Fat(src_cursor) => {
                if src_cursor.remaining() == 0 {
                    self.phase = XcopyPhase::FinishFile(xcopy_state, file_state);
                    return StepResult::Continue;
                }

                let src_vol = match state.fat_volumes[file_state.src_drive as usize].as_ref() {
                    Some(v) => v,
                    None => return StepResult::Done(1),
                };
                match src_cursor.read_chunk(src_vol, drive, dst_cluster_size) {
                    Ok(data) => data,
                    Err(_) => {
                        io.println(b"Read error");
                        return StepResult::Done(1);
                    }
                }
            }
            SourceFileCursor::Iso { entry, position } => {
                if *position >= entry.file_size {
                    self.phase = XcopyPhase::FinishFile(xcopy_state, file_state);
                    return StepResult::Continue;
                }

                match iso9660::read_file_chunk(entry, *position, dst_cluster_size, drive) {
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
        };

        if write_data.is_empty() {
            self.phase = XcopyPhase::FinishFile(xcopy_state, file_state);
            return StepResult::Continue;
        }

        self.phase = XcopyPhase::WriteChunk(xcopy_state, file_state, write_data);
        StepResult::Continue
    }

    fn step_write_chunk(
        &mut self,
        xcopy_state: XcopyState,
        mut file_state: FileCopyState,
        data: Vec<u8>,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        let (dst_file, _) =
            match filesystem::write_pending_file_chunk(state, disk, file_state.dst_file, &data) {
                Ok(result) => result,
                Err(0x001F) => {
                    io.println(b"Insufficient disk space");
                    return StepResult::Done(1);
                }
                Err(_) => {
                    io.println(b"Write error");
                    return StepResult::Done(1);
                }
            };

        file_state.dst_file = dst_file;
        self.phase = XcopyPhase::ReadChunk(xcopy_state, file_state);
        StepResult::Continue
    }

    fn step_finish_file(
        &mut self,
        mut xcopy_state: XcopyState,
        file_state: FileCopyState,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        if filesystem::finish_pending_file(state, disk, file_state.dst_file).is_err() {
            io.println(b"Unable to create destination");
            return StepResult::Done(1);
        }

        xcopy_state.files_copied += 1;

        self.phase = XcopyPhase::FindNext(xcopy_state);
        StepResult::Continue
    }

    fn step_scan_subdirs(
        &mut self,
        mut xcopy_state: XcopyState,
        state: &mut DosState,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let all_pattern = [b'?'; 11];
        let attr_mask = fat_dir::ATTR_HIDDEN | fat_dir::ATTR_SYSTEM | fat_dir::ATTR_DIRECTORY;
        let mut si = 0u16;
        let mut subdirs = Vec::new();

        loop {
            let result = filesystem::find_matching_read_entry(
                state,
                xcopy_state.src_drive,
                &xcopy_state.src_directory,
                &all_pattern,
                attr_mask,
                si,
                drive,
            );
            match result {
                Ok(Some((entry, next_index))) => {
                    if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 && !is_dot_directory(&entry) {
                        subdirs.push(entry);
                    }
                    si = next_index;
                }
                _ => break,
            }
        }

        let timestamp = state.dos_timestamp_now();

        for subdir in subdirs {
            let src_subdir_directory = match source_directory_from_entry(&subdir) {
                Some(directory) => directory,
                None => continue,
            };
            let dst_subdir_cluster = match filesystem::ensure_directory(
                state,
                drive,
                xcopy_state.dst_drive,
                xcopy_state.dst_dir_cluster,
                subdir.name,
                Some(timestamp),
            ) {
                Ok(cluster) => cluster,
                Err(_) => continue,
            };

            if !xcopy_state.copy_empty_dirs
                && !directory_has_entries(
                    state,
                    xcopy_state.src_drive,
                    &src_subdir_directory,
                    drive,
                )
                .unwrap_or(false)
            {
                continue;
            }

            xcopy_state
                .dir_stack
                .push((src_subdir_directory, dst_subdir_cluster));
        }

        if let Some((src_directory, dst_cluster)) = xcopy_state.dir_stack.pop() {
            xcopy_state.src_directory = src_directory;
            xcopy_state.dst_dir_cluster = dst_cluster;
            xcopy_state.src_search_index = 0;
            self.phase = XcopyPhase::FindNext(xcopy_state);
        } else {
            self.phase = XcopyPhase::Summary(xcopy_state.files_copied);
        }
        StepResult::Continue
    }
}

impl RunningCommand for RunningXcopy {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let phase = std::mem::replace(&mut self.phase, XcopyPhase::Init);
        match phase {
            XcopyPhase::Init => self.step_init(state, io, drive),
            XcopyPhase::FindNext(xs) => self.step_find_next(xs, state, io, drive),
            XcopyPhase::PromptFile(xs, entry) => self.step_prompt_file(xs, entry, io),
            XcopyPhase::ReadChunk(xs, fs) => self.step_read_chunk(xs, fs, state, io, drive),
            XcopyPhase::WriteChunk(xs, fs, data) => {
                self.step_write_chunk(xs, fs, data, state, io, drive)
            }
            XcopyPhase::FinishFile(xs, fs) => self.step_finish_file(xs, fs, state, io, drive),
            XcopyPhase::ScanSubdirs(xs) => self.step_scan_subdirs(xs, state, drive),
            XcopyPhase::Summary(count) => {
                let msg = format!("{} File(s) copied\r\n", count);
                io.print(msg.as_bytes());
                StepResult::Done(0)
            }
        }
    }
}

impl RunningXcopy {
    fn start_file_copy(&mut self, xcopy_state: &mut XcopyState, entry: ReadDirEntry) {
        let src_cursor = match &entry.source {
            ReadDirEntrySource::Fat(entry) => SourceFileCursor::Fat(FatFileCursor::new(entry)),
            ReadDirEntrySource::Iso(entry) => SourceFileCursor::Iso {
                entry: entry.clone(),
                position: 0,
            },
        };
        let file_state = FileCopyState {
            src_drive: xcopy_state.src_drive,
            src_cursor,
            dst_file: PendingFatFile {
                drive_index: xcopy_state.dst_drive,
                dir_cluster: xcopy_state.dst_dir_cluster,
                name: entry.name,
                attribute: destination_file_attributes(&entry),
                time: entry.time,
                date: entry.date,
                start_cluster: 0,
                file_size: 0,
                position: 0,
            },
            src_entry: entry,
        };

        // Take xcopy_state by swapping
        let taken = std::mem::replace(
            xcopy_state,
            XcopyState {
                src_drive: 0,
                src_directory: ReadDirectory::Fat(0),
                src_pattern: [0; 11],
                src_search_index: 0,
                dst_drive: 0,
                dst_dir_cluster: 0,
                files_copied: 0,
                recursive: false,
                copy_empty_dirs: false,
                prompt_each: false,
                dir_stack: Vec::new(),
            },
        );

        if file_state.src_entry.file_size == 0 {
            self.phase = XcopyPhase::FinishFile(taken, file_state);
        } else {
            self.phase = XcopyPhase::ReadChunk(taken, file_state);
        }
    }
}

fn destination_file_attributes(entry: &ReadDirEntry) -> u8 {
    match &entry.source {
        ReadDirEntrySource::Fat(_) => entry.attribute & 0x27,
        ReadDirEntrySource::Iso(_) => fat_dir::ATTR_ARCHIVE,
    }
}

fn source_directory_from_entry(entry: &ReadDirEntry) -> Option<ReadDirectory> {
    match &entry.source {
        ReadDirEntrySource::Fat(entry) => {
            (entry.start_cluster >= 2).then_some(ReadDirectory::Fat(entry.start_cluster))
        }
        ReadDirEntrySource::Iso(entry) => entry.directory.clone().map(ReadDirectory::Iso),
    }
}

fn is_dot_directory(entry: &ReadDirEntry) -> bool {
    let display_name = fat_dir::fcb_to_display_name(&entry.name);
    display_name.is_empty() || display_name == b"." || display_name == b".."
}

fn directory_has_entries(
    state: &DosState,
    drive_index: u8,
    directory: &ReadDirectory,
    drive: &mut dyn DriveIo,
) -> Result<bool, u16> {
    let all_pattern = [b'?'; 11];
    let attr_mask = fat_dir::ATTR_HIDDEN | fat_dir::ATTR_SYSTEM | fat_dir::ATTR_DIRECTORY;
    let mut search_index = 0u16;

    loop {
        match filesystem::find_matching_read_entry(
            state,
            drive_index,
            directory,
            &all_pattern,
            attr_mask,
            search_index,
            drive,
        )? {
            Some((entry, next_index)) => {
                search_index = next_index;
                if !is_dot_directory(&entry) {
                    return Ok(true);
                }
            }
            None => return Ok(false),
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Copies files and directory trees.");
    io.println(b"");
    io.println(b"XCOPY source destination [/S] [/E] [/P]");
    io.println(b"");
    io.println(b"  source       Specifies the file(s) to copy.");
    io.println(b"  destination  Specifies the location of the new files.");
    io.println(b"  /S           Copies directories and subdirectories except");
    io.println(b"               empty ones.");
    io.println(b"  /E           Copies directories and subdirectories, including");
    io.println(b"               empty ones. Same as /S /E.");
    io.println(b"  /P           Prompts before copying each file.");
}

fn init_xcopy(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    args: &[u8],
) -> Result<XcopyState, &'static [u8]> {
    let args = args.trim_ascii();
    if args.is_empty() {
        return Err(b"Required parameter missing\r\n");
    }

    let mut recursive = false;
    let mut copy_empty_dirs = false;
    let mut prompt_each = false;
    let mut parts: Vec<&[u8]> = Vec::new();

    for part in args.split(|&b| b == b' ' || b == b'\t') {
        if part.is_empty() {
            continue;
        }
        if part.len() >= 2 && part[0] == b'/' {
            match part[1].to_ascii_uppercase() {
                b'S' => recursive = true,
                b'E' => {
                    copy_empty_dirs = true;
                    recursive = true; // /E implies /S
                }
                b'P' => prompt_each = true,
                _ => {}
            }
        } else {
            parts.push(part);
        }
    }

    if parts.len() < 2 {
        return Err(b"Required parameter missing\r\n");
    }

    let source = parts[0];
    let dest = parts[1];

    let has_wildcard = source.contains(&b'*') || source.contains(&b'?');
    let (src_drive, src_directory, src_pattern) = if has_wildcard {
        let path = crate::filesystem::resolve_read_file_path(state, source, io.memory, drive)
            .map_err(|_| &b"File not found\r\n"[..])?;
        (path.drive_index, path.directory, path.name)
    } else {
        match crate::filesystem::resolve_read_dir_path(state, source, io.memory, drive) {
            Ok(path) => (path.drive_index, path.directory, [b'?'; 11]),
            Err(_) => {
                let path =
                    crate::filesystem::resolve_read_file_path(state, source, io.memory, drive)
                        .map_err(|_| &b"File not found\r\n"[..])?;
                (path.drive_index, path.directory, path.name)
            }
        }
    };

    let (dst_drive, dst_dir_cluster) =
        crate::filesystem::resolve_dir_path(state, dest, io.memory, drive)
            .map_err(|_| &b"Invalid destination\r\n"[..])?;

    if dst_drive == 25 {
        return Err(b"Access denied\r\n");
    }

    Ok(XcopyState {
        src_drive,
        src_directory,
        src_pattern,
        src_search_index: 0,
        dst_drive,
        dst_dir_cluster,
        files_copied: 0,
        recursive,
        copy_empty_dirs,
        prompt_each,
        dir_stack: Vec::new(),
    })
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
