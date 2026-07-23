//! HLE bootstrap: loads and enters the boot sector.
//!
//! Reached through pseudo-vector 0xF2 and through INT 19h. The boot order
//! comes from the CMOS boot sequence bit; the sector transfer happens through
//! direct device access, no DMA or IRQ round trip. On success the IRET frame
//! of the trapping stub is rewritten so its IRET enters the boot sector at
//! 0000:7C00.

use common::{Cpu, TraceSink, warn};

use super::{AtBus, iret_stack_base};

/// Physical load address of the boot sector.
const BOOT_SECTOR_ADDRESS: u32 = 0x7C00;
/// Boot sector size in bytes.
const BOOT_SECTOR_SIZE: usize = 512;
/// DL value handed to the boot sector for the first floppy drive.
const BOOT_DRIVE_FLOPPY: u8 = 0x00;
/// DL value handed to the boot sector for the first hard disk.
const BOOT_DRIVE_HDD: u8 = 0x80;
/// CMOS register holding the AMI miscellaneous flags.
const CMOS_AMI_MISC_FLAGS: usize = 0x2D;
/// AMI miscellaneous flag bit 5: boot A: then C: when set.
const CMOS_BOOT_FLOPPY_FIRST: u8 = 0x20;
/// FLAGS word for the boot sector entry: IF set plus the reserved bit.
const BOOT_ENTRY_FLAGS: u16 = 0x0202;
/// FLAGS word for the halt loop entry: interrupts stay disabled.
const HALT_ENTRY_FLAGS: u16 = 0x0002;
/// MBR boot signature offset within the boot sector.
const BOOT_SIGNATURE_OFFSET: usize = 510;
/// DMA channel wired to the floppy controller.
const FLOPPY_DMA_CHANNEL: usize = 2;
/// Motor shutoff tick count the boot read leaves in BDA 40:40 (about two
/// seconds, captured from the real AMI BIOS).
const FLOPPY_MOTOR_TICKS: u8 = 0x25;
/// FDC result bytes of the boot sector read as the real BIOS stores them in
/// BDA 40:42-48: ST0/ST1/ST2 clear, C/H zero, next record 2, 512-byte
/// sectors.
const FDC_RESULT_BYTES: [u8; 7] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x02];
/// Text VRAM base of the mode 03h screen.
const TEXT_VRAM_BASE: u32 = 0xB8000;
/// Attribute byte for the boot failure message: light gray on black.
const FAILURE_ATTRIBUTE: u8 = 0x07;

/// One entry of the boot attempt order.
#[derive(Clone, Copy)]
enum BootAttempt {
    /// The first floppy drive.
    Floppy,
    /// The first hard disk.
    HardDisk,
}

impl<T: TraceSink> AtBus<T> {
    /// Selects the boot device and hands control to its boot sector.
    pub(super) fn hle_bootstrap(&mut self, cpu: &mut impl Cpu) {
        let floppy_first = self.rtc.cmos[CMOS_AMI_MISC_FLAGS] & CMOS_BOOT_FLOPPY_FIRST != 0;
        let order = if floppy_first {
            [BootAttempt::Floppy, BootAttempt::HardDisk]
        } else {
            [BootAttempt::HardDisk, BootAttempt::Floppy]
        };

        for attempt in order {
            let booted = match attempt {
                BootAttempt::Floppy => self.try_boot_floppy(cpu, 0),
                BootAttempt::HardDisk => self.try_boot_hdd(cpu, 0),
            };
            if booted {
                return;
            }
        }

        self.boot_failure(cpu);
    }

    /// Reads CHS 0/0/1 of a floppy and enters it. Like the real AMI BIOS,
    /// floppy boot sectors need no 0x55AA signature; all-zero sectors are
    /// rejected. The transfer runs through DMA channel 2 so the controller
    /// registers end up exactly as the real INT 13h read leaves them.
    fn try_boot_floppy(&mut self, cpu: &mut impl Cpu, drive: usize) -> bool {
        if !self.fdc.has_drive(drive) {
            return false;
        }
        let mut sector = [0u8; BOOT_SECTOR_SIZE];
        {
            let Some(data) = self.fdc.read_sector_data(drive, 0, 0, 0, 1, 2) else {
                return false;
            };
            if data.len() < BOOT_SECTOR_SIZE {
                return false;
            }
            sector.copy_from_slice(&data[..BOOT_SECTOR_SIZE]);
        }
        if sector.iter().all(|&byte| byte == 0) {
            return false;
        }

        // Program DMA channel 2 like the INT 13h read of the real BIOS:
        // single mode write, buffer 0000:7C00, one 512-byte sector.
        self.program_floppy_dma(
            super::floppy::DMA_MODE_DEVICE_TO_MEMORY,
            BOOT_SECTOR_ADDRESS,
            BOOT_SECTOR_SIZE,
        );
        let dma_result = self
            .dma
            .transfer_write_to_memory(FLOPPY_DMA_CHANNEL, &sector);
        for (address, byte) in &dma_result.writes {
            self.memory.write_physical(*address, *byte);
        }

        // Diskette BDA state after the boot read, as the real BIOS leaves
        // it: drive 0 recalibrated, motor running with the shutoff counter
        // armed, the FDC result bytes of the read, the media established.
        // The boot-time recalibrate also consumes the disk-change latch.
        self.fdc.clear_disk_change_on_step(drive);
        self.write_mem_byte(0x43E, 0x01);
        self.write_mem_byte(0x43F, 0x01);
        self.write_mem_byte(0x440, FLOPPY_MOTOR_TICKS);
        for (offset, value) in FDC_RESULT_BYTES.iter().enumerate() {
            self.write_mem_byte(0x442 + offset as u32, *value);
        }
        let _ = self.establish_media(drive);

        self.enter_boot_sector(cpu, BOOT_DRIVE_FLOPPY);
        true
    }

    /// Reads the MBR of a hard disk and enters it when the 0x55AA signature
    /// is present. IDE transfers are PIO, so the sector is stored through
    /// plain CPU-style writes.
    fn try_boot_hdd(&mut self, cpu: &mut impl Cpu, drive: usize) -> bool {
        if !self.ide.has_drive(drive) {
            return false;
        }
        let mut sector = [0u8; BOOT_SECTOR_SIZE];
        {
            let Some(data) = self.ide.read_sector(drive, 0) else {
                return false;
            };
            if data.len() < BOOT_SECTOR_SIZE {
                return false;
            }
            sector.copy_from_slice(&data[..BOOT_SECTOR_SIZE]);
        }
        if sector[BOOT_SIGNATURE_OFFSET] != 0x55 || sector[BOOT_SIGNATURE_OFFSET + 1] != 0xAA {
            return false;
        }
        for (index, &byte) in sector.iter().enumerate() {
            self.write_mem_byte(BOOT_SECTOR_ADDRESS + index as u32, byte);
        }
        self.enter_boot_sector(cpu, BOOT_DRIVE_HDD);
        true
    }

    /// Sets DL to the boot drive and rewrites the IRET frame so the stub's
    /// IRET enters the boot sector already stored at 0000:7C00.
    fn enter_boot_sector(&mut self, cpu: &mut impl Cpu, boot_drive: u8) {
        cpu.set_dx((cpu.dx() & 0xFF00) | u16::from(boot_drive));

        let iret_base = iret_stack_base(cpu);
        self.write_mem_word(iret_base, BOOT_SECTOR_ADDRESS as u16);
        self.write_mem_word(iret_base + 2, 0x0000);
        self.write_mem_word(iret_base + 4, BOOT_ENTRY_FLAGS);
    }

    /// Prints the failure message to the text screen and retargets the IRET
    /// frame at the stub ROM halt loop.
    fn boot_failure(&mut self, cpu: &mut impl Cpu) {
        warn!("machine_at: no bootable media found; halting");

        const FAILURE_MESSAGE: &[u8] = b"No bootable media. System halted.";
        for (index, &character) in FAILURE_MESSAGE.iter().enumerate() {
            let address = TEXT_VRAM_BASE + (index as u32) * 2;
            self.write_mem_byte(address, character);
            self.write_mem_byte(address + 1, FAILURE_ATTRIBUTE);
        }

        let halt_offset = u16::from(self.memory.bios_byte(super::METADATA_HALT_LOOP))
            | (u16::from(self.memory.bios_byte(super::METADATA_HALT_LOOP + 1)) << 8);
        let iret_base = iret_stack_base(cpu);
        self.write_mem_word(iret_base, halt_offset);
        self.write_mem_word(iret_base + 2, super::BIOS_CODE_SEGMENT);
        self.write_mem_word(iret_base + 4, HALT_ENTRY_FLAGS);
    }
}
