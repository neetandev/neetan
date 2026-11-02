//! HLE disk BIOS at `FFFB:0014`.
//!
//! The register convention equals `INT 93h`: `AH` selects the function and
//! `AL` the device (high nibble class, low nibble unit). The BIOS returns the
//! status in `AH` and sets CF when it is non-zero. A hard error (80h) returns
//! its detail bits in `CX`. Reads and writes access the media through the
//! device models directly and transfer the data to and from `DS:DI` as a
//! linear address without a 64 KiB wrap.

use common::{Cpu, SegmentRegister, TraceSink};
use device::cdrom_towns::CdDriveCheck;

use crate::bus::{
    TownsBus,
    scsi_command::{SCSI_GOOD, SCSI_READ10, SCSI_WRITE10, scsi_transfer_command},
};

/// CMOS port where the disk BIOS saves the caller SS.
const CMOS_SAVED_SS: u16 = 0x319C;
/// CMOS port where the disk BIOS saves the caller SP.
const CMOS_SAVED_SP: u16 = 0x31A0;

/// Device class of the floppy drives.
const CLASS_FLOPPY: u8 = 0x2;
/// Device class of the physical memory windows.
const CLASS_MEMORY: u8 = 0x4;
/// Device class of the SCSI hard disks.
const CLASS_SCSI: u8 = 0xB;
/// Device code of the CD-ROM drive.
pub(super) const DEVICE_CDROM: u8 = 0xC0;

/// Status: undefined function.
const STATUS_UNDEFINED: u8 = 0x01;
/// Status: device number, drive mode or head number error.
const STATUS_DEVICE: u8 = 0x02;
/// Status: hard error with the detail in CX.
const STATUS_HARD_ERROR: u8 = 0x80;

/// Detail bit: drive not ready.
const DETAIL_NOT_READY: u16 = 0x0001;
/// Detail bit: parameter error (CD-ROM, SCSI, memory windows).
const DETAIL_PARAMETER: u16 = 0x0002;
/// Detail bit: write protected (floppy, memory windows).
const DETAIL_WRITE_PROTECTED: u16 = 0x0002;
/// Detail bit: device not connected (CD-ROM, SCSI).
const DETAIL_NOT_CONNECTED: u16 = 0x0004;
/// Detail bit: record not found (floppy).
const DETAIL_RECORD_NOT_FOUND: u16 = 0x0008;
/// Detail bit: CRC error (floppy) or media error (CD-ROM, SCSI).
const DETAIL_MEDIA_ERROR: u16 = 0x0010;
/// Detail bit: media changed (CD-ROM).
const DETAIL_MEDIA_CHANGED: u16 = 0x0080;

/// CD-ROM drive mode code for 2048-byte sectors.
const CD_MODE_2048: u8 = 0x04;
/// CD-ROM user data bytes per sector.
const CD_SECTOR_BYTES: usize = 2048;
/// `CH` value that selects a 32-bit physical buffer address in `SI:DI`.
const CD_EXTENDED_BUFFER: u8 = 0xFF;
/// CD-ROM frames per second.
const CD_FRAMES_PER_SECOND: u32 = 75;
/// CD-ROM frames before logical sector 0.
const CD_LEAD_IN_FRAMES: u32 = 150;

/// Number of floppy drives the BIOS addresses.
const FLOPPY_DRIVES: usize = 4;
/// Default floppy drive mode 1: MFM, 2HD, 1024-byte sectors.
pub(super) const FLOPPY_DEFAULT_MODE: u8 = 0x03;
/// Default floppy drive mode 2: two heads, eight sectors per track.
pub(super) const FLOPPY_DEFAULT_GEOMETRY: u16 = 0x0208;
/// Floppy drive status bit: not ready.
const FLOPPY_STATUS_NOT_READY: u8 = 0x01;
/// Floppy drive status bit: write protected.
const FLOPPY_STATUS_WRITE_PROTECTED: u8 = 0x02;
/// Floppy drive status bit: double sided.
const FLOPPY_STATUS_DOUBLE_SIDED: u8 = 0x04;

/// Highest SCSI ID the BIOS addresses.
const SCSI_LAST_ID: u8 = 6;
/// SCSI READ CAPACITY operation code.
const SCSI_READ_CAPACITY: u8 = 0x25;
/// Blocks transferred per SCSI command.
const SCSI_CHUNK_BLOCKS: u16 = 64;
/// SCSI drive mode code for 512-byte blocks.
const SCSI_MODE_512: u8 = 0x02;

/// Bytes per block of the memory windows.
const MEMORY_BLOCK_BYTES: u64 = 1024;

/// Outcome of a disk BIOS function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiskStatus {
    /// Success.
    Ok,
    /// Undefined function.
    Undefined,
    /// Device number, drive mode or head number error.
    Device,
    /// Hard error with the detail returned in CX.
    HardError(u16),
}

/// Direction of a sector transfer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Transfer {
    /// Medium to memory.
    Read,
    /// Memory to medium.
    Write,
    /// Read without storing the data.
    Verify,
}

/// A physical memory window of the class 4 devices.
struct MemoryWindow {
    /// Physical base address.
    base: u32,
    /// Window size in bytes.
    size: u64,
    /// Writes are rejected.
    write_protected: bool,
    /// The window needs an IC memory card.
    needs_card: bool,
}

/// Returns the physical memory window of class 4 unit `unit`.
fn memory_window(unit: u8) -> Option<MemoryWindow> {
    let window = |base: u32, size: u64, write_protected: bool| MemoryWindow {
        base,
        size,
        write_protected,
        needs_card: false,
    };
    match unit {
        0x0 => Some(window(0x0000_0000, 0x8000_0000, false)),
        0x2 | 0x9 => Some(window(0xC200_0000, 0x8_0000, true)),
        0x7 | 0x8 => Some(window(0x0000_0000, 0x1_0000_0000, false)),
        0xA => Some(MemoryWindow {
            needs_card: true,
            ..window(0xC000_0000, 0x8_0000, false)
        }),
        0xB => Some(window(0xFFFC_0000, 0x4_0000, true)),
        0xC => Some(window(0x8000_0000, 0x8_0000, false)),
        0xD => Some(window(0x8010_0000, 0x8_0000, false)),
        0xE => Some(window(0xC214_0000, 0x2000, false)),
        _ => None,
    }
}

/// Returns the linear address of the `DS:DI` buffer.
fn buffer_address(cpu: &impl Cpu) -> u32 {
    cpu.segment_base(SegmentRegister::DS)
        .wrapping_add(u32::from(cpu.di()))
}

impl<T: TraceSink> TownsBus<T> {
    /// Runs the disk BIOS (vector F1h).
    pub(super) fn hle_disk_bios(&mut self, cpu: &mut impl Cpu) {
        self.set_cmos_port_word(CMOS_SAVED_SS, cpu.ss());
        self.set_cmos_port_word(CMOS_SAVED_SP, cpu.sp().wrapping_sub(4));

        let device = cpu.al();
        let unit = device & 0x0F;
        let status = match device >> 4 {
            CLASS_FLOPPY => self.floppy_bios(cpu, unit),
            CLASS_MEMORY => self.memory_window_bios(cpu, unit),
            CLASS_SCSI => self.scsi_bios(cpu, unit),
            _ if device == DEVICE_CDROM => self.cdrom_bios(cpu),
            _ => DiskStatus::Device,
        };
        Self::apply_disk_status(cpu, status);
    }

    /// Stores `status` in AH, CF and CX.
    fn apply_disk_status(cpu: &mut impl Cpu, status: DiskStatus) {
        match status {
            DiskStatus::Ok => Self::set_bios_status(cpu, 0),
            DiskStatus::Undefined => Self::set_bios_status(cpu, STATUS_UNDEFINED),
            DiskStatus::Device => Self::set_bios_status(cpu, STATUS_DEVICE),
            DiskStatus::HardError(detail) => {
                cpu.set_cx(detail);
                Self::set_bios_status(cpu, STATUS_HARD_ERROR);
            }
        }
    }

    /// Runs a CD-ROM function (device C0h).
    fn cdrom_bios(&mut self, cpu: &mut impl Cpu) -> DiskStatus {
        match cpu.ah() {
            0x00 | 0x52 => DiskStatus::Ok,
            0x01 => {
                cpu.set_dl(CD_MODE_2048);
                DiskStatus::Ok
            }
            0x02 => {
                let status = self.cdrom_ready();
                if status == DiskStatus::Ok {
                    let last = self.cdc.total_sectors().saturating_sub(1);
                    cpu.set_al(CD_MODE_2048);
                    cpu.set_bh(0);
                    cpu.set_bl((last >> 16) as u8);
                    cpu.set_dx(last as u16);
                }
                status
            }
            0x03 | 0x04 | 0x0E | 0x14 => self.cdrom_ready(),
            0x05 => {
                let block = (u32::from(cpu.cl()) << 16) | u32::from(cpu.dx());
                self.cdrom_read(cpu, block)
            }
            0x15 => {
                let frames = (u32::from(cpu.cl()) * 60 + u32::from(cpu.dh()))
                    * CD_FRAMES_PER_SECOND
                    + u32::from(cpu.dl());
                self.cdrom_read(cpu, frames.saturating_sub(CD_LEAD_IN_FRAMES))
            }
            _ => DiskStatus::Undefined,
        }
    }

    /// Checks the CD-ROM drive like the drive check function 0Eh.
    pub(super) fn cdrom_ready(&mut self) -> DiskStatus {
        match self.cdc.firmware_drive_check() {
            CdDriveCheck::Ready => DiskStatus::Ok,
            CdDriveCheck::NotReady => DiskStatus::HardError(DETAIL_NOT_READY),
            CdDriveCheck::DiscChanged => DiskStatus::HardError(DETAIL_MEDIA_CHANGED),
        }
    }

    /// Reads `BX` CD-ROM sectors from logical sector `block` to the buffer.
    /// `CH = FFh` selects the 32-bit physical buffer address `SI:DI`.
    fn cdrom_read(&mut self, cpu: &mut impl Cpu, block: u32) -> DiskStatus {
        let ready = self.cdrom_ready();
        if ready != DiskStatus::Ok {
            return ready;
        }
        let count = cpu.bx();
        if count == 0 {
            return DiskStatus::HardError(DETAIL_PARAMETER);
        }
        let mut address = if cpu.ch() == CD_EXTENDED_BUFFER {
            (u32::from(cpu.si()) << 16) | u32::from(cpu.di())
        } else {
            buffer_address(cpu)
        };
        let mut sector = [0u8; CD_SECTOR_BYTES];
        for index in 0..count {
            if !self
                .cdc
                .read_user_data_direct(block.wrapping_add(u32::from(index)), &mut sector)
            {
                cpu.set_bx(count - index);
                return DiskStatus::HardError(DETAIL_MEDIA_ERROR);
            }
            self.write_buffer(address, &sector);
            address = address.wrapping_add(CD_SECTOR_BYTES as u32);
        }
        cpu.set_bx(0);
        DiskStatus::Ok
    }

    /// Reads one CD-ROM sector to `address` for the boot device search.
    pub(super) fn cdrom_read_sector_to(&mut self, block: u32, address: u32) -> bool {
        let mut sector = [0u8; CD_SECTOR_BYTES];
        if !self.cdc.read_user_data_direct(block, &mut sector) {
            return false;
        }
        self.write_buffer(address, &sector);
        true
    }

    /// Copies `bytes` to physical memory at `address`.
    fn write_buffer(&mut self, address: u32, bytes: &[u8]) {
        for (offset, &byte) in bytes.iter().enumerate() {
            self.hle_write_byte(address.wrapping_add(offset as u32), byte);
        }
    }

    /// Copies `length` bytes of physical memory at `address`.
    fn read_buffer(&mut self, address: u32, length: usize) -> Vec<u8> {
        (0..length)
            .map(|offset| self.hle_read_byte(address.wrapping_add(offset as u32)))
            .collect()
    }

    /// Runs a floppy function (devices 20h-23h).
    fn floppy_bios(&mut self, cpu: &mut impl Cpu, unit: u8) -> DiskStatus {
        let drive = usize::from(unit);
        if drive >= FLOPPY_DRIVES {
            return DiskStatus::Device;
        }
        match cpu.ah() {
            0x00 => {
                self.bios_state.floppy_modes[drive] = cpu.dl();
                self.bios_state.floppy_geometries[drive] = cpu.bx();
                DiskStatus::Ok
            }
            0x01 => {
                cpu.set_dl(self.bios_state.floppy_modes[drive]);
                cpu.set_bx(self.bios_state.floppy_geometries[drive]);
                DiskStatus::Ok
            }
            0x02 => {
                let mut status = FLOPPY_STATUS_DOUBLE_SIDED;
                if !self.fdc.drive_has_media(drive) {
                    status |= FLOPPY_STATUS_NOT_READY;
                }
                if self.fdc.drive_write_protected(drive) {
                    status |= FLOPPY_STATUS_WRITE_PROTECTED;
                }
                cpu.set_dl(status);
                DiskStatus::Ok
            }
            0x03 | 0x04 => {
                if !self.fdc.drive_has_media(drive) {
                    return DiskStatus::HardError(DETAIL_NOT_READY);
                }
                let cylinder = if cpu.ah() == 0x03 { 0 } else { cpu.cx() };
                let Ok(cylinder) = u8::try_from(cylinder) else {
                    return DiskStatus::Device;
                };
                self.fdc.position_head_direct(drive, cylinder, 0);
                DiskStatus::Ok
            }
            0x05 => self.floppy_transfer(cpu, drive, Transfer::Read),
            0x06 => self.floppy_transfer(cpu, drive, Transfer::Write),
            0x07 => self.floppy_transfer(cpu, drive, Transfer::Verify),
            0x08 => DiskStatus::Ok,
            0x09 => self.floppy_read_id(cpu, drive),
            _ => DiskStatus::Undefined,
        }
    }

    /// Transfers `BX` floppy sectors starting at cylinder `CX`, head `DH` and
    /// sector `DL`. The sector number counts up to the end of the track, then
    /// the head, then the cylinder. `BX` returns the sectors not transferred.
    fn floppy_transfer(
        &mut self,
        cpu: &mut impl Cpu,
        drive: usize,
        transfer: Transfer,
    ) -> DiskStatus {
        let count = cpu.bx();
        if count == 0 {
            return DiskStatus::Ok;
        }
        let (Ok(mut cylinder), mut head, mut record) = (u8::try_from(cpu.cx()), cpu.dh(), cpu.dl())
        else {
            return DiskStatus::Device;
        };
        if head > 1 {
            return DiskStatus::Device;
        }
        if !self.fdc.drive_has_media(drive) {
            return DiskStatus::HardError(DETAIL_NOT_READY);
        }
        if transfer == Transfer::Write && self.fdc.drive_write_protected(drive) {
            return DiskStatus::HardError(DETAIL_WRITE_PROTECTED);
        }
        let mut address = buffer_address(cpu);
        for index in 0..count {
            self.fdc.position_head_direct(drive, cylinder, head);
            let failure = match transfer {
                Transfer::Read | Transfer::Verify => {
                    match self.fdc.read_sector_direct(drive, cylinder, head, record) {
                        None => Some(DETAIL_RECORD_NOT_FOUND),
                        Some((data, crc_error)) => {
                            if transfer == Transfer::Read {
                                self.write_buffer(address, &data);
                            }
                            address = address.wrapping_add(data.len() as u32);
                            crc_error.then_some(DETAIL_MEDIA_ERROR)
                        }
                    }
                }
                Transfer::Write => {
                    match self
                        .fdc
                        .sector_size_code_direct(drive, cylinder, head, record)
                    {
                        None => Some(DETAIL_RECORD_NOT_FOUND),
                        Some(size_code) => {
                            let length = 128usize << size_code.min(7);
                            let data = self.read_buffer(address, length);
                            address = address.wrapping_add(length as u32);
                            (!self
                                .fdc
                                .write_sector_direct(drive, cylinder, head, record, &data))
                            .then_some(DETAIL_RECORD_NOT_FOUND)
                        }
                    }
                }
            };
            if let Some(detail) = failure {
                cpu.set_bx(count - index);
                return DiskStatus::HardError(detail);
            }
            let sectors_per_track = self.floppy_sectors_per_track(drive, cylinder, head);
            record = record.wrapping_add(1);
            if usize::from(record) > sectors_per_track {
                record = 1;
                head += 1;
                if head > 1 {
                    head = 0;
                    cylinder = cylinder.wrapping_add(1);
                }
            }
        }
        cpu.set_bx(0);
        DiskStatus::Ok
    }

    /// Returns the sectors per track of `drive`: the count of the medium, or
    /// the drive mode when the track is empty.
    fn floppy_sectors_per_track(&self, drive: usize, cylinder: u8, head: u8) -> usize {
        match self.fdc.drive_sector_count(drive, cylinder, head) {
            0 => usize::from(self.bios_state.floppy_geometries[drive] as u8),
            count => count,
        }
    }

    /// Function 09h: writes the ID of sector 1 of cylinder `CX`, head `DH` to
    /// the buffer: track, head, sector, size code and two CRC bytes.
    fn floppy_read_id(&mut self, cpu: &mut impl Cpu, drive: usize) -> DiskStatus {
        if !self.fdc.drive_has_media(drive) {
            return DiskStatus::HardError(DETAIL_NOT_READY);
        }
        let Ok(cylinder) = u8::try_from(cpu.cx()) else {
            return DiskStatus::Device;
        };
        let head = cpu.dh();
        let Some(size_code) = self.fdc.sector_size_code_direct(drive, cylinder, head, 1) else {
            return DiskStatus::HardError(DETAIL_RECORD_NOT_FOUND);
        };
        let address = buffer_address(cpu);
        self.write_buffer(address, &[cylinder, head, 1, size_code, 0, 0]);
        DiskStatus::Ok
    }

    /// Reads the first two floppy sectors of `drive` to `address` for the boot
    /// device search, as the real system ROM does. Fails without sector 1.
    pub(super) fn floppy_read_boot_sectors_to(&mut self, drive: usize, address: u32) -> bool {
        let Some((first, _)) = self.fdc.read_sector_direct(drive, 0, 0, 1) else {
            return false;
        };
        self.fdc.position_head_direct(drive, 0, 0);
        self.write_buffer(address, &first);
        if let Some((second, _)) = self.fdc.read_sector_direct(drive, 0, 0, 2) {
            self.write_buffer(address.wrapping_add(first.len() as u32), &second);
        }
        true
    }

    /// Runs a SCSI hard disk function (devices B0h-B6h).
    fn scsi_bios(&mut self, cpu: &mut impl Cpu, unit: u8) -> DiskStatus {
        if unit > SCSI_LAST_ID {
            return DiskStatus::Device;
        }
        let target = usize::from(unit);
        if !self.scsi.has_target(target) {
            return DiskStatus::HardError(DETAIL_NOT_CONNECTED);
        }
        match cpu.ah() {
            0x02 => match self.scsi_capacity(target) {
                Some((last_block, _)) => {
                    cpu.set_al(SCSI_MODE_512);
                    cpu.set_bx((last_block >> 16) as u16);
                    cpu.set_dx(last_block as u16);
                    DiskStatus::Ok
                }
                None => DiskStatus::HardError(DETAIL_NOT_READY),
            },
            0x03 | 0x08 | 0x0E => DiskStatus::Ok,
            0x05 => self.scsi_transfer(cpu, target, Transfer::Read),
            0x06 => self.scsi_transfer(cpu, target, Transfer::Write),
            0x07 => self.scsi_transfer(cpu, target, Transfer::Verify),
            _ => DiskStatus::Undefined,
        }
    }

    /// Returns the last block number and the block length of SCSI `target`.
    fn scsi_capacity(&mut self, target: usize) -> Option<(u32, usize)> {
        let command = [SCSI_READ_CAPACITY, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let (data, status) = self.scsi.execute_data_in_direct(target, &command)?;
        if status != SCSI_GOOD || data.len() < 8 {
            return None;
        }
        let last_block = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let block_length = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        Some((last_block, block_length as usize))
    }

    /// Transfers `BX` blocks of SCSI `target` starting at block `CL:DX`. `BX`
    /// returns the blocks not transferred.
    fn scsi_transfer(
        &mut self,
        cpu: &mut impl Cpu,
        target: usize,
        transfer: Transfer,
    ) -> DiskStatus {
        let count = cpu.bx();
        let mut block = (u32::from(cpu.cl()) << 16) | u32::from(cpu.dx());
        let Some((last_block, block_length)) = self.scsi_capacity(target) else {
            return DiskStatus::HardError(DETAIL_NOT_READY);
        };
        if u64::from(block) + u64::from(count) > u64::from(last_block) + 1 {
            return DiskStatus::HardError(DETAIL_PARAMETER);
        }
        let mut address = buffer_address(cpu);
        let mut remaining = count;
        while remaining > 0 {
            let chunk = remaining.min(SCSI_CHUNK_BLOCKS);
            let chunk_bytes = usize::from(chunk) * block_length;
            let success = match transfer {
                Transfer::Read | Transfer::Verify => {
                    let command = scsi_transfer_command(SCSI_READ10, block, chunk);
                    match self.scsi.execute_data_in_direct(target, &command) {
                        Some((data, SCSI_GOOD)) if data.len() >= chunk_bytes => {
                            if transfer == Transfer::Read {
                                self.write_buffer(address, &data[..chunk_bytes]);
                            }
                            true
                        }
                        _ => false,
                    }
                }
                Transfer::Write => {
                    let data = self.read_buffer(address, chunk_bytes);
                    let command = scsi_transfer_command(SCSI_WRITE10, block, chunk);
                    self.scsi.execute_data_out_direct(target, &command, &data) == Some(SCSI_GOOD)
                }
            };
            if !success {
                cpu.set_bx(remaining);
                return DiskStatus::HardError(DETAIL_MEDIA_ERROR);
            }
            block = block.wrapping_add(u32::from(chunk));
            address = address.wrapping_add(chunk_bytes as u32);
            remaining -= chunk;
        }
        cpu.set_bx(0);
        DiskStatus::Ok
    }

    /// Reads the first `length` bytes of SCSI `target` to `address` for the
    /// boot device search.
    pub(super) fn scsi_read_boot_blocks_to(
        &mut self,
        target: usize,
        address: u32,
        blocks: u16,
    ) -> bool {
        let command = scsi_transfer_command(SCSI_READ10, 0, blocks);
        match self.scsi.execute_data_in_direct(target, &command) {
            Some((data, SCSI_GOOD)) => {
                self.write_buffer(address, &data);
                true
            }
            _ => false,
        }
    }

    /// Runs a function of the class 4 physical memory windows (devices 40h-4Fh).
    fn memory_window_bios(&mut self, cpu: &mut impl Cpu, unit: u8) -> DiskStatus {
        let Some(window) = memory_window(unit) else {
            return DiskStatus::Device;
        };
        if window.needs_card {
            return DiskStatus::HardError(DETAIL_NOT_READY);
        }
        match cpu.ah() {
            0x03 | 0x08 | 0x0E => DiskStatus::Ok,
            0x05 => self.memory_window_transfer(cpu, &window, Transfer::Read),
            0x06 => self.memory_window_transfer(cpu, &window, Transfer::Write),
            _ => DiskStatus::Undefined,
        }
    }

    /// Copies `BX` KiB between the buffer and the window position
    /// `(CX:DX) shl 10`. The full `CX` word takes part in the address.
    fn memory_window_transfer(
        &mut self,
        cpu: &mut impl Cpu,
        window: &MemoryWindow,
        transfer: Transfer,
    ) -> DiskStatus {
        let position = ((u64::from(cpu.cx()) << 16) | u64::from(cpu.dx())) * MEMORY_BLOCK_BYTES;
        let length = u64::from(cpu.bx()) * MEMORY_BLOCK_BYTES;
        if transfer == Transfer::Write && window.write_protected {
            return DiskStatus::HardError(DETAIL_WRITE_PROTECTED);
        }
        if window.size < (1 << 32) && position + length > window.size {
            return DiskStatus::HardError(DETAIL_PARAMETER);
        }
        let physical = window.base.wrapping_add(position as u32);
        let buffer = buffer_address(cpu);
        for offset in 0..length as u32 {
            match transfer {
                Transfer::Read => {
                    let value = self.hle_read_byte(physical.wrapping_add(offset));
                    self.hle_write_byte(buffer.wrapping_add(offset), value);
                }
                Transfer::Write => {
                    let value = self.hle_read_byte(buffer.wrapping_add(offset));
                    self.hle_write_byte(physical.wrapping_add(offset), value);
                }
                Transfer::Verify => {}
            }
        }
        DiskStatus::Ok
    }

    /// Reads one KiB of the memory window of class 4 unit `unit` at position 0
    /// to `address` for the boot device search.
    pub(super) fn memory_window_read_boot_block_to(&mut self, unit: u8, address: u32) -> bool {
        let Some(window) = memory_window(unit) else {
            return false;
        };
        if window.needs_card {
            return false;
        }
        let data = self.read_buffer(window.base, MEMORY_BLOCK_BYTES as usize);
        self.write_buffer(address, &data);
        true
    }
}

#[cfg(test)]
mod tests {
    use common::{Cpu, SegmentRegister};

    use super::*;
    use crate::bus::bios::{
        VECTOR_DISK_BIOS,
        tests::{TestCpu, hle_bus, real_mode_cpu, trap},
    };

    /// Physical address of the transfer buffer in the tests.
    const BUFFER: u32 = 0x4000;

    /// Calls the disk BIOS with the given registers and DS:DI = 0400:0000.
    fn call(bus: &mut TownsBus, ax: u16, bx: u16, cx: u16, dx: u16) -> TestCpu {
        let mut cpu = real_mode_cpu();
        cpu.load_segment_real_mode(SegmentRegister::DS, (BUFFER >> 4) as u16);
        cpu.set_di(0);
        cpu.set_ax(ax);
        cpu.set_bx(bx);
        cpu.set_cx(cx);
        cpu.set_dx(dx);
        trap(bus, &mut cpu, VECTOR_DISK_BIOS);
        cpu
    }

    /// Builds a raw 2048-byte-sector CD image whose sector `n` is filled with `n`.
    fn cd_image(sectors: u8) -> device::cdrom::CdImage {
        let mut data = Vec::new();
        for sector in 0..sectors {
            data.extend(std::iter::repeat_n(sector, 2048));
        }
        let cue = "FILE \"disc.iso\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n";
        device::cdrom::CdImage::from_cue(cue, data).expect("cd image")
    }

    /// Builds a 2HD floppy (77 cylinders, 2 heads, 8 sectors of 1024 bytes)
    /// whose sector bytes hold the low byte of the linear sector number.
    fn floppy_image() -> device::floppy::FloppyImage {
        let mut data = Vec::new();
        for sector in 0..(77 * 2 * 8) {
            data.extend(std::iter::repeat_n(sector as u8, 1024));
        }
        device::floppy::FloppyImage::from_hdm_bytes(&data).expect("floppy image")
    }

    #[test]
    fn cdrom_read_copies_sectors_and_clears_bx() {
        let mut bus = hle_bus();
        bus.insert_cdrom(cd_image(4));
        let cpu = call(&mut bus, 0x03C0, 0, 0, 0);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_MEDIA_CHANGED);
        let cpu = call(&mut bus, 0x03C0, 0, 0, 0);
        assert_eq!(cpu.ah(), 0);

        let cpu = call(&mut bus, 0x05C0, 2, 0, 1);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(cpu.flags() & 1, 0);
        assert_eq!(cpu.bx(), 0);
        assert_eq!(cpu.al(), 0xC0);
        assert_eq!(bus.peek_byte(BUFFER), 1);
        assert_eq!(bus.peek_byte(BUFFER + 2048), 2);
    }

    #[test]
    fn cdrom_errors_report_detail() {
        let mut bus = hle_bus();
        let cpu = call(&mut bus, 0x0EC0, 0, 0, 0);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_NOT_READY);
        assert_eq!(cpu.flags() & 1, 1);

        bus.insert_cdrom(cd_image(2));
        let _ = call(&mut bus, 0x0EC0, 0, 0, 0);
        let cpu = call(&mut bus, 0x05C0, 0, 0, 0);
        assert_eq!(cpu.cx(), DETAIL_PARAMETER);
        let cpu = call(&mut bus, 0x05C0, 4, 0, 0);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_MEDIA_ERROR);
        assert_eq!(cpu.bx(), 2);
    }

    #[test]
    fn floppy_read_walks_sector_head_and_cylinder() {
        let mut bus = hle_bus();
        bus.insert_floppy_backed(0, floppy_image(), common::MediaBacking::ReadOnly);
        let cpu = call(&mut bus, 0x0320, 0, 0, 0);
        assert_eq!(cpu.ah(), 0);

        let cpu = call(&mut bus, 0x0520, 10, 0, 0x0007);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(cpu.bx(), 0);
        assert_eq!(bus.peek_byte(BUFFER), 6);
        assert_eq!(bus.peek_byte(BUFFER + 1024), 7);
        assert_eq!(bus.peek_byte(BUFFER + 2 * 1024), 8);
        assert_eq!(bus.peek_byte(BUFFER + 9 * 1024), 15);

        let cpu = call(&mut bus, 0x0520, 1, 0, 0x0109);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_RECORD_NOT_FOUND);
        assert_eq!(cpu.bx(), 1);
    }

    #[test]
    fn floppy_without_media_is_not_ready() {
        let mut bus = hle_bus();
        let cpu = call(&mut bus, 0x0521, 1, 0, 1);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_NOT_READY);
        let cpu = call(&mut bus, 0x0524, 1, 0, 1);
        assert_eq!(cpu.ah(), STATUS_DEVICE);
    }

    #[test]
    fn memory_window_unit_8_copies_sprite_ram() {
        let mut bus = hle_bus();
        bus.poke_byte(0x8100_0000, 0x12);
        bus.poke_byte(0x8100_07FF, 0x34);
        let cpu = call(&mut bus, 0x0548, 2, 0x0020, 0x4000);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(cpu.bx(), 2);
        assert_eq!(bus.peek_byte(BUFFER), 0x12);
        assert_eq!(bus.peek_byte(BUFFER + 0x7FF), 0x34);
    }

    #[test]
    fn memory_window_rejects_writes_to_rom_and_unknown_units() {
        let mut bus = hle_bus();
        let cpu = call(&mut bus, 0x0642, 1, 0, 0);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_WRITE_PROTECTED);
        let cpu = call(&mut bus, 0x0541, 1, 0, 0);
        assert_eq!(cpu.ah(), STATUS_DEVICE);
        let cpu = call(&mut bus, 0x0542, 0x201, 0, 0);
        assert_eq!(cpu.cx(), DETAIL_PARAMETER);
    }

    #[test]
    fn scsi_read_copies_blocks_and_reports_capacity() {
        let mut bus = hle_bus();
        let mut data = vec![0u8; 256 * 512];
        for (block, chunk) in data.chunks_mut(512).enumerate() {
            chunk.fill(block as u8);
        }
        let image = device::disk::HddImage::from_raw_flat(data).expect("hdd image");
        bus.insert_hdd_backed(0, image, common::MediaBacking::ReadOnly);

        let cpu = call(&mut bus, 0x02B0, 0, 0, 0);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(cpu.bx(), 0);
        assert_eq!(cpu.dx(), 255);

        let cpu = call(&mut bus, 0x05B0, 3, 0, 5);
        assert_eq!(cpu.ah(), 0);
        assert_eq!(cpu.bx(), 0);
        assert_eq!(bus.peek_byte(BUFFER), 5);
        assert_eq!(bus.peek_byte(BUFFER + 512), 6);
        assert_eq!(bus.peek_byte(BUFFER + 1535), 7);

        let cpu = call(&mut bus, 0x05B0, 2, 0, 255);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_PARAMETER);
    }

    #[test]
    fn scsi_without_target_is_not_connected() {
        let mut bus = hle_bus();
        let cpu = call(&mut bus, 0x05B0, 1, 0, 0);
        assert_eq!(cpu.ah(), 0x80);
        assert_eq!(cpu.cx(), DETAIL_NOT_CONNECTED);
        let cpu = call(&mut bus, 0x05B7, 1, 0, 0);
        assert_eq!(cpu.ah(), STATUS_DEVICE);
    }

    #[test]
    fn unknown_device_class_is_a_device_error() {
        let mut bus = hle_bus();
        let cpu = call(&mut bus, 0x0510, 1, 0, 0);
        assert_eq!(cpu.ah(), STATUS_DEVICE);
        assert_eq!(cpu.flags() & 1, 1);
    }
}
