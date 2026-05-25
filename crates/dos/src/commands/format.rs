//! FORMAT command.

use crate::{
    DiskIo, DosState, DriveIo, IoAccess,
    commands::{Command, RunningCommand, StepResult, is_help_request},
    filesystem, tables,
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

struct FormatState {
    drive_index: u8,
    da_ua: u8,
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

struct BpbParams {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,
    media_descriptor: u8,
    sectors_per_fat: u16,
    is_fat16: bool,
}

fn floppy_bpb_params(da_type: u8) -> BpbParams {
    match da_type {
        0x90 => BpbParams {
            bytes_per_sector: 1024,
            sectors_per_cluster: 1,
            reserved_sectors: 1,
            num_fats: 2,
            root_entry_count: 192,
            media_descriptor: 0xFE,
            sectors_per_fat: 2,
            is_fat16: false,
        },
        0x70 => BpbParams {
            bytes_per_sector: 512,
            sectors_per_cluster: 2,
            reserved_sectors: 1,
            num_fats: 2,
            root_entry_count: 112,
            media_descriptor: 0xFE,
            sectors_per_fat: 3,
            is_fat16: false,
        },
        _ => panic!("unsupported floppy DA type {:#04X}", da_type),
    }
}

fn fat_sectors_for(cluster_count: u32, bytes_per_sector: u16, is_fat16: bool) -> u16 {
    let fat_bytes = if is_fat16 {
        (cluster_count as u64 + 2) * 2
    } else {
        ((cluster_count as u64 + 2) * 3).div_ceil(2)
    };
    fat_bytes.div_ceil(bytes_per_sector as u64) as u16
}

fn solve_hdd_fat_layout(
    partition_sectors: u32,
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,
    is_fat16: bool,
) -> (u32, u16) {
    let root_dir_sectors = (root_entry_count as u32 * 32).div_ceil(bytes_per_sector as u32);
    let mut sectors_per_fat = 1u16;

    loop {
        let system_sectors =
            reserved_sectors as u32 + num_fats as u32 * sectors_per_fat as u32 + root_dir_sectors;
        let data_sectors = partition_sectors.saturating_sub(system_sectors);
        let cluster_count = data_sectors / sectors_per_cluster as u32;
        let needed = fat_sectors_for(cluster_count, bytes_per_sector, is_fat16);
        if needed == sectors_per_fat {
            return (cluster_count, sectors_per_fat);
        }
        sectors_per_fat = needed;
    }
}

fn hdd_bpb_params(sector_size: u16, partition_sectors: u32) -> BpbParams {
    let bytes_per_sector = sector_size;
    let reserved_sectors: u16 = 1;
    let num_fats: u8 = 2;
    let root_entry_count: u16 = 512;

    let volume_bytes = partition_sectors as u64 * sector_size as u64;
    let sectors_per_cluster: u8 = if sector_size == 256 {
        // SASI 256-byte sectors: target 2 KB clusters
        if volume_bytes <= 64 * 1024 * 1024 {
            8
        } else if volume_bytes <= 128 * 1024 * 1024 {
            16
        } else if volume_bytes <= 256 * 1024 * 1024 {
            32
        } else {
            64
        }
    } else {
        // IDE 512-byte sectors: target 2 KB clusters for small volumes
        if volume_bytes <= 64 * 1024 * 1024 {
            4
        } else if volume_bytes <= 128 * 1024 * 1024 {
            8
        } else if volume_bytes <= 256 * 1024 * 1024 {
            16
        } else {
            32
        }
    };

    let (fat12_clusters, fat12_sectors_per_fat) = solve_hdd_fat_layout(
        partition_sectors,
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        root_entry_count,
        false,
    );
    let (is_fat16, sectors_per_fat) = if fat12_clusters >= 4085 {
        let (_, fat16_sectors_per_fat) = solve_hdd_fat_layout(
            partition_sectors,
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            root_entry_count,
            true,
        );
        (true, fat16_sectors_per_fat)
    } else {
        (false, fat12_sectors_per_fat)
    };

    BpbParams {
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        root_entry_count,
        media_descriptor: 0xF8,
        sectors_per_fat,
        is_fat16,
    }
}

const KB_BUF_COUNT: u32 = 0x0528;

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

struct RunningFormat {
    args: Vec<u8>,
    phase: FormatPhase,
}

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
        match init_format(state, io, drive, &self.args) {
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
        if io.memory.read_byte(KB_BUF_COUNT) == 0 {
            self.phase = FormatPhase::Confirm(format_state);
            return StepResult::Continue;
        }
        let key = consume_key(io);
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
            .write_sectors(format_state.da_ua, lba, &fill_data)
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
        let ss = format_state.sector_size as usize;

        // Sector 0: IPL (clear to zeros)
        let ipl = vec![0u8; ss];
        if drive.write_sectors(format_state.da_ua, 0, &ipl).is_err() {
            io.println(b"Write error");
            return StepResult::Done(1);
        }

        // Sector 1: PC-98 partition table
        let mut part_sector = vec![0u8; ss];

        // One partition entry (32 bytes)
        // MID: 0x21 = DOS type (0x20) | subtype 0x01 (FAT16), not bootable (bit 7 clear)
        part_sector[0] = 0x21;
        // SID: 0x81 = active (bit 7) | system ID 0x01
        part_sector[1] = 0x81;
        // Offsets 2-3: reserved (IPL CHS, unused for our purposes)
        // Offsets 4-7: IPL CHS (cylinder 0, head 0, sector 0)

        // Data start CHS: cylinder 0, head 1, sector 0 (first track after IPL/partition table)
        part_sector[8] = 0; // start sector
        part_sector[9] = 1; // start head
        part_sector[10] = 0; // start cylinder low
        part_sector[11] = 0; // start cylinder high

        // Data end CHS
        let last_sector = format_state.total_sectors - 1;
        let end_cylinder =
            last_sector / (format_state.heads as u32 * format_state.sectors_per_track as u32);
        let remainder =
            last_sector % (format_state.heads as u32 * format_state.sectors_per_track as u32);
        let end_head = remainder / format_state.sectors_per_track as u32;
        let end_sector = remainder % format_state.sectors_per_track as u32;
        part_sector[12] = end_sector as u8;
        part_sector[13] = end_head as u8;
        part_sector[14] = end_cylinder as u8;
        part_sector[15] = (end_cylinder >> 8) as u8;

        // Partition name (16 bytes, space-padded)
        part_sector[16..32].copy_from_slice(b"NEETAN          ");

        if drive
            .write_sectors(format_state.da_ua, 1, &part_sector)
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
        if format_state.is_hdd {
            let partition_sectors = format_state.total_sectors - format_state.partition_offset;
            let bpb = hdd_bpb_params(format_state.sector_size, partition_sectors);

            let ss = format_state.sector_size as usize;
            let mut boot = vec![0u8; ss];
            boot[0] = 0xEB;
            boot[1] = 0x3C;
            boot[2] = 0x90;
            boot[3..11].copy_from_slice(b"NEETAN  ");
            boot[11..13].copy_from_slice(&bpb.bytes_per_sector.to_le_bytes());
            boot[13] = bpb.sectors_per_cluster;
            boot[14..16].copy_from_slice(&bpb.reserved_sectors.to_le_bytes());
            boot[16] = bpb.num_fats;
            boot[17..19].copy_from_slice(&bpb.root_entry_count.to_le_bytes());
            if partition_sectors <= 0xFFFF {
                boot[19..21].copy_from_slice(&(partition_sectors as u16).to_le_bytes());
            } else {
                boot[19..21].copy_from_slice(&0u16.to_le_bytes());
                boot[32..36].copy_from_slice(&partition_sectors.to_le_bytes());
            }
            boot[21] = bpb.media_descriptor;
            boot[22..24].copy_from_slice(&bpb.sectors_per_fat.to_le_bytes());
            boot[24..26].copy_from_slice(&(format_state.sectors_per_track as u16).to_le_bytes());
            boot[26..28].copy_from_slice(&(format_state.heads as u16).to_le_bytes());
            boot[28..32].copy_from_slice(&format_state.partition_offset.to_le_bytes());

            if disk
                .write_sectors(format_state.da_ua, format_state.partition_offset, &boot)
                .is_err()
            {
                io.println(b"Write error");
                return StepResult::Done(1);
            }
        } else {
            let da_type = format_state.da_ua & 0xF0;
            let bpb = floppy_bpb_params(da_type);

            let mut boot = vec![0u8; format_state.sector_size as usize];
            boot[0] = 0xEB;
            boot[1] = 0x3C;
            boot[2] = 0x90;
            boot[3..11].copy_from_slice(b"NEETAN  ");
            boot[11..13].copy_from_slice(&bpb.bytes_per_sector.to_le_bytes());
            boot[13] = bpb.sectors_per_cluster;
            boot[14..16].copy_from_slice(&bpb.reserved_sectors.to_le_bytes());
            boot[16] = bpb.num_fats;
            boot[17..19].copy_from_slice(&bpb.root_entry_count.to_le_bytes());
            let total_16 = format_state.total_sectors.min(0xFFFF) as u16;
            boot[19..21].copy_from_slice(&total_16.to_le_bytes());
            boot[21] = bpb.media_descriptor;
            boot[22..24].copy_from_slice(&bpb.sectors_per_fat.to_le_bytes());
            boot[24..26].copy_from_slice(&(format_state.sectors_per_track as u16).to_le_bytes());
            boot[26..28].copy_from_slice(&(format_state.heads as u16).to_le_bytes());

            if disk.write_sectors(format_state.da_ua, 0, &boot).is_err() {
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
        let (bpb, fat_base) = if format_state.is_hdd {
            let partition_sectors = format_state.total_sectors - format_state.partition_offset;
            let bpb = hdd_bpb_params(format_state.sector_size, partition_sectors);
            let fat_base = format_state.partition_offset + bpb.reserved_sectors as u32;
            (bpb, fat_base)
        } else {
            let da_type = format_state.da_ua & 0xF0;
            let bpb = floppy_bpb_params(da_type);
            let fat_base = bpb.reserved_sectors as u32;
            (bpb, fat_base)
        };

        let fat_size = bpb.sectors_per_fat as usize * format_state.sector_size as usize;
        let mut fat_data = vec![0u8; fat_size];

        if bpb.is_fat16 {
            // FAT16 reserved entries: 0xFFF8, 0xFFFF
            fat_data[0] = bpb.media_descriptor;
            fat_data[1] = 0xFF;
            fat_data[2] = 0xFF;
            fat_data[3] = 0xFF;
        } else {
            // FAT12 reserved entries
            fat_data[0] = bpb.media_descriptor;
            fat_data[1] = 0xFF;
            fat_data[2] = 0xFF;
        }

        for fat_idx in 0..bpb.num_fats as u32 {
            let fat_lba = fat_base + fat_idx * bpb.sectors_per_fat as u32;
            if disk
                .write_sectors(format_state.da_ua, fat_lba, &fat_data)
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
        let (bpb, root_base) = if format_state.is_hdd {
            let partition_sectors = format_state.total_sectors - format_state.partition_offset;
            let bpb = hdd_bpb_params(format_state.sector_size, partition_sectors);
            let root_base = format_state.partition_offset
                + bpb.reserved_sectors as u32
                + bpb.num_fats as u32 * bpb.sectors_per_fat as u32;
            (bpb, root_base)
        } else {
            let da_type = format_state.da_ua & 0xF0;
            let bpb = floppy_bpb_params(da_type);
            let root_base =
                bpb.reserved_sectors as u32 + bpb.num_fats as u32 * bpb.sectors_per_fat as u32;
            (bpb, root_base)
        };

        let root_dir_sectors =
            (bpb.root_entry_count as u32 * 32).div_ceil(format_state.sector_size as u32);
        let root_size = root_dir_sectors as usize * format_state.sector_size as usize;
        let root_data = vec![0u8; root_size];

        if disk
            .write_sectors(format_state.da_ua, root_base, &root_data)
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
                format_state.da_ua,
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
        let bpb = if format_state.is_hdd {
            let partition_sectors = format_state.total_sectors - format_state.partition_offset;
            hdd_bpb_params(format_state.sector_size, partition_sectors)
        } else {
            let da_type = format_state.da_ua & 0xF0;
            floppy_bpb_params(da_type)
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
        let available_bytes = data_sectors as u64 * format_state.sector_size as u64;

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
    _state: &mut DosState,
    io: &mut IoAccess,
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

    // Look up DA/UA
    let da_ua = io
        .memory
        .read_byte(tables::IOSYS_BASE + tables::IOSYS_OFF_DAUA_TABLE + drive_index as u32);
    if da_ua == 0 {
        return Err(b"Invalid drive specification\r\n");
    }

    let da_type = da_ua & 0xF0;
    if da_type != 0x90 && da_type != 0x70 && da_type != 0x80 {
        return Err(b"Invalid drive specification\r\n");
    }

    let is_hdd = da_type == 0x80;

    let (cylinders, heads, sectors_per_track) = disk
        .drive_geometry(da_ua)
        .ok_or(&b"Invalid drive specification\r\n"[..])?;
    let sector_size = disk
        .sector_size(da_ua)
        .ok_or(&b"Invalid drive specification\r\n"[..])?;

    let total_sectors = cylinders as u32 * heads as u32 * sectors_per_track as u32;
    let total_tracks = cylinders as u32 * heads as u32;

    let partition_offset = if is_hdd {
        // Partition starts after the first track (IPL at sector 0, partition table at sector 1)
        sectors_per_track as u32
    } else {
        0
    };

    Ok(FormatState {
        drive_index,
        da_ua,
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
