//! HLE boot device search.
//!
//! The search starts with the one-shot boot request at CMOS 3C28h, then the
//! default boot device at 3182h/3184h when CMOS 3180h bit 0 is set, then the
//! fixed order: the OS ROM, the CD-ROM, the floppy drives 0-3 and the SCSI
//! IDs 0-4. Each attempt reads the boot sectors of the device to B000:0000 and
//! checks the `IPL4` signature at offset 0 (entry B000:0004) or offset 3 (entry
//! B000:0000). The stub calls the IPL with a far return to its search loop on
//! the stack, so a `RETF` of the IPL tries the next device.

use common::{SegmentRegister, TraceSink, warn};

use super::{CODE_SEGMENT, METADATA_BOOT_RETRY, TownsBiosCpu, disk::DEVICE_CDROM};
use crate::bus::TownsBus;

/// CMOS port of the one-shot boot request: high nibble type (8 CD, 2 floppy,
/// 1 SCSI), low nibble unit. The search clears it.
const CMOS_BOOT_REQUEST: u16 = 0x3C28;
/// CMOS port of the default boot device flag (bit 0).
const CMOS_DEFAULT_BOOT_FLAG: u16 = 0x3180;
/// CMOS port of the default boot device type (1 SCSI, 2 floppy, 8 CD).
const CMOS_DEFAULT_BOOT_TYPE: u16 = 0x3182;
/// CMOS port of the default boot device unit.
const CMOS_DEFAULT_BOOT_UNIT: u16 = 0x3184;

/// Boot device type of a SCSI hard disk.
const BOOT_TYPE_SCSI: u8 = 1;
/// Boot device type of a floppy drive.
const BOOT_TYPE_FLOPPY: u8 = 2;
/// Boot device type of the physical memory windows.
const BOOT_TYPE_MEMORY: u8 = 4;
/// Boot device type of the CD-ROM drive.
const BOOT_TYPE_CDROM: u8 = 8;

/// Device code of the OS ROM boot attempt (class 4, unit 2).
const DEVICE_OS_ROM: u8 = 0x42;
/// Device code of floppy drive 0.
const DEVICE_FLOPPY: u8 = 0x20;
/// Device code of SCSI ID 0.
const DEVICE_SCSI: u8 = 0xB0;

/// Fixed boot device order after the CMOS selections.
const DEFAULT_ORDER: [u8; 11] = [
    DEVICE_OS_ROM,
    DEVICE_CDROM,
    DEVICE_FLOPPY,
    DEVICE_FLOPPY + 1,
    DEVICE_FLOPPY + 2,
    DEVICE_FLOPPY + 3,
    DEVICE_SCSI,
    DEVICE_SCSI + 1,
    DEVICE_SCSI + 2,
    DEVICE_SCSI + 3,
    DEVICE_SCSI + 4,
];

/// Real-mode segment of the IPL sector.
const IPL_SEGMENT: u16 = 0xB000;
/// Physical address of the IPL sector.
const IPL_ADDRESS: u32 = 0xB_0000;
/// IPL signature.
const IPL_SIGNATURE: &[u8; 4] = b"IPL4";
/// Entry offset of an IPL with the signature at offset 0.
const IPL_ENTRY_SIGNATURE_FIRST: u16 = 0x0004;
/// Entry offset of an IPL with the signature at offset 3.
const IPL_ENTRY_JUMP_FIRST: u16 = 0x0000;
/// Number of 512-byte blocks read from a SCSI boot disk.
const SCSI_BOOT_BLOCKS: u16 = 4;

/// FLAGS image at the IPL entry, as the real system ROM leaves it.
const IPL_ENTRY_FLAGS: u16 = 0x0097;
/// FLAGS image of the retry wait: interrupts disabled.
const RETRY_FLAGS: u16 = 0x0002;
/// EAX at the IPL entry.
const IPL_ENTRY_EAX: u32 = 0x0000_FFFF;
/// EDX at the IPL entry, a leftover of the real system ROM.
const IPL_ENTRY_EDX: u32 = 0x0000_31AC;
/// ESI at the IPL entry, a leftover of the real system ROM.
const IPL_ENTRY_ESI: u32 = 0xFFFE_0CD5;
/// EDI at the IPL entry, a leftover of the real system ROM.
const IPL_ENTRY_EDI: u32 = 0x0000_EB34;
/// DS at the IPL entry.
const IPL_ENTRY_DS: u16 = 0xFC00;
/// ES at the IPL entry.
const IPL_ENTRY_ES: u16 = 0xC000;

/// Returns the device code of a CMOS type and unit pair.
fn device_code(boot_type: u8, unit: u8) -> Option<u8> {
    match boot_type {
        BOOT_TYPE_CDROM => Some(DEVICE_CDROM),
        BOOT_TYPE_FLOPPY => Some(DEVICE_FLOPPY | (unit & 0x03)),
        BOOT_TYPE_SCSI => Some(DEVICE_SCSI | (unit & 0x0F)),
        _ => None,
    }
}

/// Returns the boot device type handed to the IPL in `BL` for a device code.
fn boot_type(device: u8) -> u8 {
    match device >> 4 {
        0x2 => BOOT_TYPE_FLOPPY,
        0x4 => BOOT_TYPE_MEMORY,
        0xB => BOOT_TYPE_SCSI,
        _ => BOOT_TYPE_CDROM,
    }
}

impl<T: TraceSink> TownsBus<T> {
    /// Builds the boot device queue from the CMOS and restarts the search.
    pub(super) fn start_boot_search(&mut self) {
        let mut queue = Vec::with_capacity(DEFAULT_ORDER.len() + 2);
        let request = self.cmos_port(CMOS_BOOT_REQUEST);
        self.set_cmos_port(CMOS_BOOT_REQUEST, 0);
        if let Some(device) = device_code(request >> 4, request & 0x0F) {
            queue.push(device);
        }
        if self.cmos_port(CMOS_DEFAULT_BOOT_FLAG) & 0x01 != 0
            && let Some(device) = device_code(
                self.cmos_port(CMOS_DEFAULT_BOOT_TYPE),
                self.cmos_port(CMOS_DEFAULT_BOOT_UNIT),
            )
        {
            queue.push(device);
        }
        queue.extend(DEFAULT_ORDER);
        self.bios_state.boot_queue = queue;
        self.bios_state.boot_position = 0;
    }

    /// Loads the IPL of the next boot device (vector F2h). The stub reserved
    /// an IRET frame at SS:SP above the far return to its search loop. The
    /// frame enters the IPL, or the retry wait when no device is left.
    pub(super) fn hle_bootstrap(&mut self, cpu: &mut impl TownsBiosCpu) {
        while let Some(&device) = self
            .bios_state
            .boot_queue
            .get(usize::from(self.bios_state.boot_position))
        {
            self.bios_state.boot_position += 1;
            if let Some(entry) = self.load_ipl(device) {
                self.enter_ipl(cpu, device, entry);
                return;
            }
        }

        if !self.bios_state.boot_failure_reported {
            self.bios_state.boot_failure_reported = true;
            warn!("machine_towns: no bootable device; retrying");
        }
        self.start_boot_search();
        let retry = self.stub_metadata(METADATA_BOOT_RETRY);
        self.write_iret_frame(cpu, retry, CODE_SEGMENT, RETRY_FLAGS);
    }

    /// Reads the boot sectors of `device` to B000:0000. Returns the IPL entry
    /// offset when the sector carries the `IPL4` signature.
    fn load_ipl(&mut self, device: u8) -> Option<u16> {
        let unit = device & 0x0F;
        let loaded = match device >> 4 {
            0x2 => {
                let drive = usize::from(unit);
                self.fdc.drive_has_media(drive)
                    && self.floppy_read_boot_sectors_to(drive, IPL_ADDRESS)
            }
            0x4 => self.memory_window_read_boot_block_to(unit, IPL_ADDRESS),
            0xB => {
                let target = usize::from(unit);
                self.scsi.has_target(target)
                    && self.scsi_read_boot_blocks_to(target, IPL_ADDRESS, SCSI_BOOT_BLOCKS)
            }
            _ => {
                // The drive check consumes a pending media change, as the
                // retry loop of the real system ROM does.
                self.cdrom_ready();
                self.cdc.has_disc() && self.cdrom_read_sector_to(0, IPL_ADDRESS)
            }
        };
        if !loaded {
            return None;
        }
        let signature_at = |bus: &mut Self, offset: u32| {
            (0..4u32).all(|index| {
                bus.hle_read_byte(IPL_ADDRESS + offset + index) == IPL_SIGNATURE[index as usize]
            })
        };
        if signature_at(self, 0) {
            Some(IPL_ENTRY_SIGNATURE_FIRST)
        } else if signature_at(self, 3) {
            Some(IPL_ENTRY_JUMP_FIRST)
        } else {
            None
        }
    }

    /// Sets the IPL entry registers and fills the IRET frame with the entry.
    fn enter_ipl(&mut self, cpu: &mut impl TownsBiosCpu, device: u8, entry: u16) {
        let unit = device & 0x0F;
        cpu.set_eax(IPL_ENTRY_EAX);
        cpu.set_ebx(u32::from(boot_type(device)) | (u32::from(unit) << 8));
        cpu.set_ecx(0);
        cpu.set_edx(IPL_ENTRY_EDX);
        cpu.set_esi_full(IPL_ENTRY_ESI);
        cpu.set_edi_full(IPL_ENTRY_EDI);
        cpu.set_ebp_full(0);
        cpu.load_segment_real_mode(SegmentRegister::DS, IPL_ENTRY_DS);
        cpu.load_segment_real_mode(SegmentRegister::ES, IPL_ENTRY_ES);
        let (gdt_base, gdt_limit) = self.stub_gdt_register();
        cpu.load_gdtr(gdt_base, gdt_limit);
        self.write_iret_frame(cpu, entry, IPL_SEGMENT, IPL_ENTRY_FLAGS);
    }
}

#[cfg(test)]
mod tests {
    use common::{Cpu, SegmentRegister};

    use super::*;
    use crate::bus::bios::{
        VECTOR_BOOTSTRAP,
        tests::{TestCpu, hle_bus, real_mode_cpu, trap},
    };

    /// Stack pointer of the boot search loop after it pushed the IPL return
    /// address and reserved the IRET frame.
    const FRAME_STACK_POINTER: u16 = 0x0574 - 6;

    /// Runs one bootstrap trap from the boot stack and returns the CPU.
    fn bootstrap(bus: &mut TownsBus) -> TestCpu {
        let mut cpu = real_mode_cpu();
        cpu.load_segment_real_mode(SegmentRegister::SS, 0xF7A0);
        cpu.set_sp(FRAME_STACK_POINTER);
        trap(bus, &mut cpu, VECTOR_BOOTSTRAP);
        cpu
    }

    /// Returns the IRET frame the bootstrap wrote: offset, segment and flags.
    fn frame(bus: &mut TownsBus) -> (u16, u16, u16) {
        let base = 0xF_7A00 + u32::from(FRAME_STACK_POINTER);
        (
            bus.hle_read_word(base),
            bus.hle_read_word(base + 2),
            bus.hle_read_word(base + 4),
        )
    }

    /// Builds a CD image whose first sector starts with `prefix`.
    fn cd_with_ipl(prefix: &[u8]) -> device::cdrom::CdImage {
        let mut data = vec![0u8; 2048 * 2];
        data[..prefix.len()].copy_from_slice(prefix);
        let cue = "FILE \"disc.iso\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n";
        device::cdrom::CdImage::from_cue(cue, data).expect("cd image")
    }

    /// Builds a 2HD floppy whose first sector starts with `prefix`.
    fn floppy_with_ipl(prefix: &[u8]) -> device::floppy::FloppyImage {
        let mut data = vec![0u8; 77 * 2 * 8 * 1024];
        data[..prefix.len()].copy_from_slice(prefix);
        device::floppy::FloppyImage::from_hdm_bytes(&data).expect("floppy image")
    }

    #[test]
    fn queue_puts_the_cmos_request_first_and_clears_it() {
        let mut bus = hle_bus();
        bus.set_cmos_port(CMOS_BOOT_REQUEST, 0x21);
        bus.set_cmos_port(CMOS_DEFAULT_BOOT_FLAG, 0x01);
        bus.set_cmos_port(CMOS_DEFAULT_BOOT_TYPE, BOOT_TYPE_SCSI);
        bus.set_cmos_port(CMOS_DEFAULT_BOOT_UNIT, 2);
        bus.start_boot_search();
        assert_eq!(&bus.bios_state.boot_queue[..4], &[0x21, 0xB2, 0x42, 0xC0]);
        assert_eq!(bus.cmos_port(CMOS_BOOT_REQUEST), 0);

        bus.start_boot_search();
        assert_eq!(&bus.bios_state.boot_queue[..2], &[0xB2, 0x42]);
    }

    #[test]
    fn cd_ipl_with_signature_at_offset_zero_enters_b000_0004() {
        let mut bus = hle_bus();
        bus.insert_cdrom(cd_with_ipl(b"IPL4"));
        bus.set_cmos_port(CMOS_BOOT_REQUEST, 0x80);
        bus.start_boot_search();
        let cpu = bootstrap(&mut bus);

        assert_eq!(frame(&mut bus), (0x0004, 0xB000, IPL_ENTRY_FLAGS));
        assert_eq!(cpu.eax(), 0x0000_FFFF);
        assert_eq!(cpu.ebx(), 0x0000_0008);
        assert_eq!(cpu.edx(), IPL_ENTRY_EDX);
        assert_eq!(cpu.ds(), IPL_ENTRY_DS);
        assert_eq!(cpu.es(), IPL_ENTRY_ES);
        assert_eq!(bus.peek_byte(IPL_ADDRESS), b'I');
    }

    /// Runs the bootstrap trap that enters the IPL of the HLE OS ROM. Its
    /// `RETF` continues the search with the next device.
    fn enter_os_rom_ipl(bus: &mut TownsBus) {
        let cpu = bootstrap(bus);
        assert_eq!(frame(bus), (0x0004, 0xB000, IPL_ENTRY_FLAGS));
        assert_eq!(cpu.ebx(), 0x0000_0204);
    }

    #[test]
    fn os_rom_ipl_comes_first_in_the_default_order() {
        let mut bus = hle_bus();
        bus.start_boot_search();
        enter_os_rom_ipl(&mut bus);
    }

    #[test]
    fn floppy_ipl_with_signature_at_offset_three_enters_b000_0000() {
        let mut bus = hle_bus();
        bus.insert_floppy_backed(
            1,
            floppy_with_ipl(b"\xEB\x10\x90IPL4"),
            common::MediaBacking::ReadOnly,
        );
        bus.start_boot_search();
        enter_os_rom_ipl(&mut bus);
        let cpu = bootstrap(&mut bus);
        assert_eq!(frame(&mut bus), (0x0000, 0xB000, IPL_ENTRY_FLAGS));
        assert_eq!(cpu.ebx(), 0x0000_0102);
    }

    #[test]
    fn order_skips_devices_without_ipl_and_continues_after_return() {
        let mut bus = hle_bus();
        bus.insert_cdrom(cd_with_ipl(b"NONE"));
        bus.insert_floppy_backed(0, floppy_with_ipl(b"IPL4"), common::MediaBacking::ReadOnly);
        bus.insert_floppy_backed(1, floppy_with_ipl(b"IPL4"), common::MediaBacking::ReadOnly);
        bus.start_boot_search();
        enter_os_rom_ipl(&mut bus);

        let cpu = bootstrap(&mut bus);
        assert_eq!(cpu.ebx(), 0x0000_0002);
        let cpu = bootstrap(&mut bus);
        assert_eq!(cpu.ebx(), 0x0000_0102);
    }

    #[test]
    fn exhausted_search_enters_the_retry_wait() {
        let mut bus = hle_bus();
        bus.start_boot_search();
        enter_os_rom_ipl(&mut bus);
        let _ = bootstrap(&mut bus);
        let retry = bus.stub_metadata(METADATA_BOOT_RETRY);
        assert_eq!(frame(&mut bus), (retry, CODE_SEGMENT, RETRY_FLAGS));
        assert!(bus.bios_state.boot_failure_reported);
        assert_eq!(bus.bios_state.boot_position, 0);
    }
}
