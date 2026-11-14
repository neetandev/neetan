//! FORMAT command.

use device::{
    disk::{HddGeometry, format as disk_format, pc98_ide, pc98_sasi},
    floppy::{FloppyGeometry, format as floppy_format},
};

use crate::{
    DiskIo, DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    drive_map::{DriveDevice, PartitionScheme},
    filesystem::{self, fat_bpb::Bpb},
};

pub(crate) struct Format;

impl Command for Format {
    fn name(&self) -> &'static str {
        "FORMAT"
    }

    fn start(&self, args: &[u8]) -> Box<dyn RunningCommand> {
        Box::new(RunningFormat {
            args: args.to_vec(),
            phase: FormatPhase::Init,
        })
    }
}

#[derive(Clone)]
/// Authoritative FORMAT confirmation and disk progress.
struct FormatState {
    drive_index: u8,
    device_id: u8,
    floppy_bpb: Bpb,
    heads: u8,
    sectors_per_track: u8,
    sector_size: u16,
    total_sectors: u32,
    total_tracks: u32,
    current_track: u32,
    partition_offset: u32,
    is_hdd: bool,
    quick: bool,
    verify: bool,
}

state_struct_codec!(FormatState {
    drive_index,
    device_id,
    floppy_bpb,
    heads,
    sectors_per_track,
    sector_size,
    total_sectors,
    total_tracks,
    current_track,
    partition_offset,
    is_hdd,
    quick,
    verify,
});

/// Returns whether FORMAT lays a hard disk with this sector size out in
/// the NEC layout: a PC-98 IDE disk in its BIOS geometry, a SASI disk in its
/// image geometry.
fn uses_nec_layout(sector_size: u16) -> bool {
    pc98_ide::is_ide_sector_size(sector_size) || pc98_sasi::is_sasi_sector_size(sector_size)
}

/// Returns the NEC layout of a hard disk with `total_sectors` sectors in the
/// geometry of `heads`, `sectors_per_track` and `sector_size`, or `None` for
/// a disk outside the NEC layout. A disk too small for the layout fails.
fn hard_disk_nec_layout(
    total_sectors: u32,
    heads: u8,
    sectors_per_track: u8,
    sector_size: u16,
) -> Option<Result<disk_format::Pc98NecLayout, disk_format::FormatError>> {
    if !uses_nec_layout(sector_size) {
        return None;
    }
    let sectors_per_cylinder = u32::from(heads) * u32::from(sectors_per_track);
    let cylinders = total_sectors.checked_div(sectors_per_cylinder).unwrap_or(0);
    Some(disk_format::pc98_nec_layout(HddGeometry {
        cylinders: u16::try_from(cylinders).unwrap_or(u16::MAX),
        heads,
        sectors_per_track,
        sector_size,
    }))
}

/// Returns the NEC layout FORMAT writes, or `None` for a floppy and for a
/// hard disk outside the NEC layout.
fn nec_layout(format_state: &FormatState) -> Option<disk_format::Pc98NecLayout> {
    if !format_state.is_hdd {
        return None;
    }
    hard_disk_nec_layout(
        format_state.total_sectors,
        format_state.heads,
        format_state.sectors_per_track,
        format_state.sector_size,
    )?
    .ok()
}

/// Returns the logical sector size of the volume FORMAT writes on the hard
/// disk `device_id`, or `None` for a disk FORMAT cannot lay out.
pub(crate) fn hard_disk_format_sector_size(disk: &dyn DiskIo, device_id: u8) -> Option<u16> {
    let (cylinders, heads, sectors_per_track) = disk.drive_geometry(device_id)?;
    let sector_size = disk.sector_size(device_id)?;
    let total_sectors = u32::from(cylinders) * u32::from(heads) * u32::from(sectors_per_track);
    match hard_disk_nec_layout(total_sectors, heads, sectors_per_track, sector_size) {
        Some(layout) => layout.ok().map(|layout| layout.bpb.bytes_per_sector),
        None => Some(sector_size),
    }
}

/// Converts the default BPB of a floppy drive to the format parameters.
fn floppy_bpb_params(bpb: &Bpb) -> disk_format::BpbParams {
    disk_format::BpbParams {
        bytes_per_sector: bpb.bytes_per_sector,
        sectors_per_cluster: bpb.sectors_per_cluster,
        reserved_sectors: bpb.reserved_sectors,
        num_fats: bpb.num_fats,
        root_entry_count: bpb.root_entry_count,
        media_descriptor: bpb.media_descriptor,
        sectors_per_fat: bpb.sectors_per_fat,
        is_fat16: false,
    }
}

#[derive(Clone)]
enum FormatPhase {
    Init,
    Confirm(FormatState),
    FormatTrack(FormatState),
    WritePartitionTable(FormatState),
    WriteBootSector(FormatState),
    WriteFat(FormatState),
    WriteRootDir(FormatState),
    VerifyTrack(FormatState),
    Summary(FormatState),
}

impl save_state::StateEncode for FormatPhase {
    fn encode_state(&self, output: &mut Vec<u8>) {
        let (tag, state) = match self {
            Self::Init => {
                save_state::StateEncode::encode_state(&0u8, output);
                return;
            }
            Self::Confirm(state) => (1u8, state),
            Self::FormatTrack(state) => (2u8, state),
            Self::WritePartitionTable(state) => (3u8, state),
            Self::WriteBootSector(state) => (4u8, state),
            Self::WriteFat(state) => (5u8, state),
            Self::WriteRootDir(state) => (6u8, state),
            Self::VerifyTrack(state) => (7u8, state),
            Self::Summary(state) => (8u8, state),
        };
        save_state::StateEncode::encode_state(&tag, output);
        save_state::StateEncode::encode_state(state, output);
    }
}

impl save_state::StateDecode for FormatPhase {
    fn decode_state(
        decoder: &mut save_state::StateDecoder<'_>,
    ) -> Result<Self, save_state::StateDecodeError> {
        match <u8 as save_state::StateDecode>::decode_state(decoder)? {
            0 => Ok(Self::Init),
            1 => Ok(Self::Confirm(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            2 => Ok(Self::FormatTrack(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            3 => Ok(Self::WritePartitionTable(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            4 => Ok(Self::WriteBootSector(
                save_state::StateDecode::decode_state(decoder)?,
            )),
            5 => Ok(Self::WriteFat(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            6 => Ok(Self::WriteRootDir(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            7 => Ok(Self::VerifyTrack(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            8 => Ok(Self::Summary(save_state::StateDecode::decode_state(
                decoder,
            )?)),
            _ => Err(save_state::StateDecodeError::InvalidTag),
        }
    }
}

#[derive(Clone)]
/// Serializable state of an executing FORMAT command.
pub(crate) struct RunningFormat {
    args: Vec<u8>,
    phase: FormatPhase,
}

state_struct_codec!(RunningFormat { args, phase });

impl RunningFormat {
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
        match init_format(state, drive, &self.args) {
            Ok(format_state) => {
                let drive_letter = (b'A' + format_state.drive_index) as char;
                let msg = format!(
                    "\r\nWARNING, ALL DATA ON DRIVE {}: WILL BE LOST!\r\nProceed with Format (Y/N)?",
                    drive_letter
                );
                io.print(msg.as_bytes());
                self.phase = FormatPhase::Confirm(format_state);
                StepResult::Continue
            }
            Err(msg) => {
                io.print(msg);
                StepResult::Done(1)
            }
        }
    }

    fn step_confirm(&mut self, format_state: FormatState, io: &mut IoAccess) -> StepResult {
        if !io.key_available() {
            self.phase = FormatPhase::Confirm(format_state);
            return StepResult::Continue;
        }
        let key = io.take_key_character();
        io.println(b"");

        match key.to_ascii_uppercase() {
            b'Y' => {
                if format_state.is_hdd {
                    self.phase = FormatPhase::WritePartitionTable(format_state);
                } else if format_state.quick {
                    self.phase = FormatPhase::WriteBootSector(format_state);
                } else {
                    self.phase = FormatPhase::FormatTrack(format_state);
                }
                StepResult::Continue
            }
            _ => {
                io.println(b"Format terminated.");
                StepResult::Done(0)
            }
        }
    }

    fn step_format_track(
        &mut self,
        mut format_state: FormatState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if format_state.current_track >= format_state.total_tracks {
            self.phase = FormatPhase::WriteBootSector(format_state);
            return StepResult::Continue;
        }

        let track_size =
            format_state.sectors_per_track as usize * format_state.sector_size as usize;
        let fill_data = vec![0xF6u8; track_size];
        let lba = format_state.current_track * format_state.sectors_per_track as u32;

        if drive
            .write_sectors(format_state.device_id, lba, &fill_data)
            .is_err()
        {
            io.println(b"Write error during format");
            return StepResult::Done(1);
        }

        format_state.current_track += 1;
        self.phase = FormatPhase::FormatTrack(format_state);
        StepResult::Continue
    }

    fn step_write_partition_table(
        &mut self,
        format_state: FormatState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        if drive
            .write_sectors(
                format_state.device_id,
                0,
                &disk_format::pc98_ipl_sector(format_state.sector_size),
            )
            .is_err()
        {
            io.println(b"Write error");
            return StepResult::Done(1);
        }

        // Sector 1: PC-98 partition table
        let part_sector = match nec_layout(&format_state) {
            Some(layout) => layout.partition_table_sector(),
            None => disk_format::pc98_partition_table_sector(
                format_state.sector_size,
                format_state.total_sectors,
                format_state.heads,
                format_state.sectors_per_track,
            ),
        };

        if drive
            .write_sectors(format_state.device_id, 1, &part_sector)
            .is_err()
        {
            io.println(b"Write error");
            return StepResult::Done(1);
        }

        self.phase = FormatPhase::WriteBootSector(format_state);
        StepResult::Continue
    }

    fn step_write_boot_sector(
        &mut self,
        format_state: FormatState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        if let Some(layout) = nec_layout(&format_state) {
            if disk
                .write_sectors(
                    format_state.device_id,
                    layout.partition_start(),
                    &layout.reserved_area(),
                )
                .is_err()
            {
                io.println(b"Write error");
                return StepResult::Done(1);
            }
        } else if format_state.is_hdd {
            let partition_sectors = format_state.total_sectors - format_state.partition_offset;
            let bpb = disk_format::hdd_bpb_params(format_state.sector_size, partition_sectors);
            let boot = disk_format::hdd_boot_sector(
                &bpb,
                format_state.sector_size,
                partition_sectors,
                format_state.sectors_per_track,
                format_state.heads,
                format_state.partition_offset,
            );

            if disk
                .write_sectors(format_state.device_id, format_state.partition_offset, &boot)
                .is_err()
            {
                io.println(b"Write error");
                return StepResult::Done(1);
            }
        } else {
            let bpb = floppy_bpb_params(&format_state.floppy_bpb);

            let geometry = FloppyGeometry {
                cylinders: (format_state.total_tracks / u32::from(format_state.heads)) as u16,
                heads: format_state.heads,
                sectors_per_track: format_state.sectors_per_track,
                sector_size: format_state.sector_size,
            };
            let boot = floppy_format::floppy_boot_sector(&bpb, &geometry);

            if disk
                .write_sectors(format_state.device_id, 0, &boot)
                .is_err()
            {
                io.println(b"Write error");
                return StepResult::Done(1);
            }
        }

        self.phase = FormatPhase::WriteFat(format_state);
        StepResult::Continue
    }

    fn step_write_fat(
        &mut self,
        format_state: FormatState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        if let Some(layout) = nec_layout(&format_state) {
            let fat_data = layout.fat();
            for copy in 0..layout.bpb.num_fats {
                if disk
                    .write_sectors(format_state.device_id, layout.fat_lba(copy), &fat_data)
                    .is_err()
                {
                    io.println(b"Write error");
                    return StepResult::Done(1);
                }
            }
            self.phase = FormatPhase::WriteRootDir(format_state);
            return StepResult::Continue;
        }

        let (bpb, fat_base) = if format_state.is_hdd {
            let partition_sectors = format_state.total_sectors - format_state.partition_offset;
            let bpb = disk_format::hdd_bpb_params(format_state.sector_size, partition_sectors);
            let fat_base = format_state.partition_offset + bpb.reserved_sectors as u32;
            (bpb, fat_base)
        } else {
            let bpb = floppy_bpb_params(&format_state.floppy_bpb);
            let fat_base = bpb.reserved_sectors as u32;
            (bpb, fat_base)
        };

        let fat_data = disk_format::fat_region(&bpb, format_state.sector_size);

        for fat_idx in 0..bpb.num_fats as u32 {
            let fat_lba = fat_base + fat_idx * bpb.sectors_per_fat as u32;
            if disk
                .write_sectors(format_state.device_id, fat_lba, &fat_data)
                .is_err()
            {
                io.println(b"Write error");
                return StepResult::Done(1);
            }
        }

        self.phase = FormatPhase::WriteRootDir(format_state);
        StepResult::Continue
    }

    fn step_write_root_dir(
        &mut self,
        mut format_state: FormatState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        if let Some(layout) = nec_layout(&format_state) {
            if disk
                .write_sectors(
                    format_state.device_id,
                    layout.root_directory_lba(),
                    &layout.root_directory(),
                )
                .is_err()
            {
                io.println(b"Write error");
                return StepResult::Done(1);
            }
            self.phase = FormatPhase::Summary(format_state);
            return StepResult::Continue;
        }

        let (bpb, root_base) = if format_state.is_hdd {
            let partition_sectors = format_state.total_sectors - format_state.partition_offset;
            let bpb = disk_format::hdd_bpb_params(format_state.sector_size, partition_sectors);
            let root_base = format_state.partition_offset
                + bpb.reserved_sectors as u32
                + bpb.num_fats as u32 * bpb.sectors_per_fat as u32;
            (bpb, root_base)
        } else {
            let bpb = floppy_bpb_params(&format_state.floppy_bpb);
            let root_base =
                bpb.reserved_sectors as u32 + bpb.num_fats as u32 * bpb.sectors_per_fat as u32;
            (bpb, root_base)
        };

        let root_dir_sectors =
            (bpb.root_entry_count as u32 * 32).div_ceil(format_state.sector_size as u32);
        let root_size = root_dir_sectors as usize * format_state.sector_size as usize;
        let root_data = vec![0u8; root_size];

        if disk
            .write_sectors(format_state.device_id, root_base, &root_data)
            .is_err()
        {
            io.println(b"Write error");
            return StepResult::Done(1);
        }

        if !format_state.is_hdd && format_state.verify {
            format_state.current_track = 0;
            self.phase = FormatPhase::VerifyTrack(format_state);
        } else {
            self.phase = FormatPhase::Summary(format_state);
        }
        StepResult::Continue
    }

    fn step_verify_track(
        &mut self,
        mut format_state: FormatState,
        io: &mut IoAccess,
        disk: &mut dyn DiskIo,
    ) -> StepResult {
        if format_state.current_track >= format_state.total_tracks {
            self.phase = FormatPhase::Summary(format_state);
            return StepResult::Continue;
        }

        let lba = format_state.current_track * format_state.sectors_per_track as u32;
        if disk
            .read_sectors(
                format_state.device_id,
                lba,
                format_state.sectors_per_track as u32,
            )
            .is_err()
        {
            io.println(b"Verify error");
            return StepResult::Done(1);
        }

        format_state.current_track += 1;
        self.phase = FormatPhase::VerifyTrack(format_state);
        StepResult::Continue
    }

    fn step_summary(
        &mut self,
        format_state: FormatState,
        state: &mut DosState,
        io: &mut IoAccess,
    ) -> StepResult {
        let (total_bytes, available_bytes) = summary_bytes(&format_state);

        io.println(b"Format complete.");
        io.println(b"");
        let msg = format!("  {:>12} bytes total disk space\r\n", total_bytes);
        io.print(msg.as_bytes());
        let msg = format!("  {:>12} bytes available on disk\r\n", available_bytes);
        io.print(msg.as_bytes());

        // Invalidate cached volume so next access re-mounts from fresh disk
        state.fat_volumes[format_state.drive_index as usize] = None;

        StepResult::Done(0)
    }
}

impl RunningCommand for RunningFormat {
    fn step(
        &mut self,
        state: &mut DosState,
        io: &mut IoAccess,
        drive: &mut dyn DriveIo,
    ) -> StepResult {
        let phase = std::mem::replace(&mut self.phase, FormatPhase::Init);
        match phase {
            FormatPhase::Init => self.step_init(state, io, drive),
            FormatPhase::Confirm(fs) => self.step_confirm(fs, io),
            FormatPhase::FormatTrack(fs) => self.step_format_track(fs, io, drive),
            FormatPhase::WritePartitionTable(fs) => self.step_write_partition_table(fs, io, drive),
            FormatPhase::WriteBootSector(fs) => self.step_write_boot_sector(fs, io, drive),
            FormatPhase::WriteFat(fs) => self.step_write_fat(fs, io, drive),
            FormatPhase::WriteRootDir(fs) => self.step_write_root_dir(fs, io, drive),
            FormatPhase::VerifyTrack(fs) => self.step_verify_track(fs, io, drive),
            FormatPhase::Summary(fs) => self.step_summary(fs, state, io),
        }
    }
}

/// Returns the bytes of `data_sectors` sectors of `bytes_per_sector` bytes
/// that whole clusters of `sectors_per_cluster` sectors hold.
fn cluster_bytes(data_sectors: u32, sectors_per_cluster: u8, bytes_per_sector: u16) -> u64 {
    let sectors_per_cluster = u32::from(sectors_per_cluster.max(1));
    u64::from(data_sectors / sectors_per_cluster * sectors_per_cluster)
        * u64::from(bytes_per_sector)
}

/// Returns the total bytes of the formatted volume and the bytes of its data
/// clusters.
fn summary_bytes(format_state: &FormatState) -> (u64, u64) {
    match nec_layout(format_state) {
        Some(layout) => (
            u64::from(layout.volume_sectors) * u64::from(layout.bpb.bytes_per_sector),
            cluster_bytes(
                layout.data_sectors(),
                layout.bpb.sectors_per_cluster,
                layout.bpb.bytes_per_sector,
            ),
        ),
        None => whole_disk_summary(format_state),
    }
}

/// Returns the total bytes and the bytes of the data clusters of a floppy or
/// of a hard disk volume that spans the disk from its second track.
fn whole_disk_summary(format_state: &FormatState) -> (u64, u64) {
    let bpb = if format_state.is_hdd {
        let partition_sectors = format_state.total_sectors - format_state.partition_offset;
        disk_format::hdd_bpb_params(format_state.sector_size, partition_sectors)
    } else {
        floppy_bpb_params(&format_state.floppy_bpb)
    };

    let volume_sectors = if format_state.is_hdd {
        format_state.total_sectors - format_state.partition_offset
    } else {
        format_state.total_sectors
    };
    let total_bytes = volume_sectors as u64 * format_state.sector_size as u64;

    let root_dir_sectors =
        (bpb.root_entry_count as u32 * 32).div_ceil(format_state.sector_size as u32);
    let system_sectors = bpb.reserved_sectors as u32
        + bpb.num_fats as u32 * bpb.sectors_per_fat as u32
        + root_dir_sectors;
    let data_sectors = volume_sectors.saturating_sub(system_sectors);
    (
        total_bytes,
        cluster_bytes(
            data_sectors,
            bpb.sectors_per_cluster,
            format_state.sector_size,
        ),
    )
}

fn print_help(io: &mut IoAccess) {
    io.println(b"Formats a disk for use with the operating system.");
    io.println(b"");
    io.println(b"FORMAT drive: [/Q] [/V]");
    io.println(b"");
    io.println(b"  drive:  Specifies the drive to format.");
    io.println(b"  /Q      Performs a quick format.");
    io.println(b"  /V      Verifies sectors after formatting.");
}

fn init_format(
    state: &mut DosState,
    disk: &mut dyn DiskIo,
    args: &[u8],
) -> Result<FormatState, &'static [u8]> {
    let args = args.trim_ascii();
    if args.is_empty() {
        return Err(b"Required parameter missing\r\n");
    }

    let mut quick = false;
    let mut verify = false;
    let mut drive_token: Option<&[u8]> = None;

    for part in args.split(|&b| b == b' ' || b == b'\t') {
        if part.is_empty() {
            continue;
        }
        if part.len() >= 2 && part[0] == b'/' {
            match part[1].to_ascii_uppercase() {
                b'Q' => quick = true,
                b'V' => verify = true,
                _ => {}
            }
        } else {
            drive_token = Some(part);
        }
    }

    let drive_token = drive_token.ok_or(&b"Required parameter missing\r\n"[..])?;

    // Extract drive letter
    let (drive_opt, _, _) = filesystem::split_path(drive_token);
    let drive_index = drive_opt.ok_or(&b"Invalid drive specification\r\n"[..])?;

    let entry = state
        .drive_map
        .entry(drive_index)
        .ok_or(&b"Invalid drive specification\r\n"[..])?;
    let geometry = entry
        .geometry
        .ok_or(&b"Invalid drive specification\r\n"[..])?;
    let (device_id, is_hdd) = match (entry.device, entry.partition_scheme) {
        (DriveDevice::Floppy { device_id }, _) => (device_id, false),
        (DriveDevice::HardDisk { device_id }, PartitionScheme::Pc98) => (device_id, true),
        _ => return Err(b"Invalid drive specification\r\n"),
    };

    let (cylinders, heads, sectors_per_track) = disk
        .drive_geometry(device_id)
        .ok_or(&b"Invalid drive specification\r\n"[..])?;
    let sector_size = disk
        .sector_size(device_id)
        .ok_or(&b"Invalid drive specification\r\n"[..])?;

    let total_sectors = cylinders as u32 * heads as u32 * sectors_per_track as u32;
    let total_tracks = cylinders as u32 * heads as u32;

    let partition_offset = if !is_hdd {
        0
    } else {
        match hard_disk_nec_layout(total_sectors, heads, sectors_per_track, sector_size) {
            Some(Ok(layout)) => layout.partition_start(),
            Some(Err(_)) => return Err(b"Invalid media or Track 0 bad - disk unusable\r\n"),
            None => u32::from(sectors_per_track),
        }
    };

    Ok(FormatState {
        drive_index,
        device_id,
        floppy_bpb: geometry.default_bpb,
        heads,
        sectors_per_track,
        sector_size,
        total_sectors,
        total_tracks,
        current_track: 0,
        partition_offset,
        is_hdd,
        quick,
        verify,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ide_disk_takes_the_nec_layout_of_the_bios_geometry() {
        let layout = hard_disk_nec_layout(7522 * 136, 8, 17, 512)
            .unwrap()
            .unwrap();
        assert_eq!(layout.partition_start(), 136);
        assert_eq!(layout.end_cylinder, 7515);
        assert_eq!(layout.bpb.bytes_per_sector, 512);
        let table = layout.partition_table_sector();
        assert_eq!(&table[4..8], &[0, 0, 1, 0], "IPL on cylinder 1");
        assert_eq!(&table[8..12], &[0, 0, 1, 0], "data start on cylinder 1");
        let boot = layout.reserved_area();
        assert_eq!(&boot[24..28], &[17, 0, 8, 0]);
        assert_eq!(
            u32::from_le_bytes([boot[28], boot[29], boot[30], boot[31]]),
            136
        );
    }

    #[test]
    fn sasi_disk_takes_the_nec_layout_of_the_image_geometry() {
        let layout = hard_disk_nec_layout(615 * 8 * 33, 8, 33, 256)
            .unwrap()
            .unwrap();
        assert_eq!(layout.partition_start(), 264);
        assert_eq!(layout.end_cylinder, 600);
        assert_eq!(layout.bpb.bytes_per_sector, 1024);
        assert_eq!(layout.fat_lba(0), 268);
        let boot = layout.reserved_area();
        assert_eq!(&boot[24..28], &[33, 0, 8, 0]);
    }

    #[test]
    fn hard_disk_too_small_for_the_nec_layout_fails() {
        assert!(matches!(
            hard_disk_nec_layout(20 * 8 * 17, 8, 17, 256),
            Some(Err(disk_format::FormatError::ImageTooSmall))
        ));
    }

    /// Returns the format state of a hard disk with `cylinders` cylinders of
    /// 8 heads and 17 sectors of 512 bytes.
    fn ide_format_state(cylinders: u32) -> FormatState {
        let total_sectors = cylinders * 136;
        FormatState {
            drive_index: 0,
            device_id: 0x80,
            floppy_bpb: crate::drive_map::pc98::HARD_DISK_BPB,
            heads: 8,
            sectors_per_track: 17,
            sector_size: 512,
            total_sectors,
            total_tracks: cylinders * 8,
            current_track: 0,
            partition_offset: 136,
            is_hdd: true,
            quick: true,
            verify: false,
        }
    }

    #[test]
    fn summary_counts_the_bytes_of_whole_clusters() {
        assert_eq!(
            summary_bytes(&ide_format_state(3017)),
            (409_600 * 512, 51_125 * 4096)
        );
    }

    #[test]
    fn hard_disk_with_another_sector_size_spans_the_disk() {
        assert!(hard_disk_nec_layout(615 * 8 * 8, 8, 8, 1024).is_none());
    }
}
