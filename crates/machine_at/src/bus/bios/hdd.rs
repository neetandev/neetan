//! INT 13h hard disk services (DL >= 80h) and the INT 76h (IRQ 14)
//! completion flag.
//!
//! Transfers run synchronously through direct IDE device access, moving the
//! data through the paging-aware memory accessors so calls from V86 mode
//! with paging enabled land where the guest's page tables say. IDE INT 13h
//! is PIO on the real BIOS, so no DMA registers are touched and no 64 KiB
//! boundary applies. Behavior details (status codes, register effects, BDA
//! bytes) are matched to the real AMI BIOS through the side-by-side probe
//! tests in `tests/bios/int13h_hdd.rs`: transfers return AL=0, a zero
//! sector count is a successful no-op, only the cylinder is validated (the
//! device wraps the 4-bit head field), AH=01h returns the status in AL and
//! clears it, and AH=08h never fails for the two drive slots.

use common::{Cpu, SegmentRegister, TraceSink};
use device::disk::HddGeometry;

use super::AtBus;

/// BIOS data area: hard disk status of the last operation.
const BDA_HDD_STATUS: u32 = 0x474;
/// BIOS data area: number of fixed disks.
const BDA_HDD_COUNT: u32 = 0x475;
/// BIOS data area: hard disk operation-complete interrupt flag.
const BDA_HDD_INTERRUPT_FLAG: u32 = 0x48E;

/// Hard disk status: success.
const STATUS_OK: u8 = 0x00;
/// Hard disk status: invalid function or parameter. Also reported for
/// requests to absent drives and cylinders past the FDPT count, matching
/// the probed AMI handler.
const STATUS_BAD_COMMAND: u8 = 0x01;
/// Hard disk status: sector not found, the controller's IDNF error.
const STATUS_SECTOR_NOT_FOUND: u8 = 0x04;
/// Hard disk status: drive not ready.
const STATUS_NOT_READY: u8 = 0xAA;
/// Hard disk status: write fault.
const STATUS_WRITE_FAULT: u8 = 0xCC;

/// INT 13h AH=15h: no drive present.
const DISK_TYPE_NONE: u8 = 0x00;
/// INT 13h AH=15h: fixed disk.
const DISK_TYPE_FIXED: u8 = 0x03;

/// IDE status register value of a ready drive: DRDY and DSC. AH=10h returns
/// it in AL like the probed AMI handler.
const IDE_STATUS_READY: u8 = 0x50;
/// IDE diagnostic code of a healthy drive. AH=14h returns it in AL.
const DIAGNOSTIC_NO_ERROR: u8 = 0x01;

/// Number of hard disk drive slots on the controller.
const HARD_DISK_DRIVE_COUNT: usize = 2;

/// The device head register holds four bits; larger head numbers wrap.
const DEVICE_HEAD_MASK: u8 = 0x0F;

/// Value the IRQ 14 handler stores in the completion flag.
const HDD_INTERRUPT_COMPLETE: u8 = 0xFF;

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

/// One CHS sector address decoded from the INT 13h register packing:
/// cylinder from CH plus CL bits 7:6, sector from CL bits 5:0, head
/// from DH.
struct ChsAddress {
    cylinder: u16,
    head: u8,
    sector: u8,
}

impl ChsAddress {
    /// Decodes the CH/CL/DH register packing.
    fn from_registers(cpu: &impl Cpu) -> Self {
        Self {
            cylinder: u16::from(cpu.ch()) | (u16::from(cpu.cl() & 0xC0) << 2),
            head: cpu.dh(),
            sector: cpu.cl() & 0x3F,
        }
    }

    /// Returns the linear sector number of the address, with the head
    /// wrapped to the device register width.
    fn to_lba(&self, geometry: &HddGeometry) -> u32 {
        let head = self.head & DEVICE_HEAD_MASK;
        (u32::from(self.cylinder) * u32::from(geometry.heads) + u32::from(head))
            * u32::from(geometry.sectors_per_track)
            + u32::from(self.sector)
            - 1
    }
}

impl<T: TraceSink> AtBus<T> {
    /// INT 13h hard disk services dispatch, entered for DL >= 80h when at
    /// least one hard disk is installed.
    pub(super) fn hle_int13h_hdd(&mut self, cpu: &mut impl Cpu) {
        match cpu.ah() {
            0x00 => self.int13h_hdd_reset(cpu),
            0x01 => self.int13h_hdd_last_status(cpu),
            0x02 => self.int13h_hdd_transfer(cpu, TransferKind::Read),
            0x03 => self.int13h_hdd_transfer(cpu, TransferKind::Write),
            0x04 => self.int13h_hdd_transfer(cpu, TransferKind::Verify),
            0x05 => self.int13h_hdd_format_track(cpu),
            0x08 => self.int13h_hdd_drive_parameters(cpu),
            0x09 => self.int13h_hdd_set_parameters(cpu),
            0x0C => self.int13h_hdd_seek(cpu),
            0x0D => self.int13h_hdd_alternate_reset(cpu),
            0x10 => self.int13h_hdd_drive_ready(cpu),
            0x11 => self.int13h_hdd_recalibrate(cpu),
            0x14 => self.int13h_hdd_diagnostics(cpu),
            0x15 => self.int13h_hdd_disk_type(cpu),
            _ => self.set_hdd_result(cpu, STATUS_BAD_COMMAND),
        }
    }

    /// INT 76h: records the operation-complete interrupt in the BDA flag.
    /// The ROM stub acknowledges both PICs itself after the trap; the
    /// handler never touches CPU registers or the IRET frame.
    pub(super) fn hle_int76h(&mut self) {
        self.write_mem_byte(BDA_HDD_INTERRUPT_FLAG, HDD_INTERRUPT_COMPLETE);
    }

    /// Stores the hard disk status to AH, BDA 40:74 and the IRET carry flag.
    fn set_hdd_result(&mut self, cpu: &mut impl Cpu, status: u8) {
        cpu.set_ah(status);
        self.write_mem_byte(BDA_HDD_STATUS, status);
        self.set_iret_cf(cpu, status != STATUS_OK);
    }

    /// Returns the addressed drive index and its geometry, or the bad
    /// command status for a drive that is not installed.
    fn hdd_drive_geometry(&self, cpu: &impl Cpu) -> Result<(usize, HddGeometry), u8> {
        let drive = usize::from(cpu.dl() & 0x7F);
        if drive >= HARD_DISK_DRIVE_COUNT {
            return Err(STATUS_BAD_COMMAND);
        }
        match self.ide.drive_geometry(drive) {
            Some(geometry) => Ok((drive, geometry)),
            None => Err(STATUS_BAD_COMMAND),
        }
    }

    /// AH=00h: resets the disk system. A DL >= 80h reset covers the
    /// diskette controller too, like on the real BIOS.
    fn int13h_hdd_reset(&mut self, cpu: &mut impl Cpu) {
        self.floppy_controller_reset();
        self.set_hdd_result(cpu, STATUS_OK);
    }

    /// AH=0Dh: alternate reset, without touching the diskette controller.
    fn int13h_hdd_alternate_reset(&mut self, cpu: &mut impl Cpu) {
        self.set_hdd_result(cpu, STATUS_OK);
    }

    /// AH=01h: returns the status of the last operation in AL and clears
    /// it. The probed AMI handler reports it there, with AH=0 and the carry
    /// clear even for a stored error.
    fn int13h_hdd_last_status(&mut self, cpu: &mut impl Cpu) {
        let status = self.read_mem_byte(BDA_HDD_STATUS);
        cpu.set_al(status);
        cpu.set_ah(0);
        self.write_mem_byte(BDA_HDD_STATUS, STATUS_OK);
        self.set_iret_cf(cpu, false);
    }

    /// AH=02h/03h/04h: runs the shared transfer core. AL returns zero like
    /// the probed AMI handler (the spent sector count register), except for
    /// an absent drive, where it is preserved.
    fn int13h_hdd_transfer(&mut self, cpu: &mut impl Cpu, kind: TransferKind) {
        let (drive, geometry) = match self.hdd_drive_geometry(cpu) {
            Ok(drive_geometry) => drive_geometry,
            Err(status) => {
                self.set_hdd_result(cpu, status);
                return;
            }
        };
        let status = self.hdd_transfer(cpu, drive, &geometry, kind);
        cpu.set_al(0);
        self.set_hdd_result(cpu, status);
    }

    /// The shared read/write/verify core: validates the cylinder against
    /// the drive geometry and moves whole sectors between the drive and
    /// guest memory. Returns the status byte.
    fn hdd_transfer(
        &mut self,
        cpu: &mut impl Cpu,
        drive: usize,
        geometry: &HddGeometry,
        kind: TransferKind,
    ) -> u8 {
        let sector_count = cpu.al();
        if sector_count == 0 {
            return STATUS_OK;
        }
        let address = ChsAddress::from_registers(cpu);
        if address.cylinder >= geometry.cylinders {
            return STATUS_BAD_COMMAND;
        }
        // The head wraps in the 4-bit device register instead of failing;
        // only a sector number the controller can never find is an error.
        // (The probed sector-0 behavior of the real BIOS is an artifact of
        // the emulated controller, so the IDNF status stands in for it.)
        if address.sector == 0 || address.sector > geometry.sectors_per_track {
            return STATUS_SECTOR_NOT_FOUND;
        }
        let start_lba = address.to_lba(geometry);
        let sector_bytes = usize::from(geometry.sector_size);
        let total_sectors = geometry.total_sectors();
        let buffer_linear = cpu
            .segment_base(SegmentRegister::ES)
            .wrapping_add(u32::from(cpu.bx()));

        for index in 0..u32::from(sector_count) {
            // Multi-sector runs continue across head and cylinder boundaries
            // (the controller auto-increments the CHS address) but stop at
            // the end of the drive.
            let lba = start_lba + index;
            if lba >= total_sectors {
                return STATUS_SECTOR_NOT_FOUND;
            }
            let buffer_offset = index as usize * sector_bytes;
            match kind {
                TransferKind::Read => {
                    let Some(data) = self.ide.read_sector(drive, lba).map(<[u8]>::to_vec) else {
                        return STATUS_SECTOR_NOT_FOUND;
                    };
                    for (byte_index, byte) in data.iter().enumerate() {
                        self.write_mem_byte(
                            buffer_linear + (buffer_offset + byte_index) as u32,
                            *byte,
                        );
                    }
                }
                TransferKind::Write => {
                    let mut sector_data = vec![0u8; sector_bytes];
                    for (byte_index, byte) in sector_data.iter_mut().enumerate() {
                        *byte =
                            self.read_mem_byte(buffer_linear + (buffer_offset + byte_index) as u32);
                    }
                    if !self.ide.write_sector(drive, lba, &sector_data) {
                        return STATUS_WRITE_FAULT;
                    }
                }
                TransferKind::Verify => {
                    if self.ide.read_sector(drive, lba).is_none() {
                        return STATUS_SECTOR_NOT_FOUND;
                    }
                }
            }
        }
        STATUS_OK
    }

    /// AH=05h: format track. The emulated images are pre-formatted, so a
    /// request inside the cylinder count succeeds without touching the
    /// drive. (The real BIOS run in the emulator times out here because the
    /// LLE controller has no FORMAT TRACK command; real hardware formats.)
    fn int13h_hdd_format_track(&mut self, cpu: &mut impl Cpu) {
        let status = match self.hdd_drive_geometry(cpu) {
            Ok((_, geometry)) => {
                let address = ChsAddress::from_registers(cpu);
                if address.cylinder < geometry.cylinders {
                    STATUS_OK
                } else {
                    STATUS_BAD_COMMAND
                }
            }
            Err(status) => status,
        };
        self.set_hdd_result(cpu, status);
    }

    /// AH=08h: returns the drive geometry in the CHS register packing. The
    /// probed AMI handler never fails for the two drive slots: the values
    /// come from the FDPT, which stays zeroed for an absent drive, and AL
    /// carries the sectors per track. The last cylinder is reserved for
    /// diagnostics, so the maximum usable cylinder number is the count
    /// minus two.
    fn int13h_hdd_drive_parameters(&mut self, cpu: &mut impl Cpu) {
        let drive = usize::from(cpu.dl() & 0x7F);
        if drive >= HARD_DISK_DRIVE_COUNT {
            self.set_hdd_result(cpu, STATUS_BAD_COMMAND);
            return;
        }
        let geometry = self.ide.drive_geometry(drive).unwrap_or(HddGeometry {
            cylinders: 0,
            heads: 0,
            sectors_per_track: 0,
            sector_size: 0,
        });
        let maximum_cylinder = geometry.cylinders.saturating_sub(2);
        cpu.set_al(geometry.sectors_per_track);
        cpu.set_ch(maximum_cylinder as u8);
        cpu.set_cl((geometry.sectors_per_track & 0x3F) | ((maximum_cylinder >> 2) & 0xC0) as u8);
        cpu.set_dh(geometry.heads.saturating_sub(1));
        cpu.set_dl(self.read_mem_byte(BDA_HDD_COUNT));
        cpu.set_ah(STATUS_OK);
        self.write_mem_byte(BDA_HDD_STATUS, STATUS_OK);
        self.set_iret_cf(cpu, false);
    }

    /// AH=09h: set drive parameters from the fixed disk parameter table.
    /// The geometry lives in the device, so a present drive succeeds
    /// without further work.
    fn int13h_hdd_set_parameters(&mut self, cpu: &mut impl Cpu) {
        let status = match self.hdd_drive_geometry(cpu) {
            Ok(_) => STATUS_OK,
            Err(status) => status,
        };
        self.set_hdd_result(cpu, status);
    }

    /// AH=0Ch: seek to cylinder. The probed AMI handler passes the address
    /// straight to the controller without validating it.
    fn int13h_hdd_seek(&mut self, cpu: &mut impl Cpu) {
        let status = match self.hdd_drive_geometry(cpu) {
            Ok(_) => STATUS_OK,
            Err(status) => status,
        };
        self.set_hdd_result(cpu, status);
    }

    /// AH=10h: drive ready check. AL returns the IDE status register of the
    /// ready drive like the probed AMI handler.
    fn int13h_hdd_drive_ready(&mut self, cpu: &mut impl Cpu) {
        match self.hdd_drive_geometry(cpu) {
            Ok(_) => {
                cpu.set_al(IDE_STATUS_READY);
                self.set_hdd_result(cpu, STATUS_OK);
            }
            Err(_) => self.set_hdd_result(cpu, STATUS_NOT_READY),
        }
    }

    /// AH=11h: recalibrate.
    fn int13h_hdd_recalibrate(&mut self, cpu: &mut impl Cpu) {
        let status = match self.hdd_drive_geometry(cpu) {
            Ok(_) => STATUS_OK,
            Err(_) => STATUS_NOT_READY,
        };
        self.set_hdd_result(cpu, status);
    }

    /// AH=14h: controller diagnostics. AL returns the no-error diagnostic
    /// code like the probed AMI handler, independent of the drive.
    fn int13h_hdd_diagnostics(&mut self, cpu: &mut impl Cpu) {
        cpu.set_al(DIAGNOSTIC_NO_ERROR);
        self.set_hdd_result(cpu, STATUS_OK);
    }

    /// AH=15h: disk type and sector count. CX:DX holds the sector count of
    /// the usable cylinders (the diagnostic cylinder is excluded, matching
    /// the AH=08h geometry) and AL keeps the count's low byte like the
    /// probed AMI handler. An absent drive is not an error, but the probed
    /// handler still stores a bad command status in the BDA.
    fn int13h_hdd_disk_type(&mut self, cpu: &mut impl Cpu) {
        match self.hdd_drive_geometry(cpu) {
            Ok((_, geometry)) => {
                let usable_cylinders = u32::from(geometry.cylinders.saturating_sub(1));
                let sectors = usable_cylinders
                    * u32::from(geometry.heads)
                    * u32::from(geometry.sectors_per_track);
                cpu.set_cx((sectors >> 16) as u16);
                cpu.set_dx(sectors as u16);
                cpu.set_al(sectors as u8);
                cpu.set_ah(DISK_TYPE_FIXED);
                self.write_mem_byte(BDA_HDD_STATUS, STATUS_OK);
                self.set_iret_cf(cpu, false);
            }
            Err(status) => {
                cpu.set_cx(0);
                cpu.set_dx(0);
                cpu.set_ah(DISK_TYPE_NONE);
                self.write_mem_byte(BDA_HDD_STATUS, status);
                self.set_iret_cf(cpu, false);
            }
        }
    }
}
