//! DISKCOPY command.

use crate::{
    DiskIo, DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    drive_map::{DriveDevice, DriveMap},
    filesystem,
};

pub(crate) struct Diskcopy;

impl Command for Diskcopy {
    fn name(&self) -> &'static str {
        "DISKCOPY"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningDiskcopy {
            args: args.to_vec(),
            phase: DiskcopyPhase::Init,
        })
    }
}

#[derive(Clone)]
/// Authoritative DISKCOPY track transfer and verification state.
struct DiskcopyState {
    src_drive_index: u8,
    src_device_id: u8,
    dst_drive_index: u8,
    dst_device_id: u8,
    sectors_per_track: u8,
    sector_size: u16,
    total_tracks: u32,
    current_track: u32,
    same_drive: bool,
    verify: bool,
    disk_buffer: Vec<u8>,
}

state_struct_codec!(DiskcopyState {
    src_drive_index,
    src_device_id,
    dst_drive_index,
    dst_device_id,
    sectors_per_track,
    sector_size,
    total_tracks,
    current_track,
    same_drive,
    verify,
    disk_buffer,
});

#[derive(Clone)]
enum DiskcopyPhase {
    Init,
    PromptInsertSource(DiskcopyState),
    ReadTracks(DiskcopyState),
    PromptInsertDest(DiskcopyState),
    WriteTracks(DiskcopyState),
    VerifyTracks(DiskcopyState),
    Summary(DiskcopyState),
    PromptAnother(DiskcopyState),
}

impl save_state::StateEncode for DiskcopyPhase {
    fn encode_state(&self, output: &mut Vec<u8>) {
        let (tag, state) = match self {
            Self::Init => {
                save_state::StateEncode::encode_state(&0u8, output);
                return;
            }
            Self::PromptInsertSource(state) => (1u8, state),
            Self::ReadTracks(state) => (2u8, state),
            Self::PromptInsertDest(state) => (3u8, state),
            Self::WriteTracks(state) => (4u8, state),
            Self::VerifyTracks(state) => (5u8, state),
            Self::Summary(state) => (6u8, state),
            Self::PromptAnother(state) => (7u8, state),
        };
        save_state::StateEncode::encode_state(&tag, output);
        save_state::StateEncode::encode_state(state, output);
    }
}

impl save_state::StateDecode for DiskcopyPhase {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        let state = |decoder: &mut save_state::StateDecoder<'_>| {
            save_state::StateDecode::decode_state(decoder)
        };
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Init),
            1 => Ok(Self::PromptInsertSource(state(decoder)?)),
            2 => Ok(Self::ReadTracks(state(decoder)?)),
            3 => Ok(Self::PromptInsertDest(state(decoder)?)),
            4 => Ok(Self::WriteTracks(state(decoder)?)),
            5 => Ok(Self::VerifyTracks(state(decoder)?)),
            6 => Ok(Self::Summary(state(decoder)?)),
            7 => Ok(Self::PromptAnother(state(decoder)?)),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

#[derive(Clone)]
/// Serializable state of an executing DISKCOPY command.
pub(crate) struct RunningDiskcopy {
    args: Vec<u8>,
    phase: DiskcopyPhase,
}

state_struct_codec!(RunningDiskcopy { args, phase });

impl RunningDiskcopy {
    fn step_init(
        &mut self,
        state: &DosState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        if is_help_request(&self.args) || self.args.trim_ascii().is_empty() {
            print_help(io);
            return StepResult::Done(0);
        }
        match init_diskcopy(state, disk, &self.args) {
            Ok(diskcopy_state) => {
                if diskcopy_state.same_drive {
                    let drive_letter = (b'A' + diskcopy_state.src_drive_index) as char;
                    let msg = format!(
                        "Insert SOURCE diskette in drive {}:\r\nPress any key to continue . . .",
                        drive_letter
                    );
                    io.print(msg.as_bytes());
                    self.phase = DiskcopyPhase::PromptInsertSource(diskcopy_state);
                } else {
                    io.println(b"");
                    io.println(b"Reading from source disk . . .");
                    self.phase = DiskcopyPhase::ReadTracks(diskcopy_state);
                }
                StepResult::Continue
            }
            Err(msg) => {
                io.print(msg);
                StepResult::Done(1)
            }
        }
    }

    fn step_prompt_insert_source(
        &mut self,
        diskcopy_state: DiskcopyState,
        io: &mut IoAccess,
    ) -> StepResult {
        if !io.key_available() {
            self.phase = DiskcopyPhase::PromptInsertSource(diskcopy_state);
            return StepResult::Continue;
        }
        io.take_key_character();
        io.println(b"");
        io.println(b"");
        io.println(b"Reading from source disk . . .");
        self.phase = DiskcopyPhase::ReadTracks(diskcopy_state);
        StepResult::Continue
    }

    fn step_read_tracks(
        &mut self,
        mut diskcopy_state: DiskcopyState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if diskcopy_state.current_track >= diskcopy_state.total_tracks {
            if diskcopy_state.same_drive {
                let drive_letter = (b'A' + diskcopy_state.dst_drive_index) as char;
                let msg = format!(
                    "\r\nInsert DESTINATION diskette in drive {}:\r\nPress any key to continue . . .",
                    drive_letter
                );
                io.print(msg.as_bytes());
                diskcopy_state.current_track = 0;
                self.phase = DiskcopyPhase::PromptInsertDest(diskcopy_state);
            } else {
                io.println(b"");
                io.println(b"Writing to destination disk . . .");
                diskcopy_state.current_track = 0;
                self.phase = DiskcopyPhase::WriteTracks(diskcopy_state);
            }
            return StepResult::Continue;
        }

        let lba = diskcopy_state.current_track * diskcopy_state.sectors_per_track as u32;
        match drive.read_sectors(
            diskcopy_state.src_device_id,
            lba,
            diskcopy_state.sectors_per_track as u32,
        ) {
            Ok(data) => {
                diskcopy_state.disk_buffer.extend_from_slice(&data);
                diskcopy_state.current_track += 1;
                self.phase = DiskcopyPhase::ReadTracks(diskcopy_state);
                StepResult::Continue
            }
            Err(_) => {
                io.println(b"Read error on source disk");
                StepResult::Done(1)
            }
        }
    }

    fn step_prompt_insert_dest(
        &mut self,
        diskcopy_state: DiskcopyState,
        io: &mut IoAccess,
    ) -> StepResult {
        if !io.key_available() {
            self.phase = DiskcopyPhase::PromptInsertDest(diskcopy_state);
            return StepResult::Continue;
        }
        io.take_key_character();
        io.println(b"");
        io.println(b"");
        io.println(b"Writing to destination disk . . .");
        self.phase = DiskcopyPhase::WriteTracks(diskcopy_state);
        StepResult::Continue
    }

    fn step_write_tracks(
        &mut self,
        mut diskcopy_state: DiskcopyState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if diskcopy_state.current_track >= diskcopy_state.total_tracks {
            if diskcopy_state.verify {
                io.println(b"");
                io.println(b"Verifying . . .");
                diskcopy_state.current_track = 0;
                self.phase = DiskcopyPhase::VerifyTracks(diskcopy_state);
            } else {
                self.phase = DiskcopyPhase::Summary(diskcopy_state);
            }
            return StepResult::Continue;
        }

        let track_size =
            diskcopy_state.sectors_per_track as usize * diskcopy_state.sector_size as usize;
        let buffer_offset = diskcopy_state.current_track as usize * track_size;
        let track_data = &diskcopy_state.disk_buffer[buffer_offset..buffer_offset + track_size];
        let lba = diskcopy_state.current_track * diskcopy_state.sectors_per_track as u32;

        if drive
            .write_sectors(diskcopy_state.dst_device_id, lba, track_data)
            .is_err()
        {
            io.println(b"Write error on destination disk");
            return StepResult::Done(1);
        }

        diskcopy_state.current_track += 1;
        self.phase = DiskcopyPhase::WriteTracks(diskcopy_state);
        StepResult::Continue
    }

    fn step_verify_tracks(
        &mut self,
        mut diskcopy_state: DiskcopyState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if diskcopy_state.current_track >= diskcopy_state.total_tracks {
            self.phase = DiskcopyPhase::Summary(diskcopy_state);
            return StepResult::Continue;
        }

        let track_size =
            diskcopy_state.sectors_per_track as usize * diskcopy_state.sector_size as usize;
        let buffer_offset = diskcopy_state.current_track as usize * track_size;
        let expected = &diskcopy_state.disk_buffer[buffer_offset..buffer_offset + track_size];
        let lba = diskcopy_state.current_track * diskcopy_state.sectors_per_track as u32;

        match drive.read_sectors(
            diskcopy_state.dst_device_id,
            lba,
            diskcopy_state.sectors_per_track as u32,
        ) {
            Ok(readback) => {
                if readback[..track_size] != expected[..track_size] {
                    io.println(b"Verify error");
                    return StepResult::Done(1);
                }
            }
            Err(_) => {
                io.println(b"Verify error");
                return StepResult::Done(1);
            }
        }

        diskcopy_state.current_track += 1;
        self.phase = DiskcopyPhase::VerifyTracks(diskcopy_state);
        StepResult::Continue
    }

    fn step_summary(
        &mut self,
        diskcopy_state: DiskcopyState,
        state: &mut DosState,
        io: &mut IoAccess,
    ) -> StepResult {
        io.println(b"");
        io.println(b"Copy complete.");

        // Invalidate destination volume cache
        state.fat_volumes[diskcopy_state.dst_drive_index as usize] = None;

        io.print(b"\r\nCopy another diskette (Y/N)?");
        self.phase = DiskcopyPhase::PromptAnother(diskcopy_state);
        StepResult::Continue
    }

    fn step_prompt_another(
        &mut self,
        mut diskcopy_state: DiskcopyState,
        io: &mut IoAccess,
    ) -> StepResult {
        if !io.key_available() {
            self.phase = DiskcopyPhase::PromptAnother(diskcopy_state);
            return StepResult::Continue;
        }
        let key = io.take_key_character();
        io.println(b"");

        match key.to_ascii_uppercase() {
            b'Y' => {
                diskcopy_state.current_track = 0;
                diskcopy_state.disk_buffer.clear();
                if diskcopy_state.same_drive {
                    let drive_letter = (b'A' + diskcopy_state.src_drive_index) as char;
                    let msg = format!(
                        "\r\nInsert SOURCE diskette in drive {}:\r\nPress any key to continue . . .",
                        drive_letter
                    );
                    io.print(msg.as_bytes());
                    self.phase = DiskcopyPhase::PromptInsertSource(diskcopy_state);
                } else {
                    io.println(b"");
                    io.println(b"Reading from source disk . . .");
                    self.phase = DiskcopyPhase::ReadTracks(diskcopy_state);
                }
                StepResult::Continue
            }
            _ => StepResult::Done(0),
        }
    }
}

impl RunningCommand for RunningDiskcopy {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let phase = std::mem::replace(&mut self.phase, DiskcopyPhase::Init);
        match phase {
            DiskcopyPhase::Init => self.step_init(state, io, drive),
            DiskcopyPhase::PromptInsertSource(ds) => self.step_prompt_insert_source(ds, io),
            DiskcopyPhase::ReadTracks(ds) => self.step_read_tracks(ds, io, drive),
            DiskcopyPhase::PromptInsertDest(ds) => self.step_prompt_insert_dest(ds, io),
            DiskcopyPhase::WriteTracks(ds) => self.step_write_tracks(ds, io, drive),
            DiskcopyPhase::VerifyTracks(ds) => self.step_verify_tracks(ds, io, drive),
            DiskcopyPhase::Summary(ds) => self.step_summary(ds, state, io),
            DiskcopyPhase::PromptAnother(ds) => self.step_prompt_another(ds, io),
        }
    }
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Copies the contents of one floppy disk to another.");
    io.println(b"");
    io.println(b"DISKCOPY [/V] source: [destination:]");
    io.println(b"");
    io.println(b"  source:       Drive containing the source disk.");
    io.println(b"  destination:  Drive for the destination disk.");
    io.println(b"  /V            Verifies that the copy is made correctly.");
}

fn init_diskcopy(
    state: &DosState,
    disk: &mut dyn DiskIo,
    args: &[u8],
) -> Result<DiskcopyState, &'static [u8]> {
    let args = args.trim_ascii();
    if args.is_empty() {
        return Err(b"Required parameter missing\r\n");
    }

    let mut verify = false;
    let mut drive_tokens: Vec<&[u8]> = Vec::new();

    for part in args.split(|&b| b == b' ' || b == b'\t') {
        if part.is_empty() {
            continue;
        }
        if part.len() >= 2 && part[0] == b'/' {
            if part[1].eq_ignore_ascii_case(&b'V') {
                verify = true;
            }
        } else {
            drive_tokens.push(part);
        }
    }

    if drive_tokens.is_empty() {
        return Err(b"Required parameter missing\r\n");
    }

    // Parse source drive
    let (src_drive_opt, _, _) = filesystem::split_path(drive_tokens[0]);
    let src_drive_index = src_drive_opt.ok_or(&b"Invalid drive specification\r\n"[..])?;

    let src_device_id = floppy_device_id(&state.drive_map, src_drive_index)?;

    // Parse destination drive (or same as source)
    let (dst_drive_index, dst_device_id) = if drive_tokens.len() >= 2 {
        let (dst_drive_opt, _, _) = filesystem::split_path(drive_tokens[1]);
        let dst_drive_index = dst_drive_opt.ok_or(&b"Invalid drive specification\r\n"[..])?;
        (
            dst_drive_index,
            floppy_device_id(&state.drive_map, dst_drive_index)?,
        )
    } else {
        (src_drive_index, src_device_id)
    };

    let same_drive = src_drive_index == dst_drive_index;

    // Get source geometry
    let (src_cylinders, src_heads, src_spt) = disk
        .drive_geometry(src_device_id)
        .ok_or(&b"Invalid drive specification\r\n"[..])?;
    let src_sector_size = disk
        .sector_size(src_device_id)
        .ok_or(&b"Invalid drive specification\r\n"[..])?;

    // Check destination geometry matches (unless same drive)
    if !same_drive {
        let (dst_cylinders, dst_heads, dst_spt) = disk
            .drive_geometry(dst_device_id)
            .ok_or(&b"Invalid drive specification\r\n"[..])?;
        let dst_sector_size = disk
            .sector_size(dst_device_id)
            .ok_or(&b"Invalid drive specification\r\n"[..])?;

        if src_cylinders != dst_cylinders
            || src_heads != dst_heads
            || src_spt != dst_spt
            || src_sector_size != dst_sector_size
        {
            return Err(b"Drive types or diskette types not compatible\r\n");
        }
    }

    let total_tracks = src_cylinders as u32 * src_heads as u32;
    let total_disk_bytes = total_tracks as usize * src_spt as usize * src_sector_size as usize;

    Ok(DiskcopyState {
        src_drive_index,
        src_device_id,
        dst_drive_index,
        dst_device_id,
        sectors_per_track: src_spt,
        sector_size: src_sector_size,
        total_tracks,
        current_track: 0,
        same_drive,
        verify,
        disk_buffer: Vec::with_capacity(total_disk_bytes),
    })
}

/// Returns the `DiskIo` device id of a floppy drive.
fn floppy_device_id(drive_map: &DriveMap, drive_index: u8) -> Result<u8, &'static [u8]> {
    match drive_map.entry(drive_index).map(|entry| entry.device) {
        Some(DriveDevice::Floppy { device_id }) => Ok(device_id),
        _ => Err(b"Invalid drive specification\r\n"),
    }
}
