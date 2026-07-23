//! INT 13h floppy disk services, also reachable through the INT 40h alias.
//!
//! Transfers run synchronously through direct device access. The DMA
//! controller is still programmed and stepped through the guest-visible I/O
//! ports so its registers end up exactly as a real INT 13h operation leaves
//! them, but the data itself moves through the paging-aware memory accessors
//! so calls from V86 mode with paging enabled land where the guest's page
//! tables say. Behavior details (status codes, register effects, BDA bytes)
//! are matched to the real AMI BIOS through the side-by-side probe tests in
//! `tests/bios/int13h_floppy.rs`.

use common::{Cpu, SegmentRegister, TraceSink};

use super::{AtBus, BIOS_CODE_SEGMENT, METADATA_DISKETTE_PARAMETER_TABLE};
use crate::{
    cmos::{FLOPPY_TYPE_360K, FLOPPY_TYPE_720K, FLOPPY_TYPE_1200K, FLOPPY_TYPE_1440K},
    scheduler::EventAt,
};

/// BIOS data area: diskette recalibrate and interrupt status.
const BDA_FLOPPY_RECALIBRATE: u32 = 0x43E;
/// BIOS data area: diskette motor status.
const BDA_FLOPPY_MOTOR: u32 = 0x43F;
/// BIOS data area: diskette motor shutoff counter.
const BDA_FLOPPY_MOTOR_COUNT: u32 = 0x440;
/// BIOS data area: diskette status of the last operation.
const BDA_FLOPPY_STATUS: u32 = 0x441;
/// BIOS data area: the seven FDC result bytes of the last operation.
const BDA_FLOPPY_FDC_RESULT: u32 = 0x442;
/// BIOS data area: diskette media control byte.
const BDA_FLOPPY_MEDIA_CONTROL: u32 = 0x48B;
/// BIOS data area: drive 0 media state (drive 1 follows at 40:91).
const BDA_FLOPPY_MEDIA_STATE_0: u32 = 0x490;
/// BIOS data area: drive 0 current track (drive 1 follows at 40:95).
const BDA_FLOPPY_TRACK_0: u32 = 0x494;
/// BIOS data area: equipment word.
const BDA_EQUIPMENT: u32 = 0x410;
/// Equipment word bit 0: diskette drives installed.
const EQUIPMENT_FLOPPY_INSTALLED: u16 = 0x0001;

/// Diskette status: success.
const STATUS_OK: u8 = 0x00;
/// Diskette status: invalid function or parameter.
const STATUS_BAD_COMMAND: u8 = 0x01;
/// Diskette status: address mark not found. The controller reports this for
/// every sector search that fails, so it also covers requests past the end
/// of the track or media.
const STATUS_ADDRESS_MARK: u8 = 0x02;
/// Diskette status: write attempted on a protected disk.
const STATUS_WRITE_PROTECT: u8 = 0x03;
/// Diskette status: media changed.
const STATUS_MEDIA_CHANGE: u8 = 0x06;
/// Diskette status: DMA transfer would cross a 64 KiB boundary.
const STATUS_DMA_BOUNDARY: u8 = 0x09;
/// Diskette status: unsupported track or media combination.
const STATUS_UNSUPPORTED_COMBINATION: u8 = 0x0C;
/// Diskette status: drive timeout, no media present.
const STATUS_TIMEOUT: u8 = 0x80;

/// INT 13h AH=15h: no drive present.
const DRIVE_TYPE_NONE: u8 = 0x00;
/// INT 13h AH=15h: diskette drive with a change line.
const DRIVE_TYPE_FLOPPY_CHANGE_LINE: u8 = 0x02;

/// First DL value addressing a hard disk.
pub(super) const FIRST_HARD_DISK_DRIVE: u8 = 0x80;
/// Number of diskette drive slots on the controller.
const FLOPPY_DRIVE_COUNT: usize = 2;
/// Largest sector identifier count one format call accepts.
const FORMAT_IDENTIFIER_LIMIT: u8 = 40;
/// Fill byte of freshly formatted sectors.
const FORMAT_FILL_BYTE: u8 = 0xF6;
/// Motor shutoff tick count armed after every operation (about two seconds,
/// captured from the real AMI BIOS).
const FLOPPY_MOTOR_TICKS: u8 = 0x25;

/// BDA 40:3E bit 7: a diskette interrupt occurred.
const FLOPPY_INTERRUPT_FLAG: u8 = 0x80;
/// BDA 40:3E bits 3:0: drives whose position is calibrated.
const RECALIBRATED_MASK: u8 = 0x0F;
/// Media state bit 4: the media geometry has been established.
const MEDIA_STATE_ESTABLISHED: u8 = 0x10;
/// Media state: 360K media in a 360K drive, established.
const MEDIA_STATE_360K_IN_360K: u8 = 0x93;
/// Media state: 360K media in a 1.2M drive, established.
const MEDIA_STATE_360K_IN_1200K: u8 = 0x74;
/// Media state: 1.2M media in a 1.2M drive, established.
const MEDIA_STATE_1200K: u8 = 0x15;
/// Media state: 720K media in a 720K drive, established.
const MEDIA_STATE_720K: u8 = 0x97;
/// Media state: media in a 1.44M drive, established.
const MEDIA_STATE_1440K: u8 = 0x17;
/// Media control byte the real AMI BIOS leaves in 40:8B after establishing
/// media, captured from the post-boot state.
const MEDIA_CONTROL_ESTABLISHED: u8 = 0x81;
/// Bits 7:6 of the media state and control bytes hold the data transfer rate.
const DATA_RATE_MASK: u8 = 0xC0;

/// FDC status register 0: abnormal termination.
const ST0_ABNORMAL_TERMINATION: u8 = 0x40;
/// FDC status register 0: seek end, set by a completed recalibrate or seek.
const ST0_SEEK_END: u8 = 0x20;
/// FDC status register 1: missing address mark.
const ST1_MISSING_ADDRESS_MARK: u8 = 0x01;
/// ST0 of the Sense Interrupt Status that ends a controller reset: the
/// invalid-command code (bits 7:6 set) with no drive selected.
const RESET_SENSE_ST0: u8 = 0xC0;

/// FDC digital output register port.
const FDC_DOR_PORT: u16 = 0x3F2;
/// FDC configuration control register port.
const FDC_CCR_PORT: u16 = 0x3F7;
/// DOR bit 2: controller out of reset.
const DOR_NOT_RESET: u8 = 0x04;
/// DOR bit 3: IRQ and DMA gates open.
const DOR_IRQ_DMA_ENABLE: u8 = 0x08;
/// DOR bit 4: drive 0 motor enable (drive 1 follows at bit 5).
const DOR_MOTOR_0: u8 = 0x10;
/// CCR data rate select: 500 kbps.
const DATA_RATE_500_KBPS: u8 = 0x00;
/// CCR data rate select: 250 kbps.
const DATA_RATE_250_KBPS: u8 = 0x02;

/// DMA channel wired to the FDC.
const FLOPPY_DMA_CHANNEL: usize = 2;
/// DMA controller 1 mode register port.
const DMA_MODE_PORT: u16 = 0x0B;
/// DMA controller 1 single mask register port.
const DMA_SINGLE_MASK_PORT: u16 = 0x0A;
/// DMA controller 1 flip-flop clear port.
const DMA_CLEAR_FLIP_FLOP_PORT: u16 = 0x0C;
/// DMA channel 2 address register port.
const DMA_CHANNEL_2_ADDRESS_PORT: u16 = 0x04;
/// DMA channel 2 count register port.
const DMA_CHANNEL_2_COUNT_PORT: u16 = 0x05;
/// DMA channel 2 page register port.
const DMA_CHANNEL_2_PAGE_PORT: u16 = 0x81;
/// Single mask register value unmasking channel 2.
const DMA_UNMASK_CHANNEL_2: u8 = 0x02;
/// DMA mode: single transfer, device to memory, channel 2.
pub(super) const DMA_MODE_DEVICE_TO_MEMORY: u8 = 0x46;
/// DMA mode: single transfer, memory to device, channel 2.
const DMA_MODE_MEMORY_TO_DEVICE: u8 = 0x4A;
/// DMA mode: single transfer, verify, channel 2.
const DMA_MODE_VERIFY: u8 = 0x42;

/// CMOS register holding the diskette drive type nibbles.
const CMOS_FLOPPY_TYPE: usize = 0x10;

/// Direction of a sector transfer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TransferKind {
    /// AH=02h: device to memory.
    Read,
    /// AH=03h: memory to device.
    Write,
    /// AH=04h: device access without a memory transfer.
    Verify,
}

/// One diskette drive type the INT 13h service can address. The geometry
/// follows the CMOS drive type, not the mounted media: media the drive type
/// cannot address (like 1.23 MB 3-mode disks with 1024-byte sectors) fails
/// sector by sector like it does on the real controller.
pub(super) struct FloppyGeometry {
    /// Cylinder count.
    cylinders: u8,
    /// Sectors per track and head.
    sectors_per_track: u8,
    /// FDC size code of the sectors.
    sector_size_code: u8,
    /// BDA 40:90/91 value after the media is established.
    media_state: u8,
    /// Configuration control register data rate select value.
    data_rate_select: u8,
}

/// 360K drive: 40 cylinders of 9 sectors at 250 kbps.
const GEOMETRY_360K: FloppyGeometry = FloppyGeometry {
    cylinders: 40,
    sectors_per_track: 9,
    sector_size_code: 2,
    media_state: MEDIA_STATE_360K_IN_360K,
    data_rate_select: DATA_RATE_250_KBPS,
};
/// 720K drive: 80 cylinders of 9 sectors at 250 kbps.
const GEOMETRY_720K: FloppyGeometry = FloppyGeometry {
    cylinders: 80,
    sectors_per_track: 9,
    sector_size_code: 2,
    media_state: MEDIA_STATE_720K,
    data_rate_select: DATA_RATE_250_KBPS,
};
/// 1.2M drive: 80 cylinders of 15 sectors at 500 kbps.
const GEOMETRY_1200K: FloppyGeometry = FloppyGeometry {
    cylinders: 80,
    sectors_per_track: 15,
    sector_size_code: 2,
    media_state: MEDIA_STATE_1200K,
    data_rate_select: DATA_RATE_500_KBPS,
};
/// 1.44M drive: 80 cylinders of 18 sectors at 500 kbps.
const GEOMETRY_1440K: FloppyGeometry = FloppyGeometry {
    cylinders: 80,
    sectors_per_track: 18,
    sector_size_code: 2,
    media_state: MEDIA_STATE_1440K,
    data_rate_select: DATA_RATE_500_KBPS,
};
/// Every drive geometry the INT 13h service can address.
const FLOPPY_GEOMETRIES: [&FloppyGeometry; 4] = [
    &GEOMETRY_360K,
    &GEOMETRY_720K,
    &GEOMETRY_1200K,
    &GEOMETRY_1440K,
];

/// Maps a CMOS drive type nibble to its geometry. Unknown types fall back
/// to the 1.44M drive like the POST default in `cmos.rs`.
fn geometry_for_drive_type(drive_type: u8) -> &'static FloppyGeometry {
    match drive_type {
        FLOPPY_TYPE_360K => &GEOMETRY_360K,
        FLOPPY_TYPE_720K => &GEOMETRY_720K,
        FLOPPY_TYPE_1200K => &GEOMETRY_1200K,
        FLOPPY_TYPE_1440K => &GEOMETRY_1440K,
        _ => &GEOMETRY_1440K,
    }
}

/// Returns whether a transfer from `address` would cross a 64 KiB DMA page.
fn dma_boundary_violated(address: u32, byte_count: usize) -> bool {
    (address & 0xFFFF) as usize + byte_count > 0x1_0000
}

impl<T: TraceSink> AtBus<T> {
    /// INT 13h diskette services dispatch, entered directly through the
    /// INT 40h alias and for DL < 80h through INT 13h. Hard disk requests
    /// reaching the diskette handler (a DL >= 80h call through INT 40h)
    /// fail with a bad command like on the real revectored BIOS.
    pub(super) fn hle_int13h_floppy(&mut self, cpu: &mut impl Cpu) {
        if cpu.dl() >= FIRST_HARD_DISK_DRIVE {
            self.set_floppy_result(cpu, STATUS_BAD_COMMAND);
            return;
        }
        match cpu.ah() {
            0x00 => self.int13h_reset(cpu),
            0x01 => self.int13h_last_status(cpu),
            0x02 => self.int13h_transfer(cpu, TransferKind::Read),
            0x03 => self.int13h_transfer(cpu, TransferKind::Write),
            0x04 => self.int13h_transfer(cpu, TransferKind::Verify),
            0x05 => self.int13h_format_track(cpu),
            0x08 => self.int13h_drive_parameters(cpu),
            0x15 => self.int13h_drive_type(cpu),
            0x16 => self.int13h_media_change(cpu),
            0x17 => self.int13h_set_dasd_type(cpu),
            0x18 => self.int13h_set_media_type(cpu),
            _ => self.set_floppy_result(cpu, STATUS_BAD_COMMAND),
        }
    }

    /// Stores the diskette status to AH, BDA 40:41 and the IRET carry flag.
    fn set_floppy_result(&mut self, cpu: &mut impl Cpu, status: u8) {
        cpu.set_ah(status);
        self.write_mem_byte(BDA_FLOPPY_STATUS, status);
        self.set_iret_cf(cpu, status != STATUS_OK);
    }

    /// AH=00h: resets the controller through a guest-visible DOR pulse,
    /// leaving every drive uncalibrated.
    fn int13h_reset(&mut self, cpu: &mut impl Cpu) {
        self.floppy_controller_reset();
        self.set_floppy_result(cpu, STATUS_OK);
    }

    /// The device side of the diskette reset, shared with the hard disk
    /// AH=00h reset (which resets the diskette system too). The FDC result
    /// bytes are the sense interrupt results of the reset drain, as the
    /// real handler leaves them.
    pub(super) fn floppy_controller_reset(&mut self) {
        self.fdc_io_write(FDC_DOR_PORT, DOR_IRQ_DMA_ENABLE);
        self.fdc_io_write(FDC_DOR_PORT, DOR_NOT_RESET | DOR_IRQ_DMA_ENABLE);
        self.fdc_reset_poll_pending = false;
        self.scheduler.cancel(EventAt::FdcInterrupt);
        self.update_next_event_cycle();

        let recalibrate = self.read_mem_byte(BDA_FLOPPY_RECALIBRATE);
        self.write_mem_byte(
            BDA_FLOPPY_RECALIBRATE,
            recalibrate & !(FLOPPY_INTERRUPT_FLAG | RECALIBRATED_MASK),
        );
        // The reset ends on a Sense Interrupt Status, whose two result bytes
        // (ST0 with the invalid-command code, present cylinder 0) land in the
        // first two FDC result bytes. The real BIOS leaves the remaining five
        // untouched, so they keep the values of the last real operation.
        self.write_mem_byte(BDA_FLOPPY_FDC_RESULT, RESET_SENSE_ST0);
        self.write_mem_byte(BDA_FLOPPY_FDC_RESULT + 1, 0x00);
    }

    /// AH=01h: returns the status of the last operation. 40:41 keeps its
    /// value; the real AMI handler does not consume it.
    fn int13h_last_status(&mut self, cpu: &mut impl Cpu) {
        let status = self.read_mem_byte(BDA_FLOPPY_STATUS);
        cpu.set_ah(status);
        self.set_iret_cf(cpu, status != STATUS_OK);
    }

    /// AH=02h/03h/04h: runs the shared transfer core. AL returns the sector
    /// count actually transferred, zero when the operation failed early.
    fn int13h_transfer(&mut self, cpu: &mut impl Cpu, kind: TransferKind) {
        let (status, transferred) = self.floppy_transfer(cpu, kind);
        cpu.set_al(transferred);
        self.set_floppy_result(cpu, status);
    }

    /// The shared read/write/verify core: validates the request, establishes
    /// the media, moves whole sectors and maintains the diskette BDA state.
    /// Returns the status byte and the sectors transferred.
    fn floppy_transfer(&mut self, cpu: &mut impl Cpu, kind: TransferKind) -> (u8, u8) {
        let drive = usize::from(cpu.dl());
        if drive >= FLOPPY_DRIVE_COUNT {
            return (STATUS_BAD_COMMAND, 0);
        }
        if !self.fdc.has_drive(drive) {
            return (STATUS_TIMEOUT, 0);
        }
        let sector_count = cpu.al();
        if sector_count == 0 {
            return (STATUS_BAD_COMMAND, 0);
        }
        if let Some(status) = self.consume_media_change(drive) {
            return (status, 0);
        }
        let (geometry, already_established) = self.establish_media(drive);
        if already_established {
            self.refresh_media_data_rate(drive);
        }
        let sectors_per_track = geometry.sectors_per_track;
        let sector_size_code = geometry.sector_size_code;
        let sector_bytes = 128usize << sector_size_code;

        if kind == TransferKind::Write && self.fdc.is_write_protected(drive) {
            return (STATUS_WRITE_PROTECT, 0);
        }

        let byte_count = usize::from(sector_count) * sector_bytes;
        let buffer_linear = cpu
            .segment_base(SegmentRegister::ES)
            .wrapping_add(u32::from(cpu.bx()));
        if dma_boundary_violated(buffer_linear, byte_count) {
            return (STATUS_DMA_BOUNDARY, 0);
        }

        self.floppy_motor_on(drive);
        let dma_mode = match kind {
            TransferKind::Read => DMA_MODE_DEVICE_TO_MEMORY,
            TransferKind::Write => DMA_MODE_MEMORY_TO_DEVICE,
            TransferKind::Verify => DMA_MODE_VERIFY,
        };
        self.program_floppy_dma(dma_mode, buffer_linear, byte_count);

        let cylinder = cpu.ch();
        let head = cpu.dh();
        let track_index = usize::from(cylinder) * 2 + usize::from(head);
        let mut transferred: Vec<u8> = Vec::with_capacity(byte_count);
        let mut record = cpu.cl();
        let mut sectors_done: u8 = 0;
        let mut status = STATUS_OK;
        while sectors_done < sector_count {
            // The controller searches the track for the requested record; a
            // record past the end of the track (or record zero) is never
            // found, so multi-sector runs stop at the end of the track.
            if record == 0 || record > sectors_per_track {
                status = STATUS_ADDRESS_MARK;
                break;
            }
            match kind {
                TransferKind::Read | TransferKind::Verify => {
                    let Some(data) = self.fdc.read_sector_data(
                        drive,
                        track_index,
                        cylinder,
                        head,
                        record,
                        sector_size_code,
                    ) else {
                        status = STATUS_ADDRESS_MARK;
                        break;
                    };
                    transferred.extend_from_slice(data);
                }
                TransferKind::Write => {
                    let offset = usize::from(sectors_done) * sector_bytes;
                    let mut sector_data = vec![0u8; sector_bytes];
                    for (index, byte) in sector_data.iter_mut().enumerate() {
                        *byte = self.read_mem_byte(buffer_linear + (offset + index) as u32);
                    }
                    let written = self.fdc.write_sector_data(
                        drive,
                        track_index,
                        cylinder,
                        head,
                        record,
                        sector_size_code,
                        &sector_data,
                    );
                    if !written {
                        status = STATUS_ADDRESS_MARK;
                        break;
                    }
                    transferred.extend_from_slice(&sector_data);
                }
            }
            sectors_done += 1;
            record += 1;
        }

        // Step the guest-visible DMA registers over the transferred bytes;
        // the data itself moves through the paging-aware accessors.
        match kind {
            TransferKind::Read => {
                let _ = self
                    .dma
                    .transfer_write_to_memory(FLOPPY_DMA_CHANNEL, &transferred);
                for (index, &byte) in transferred.iter().enumerate() {
                    self.write_mem_byte(buffer_linear + index as u32, byte);
                }
            }
            TransferKind::Verify => {
                let _ = self
                    .dma
                    .transfer_write_to_memory(FLOPPY_DMA_CHANNEL, &transferred);
            }
            TransferKind::Write => {
                let _ = self
                    .dma
                    .transfer_read_from_memory(FLOPPY_DMA_CHANNEL, transferred.len());
            }
        }

        let (st0, st1) = if status == STATUS_OK {
            ((head << 2) | drive as u8, 0)
        } else {
            (
                ST0_ABNORMAL_TERMINATION | (head << 2) | drive as u8,
                ST1_MISSING_ADDRESS_MARK,
            )
        };
        self.floppy_operation_epilogue(
            drive,
            cylinder,
            [st0, st1, 0, cylinder, head, record, sector_size_code],
        );
        (status, sectors_done)
    }

    /// AH=05h: formats one track from the caller's identifier list.
    fn int13h_format_track(&mut self, cpu: &mut impl Cpu) {
        let status = self.floppy_format_track(cpu);
        self.set_floppy_result(cpu, status);
    }

    /// Format core: validates the request and rewrites the track's sector
    /// identifiers with freshly filled data fields.
    fn floppy_format_track(&mut self, cpu: &mut impl Cpu) -> u8 {
        let drive = usize::from(cpu.dl());
        if drive >= FLOPPY_DRIVE_COUNT {
            return STATUS_BAD_COMMAND;
        }
        if !self.fdc.has_drive(drive) {
            return STATUS_TIMEOUT;
        }
        let identifier_count = cpu.al();
        if identifier_count == 0 || identifier_count > FORMAT_IDENTIFIER_LIMIT {
            return STATUS_BAD_COMMAND;
        }
        if let Some(status) = self.consume_media_change(drive) {
            return status;
        }
        let (geometry, already_established) = self.establish_media(drive);
        if already_established {
            self.refresh_media_data_rate(drive);
        }
        let sector_size_code = geometry.sector_size_code;
        if self.fdc.is_write_protected(drive) {
            return STATUS_WRITE_PROTECT;
        }

        let byte_count = usize::from(identifier_count) * 4;
        let buffer_linear = cpu
            .segment_base(SegmentRegister::ES)
            .wrapping_add(u32::from(cpu.bx()));
        if dma_boundary_violated(buffer_linear, byte_count) {
            return STATUS_DMA_BOUNDARY;
        }

        self.floppy_motor_on(drive);
        self.program_floppy_dma(DMA_MODE_MEMORY_TO_DEVICE, buffer_linear, byte_count);
        let mut identifiers = Vec::with_capacity(usize::from(identifier_count));
        for index in 0..u32::from(identifier_count) {
            let base = buffer_linear + index * 4;
            identifiers.push((
                self.read_mem_byte(base),
                self.read_mem_byte(base + 1),
                self.read_mem_byte(base + 2),
                self.read_mem_byte(base + 3),
            ));
        }
        let _ = self
            .dma
            .transfer_read_from_memory(FLOPPY_DMA_CHANNEL, byte_count);

        let cylinder = cpu.ch();
        let head = cpu.dh();
        let track_index = usize::from(cylinder) * 2 + usize::from(head);
        self.fdc.format_track(
            drive,
            track_index,
            &identifiers,
            sector_size_code,
            FORMAT_FILL_BYTE,
        );

        self.floppy_operation_epilogue(
            drive,
            cylinder,
            [
                (head << 2) | drive as u8,
                0,
                0,
                cylinder,
                head,
                1,
                sector_size_code,
            ],
        );
        STATUS_OK
    }

    /// AH=08h: returns the drive parameters from the CMOS drive type. An
    /// invalid drive number is not an error: like the real AMI handler it
    /// returns zeroed parameters with the drive count in DL and CF clear.
    fn int13h_drive_parameters(&mut self, cpu: &mut impl Cpu) {
        let drive = usize::from(cpu.dl());
        let drive_count = self.floppy_drive_count();
        let type_nibbles = self.rtc.cmos[CMOS_FLOPPY_TYPE];
        let drive_type = match drive {
            0 => type_nibbles >> 4,
            1 => type_nibbles & 0x0F,
            _ => 0,
        };
        if drive >= FLOPPY_DRIVE_COUNT || drive_type == 0 {
            cpu.set_ax(0);
            cpu.set_bx(0);
            cpu.set_cx(0);
            cpu.set_dx(u16::from(drive_count));
            cpu.load_segment_real_mode(SegmentRegister::ES, 0);
            cpu.set_di(0);
            self.set_floppy_result(cpu, STATUS_OK);
            return;
        }

        let geometry = geometry_for_drive_type(drive_type);
        cpu.set_al(0);
        cpu.set_bh(0);
        cpu.set_bl(drive_type);
        cpu.set_ch(geometry.cylinders - 1);
        cpu.set_cl(geometry.sectors_per_track);
        cpu.set_dh(1);
        cpu.set_dl(drive_count);
        let table_offset = self.stub_rom_metadata_word(METADATA_DISKETTE_PARAMETER_TABLE);
        cpu.load_segment_real_mode(SegmentRegister::ES, BIOS_CODE_SEGMENT);
        cpu.set_di(table_offset);
        self.set_floppy_result(cpu, STATUS_OK);
    }

    /// AH=15h: reports the drive type.
    fn int13h_drive_type(&mut self, cpu: &mut impl Cpu) {
        let drive = usize::from(cpu.dl());
        if drive >= FLOPPY_DRIVE_COUNT {
            self.set_floppy_result(cpu, STATUS_BAD_COMMAND);
            return;
        }
        let drive_type = if (cpu.dl()) < self.floppy_drive_count() {
            DRIVE_TYPE_FLOPPY_CHANGE_LINE
        } else {
            DRIVE_TYPE_NONE
        };
        cpu.set_ah(drive_type);
        self.write_mem_byte(BDA_FLOPPY_STATUS, STATUS_OK);
        self.set_iret_cf(cpu, false);
    }

    /// AH=16h: reports the change line without consuming the latch.
    fn int13h_media_change(&mut self, cpu: &mut impl Cpu) {
        let drive = usize::from(cpu.dl());
        if drive >= FLOPPY_DRIVE_COUNT {
            self.set_floppy_result(cpu, STATUS_BAD_COMMAND);
            return;
        }
        if self.fdc.disk_changed(drive) {
            self.set_floppy_result(cpu, STATUS_MEDIA_CHANGE);
        } else {
            self.set_floppy_result(cpu, STATUS_OK);
        }
    }

    /// AH=17h: sets the media state byte from the DASD type code.
    fn int13h_set_dasd_type(&mut self, cpu: &mut impl Cpu) {
        let drive = usize::from(cpu.dl());
        if drive >= FLOPPY_DRIVE_COUNT {
            self.set_floppy_result(cpu, STATUS_BAD_COMMAND);
            return;
        }
        if !self.fdc.has_drive(drive) {
            self.set_floppy_result(cpu, STATUS_TIMEOUT);
            return;
        }
        let media_state = match cpu.al() {
            0x01 => MEDIA_STATE_360K_IN_360K,
            0x02 => MEDIA_STATE_360K_IN_1200K,
            0x03 => MEDIA_STATE_1200K,
            0x04 => MEDIA_STATE_720K,
            _ => {
                self.set_floppy_result(cpu, STATUS_BAD_COMMAND);
                return;
            }
        };
        self.write_mem_byte(BDA_FLOPPY_MEDIA_STATE_0 + drive as u32, media_state);
        self.set_floppy_result(cpu, STATUS_OK);
    }

    /// AH=18h: sets the media type for format from the track and sector
    /// counts, returning the parameter table like AH=08h.
    fn int13h_set_media_type(&mut self, cpu: &mut impl Cpu) {
        let drive = usize::from(cpu.dl());
        if drive >= FLOPPY_DRIVE_COUNT {
            self.set_floppy_result(cpu, STATUS_BAD_COMMAND);
            return;
        }
        if !self.fdc.has_drive(drive) {
            self.set_floppy_result(cpu, STATUS_TIMEOUT);
            return;
        }
        let cylinders_minus_one = cpu.ch();
        let sectors_per_track = cpu.cl();
        let matched = FLOPPY_GEOMETRIES.iter().find(|geometry| {
            geometry.cylinders - 1 == cylinders_minus_one
                && geometry.sectors_per_track == sectors_per_track
        });
        match matched {
            Some(geometry) => {
                self.write_mem_byte(
                    BDA_FLOPPY_MEDIA_STATE_0 + drive as u32,
                    geometry.media_state,
                );
                let table_offset = self.stub_rom_metadata_word(METADATA_DISKETTE_PARAMETER_TABLE);
                cpu.load_segment_real_mode(SegmentRegister::ES, BIOS_CODE_SEGMENT);
                cpu.set_di(table_offset);
                self.set_floppy_result(cpu, STATUS_OK);
            }
            None => self.set_floppy_result(cpu, STATUS_UNSUPPORTED_COMBINATION),
        }
    }

    /// Reports and consumes a pending media change, clearing the established
    /// bit so the next operation re-establishes the media. Clearing the latch
    /// steps the drive with a recalibrate, whose Sense Interrupt Status (seek
    /// end, present cylinder 0) is left in the first two FDC result bytes.
    fn consume_media_change(&mut self, drive: usize) -> Option<u8> {
        if !self.fdc.disk_changed(drive) {
            return None;
        }
        self.fdc.clear_disk_change_on_step(drive);
        let media_state_address = BDA_FLOPPY_MEDIA_STATE_0 + drive as u32;
        let media_state = self.read_mem_byte(media_state_address);
        self.write_mem_byte(media_state_address, media_state & !MEDIA_STATE_ESTABLISHED);
        self.write_mem_byte(BDA_FLOPPY_FDC_RESULT, ST0_SEEK_END | drive as u8);
        self.write_mem_byte(BDA_FLOPPY_FDC_RESULT + 1, 0x00);
        Some(STATUS_MEDIA_CHANGE)
    }

    /// Establishes the media of `drive` from its CMOS drive type: programs
    /// the data rate and writes the media state and control bytes on first
    /// use. Media the drive type cannot address fails at the sector level.
    /// Returns the geometry and whether the media was already established
    /// before this call.
    pub(super) fn establish_media(&mut self, drive: usize) -> (&'static FloppyGeometry, bool) {
        let type_nibbles = self.rtc.cmos[CMOS_FLOPPY_TYPE];
        let drive_type = if drive == 0 {
            type_nibbles >> 4
        } else {
            type_nibbles & 0x0F
        };
        let geometry = geometry_for_drive_type(drive_type);
        let media_state_address = BDA_FLOPPY_MEDIA_STATE_0 + drive as u32;
        let already_established =
            self.read_mem_byte(media_state_address) & MEDIA_STATE_ESTABLISHED != 0;
        if !already_established {
            self.fdc_io_write(FDC_CCR_PORT, geometry.data_rate_select);
            self.write_mem_byte(media_state_address, geometry.media_state);
            self.write_mem_byte(BDA_FLOPPY_MEDIA_CONTROL, MEDIA_CONTROL_ESTABLISHED);
        }
        (geometry, already_established)
    }

    /// Refreshes 40:8B for an operation on already established media, taking
    /// the data rate bits from the media state and keeping the low six bits.
    /// The real BIOS runs this on every access once the media is known, which
    /// is why 40:8B settles from its post-boot value to the operating rate.
    fn refresh_media_data_rate(&mut self, drive: usize) {
        let media_state = self.read_mem_byte(BDA_FLOPPY_MEDIA_STATE_0 + drive as u32);
        let control = self.read_mem_byte(BDA_FLOPPY_MEDIA_CONTROL);
        let refreshed = (control & !DATA_RATE_MASK) | (media_state & DATA_RATE_MASK);
        self.write_mem_byte(BDA_FLOPPY_MEDIA_CONTROL, refreshed);
    }

    /// Selects `drive` in the DOR with its motor running and mirrors the
    /// motor state into the BDA.
    fn floppy_motor_on(&mut self, drive: usize) {
        let dor_value = DOR_NOT_RESET | DOR_IRQ_DMA_ENABLE | (DOR_MOTOR_0 << drive) | drive as u8;
        self.fdc_io_write(FDC_DOR_PORT, dor_value);
        self.write_mem_byte(BDA_FLOPPY_MOTOR, 1 << drive);
    }

    /// Programs DMA channel 2 through the guest-visible ports the way the
    /// real INT 13h handler does before a transfer.
    pub(super) fn program_floppy_dma(&mut self, mode: u8, address: u32, byte_count: usize) {
        self.io_write(DMA_MODE_PORT, mode);
        self.io_write(DMA_SINGLE_MASK_PORT, DMA_UNMASK_CHANNEL_2);
        self.io_write(DMA_CLEAR_FLIP_FLOP_PORT, 0x00);
        self.io_write(DMA_CHANNEL_2_ADDRESS_PORT, address as u8);
        self.io_write(DMA_CHANNEL_2_ADDRESS_PORT, (address >> 8) as u8);
        let count = byte_count.saturating_sub(1);
        self.io_write(DMA_CHANNEL_2_COUNT_PORT, count as u8);
        self.io_write(DMA_CHANNEL_2_COUNT_PORT, (count >> 8) as u8);
        self.io_write(DMA_CHANNEL_2_PAGE_PORT, (address >> 16) as u8);
    }

    /// Arms the motor shutoff counter, marks the drive calibrated with the
    /// interrupt flag consumed, and stores the FDC result bytes and the
    /// seeked track.
    fn floppy_operation_epilogue(&mut self, drive: usize, cylinder: u8, result_bytes: [u8; 7]) {
        self.write_mem_byte(BDA_FLOPPY_MOTOR_COUNT, FLOPPY_MOTOR_TICKS);
        let recalibrate = self.read_mem_byte(BDA_FLOPPY_RECALIBRATE);
        self.write_mem_byte(
            BDA_FLOPPY_RECALIBRATE,
            (recalibrate | (1 << drive)) & !FLOPPY_INTERRUPT_FLAG,
        );
        for (offset, value) in result_bytes.iter().enumerate() {
            self.write_mem_byte(BDA_FLOPPY_FDC_RESULT + offset as u32, *value);
        }
        self.write_mem_byte(BDA_FLOPPY_TRACK_0 + drive as u32, cylinder);
    }

    /// Returns the diskette drive count from the BDA equipment word.
    fn floppy_drive_count(&mut self) -> u8 {
        let equipment = self.read_mem_word(BDA_EQUIPMENT);
        if equipment & EQUIPMENT_FLOPPY_INSTALLED == 0 {
            0
        } else {
            ((equipment >> 6) as u8 & 0x03) + 1
        }
    }
}
