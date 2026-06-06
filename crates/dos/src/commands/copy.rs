//! COPY command. Supports DOS-to-DOS copies (single files with wildcards,
//! concatenation, and recursive directory trees) and copies that cross the
//! emulator boundary via a `host:` prefix on either source or destination.

use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

use crate::{
    DiskIo, DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    copy_common::{dos_leaf_basename, validate_dos_basename, validate_dos_components},
    filesystem::{
        self, PendingFatFile, ReadDirEntry, ReadDirEntrySource, ReadDirectory, fat_dir,
        fat_file::FatFileCursor, iso9660,
    },
};

pub(crate) struct Copy;

impl Command for Copy {
    fn name(&self) -> &'static str {
        "COPY"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningCopy {
            args: args.to_vec(),
            phase: CopyPhase::Init,
        })
    }
}

const KB_BUF_COUNT: u32 = 0x0528;
const HOST_CHUNK_BYTES: usize = 4096;

enum ArgKind {
    Dos(Vec<u8>),
    Host(PathBuf),
}

fn parse_arg(token: &[u8]) -> ArgKind {
    if token.len() >= 5 && token[..5].eq_ignore_ascii_case(b"host:") {
        let rest = String::from_utf8_lossy(&token[5..]).into_owned();
        ArgKind::Host(PathBuf::from(rest))
    } else {
        ArgKind::Dos(token.to_vec())
    }
}

struct DosPatternSpec {
    drive: u8,
    directory: ReadDirectory,
    pattern: [u8; 11],
}

enum WorkItem {
    EnumerateDos(EnumerateDos),
    EnumerateHost(EnumerateHost),
    File(FileJob),
}

struct EnumerateDos {
    src_drive: u8,
    src_directory: ReadDirectory,
    dst: DirHandle,
    search_index: u16,
}

struct EnumerateHost {
    src_path: PathBuf,
    dst: DirHandle,
}

#[derive(Clone)]
enum DirHandle {
    Dos { drive: u8, dir_cluster: u16 },
    Host { path: PathBuf },
}

struct FileJob {
    display_name: Vec<u8>,
    src: FileSource,
    dst: FileDest,
}

enum FileSource {
    Dos { drive: u8, entry: ReadDirEntry },
    Host { path: PathBuf },
}

enum FileDest {
    Dos {
        drive: u8,
        dir_cluster: u16,
        fcb_name: [u8; 11],
        attribute: u8,
        time: u16,
        date: u16,
    },
    Host {
        path: PathBuf,
    },
}

struct CopyState {
    work_stack: Vec<WorkItem>,
    // Used for the wildcard/concat path (no recursion involved).
    wildcard: Option<WildcardSpec>,
    concat_remaining: Option<Vec<DosPatternSpec>>,
    files_copied: u32,
    verify: bool,
    overwrite_all: bool,
}

struct WildcardSpec {
    spec: DosPatternSpec,
    search_index: u16,
    dst: WildcardDest,
}

enum WildcardDest {
    /// Destination resolved to an existing directory; each match uses its own name inside it.
    DosDir {
        drive: u8,
        dir_cluster: u16,
    },
    HostDir {
        path: PathBuf,
    },
    /// Destination is a single named file (only legal when the wildcard matches at most one file).
    DosFile {
        drive: u8,
        dir_cluster: u16,
        fcb_name: [u8; 11],
    },
    HostFile {
        path: PathBuf,
    },
}

enum ResolvedDosDestination {
    Dir {
        drive: u8,
        dir_cluster: u16,
    },
    File {
        drive: u8,
        dir_cluster: u16,
        fcb_name: [u8; 11],
    },
}

struct ActiveTransfer {
    display_name: Vec<u8>,
    src: TransferSrc,
    dst: TransferDst,
}

enum TransferSrc {
    Dos { drive: u8, cursor: SourceFileCursor },
    Host { file: fs::File },
}

enum SourceFileCursor {
    Fat(FatFileCursor),
    Iso {
        entry: iso9660::IsoDirEntry,
        position: u32,
    },
}

enum TransferDst {
    Dos(PendingFatFile),
    Host(fs::File),
}

enum CopyPhase {
    Init,
    NextItem(CopyState),
    EnumerateDosStep(CopyState, EnumerateDos),
    ConfirmOverwrite(CopyState, FileJob),
    ReadChunk(CopyState, ActiveTransfer),
    WriteChunk(CopyState, ActiveTransfer, Vec<u8>),
    VerifyChunk(CopyState, ActiveTransfer, u16, Vec<u8>),
    FinishFile(CopyState, ActiveTransfer),
    ConcatNextSource(CopyState, ActiveTransfer),
    ConcatRead(CopyState, ActiveTransfer),
    ConcatWrite(CopyState, ActiveTransfer, Vec<u8>),
    Summary(u32),
}

struct RunningCopy {
    args: Vec<u8>,
    phase: CopyPhase,
}

impl RunningCommand for RunningCopy {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let phase = std::mem::replace(&mut self.phase, CopyPhase::Init);
        match phase {
            CopyPhase::Init => self.step_init(state, io, drive),
            CopyPhase::NextItem(cs) => self.step_next_item(cs, state, io, drive),
            CopyPhase::EnumerateDosStep(cs, es) => {
                self.step_enumerate_dos(cs, es, state, io, drive)
            }
            CopyPhase::ConfirmOverwrite(cs, fj) => self.step_confirm_overwrite(cs, fj, io),
            CopyPhase::ReadChunk(cs, at) => self.step_read_chunk(cs, at, state, io, drive),
            CopyPhase::WriteChunk(cs, at, data) => {
                self.step_write_chunk(cs, at, data, state, io, drive)
            }
            CopyPhase::VerifyChunk(cs, at, cluster, data) => {
                self.step_verify_chunk(cs, at, cluster, data, state, io, drive)
            }
            CopyPhase::FinishFile(cs, at) => self.step_finish_file(cs, at, state, io, drive),
            CopyPhase::ConcatNextSource(cs, at) => {
                self.step_concat_next_source(cs, at, state, io, drive)
            }
            CopyPhase::ConcatRead(cs, at) => self.step_concat_read(cs, at, state, io, drive),
            CopyPhase::ConcatWrite(cs, at, data) => {
                self.step_concat_write(cs, at, data, state, io, drive)
            }
            CopyPhase::Summary(count) => {
                let msg = format!("     {count:>4} file(s) copied\r\n");
                io.print(msg.as_bytes());
                StepResult::Done(0)
            }
        }
    }
}

impl RunningCopy {
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
        match init_copy(state, io, drive, &self.args) {
            Ok(InitOutcome::Plan(cs)) => {
                self.phase = CopyPhase::NextItem(cs);
                StepResult::Continue
            }
            Ok(InitOutcome::Concat { cs, transfer }) => {
                self.phase = CopyPhase::ConcatRead(cs, transfer);
                StepResult::Continue
            }
            Err(msg) => {
                io.print(msg);
                StepResult::Done(1)
            }
        }
    }

    fn step_next_item(
        &mut self,
        mut cs: CopyState,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if let Some(wildcard) = cs.wildcard.as_mut() {
            let spec = &wildcard.spec;
            let result = filesystem::find_matching_read_entry(
                state,
                spec.drive,
                &spec.directory,
                &spec.pattern,
                0,
                wildcard.search_index,
                drive,
            );
            match result {
                Ok(Some((entry, next_index))) => {
                    if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 {
                        wildcard.search_index = next_index;
                        self.phase = CopyPhase::NextItem(cs);
                        return StepResult::Continue;
                    }
                    wildcard.search_index = next_index;
                    let job = match build_wildcard_file_job(&entry, spec.drive, &wildcard.dst) {
                        Ok(job) => job,
                        Err(msg) => {
                            io.print(msg);
                            return StepResult::Done(1);
                        }
                    };
                    return self.begin_file_job(cs, job, state, io, drive);
                }
                Ok(None) => {
                    cs.wildcard = None;
                }
                Err(_) => {
                    io.println(b"File not found");
                    return StepResult::Done(1);
                }
            }
        }

        let Some(item) = cs.work_stack.pop() else {
            if cs.files_copied == 0 {
                io.println(b"File not found");
                return StepResult::Done(1);
            }
            self.phase = CopyPhase::Summary(cs.files_copied);
            return StepResult::Continue;
        };

        match item {
            WorkItem::File(job) => self.begin_file_job(cs, job, state, io, drive),
            WorkItem::EnumerateDos(es) => {
                self.phase = CopyPhase::EnumerateDosStep(cs, es);
                StepResult::Continue
            }
            WorkItem::EnumerateHost(eh) => self.enumerate_host(cs, eh, state, io, drive),
        }
    }

    fn step_enumerate_dos(
        &mut self,
        mut cs: CopyState,
        mut es: EnumerateDos,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let all_pattern = [b'?'; 11];
        let attr_mask = fat_dir::ATTR_HIDDEN | fat_dir::ATTR_SYSTEM | fat_dir::ATTR_DIRECTORY;
        match filesystem::find_matching_read_entry(
            state,
            es.src_drive,
            &es.src_directory,
            &all_pattern,
            attr_mask,
            es.search_index,
            drive,
        ) {
            Ok(Some((entry, next_index))) => {
                es.search_index = next_index;
                if is_dot_directory(&entry) {
                    self.phase = CopyPhase::EnumerateDosStep(cs, es);
                    return StepResult::Continue;
                }
                if entry.attribute & fat_dir::ATTR_DIRECTORY != 0 {
                    let subdir_dir = match source_directory_from_entry(&entry) {
                        Some(d) => d,
                        None => {
                            self.phase = CopyPhase::EnumerateDosStep(cs, es);
                            return StepResult::Continue;
                        }
                    };
                    let child_dst = match make_dest_subdir(state, drive, &es.dst, &entry.name) {
                        Ok(handle) => handle,
                        Err(msg) => {
                            io.print(msg);
                            return StepResult::Done(1);
                        }
                    };
                    cs.work_stack.push(WorkItem::EnumerateDos(EnumerateDos {
                        src_drive: es.src_drive,
                        src_directory: subdir_dir,
                        dst: child_dst,
                        search_index: 0,
                    }));
                    self.phase = CopyPhase::EnumerateDosStep(cs, es);
                    return StepResult::Continue;
                }

                let job = match build_file_job_dos_to_dest(&entry, es.src_drive, &es.dst) {
                    Ok(job) => job,
                    Err(msg) => {
                        io.print(msg);
                        return StepResult::Done(1);
                    }
                };
                cs.work_stack.push(WorkItem::EnumerateDos(es));
                self.begin_file_job(cs, job, state, io, drive)
            }
            Ok(None) => {
                self.phase = CopyPhase::NextItem(cs);
                StepResult::Continue
            }
            Err(_) => {
                io.println(b"Read error");
                StepResult::Done(1)
            }
        }
    }

    fn enumerate_host(
        &mut self,
        mut cs: CopyState,
        eh: EnumerateHost,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let entries = match fs::read_dir(&eh.src_path) {
            Ok(entries) => entries,
            Err(_) => {
                io.println(b"Read error");
                return StepResult::Done(1);
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let basename = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_owned(),
                None => continue,
            };
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                let child_dst = if let DirHandle::Dos { .. } = &eh.dst {
                    let dos_name = match validate_dos_basename(basename.as_bytes()) {
                        Ok(name) => name,
                        Err(_) => {
                            io.print(b"Long host filename cannot be represented as 8.3\r\n");
                            return StepResult::Done(1);
                        }
                    };
                    let fcb = fat_dir::name_to_fcb(&dos_name);
                    match make_dest_subdir(state, drive, &eh.dst, &fcb) {
                        Ok(handle) => handle,
                        Err(msg) => {
                            io.print(msg);
                            return StepResult::Done(1);
                        }
                    }
                } else {
                    let host_dst = match &eh.dst {
                        DirHandle::Host { path } => path.join(&basename),
                        DirHandle::Dos { .. } => unreachable!(),
                    };
                    if let Err(e) = fs::create_dir_all(&host_dst) {
                        io.print(format!("Cannot create host dir: {e}\r\n").as_bytes());
                        return StepResult::Done(1);
                    }
                    DirHandle::Host { path: host_dst }
                };
                cs.work_stack.push(WorkItem::EnumerateHost(EnumerateHost {
                    src_path: path,
                    dst: child_dst,
                }));
            } else if metadata.is_file() {
                let dst = match &eh.dst {
                    DirHandle::Dos { drive, dir_cluster } => {
                        let dos_name = match validate_dos_basename(basename.as_bytes()) {
                            Ok(name) => name,
                            Err(_) => {
                                io.print(b"Long host filename cannot be represented as 8.3\r\n");
                                return StepResult::Done(1);
                            }
                        };
                        let fcb = fat_dir::name_to_fcb(&dos_name);
                        let (time, date) = state.dos_timestamp_now();
                        FileDest::Dos {
                            drive: *drive,
                            dir_cluster: *dir_cluster,
                            fcb_name: fcb,
                            attribute: fat_dir::ATTR_ARCHIVE,
                            time,
                            date,
                        }
                    }
                    DirHandle::Host { path } => FileDest::Host {
                        path: path.join(&basename),
                    },
                };
                let display =
                    fat_dir::fcb_to_display_name(&fat_dir::name_to_fcb(basename.as_bytes()));
                cs.work_stack.push(WorkItem::File(FileJob {
                    display_name: display,
                    src: FileSource::Host { path },
                    dst,
                }));
            }
        }

        self.phase = CopyPhase::NextItem(cs);
        StepResult::Continue
    }

    fn begin_file_job(
        &mut self,
        cs: CopyState,
        job: FileJob,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let exists = match destination_exists(state, drive, &job.dst) {
            Ok(exists) => exists,
            Err(msg) => {
                io.print(msg);
                return StepResult::Done(1);
            }
        };
        if exists && !cs.overwrite_all {
            io.print(b"Overwrite (Yes/No/All)?");
            self.phase = CopyPhase::ConfirmOverwrite(cs, job);
            return StepResult::Continue;
        }
        self.start_transfer(cs, job, io)
    }

    fn start_transfer(&mut self, cs: CopyState, job: FileJob, io: &mut IoAccess) -> StepResult {
        for &byte in &job.display_name {
            io.output_byte(byte);
        }
        io.println(b"");

        let transfer = match build_transfer(job) {
            Ok(t) => t,
            Err(msg) => {
                io.print(msg);
                return StepResult::Done(1);
            }
        };

        if transfer_is_empty(&transfer) {
            self.phase = CopyPhase::FinishFile(cs, transfer);
        } else {
            self.phase = CopyPhase::ReadChunk(cs, transfer);
        }
        StepResult::Continue
    }

    fn step_confirm_overwrite(
        &mut self,
        mut cs: CopyState,
        job: FileJob,
        io: &mut IoAccess,
    ) -> StepResult {
        if io.memory.read_byte(KB_BUF_COUNT) == 0 {
            self.phase = CopyPhase::ConfirmOverwrite(cs, job);
            return StepResult::Continue;
        }
        let key = consume_key(io);
        io.output_byte(b'\r');
        io.output_byte(b'\n');

        match key.to_ascii_uppercase() {
            b'Y' => self.start_transfer(cs, job, io),
            b'A' => {
                cs.overwrite_all = true;
                self.start_transfer(cs, job, io)
            }
            _ => {
                self.phase = CopyPhase::NextItem(cs);
                StepResult::Continue
            }
        }
    }

    fn step_read_chunk(
        &mut self,
        cs: CopyState,
        mut transfer: ActiveTransfer,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let cluster_size = dst_cluster_size(state, &transfer.dst);
        let chunk_size = cluster_size.unwrap_or(HOST_CHUNK_BYTES);

        match read_source(state, drive, &mut transfer.src, chunk_size) {
            Ok(Some(data)) => {
                if data.is_empty() {
                    self.phase = CopyPhase::FinishFile(cs, transfer);
                } else {
                    self.phase = CopyPhase::WriteChunk(cs, transfer, data);
                }
                StepResult::Continue
            }
            Ok(None) => {
                self.phase = CopyPhase::FinishFile(cs, transfer);
                StepResult::Continue
            }
            Err(()) => {
                io.println(b"Read error");
                StepResult::Done(1)
            }
        }
    }

    fn step_write_chunk(
        &mut self,
        cs: CopyState,
        transfer: ActiveTransfer,
        data: Vec<u8>,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        let ActiveTransfer {
            display_name,
            src,
            dst,
        } = transfer;
        match dst {
            TransferDst::Dos(pending) => {
                match filesystem::write_pending_file_chunk(state, disk, pending, &data) {
                    Ok((updated, new_cluster)) => {
                        let rebuilt = ActiveTransfer {
                            display_name,
                            src,
                            dst: TransferDst::Dos(updated),
                        };
                        if cs.verify {
                            self.phase = CopyPhase::VerifyChunk(cs, rebuilt, new_cluster, data);
                        } else {
                            self.phase = CopyPhase::ReadChunk(cs, rebuilt);
                        }
                        StepResult::Continue
                    }
                    Err(0x001F) => {
                        io.println(b"Insufficient disk space");
                        StepResult::Done(1)
                    }
                    Err(_) => {
                        io.println(b"Write error");
                        StepResult::Done(1)
                    }
                }
            }
            TransferDst::Host(mut file) => {
                if file.write_all(&data).is_err() {
                    io.println(b"Write error");
                    return StepResult::Done(1);
                }
                let rebuilt = ActiveTransfer {
                    display_name,
                    src,
                    dst: TransferDst::Host(file),
                };
                self.phase = CopyPhase::ReadChunk(cs, rebuilt);
                StepResult::Continue
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step_verify_chunk(
        &mut self,
        cs: CopyState,
        transfer: ActiveTransfer,
        written_cluster: u16,
        original_data: Vec<u8>,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        let drive_index = match &transfer.dst {
            TransferDst::Dos(pending) => pending.drive_index,
            TransferDst::Host(_) => {
                self.phase = CopyPhase::ReadChunk(cs, transfer);
                return StepResult::Continue;
            }
        };
        let vol = match state.fat_volumes[drive_index as usize].as_ref() {
            Some(v) => v,
            None => return StepResult::Done(1),
        };
        let readback = match vol.read_cluster(written_cluster, disk) {
            Ok(d) => d,
            Err(_) => {
                io.println(b"Verify error");
                return StepResult::Done(1);
            }
        };
        let compare_len = original_data.len();
        if readback[..compare_len] != original_data[..compare_len] {
            io.println(b"Verify error");
            return StepResult::Done(1);
        }
        self.phase = CopyPhase::ReadChunk(cs, transfer);
        StepResult::Continue
    }

    fn step_finish_file(
        &mut self,
        mut cs: CopyState,
        transfer: ActiveTransfer,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        match transfer.dst {
            TransferDst::Dos(pending) => {
                if filesystem::finish_pending_file(state, disk, pending).is_err() {
                    io.println(b"Unable to create destination");
                    return StepResult::Done(1);
                }
            }
            TransferDst::Host(mut file) => {
                if file.flush().is_err() {
                    io.println(b"Write error");
                    return StepResult::Done(1);
                }
            }
        }
        cs.files_copied += 1;
        self.phase = CopyPhase::NextItem(cs);
        StepResult::Continue
    }

    fn step_concat_next_source(
        &mut self,
        mut cs: CopyState,
        transfer: ActiveTransfer,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let remaining = match cs.concat_remaining.as_mut() {
            Some(v) => v,
            None => {
                self.phase = CopyPhase::FinishFile(cs, transfer);
                return StepResult::Continue;
            }
        };
        if remaining.is_empty() {
            self.phase = CopyPhase::FinishFile(cs, transfer);
            return StepResult::Continue;
        }
        let spec = remaining.remove(0);
        let entry = match filesystem::find_matching_read_entry(
            state,
            spec.drive,
            &spec.directory,
            &spec.pattern,
            0,
            0,
            drive,
        ) {
            Ok(Some((e, _))) => e,
            _ => {
                self.phase = CopyPhase::ConcatNextSource(cs, transfer);
                return StepResult::Continue;
            }
        };

        let display_name = fat_dir::fcb_to_display_name(&entry.name);
        for &byte in &display_name {
            io.output_byte(byte);
        }
        io.println(b"");

        let new_cursor = match &entry.source {
            ReadDirEntrySource::Fat(fat_entry) => {
                SourceFileCursor::Fat(FatFileCursor::new(fat_entry))
            }
            ReadDirEntrySource::Iso(iso_entry) => SourceFileCursor::Iso {
                entry: iso_entry.clone(),
                position: 0,
            },
        };
        let new_src = TransferSrc::Dos {
            drive: spec.drive,
            cursor: new_cursor,
        };
        let new_transfer = ActiveTransfer {
            display_name: transfer.display_name,
            src: new_src,
            dst: transfer.dst,
        };
        let file_size = entry.file_size;
        if file_size == 0 {
            self.phase = CopyPhase::ConcatNextSource(cs, new_transfer);
        } else {
            self.phase = CopyPhase::ConcatRead(cs, new_transfer);
        }
        StepResult::Continue
    }

    fn step_concat_read(
        &mut self,
        cs: CopyState,
        mut transfer: ActiveTransfer,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let cluster_size = dst_cluster_size(state, &transfer.dst).unwrap_or(HOST_CHUNK_BYTES);
        match read_source(state, drive, &mut transfer.src, cluster_size) {
            Ok(Some(data)) => {
                if data.is_empty() {
                    self.phase = CopyPhase::ConcatNextSource(cs, transfer);
                } else {
                    self.phase = CopyPhase::ConcatWrite(cs, transfer, data);
                }
                StepResult::Continue
            }
            Ok(None) => {
                self.phase = CopyPhase::ConcatNextSource(cs, transfer);
                StepResult::Continue
            }
            Err(()) => {
                io.println(b"Read error");
                StepResult::Done(1)
            }
        }
    }

    fn step_concat_write(
        &mut self,
        cs: CopyState,
        transfer: ActiveTransfer,
        data: Vec<u8>,
        state: &mut DosState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        let ActiveTransfer {
            display_name,
            src,
            dst,
        } = transfer;
        let pending = match dst {
            TransferDst::Dos(p) => p,
            TransferDst::Host(_) => unreachable!("concat only supports DOS destinations"),
        };
        match filesystem::write_pending_file_chunk(state, disk, pending, &data) {
            Ok((updated, _)) => {
                let rebuilt = ActiveTransfer {
                    display_name,
                    src,
                    dst: TransferDst::Dos(updated),
                };
                self.phase = CopyPhase::ConcatRead(cs, rebuilt);
                StepResult::Continue
            }
            Err(0x001F) => {
                io.println(b"Insufficient disk space");
                StepResult::Done(1)
            }
            Err(_) => {
                io.println(b"Write error");
                StepResult::Done(1)
            }
        }
    }
}

// Helpers

fn destination_file_attributes(entry: &ReadDirEntry) -> u8 {
    match &entry.source {
        ReadDirEntrySource::Fat(_) => entry.attribute & 0x27,
        ReadDirEntrySource::Iso(_) => fat_dir::ATTR_ARCHIVE,
    }
}

fn dst_cluster_size(state: &DosState, dst: &TransferDst) -> Option<usize> {
    match dst {
        TransferDst::Dos(pending) => state.fat_volumes[pending.drive_index as usize]
            .as_ref()
            .map(|v| v.bpb.cluster_size() as usize),
        TransferDst::Host(_) => None,
    }
}

fn read_source(
    state: &DosState,
    drive: &mut dyn DriveIo,
    src: &mut TransferSrc,
    chunk_hint: usize,
) -> Result<Option<Vec<u8>>, ()> {
    match src {
        TransferSrc::Dos {
            drive: src_drive,
            cursor,
        } => match cursor {
            SourceFileCursor::Fat(fat_cursor) => {
                if fat_cursor.remaining() == 0 {
                    return Ok(None);
                }
                let volume = state.fat_volumes[*src_drive as usize].as_ref().ok_or(())?;
                let data = fat_cursor
                    .read_chunk(volume, drive, chunk_hint)
                    .map_err(|_| ())?;
                Ok(Some(data))
            }
            SourceFileCursor::Iso { entry, position } => {
                if *position >= entry.file_size {
                    return Ok(None);
                }
                let chunk = iso9660::read_file_chunk(entry, *position, chunk_hint.min(2048), drive)
                    .map_err(|_| ())?;
                *position += chunk.len() as u32;
                Ok(Some(chunk))
            }
        },
        TransferSrc::Host { file } => {
            let mut buf = vec![0u8; chunk_hint.clamp(1024, HOST_CHUNK_BYTES)];
            let n = file.read(&mut buf).map_err(|_| ())?;
            if n == 0 {
                Ok(None)
            } else {
                buf.truncate(n);
                Ok(Some(buf))
            }
        }
    }
}

fn transfer_is_empty(transfer: &ActiveTransfer) -> bool {
    match &transfer.src {
        TransferSrc::Dos { cursor, .. } => match cursor {
            SourceFileCursor::Fat(c) => c.remaining() == 0,
            SourceFileCursor::Iso { entry, position } => *position >= entry.file_size,
        },
        TransferSrc::Host { file } => match file.metadata() {
            Ok(m) => m.len() == 0,
            Err(_) => false,
        },
    }
}

fn build_transfer(job: FileJob) -> Result<ActiveTransfer, &'static [u8]> {
    let src = match job.src {
        FileSource::Dos { drive, entry } => {
            let cursor = match &entry.source {
                ReadDirEntrySource::Fat(fat_entry) => {
                    SourceFileCursor::Fat(FatFileCursor::new(fat_entry))
                }
                ReadDirEntrySource::Iso(iso_entry) => SourceFileCursor::Iso {
                    entry: iso_entry.clone(),
                    position: 0,
                },
            };
            TransferSrc::Dos { drive, cursor }
        }
        FileSource::Host { path } => {
            let file = fs::File::open(&path).map_err(|_| &b"Read error\r\n"[..])?;
            TransferSrc::Host { file }
        }
    };
    let dst = match job.dst {
        FileDest::Dos {
            drive,
            dir_cluster,
            fcb_name,
            attribute,
            time,
            date,
        } => TransferDst::Dos(PendingFatFile {
            drive_index: drive,
            dir_cluster,
            name: fcb_name,
            attribute,
            time,
            date,
            start_cluster: 0,
            file_size: 0,
            position: 0,
        }),
        FileDest::Host { path } => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|_| &b"Write error\r\n"[..])?;
            }
            let file = fs::File::create(&path).map_err(|_| &b"Write error\r\n"[..])?;
            TransferDst::Host(file)
        }
    };
    Ok(ActiveTransfer {
        display_name: job.display_name,
        src,
        dst,
    })
}

fn destination_exists(
    state: &DosState,
    drive: &mut dyn DriveIo,
    dst: &FileDest,
) -> Result<bool, &'static [u8]> {
    match dst {
        FileDest::Dos {
            drive: idx,
            dir_cluster,
            fcb_name,
            ..
        } => {
            let vol = state.fat_volumes[*idx as usize]
                .as_ref()
                .ok_or(&b"Drive not ready\r\n"[..])?;
            Ok(fat_dir::find_entry(vol, *dir_cluster, fcb_name, drive)
                .ok()
                .flatten()
                .is_some())
        }
        FileDest::Host { path } => Ok(fs::metadata(path).is_ok()),
    }
}

fn file_dest_from_entry(
    drive: u8,
    dir_cluster: u16,
    fcb_name: [u8; 11],
    entry: &ReadDirEntry,
) -> FileDest {
    dos_file_dest(
        drive,
        dir_cluster,
        fcb_name,
        destination_file_attributes(entry),
        entry.time,
        entry.date,
    )
}

fn dos_file_dest(
    drive: u8,
    dir_cluster: u16,
    fcb_name: [u8; 11],
    attribute: u8,
    time: u16,
    date: u16,
) -> FileDest {
    FileDest::Dos {
        drive,
        dir_cluster,
        fcb_name,
        attribute,
        time,
        date,
    }
}

fn reject_protected_drive(drive_index: u8) -> Result<(), &'static [u8]> {
    if drive_index == 25 {
        Err(b"Access denied\r\n")
    } else {
        Ok(())
    }
}

fn resolve_dos_destination(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    dos_path: &[u8],
) -> Result<ResolvedDosDestination, &'static [u8]> {
    if let Ok((drive_index, dir_cluster)) =
        filesystem::resolve_dir_path(state, dos_path, io.memory, drive)
    {
        reject_protected_drive(drive_index)?;
        return Ok(ResolvedDosDestination::Dir {
            drive: drive_index,
            dir_cluster,
        });
    }

    let (drive_index, dir_cluster, fcb_name) =
        filesystem::resolve_file_path(state, dos_path, io.memory, drive)
            .map_err(|_| &b"Invalid destination\r\n"[..])?;
    reject_protected_drive(drive_index)?;
    Ok(ResolvedDosDestination::File {
        drive: drive_index,
        dir_cluster,
        fcb_name,
    })
}

fn ensure_dos_directory(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    drive_index: u8,
    parent_cluster: u16,
    fcb_name: [u8; 11],
) -> Result<u16, &'static [u8]> {
    let timestamp = state.dos_timestamp_now();
    filesystem::ensure_directory(
        state,
        disk,
        drive_index,
        parent_cluster,
        fcb_name,
        Some(timestamp),
    )
    .map_err(|_| &b"Unable to create directory\r\n"[..])
}

fn build_wildcard_file_job(
    entry: &ReadDirEntry,
    src_drive: u8,
    dst: &WildcardDest,
) -> Result<FileJob, &'static [u8]> {
    let display_name = fat_dir::fcb_to_display_name(&entry.name);
    let dest = match dst {
        WildcardDest::DosDir { drive, dir_cluster } => {
            file_dest_from_entry(*drive, *dir_cluster, entry.name, entry)
        }
        WildcardDest::HostDir { path } => {
            let leaf = fat_dir::fcb_to_display_name(&entry.name);
            let leaf_str = String::from_utf8_lossy(&leaf).into_owned();
            FileDest::Host {
                path: path.join(leaf_str),
            }
        }
        WildcardDest::DosFile {
            drive,
            dir_cluster,
            fcb_name,
        } => file_dest_from_entry(*drive, *dir_cluster, *fcb_name, entry),
        WildcardDest::HostFile { path } => FileDest::Host { path: path.clone() },
    };
    Ok(FileJob {
        display_name,
        src: FileSource::Dos {
            drive: src_drive,
            entry: entry.clone(),
        },
        dst: dest,
    })
}

fn build_file_job_dos_to_dest(
    entry: &ReadDirEntry,
    src_drive: u8,
    dst: &DirHandle,
) -> Result<FileJob, &'static [u8]> {
    let display_name = fat_dir::fcb_to_display_name(&entry.name);
    let dest = match dst {
        DirHandle::Dos { drive, dir_cluster } => {
            file_dest_from_entry(*drive, *dir_cluster, entry.name, entry)
        }
        DirHandle::Host { path } => {
            let leaf = fat_dir::fcb_to_display_name(&entry.name);
            let leaf_str = String::from_utf8_lossy(&leaf).into_owned();
            FileDest::Host {
                path: path.join(leaf_str),
            }
        }
    };
    Ok(FileJob {
        display_name,
        src: FileSource::Dos {
            drive: src_drive,
            entry: entry.clone(),
        },
        dst: dest,
    })
}

fn make_dest_subdir(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    parent: &DirHandle,
    name_fcb: &[u8; 11],
) -> Result<DirHandle, &'static [u8]> {
    match parent {
        DirHandle::Dos { drive, dir_cluster } => {
            let cluster = ensure_dos_directory(state, disk, *drive, *dir_cluster, *name_fcb)?;
            Ok(DirHandle::Dos {
                drive: *drive,
                dir_cluster: cluster,
            })
        }
        DirHandle::Host { path } => {
            let leaf = fat_dir::fcb_to_display_name(name_fcb);
            let leaf_str = String::from_utf8_lossy(&leaf).into_owned();
            let new_path = path.join(leaf_str);
            fs::create_dir_all(&new_path)
                .map_err(|_| &b"Unable to create host directory\r\n"[..])?;
            Ok(DirHandle::Host { path: new_path })
        }
    }
}

fn is_dot_directory(entry: &ReadDirEntry) -> bool {
    let display = fat_dir::fcb_to_display_name(&entry.name);
    display == b"." || display == b".."
}

fn source_directory_from_entry(entry: &ReadDirEntry) -> Option<ReadDirectory> {
    match &entry.source {
        ReadDirEntrySource::Fat(fat_entry) => {
            (fat_entry.start_cluster >= 2).then_some(ReadDirectory::Fat(fat_entry.start_cluster))
        }
        ReadDirEntrySource::Iso(iso_entry) => iso_entry.directory.clone().map(ReadDirectory::Iso),
    }
}

// Initialization

enum InitOutcome {
    Plan(CopyState),
    Concat {
        cs: CopyState,
        transfer: ActiveTransfer,
    },
}

fn init_copy(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    args: &[u8],
) -> Result<InitOutcome, &'static [u8]> {
    let args = args.trim_ascii();
    if args.is_empty() {
        return Err(b"Required parameter missing\r\n");
    }

    let mut verify = false;
    let mut overwrite_all = false;
    let mut tokens: Vec<&[u8]> = Vec::new();

    for part in args.split(|&b| b == b' ' || b == b'\t') {
        if part.is_empty() {
            continue;
        }
        if part.len() >= 2 && part[0] == b'/' {
            match part[1].to_ascii_uppercase() {
                b'V' => verify = true,
                b'Y' => overwrite_all = true,
                _ => {}
            }
        } else {
            tokens.push(part);
        }
    }

    if tokens.is_empty() {
        return Err(b"Required parameter missing\r\n");
    }

    if let Some((source_part, dest_token)) = concat_tokens(&tokens) {
        return init_concat(
            state,
            io,
            drive,
            &source_part,
            dest_token,
            verify,
            overwrite_all,
        );
    }

    if tokens.len() < 2 {
        return Err(b"Required parameter missing\r\n");
    }
    let source_arg = parse_arg(tokens[0]);
    let dest_arg = parse_arg(tokens[1]);

    if matches!(source_arg, ArgKind::Host(_)) && matches!(dest_arg, ArgKind::Host(_)) {
        return Err(b"Host-to-host copy is not supported\r\n");
    }

    let mut cs = CopyState {
        work_stack: Vec::new(),
        wildcard: None,
        concat_remaining: None,
        files_copied: 0,
        verify,
        overwrite_all,
    };

    match source_arg {
        ArgKind::Dos(src_bytes) => {
            init_dos_source(state, io, drive, &src_bytes, dest_arg, &mut cs)?
        }
        ArgKind::Host(src_path) => init_host_source(state, io, drive, src_path, dest_arg, &mut cs)?,
    }

    Ok(InitOutcome::Plan(cs))
}

fn starts_with_host_prefix(token: &[u8]) -> bool {
    token.len() >= 5 && token[..5].eq_ignore_ascii_case(b"host:")
}

fn concat_tokens<'a>(tokens: &[&'a [u8]]) -> Option<(Vec<u8>, &'a [u8])> {
    if tokens.len() < 2 {
        return None;
    }

    let dest_token = *tokens.last()?;
    let source_tokens = &tokens[..tokens.len() - 1];
    if source_tokens
        .iter()
        .any(|token| starts_with_host_prefix(token))
    {
        return None;
    }

    let mut source_part = Vec::new();
    for token in source_tokens {
        source_part.extend_from_slice(token);
    }

    source_part
        .contains(&b'+')
        .then_some((source_part, dest_token))
}

fn init_dos_source(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    src_bytes: &[u8],
    dest_arg: ArgKind,
    cs: &mut CopyState,
) -> Result<(), &'static [u8]> {
    let has_wildcard = src_bytes.contains(&b'*') || src_bytes.contains(&b'?');
    if has_wildcard {
        let read_path = filesystem::resolve_read_file_path(state, src_bytes, io.memory, drive)
            .map_err(|_| &b"File not found\r\n"[..])?;
        if read_path.drive_index == 25 {
            return Err(b"Access denied\r\n");
        }
        let spec = DosPatternSpec {
            drive: read_path.drive_index,
            directory: read_path.directory,
            pattern: read_path.name,
        };
        let dst = resolve_wildcard_destination(state, io, drive, dest_arg)?;
        cs.wildcard = Some(WildcardSpec {
            spec,
            search_index: 0,
            dst,
        });
        return Ok(());
    }

    if let Ok(read_dir) = filesystem::resolve_read_dir_path(state, src_bytes, io.memory, drive) {
        if read_dir.drive_index == 25 {
            return Err(b"Access denied\r\n");
        }
        let src_leaf = dos_leaf_basename(src_bytes);
        let dst_handle = resolve_destination_directory_for_tree(
            state,
            io,
            drive,
            dest_arg,
            src_leaf.as_deref(),
        )?;
        cs.work_stack.push(WorkItem::EnumerateDos(EnumerateDos {
            src_drive: read_dir.drive_index,
            src_directory: read_dir.directory,
            dst: dst_handle,
            search_index: 0,
        }));
        return Ok(());
    }

    let read_path = filesystem::resolve_read_file_path(state, src_bytes, io.memory, drive)
        .map_err(|_| &b"File not found\r\n"[..])?;
    if read_path.drive_index == 25 {
        return Err(b"Access denied\r\n");
    }
    let entry = filesystem::find_matching_read_entry(
        state,
        read_path.drive_index,
        &read_path.directory,
        &read_path.name,
        0,
        0,
        drive,
    )
    .map_err(|_| &b"File not found\r\n"[..])?
    .ok_or(&b"File not found\r\n"[..])?
    .0;
    let job = build_single_file_job(state, io, drive, entry, read_path.drive_index, dest_arg)?;
    cs.work_stack.push(WorkItem::File(job));
    Ok(())
}

fn init_host_source(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    src_path: PathBuf,
    dest_arg: ArgKind,
    cs: &mut CopyState,
) -> Result<(), &'static [u8]> {
    let meta = fs::metadata(&src_path).map_err(|_| &b"File not found\r\n"[..])?;

    if meta.is_dir() {
        if let ArgKind::Dos(_) = &dest_arg {
            preflight_host_tree_for_dos(&src_path)?;
        }
        let leaf = src_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_owned());
        let dst_handle = resolve_destination_directory_for_host_tree(
            state,
            io,
            drive,
            dest_arg,
            leaf.as_deref(),
        )?;
        cs.work_stack.push(WorkItem::EnumerateHost(EnumerateHost {
            src_path,
            dst: dst_handle,
        }));
        return Ok(());
    }

    if !meta.is_file() {
        return Err(b"File not found\r\n");
    }

    let basename = src_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(&b"Invalid host filename\r\n"[..])?
        .to_owned();
    let display_fcb = match validate_dos_basename(basename.as_bytes()) {
        Ok(name) => fat_dir::name_to_fcb(&name),
        Err(_) => fat_dir::name_to_fcb(b"HOSTFILE"),
    };
    let display_name = fat_dir::fcb_to_display_name(&display_fcb);

    let dest = resolve_host_source_destination(state, io, drive, dest_arg, &basename)?;
    cs.work_stack.push(WorkItem::File(FileJob {
        display_name,
        src: FileSource::Host { path: src_path },
        dst: dest,
    }));
    Ok(())
}

fn preflight_host_tree_for_dos(root: &std::path::Path) -> Result<(), &'static [u8]> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|_| &b"Read error\r\n"[..])?;
        for entry in entries.flatten() {
            let path = entry.path();
            let basename = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name,
                None => return Err(b"Non-UTF-8 host filename\r\n"),
            };
            if validate_dos_basename(basename.as_bytes()).is_err() {
                return Err(b"Long host filename cannot be represented as 8.3\r\n");
            }
            let metadata = entry.metadata().map_err(|_| &b"Read error\r\n"[..])?;
            if metadata.is_dir() {
                stack.push(path);
            }
        }
    }
    Ok(())
}

fn resolve_wildcard_destination(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    dest_arg: ArgKind,
) -> Result<WildcardDest, &'static [u8]> {
    match dest_arg {
        ArgKind::Dos(dest_bytes) => match resolve_dos_destination(state, io, drive, &dest_bytes)? {
            ResolvedDosDestination::Dir { drive, dir_cluster } => {
                Ok(WildcardDest::DosDir { drive, dir_cluster })
            }
            ResolvedDosDestination::File {
                drive,
                dir_cluster,
                fcb_name,
            } => Ok(WildcardDest::DosFile {
                drive,
                dir_cluster,
                fcb_name,
            }),
        },
        ArgKind::Host(path) => {
            if path.is_dir() {
                Ok(WildcardDest::HostDir { path })
            } else if path.exists() {
                Ok(WildcardDest::HostFile { path })
            } else if path.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR) {
                fs::create_dir_all(&path).map_err(|_| &b"Cannot create host dir\r\n"[..])?;
                Ok(WildcardDest::HostDir { path })
            } else {
                Ok(WildcardDest::HostFile { path })
            }
        }
    }
}

fn resolve_destination_directory_for_tree(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    dest_arg: ArgKind,
    src_leaf: Option<&[u8]>,
) -> Result<DirHandle, &'static [u8]> {
    match dest_arg {
        ArgKind::Dos(dest_bytes) => {
            validate_dos_components(&dest_bytes).map_err(|_| &b"Invalid destination\r\n"[..])?;
            match resolve_dos_destination(state, io, drive, &dest_bytes)? {
                ResolvedDosDestination::Dir {
                    drive: drive_index,
                    dir_cluster,
                } => {
                    let dir_cluster = if let Some(leaf) = src_leaf {
                        let fcb_name = fat_dir::name_to_fcb(leaf);
                        ensure_dos_directory(state, drive, drive_index, dir_cluster, fcb_name)?
                    } else {
                        dir_cluster
                    };
                    Ok(DirHandle::Dos {
                        drive: drive_index,
                        dir_cluster,
                    })
                }
                ResolvedDosDestination::File {
                    drive: drive_index,
                    dir_cluster,
                    fcb_name,
                } => {
                    let dir_cluster =
                        ensure_dos_directory(state, drive, drive_index, dir_cluster, fcb_name)?;
                    Ok(DirHandle::Dos {
                        drive: drive_index,
                        dir_cluster,
                    })
                }
            }
        }
        ArgKind::Host(path) => {
            let target = if path.is_dir() {
                if let Some(leaf) = src_leaf {
                    let leaf_str = String::from_utf8_lossy(leaf).into_owned();
                    path.join(leaf_str)
                } else {
                    path
                }
            } else {
                path
            };
            fs::create_dir_all(&target).map_err(|_| &b"Cannot create host dir\r\n"[..])?;
            Ok(DirHandle::Host { path: target })
        }
    }
}

fn resolve_destination_directory_for_host_tree(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    dest_arg: ArgKind,
    src_leaf: Option<&str>,
) -> Result<DirHandle, &'static [u8]> {
    match dest_arg {
        ArgKind::Dos(dest_bytes) => {
            validate_dos_components(&dest_bytes).map_err(|_| &b"Invalid destination\r\n"[..])?;
            let (drive_index, parent_cluster, fcb_name) =
                match resolve_dos_destination(state, io, drive, &dest_bytes)? {
                    ResolvedDosDestination::Dir {
                        drive: drive_index,
                        dir_cluster,
                    } => {
                        let leaf_bytes = src_leaf.unwrap_or("HOSTDIR");
                        let dos_name =
                            validate_dos_basename(leaf_bytes.as_bytes()).map_err(|_| {
                                &b"Long host filename cannot be represented as 8.3\r\n"[..]
                            })?;
                        (drive_index, dir_cluster, fat_dir::name_to_fcb(&dos_name))
                    }
                    ResolvedDosDestination::File {
                        drive: drive_index,
                        dir_cluster,
                        fcb_name,
                    } => (drive_index, dir_cluster, fcb_name),
                };
            let dir_cluster =
                ensure_dos_directory(state, drive, drive_index, parent_cluster, fcb_name)?;
            Ok(DirHandle::Dos {
                drive: drive_index,
                dir_cluster,
            })
        }
        ArgKind::Host(path) => {
            let target = if path.is_dir() {
                if let Some(leaf) = src_leaf {
                    path.join(leaf)
                } else {
                    path
                }
            } else {
                path
            };
            fs::create_dir_all(&target).map_err(|_| &b"Cannot create host dir\r\n"[..])?;
            Ok(DirHandle::Host { path: target })
        }
    }
}

fn resolve_host_source_destination(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    dest_arg: ArgKind,
    src_basename: &str,
) -> Result<FileDest, &'static [u8]> {
    match dest_arg {
        ArgKind::Dos(dest_bytes) => match resolve_dos_destination(state, io, drive, &dest_bytes)? {
            ResolvedDosDestination::Dir {
                drive: drive_index,
                dir_cluster,
            } => {
                let dos_name = validate_dos_basename(src_basename.as_bytes())
                    .map_err(|_| &b"Long host filename cannot be represented as 8.3\r\n"[..])?;
                let (time, date) = state.dos_timestamp_now();
                Ok(dos_file_dest(
                    drive_index,
                    dir_cluster,
                    fat_dir::name_to_fcb(&dos_name),
                    fat_dir::ATTR_ARCHIVE,
                    time,
                    date,
                ))
            }
            ResolvedDosDestination::File {
                drive: drive_index,
                dir_cluster,
                fcb_name,
            } => {
                let (time, date) = state.dos_timestamp_now();
                Ok(dos_file_dest(
                    drive_index,
                    dir_cluster,
                    fcb_name,
                    fat_dir::ATTR_ARCHIVE,
                    time,
                    date,
                ))
            }
        },
        ArgKind::Host(path) => {
            if path.is_dir() {
                Ok(FileDest::Host {
                    path: path.join(src_basename),
                })
            } else {
                Ok(FileDest::Host { path })
            }
        }
    }
}

fn build_single_file_job(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    entry: ReadDirEntry,
    src_drive: u8,
    dest_arg: ArgKind,
) -> Result<FileJob, &'static [u8]> {
    let display_name = fat_dir::fcb_to_display_name(&entry.name);

    let dst = match dest_arg {
        ArgKind::Dos(dest_bytes) => match resolve_dos_destination(state, io, drive, &dest_bytes)? {
            ResolvedDosDestination::Dir {
                drive: drive_index,
                dir_cluster,
            } => file_dest_from_entry(drive_index, dir_cluster, entry.name, &entry),
            ResolvedDosDestination::File {
                drive: drive_index,
                dir_cluster,
                fcb_name,
            } => file_dest_from_entry(drive_index, dir_cluster, fcb_name, &entry),
        },
        ArgKind::Host(path) => {
            if path.is_dir() {
                let leaf = String::from_utf8_lossy(&display_name).into_owned();
                FileDest::Host {
                    path: path.join(leaf),
                }
            } else {
                FileDest::Host { path }
            }
        }
    };

    Ok(FileJob {
        display_name,
        src: FileSource::Dos {
            drive: src_drive,
            entry,
        },
        dst,
    })
}

fn init_concat(
    state: &mut DosState,
    io: &mut IoAccess,
    drive: &mut dyn DriveIo,
    source_part: &[u8],
    dest_token: &[u8],
    verify: bool,
    overwrite_all: bool,
) -> Result<InitOutcome, &'static [u8]> {
    if starts_with_host_prefix(dest_token) {
        return Err(b"Concatenation requires a DOS destination\r\n");
    }
    let mut sources = Vec::new();
    for src_name in source_part.split(|&b| b == b'+') {
        if src_name.is_empty() {
            continue;
        }
        if starts_with_host_prefix(src_name) {
            return Err(b"Concatenation sources must be DOS paths\r\n");
        }
        let read_path = filesystem::resolve_read_file_path(state, src_name, io.memory, drive)
            .map_err(|_| &b"File not found\r\n"[..])?;
        if read_path.drive_index == 25 {
            return Err(b"Access denied\r\n");
        }
        sources.push(DosPatternSpec {
            drive: read_path.drive_index,
            directory: read_path.directory,
            pattern: read_path.name,
        });
    }
    if sources.is_empty() {
        return Err(b"Required parameter missing\r\n");
    }

    let (drive_index, dir_cluster, fcb_name) =
        match resolve_dos_destination(state, io, drive, dest_token)? {
            ResolvedDosDestination::Dir {
                drive: drive_index,
                dir_cluster,
            } => (drive_index, dir_cluster, fat_dir::name_to_fcb(b"CONCAT")),
            ResolvedDosDestination::File {
                drive: drive_index,
                dir_cluster,
                fcb_name,
            } => (drive_index, dir_cluster, fcb_name),
        };

    let first = sources.remove(0);
    let first_entry = filesystem::find_matching_read_entry(
        state,
        first.drive,
        &first.directory,
        &first.pattern,
        0,
        0,
        drive,
    )
    .map_err(|_| &b"File not found\r\n"[..])?
    .ok_or(&b"File not found\r\n"[..])?
    .0;

    let display_name = fat_dir::fcb_to_display_name(&first_entry.name);
    for &byte in &display_name {
        io.output_byte(byte);
    }
    io.println(b"");

    let attribute = destination_file_attributes(&first_entry);
    let time = first_entry.time;
    let date = first_entry.date;
    let pending = PendingFatFile {
        drive_index,
        dir_cluster,
        name: fcb_name,
        attribute,
        time,
        date,
        start_cluster: 0,
        file_size: 0,
        position: 0,
    };
    let cursor = match &first_entry.source {
        ReadDirEntrySource::Fat(fat_entry) => SourceFileCursor::Fat(FatFileCursor::new(fat_entry)),
        ReadDirEntrySource::Iso(iso_entry) => SourceFileCursor::Iso {
            entry: iso_entry.clone(),
            position: 0,
        },
    };
    let src = TransferSrc::Dos {
        drive: first.drive,
        cursor,
    };
    let transfer = ActiveTransfer {
        display_name,
        src,
        dst: TransferDst::Dos(pending),
    };

    let cs = CopyState {
        work_stack: Vec::new(),
        wildcard: None,
        concat_remaining: Some(sources),
        files_copied: 0,
        verify,
        overwrite_all,
    };
    Ok(InitOutcome::Concat { cs, transfer })
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Copies one or more files to another location.");
    io.println(b"");
    io.println(b"COPY [/V] [/Y] source destination");
    io.println(b"COPY [/V] [/Y] source1+source2[+...] destination");
    io.println(b"");
    io.println(b"  /V  Verifies that new files are written correctly.");
    io.println(b"  /Y  Overwrites existing files without prompting.");
    io.println(b"");
    io.println(b"Either path may be prefixed with 'host:' to reach the host filesystem.");
    io.println(b"Directories are copied recursively. Host-to-host copies are not supported.");
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
