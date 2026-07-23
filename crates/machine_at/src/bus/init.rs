//! POST initialization: reproduces the machine state the real AMI BIOS
//! leaves behind after its power-on self test.
//!
//! `initialize_post_boot_state` runs inside the pseudo-vector 0xF0 trap (the
//! stub ROM cold entry), not in the bus constructor: POST leaves A20 disabled
//! and the A20 mask applies to the 486 reset fetch at 0xFFFFFFF0, so the
//! reset path must run with A20 still enabled. Device state is programmed
//! through the same I/O ports the real BIOS uses so the guest-visible
//! readback matches; the target values were captured from the real AMI
//! CS4031 BIOS running in this emulator.

use common::TraceSink;

use super::{
    AtBus,
    bios::{BIOS_CODE_SEGMENT, METADATA_VECTOR_TABLE},
};
use crate::{config::PIT_CLOCK_HZ, scheduler::EventAt};

/// BIOS data area: COM1 base I/O address (word).
const BDA_COM1_BASE: u32 = 0x400;
/// BIOS data area: equipment word.
const BDA_EQUIPMENT: u32 = 0x410;
/// BIOS data area: base memory size in KiB (word).
const BDA_MEMORY_SIZE: u32 = 0x413;
/// BIOS data area: keyboard shift flags byte 1.
const BDA_KEYBOARD_FLAGS_1: u32 = 0x417;
/// BIOS data area: keyboard buffer head pointer (word).
const BDA_KEYBOARD_HEAD: u32 = 0x41A;
/// BIOS data area: keyboard buffer tail pointer (word).
const BDA_KEYBOARD_TAIL: u32 = 0x41C;
/// BIOS data area: diskette recalibrate status.
const BDA_FLOPPY_RECALIBRATE: u32 = 0x43E;
/// BIOS data area: diskette motor status.
const BDA_FLOPPY_MOTOR: u32 = 0x43F;
/// BIOS data area: diskette motor shutoff counter.
const BDA_FLOPPY_MOTOR_COUNT: u32 = 0x440;
/// BIOS data area: diskette last operation status.
const BDA_FLOPPY_STATUS: u32 = 0x441;
/// BIOS data area: timer tick count (dword).
const BDA_TIMER_COUNT: u32 = 0x46C;
/// BIOS data area: timer 24-hour rollover flag.
const BDA_TIMER_OVERFLOW: u32 = 0x470;
/// BIOS data area: break flag.
const BDA_BREAK_FLAG: u32 = 0x471;
/// BIOS data area: reset flag (word, 0x1234 requests a warm boot).
const BDA_RESET_FLAG: u32 = 0x472;
/// Reset flag value requesting a warm boot.
const WARM_BOOT_FLAG: u16 = 0x1234;
/// Reset flag residue the real AMI BIOS leaves after a warm boot (probed;
/// a cold boot leaves 0).
const WARM_BOOT_RESIDUE: u16 = 0x1200;
/// End of conventional memory zeroed by the POST memory test.
const CONVENTIONAL_MEMORY_END: u32 = 0xA0000;
/// BIOS data area: fixed disk last operation status.
const BDA_DISK_STATUS: u32 = 0x474;
/// BIOS data area: number of fixed disk drives.
const BDA_DISK_COUNT: u32 = 0x475;
/// BIOS data area: LPT1-LPT4 timeout values.
const BDA_LPT_TIMEOUT: u32 = 0x478;
/// BIOS data area: COM1-COM4 timeout values.
const BDA_COM_TIMEOUT: u32 = 0x47C;
/// BIOS data area: keyboard buffer start offset (word).
const BDA_KEYBOARD_BUFFER_START: u32 = 0x480;
/// BIOS data area: keyboard buffer end offset (word).
const BDA_KEYBOARD_BUFFER_END: u32 = 0x482;
/// BIOS data area: diskette media control (last data rate).
const BDA_FLOPPY_MEDIA_CONTROL: u32 = 0x48B;
/// BIOS data area: diskette drive information.
const BDA_FLOPPY_DRIVE_INFO: u32 = 0x48F;
/// BIOS data area: diskette drive 0 media state.
const BDA_FLOPPY_MEDIA_STATE_0: u32 = 0x490;
/// BIOS data area: keyboard mode/type flags.
const BDA_KEYBOARD_MODE: u32 = 0x496;
/// BIOS data area: keyboard LED flags.
const BDA_KEYBOARD_LEDS: u32 = 0x497;
/// Print screen status byte at 50:00.
const PRINT_SCREEN_STATUS: u32 = 0x500;

/// Keyboard buffer start offset within segment 0x40.
const KEYBOARD_BUFFER_START: u16 = 0x001E;
/// Keyboard buffer end offset within segment 0x40.
const KEYBOARD_BUFFER_END: u16 = 0x003E;
/// Equipment word bits 5:4: initial video mode 80x25 color.
const EQUIPMENT_VIDEO_COLOR_80: u16 = 0x0020;
/// Equipment word bits 11:9: one serial port fitted.
const EQUIPMENT_ONE_SERIAL_PORT: u16 = 0x0200;
/// CMOS register: equipment byte.
const CMOS_EQUIPMENT: usize = 0x14;
/// CMOS register: RTC seconds (BCD).
const CMOS_RTC_SECONDS: usize = 0x00;
/// CMOS register: RTC minutes (BCD).
const CMOS_RTC_MINUTES: usize = 0x02;
/// CMOS register: RTC hours (BCD).
const CMOS_RTC_HOURS: usize = 0x04;

/// Master PIC ICW2: IRQ 0-7 at vectors 0x08-0x0F.
const MASTER_PIC_VECTOR_BASE: u8 = 0x08;
/// Slave PIC ICW2: IRQ 8-15 at vectors 0x70-0x77.
const SLAVE_PIC_VECTOR_BASE: u8 = 0x70;
/// Master PIC interrupt mask after POST (captured from the AMI BIOS).
const MASTER_PIC_MASK: u8 = 0xB8;
/// Slave PIC interrupt mask after POST (captured from the AMI BIOS).
const SLAVE_PIC_MASK: u8 = 0xDD;
/// KBC command byte after POST: translation on, system flag, IRQ 1 enabled.
const KBC_COMMAND_BYTE: u8 = 0x45;
/// Keyboard LED state the AMI POST programs (0xED parameter).
const KEYBOARD_LEDS: u8 = 0x04;
/// Keyboard typematic rate the AMI POST programs (0xF3 parameter).
const KEYBOARD_TYPEMATIC: u8 = 0x20;
/// CS4031 configuration registers as the AMI BIOS leaves them (captured).
/// Register 0x08 comes last so the config index latch ends on it like the
/// real POST does.
const CS4031_POST_REGISTERS: [(u8, u8); 14] = [
    (0x05, 0x05),
    (0x06, 0x24),
    (0x07, 0x0B),
    (0x09, 0x34),
    (0x10, 0x20),
    (0x11, 0x03),
    (0x12, 0x33),
    (0x15, 0x10),
    (0x18, 0x50),
    (0x19, 0x43),
    (0x1A, 0x00),
    (0x1B, 0x40),
    (0x1C, 0xB2),
    (0x08, 0x3F),
];
/// Shadow regions the POST copies the BIOS images into: C0000, C4000, F0000.
const SHADOWED_REGIONS: u8 = 0x43;
/// System BIOS shadow copy size in bytes.
const SYSTEM_BIOS_SHADOW_SIZE: u32 = 0x1_0000;
/// VGA BIOS shadow copy size in bytes.
const VGA_BIOS_SHADOW_SIZE: u32 = 0x8000;
/// Port B write value after POST (captured from the AMI BIOS).
const PORT_B_POST: u8 = 0x0C;
/// FDC digital output register value after POST: reset released, IRQ and
/// DMA gates open, motors off.
const FDC_DOR_POST: u8 = 0x0C;
impl<T: TraceSink> AtBus<T> {
    /// Populates the IVT at 0x0000-0x03FF from the stub BIOS ROM's vector
    /// table.
    ///
    /// The ROM metadata header publishes the table offset; each entry is a
    /// (vector_number, handler_offset) pair of 16-bit words, terminated by
    /// 0xFFFF. The handler offsets are relative to segment 0xF000.
    pub(super) fn populate_ivt_from_stub_bios(&mut self) {
        let vector_table_offset = u16::from_le_bytes([
            self.memory.bios_byte(METADATA_VECTOR_TABLE),
            self.memory.bios_byte(METADATA_VECTOR_TABLE + 1),
        ]) as usize;

        let mut rom_pos = vector_table_offset;
        loop {
            let vector_num = u16::from_le_bytes([
                self.memory.bios_byte(rom_pos),
                self.memory.bios_byte(rom_pos + 1),
            ]);
            if vector_num == 0xFFFF {
                break;
            }
            let handler_offset = u16::from_le_bytes([
                self.memory.bios_byte(rom_pos + 2),
                self.memory.bios_byte(rom_pos + 3),
            ]);
            let ivt_addr = u32::from(vector_num) * 4;
            self.memory.write_physical(ivt_addr, handler_offset as u8);
            self.memory
                .write_physical(ivt_addr + 1, (handler_offset >> 8) as u8);
            self.memory
                .write_physical(ivt_addr + 2, BIOS_CODE_SEGMENT as u8);
            self.memory
                .write_physical(ivt_addr + 3, (BIOS_CODE_SEGMENT >> 8) as u8);
            rom_pos += 4;
        }
    }

    /// Reads one little-endian word from the BIOS data area.
    fn bda_read_u16(&self, address: u32) -> u16 {
        u16::from(self.memory.read_physical(address))
            | (u16::from(self.memory.read_physical(address + 1)) << 8)
    }

    /// Writes one byte into the BIOS data area.
    fn bda_write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_physical(address, value);
    }

    /// Writes one little-endian word into the BIOS data area.
    fn bda_write_u16(&mut self, address: u32, value: u16) {
        self.memory.write_physical(address, value as u8);
        self.memory.write_physical(address + 1, (value >> 8) as u8);
    }

    /// Writes one little-endian doubleword into the BIOS data area.
    fn bda_write_u32(&mut self, address: u32, value: u32) {
        for offset in 0..4u32 {
            self.memory
                .write_physical(address + offset, (value >> (offset * 8)) as u8);
        }
    }

    /// Delivers every pending KBC output byte and consumes it through port
    /// 0x60, mirroring the polling loop of the real BIOS.
    fn drain_kbc_output(&mut self) {
        while self.kbc.deliver_next().is_some() {
            let _ = self.io_read(0x60);
        }
    }

    /// Returns the timer tick count matching the RTC time of day.
    fn ticks_since_midnight(&self) -> u32 {
        let bcd = |value: u8| u32::from(value >> 4) * 10 + u32::from(value & 0x0F);
        let seconds = bcd(self.rtc.cmos[CMOS_RTC_SECONDS])
            + bcd(self.rtc.cmos[CMOS_RTC_MINUTES]) * 60
            + bcd(self.rtc.cmos[CMOS_RTC_HOURS]) * 3600;
        (u64::from(seconds) * u64::from(PIT_CLOCK_HZ) / 65_536) as u32
    }

    /// Reproduces the machine state the real AMI BIOS leaves behind after
    /// POST: IVT, BIOS data area, and every device in its post-POST state.
    pub(super) fn initialize_post_boot_state(&mut self) {
        // The reset flag at 40:72 distinguishes warm (0x1234) from cold
        // boots before the BDA is rebuilt.
        let warm_boot = self.bda_read_u16(BDA_RESET_FLAG) == WARM_BOOT_FLAG;

        // The POST memory test zeroes conventional memory above the BDA on
        // every boot path, warm or cold (probed on the real AMI BIOS).
        for address in 0x500..CONVENTIONAL_MEMORY_END {
            self.memory.write_physical(address, 0);
        }

        // IVT: clear, then install the stub ROM vectors.
        for address in 0..0x400u32 {
            self.memory.write_physical(address, 0);
        }
        self.populate_ivt_from_stub_bios();

        // BIOS data area: clear 0x400-0x4FF plus the print screen byte.
        for address in 0x400..0x500u32 {
            self.memory.write_physical(address, 0);
        }
        self.bda_write_u8(PRINT_SCREEN_STATUS, 0);

        self.bda_write_u16(BDA_COM1_BASE, 0x03F8);
        let cmos_equipment = self.rtc.cmos[CMOS_EQUIPMENT];
        let equipment =
            u16::from(cmos_equipment & 0xC3) | EQUIPMENT_VIDEO_COLOR_80 | EQUIPMENT_ONE_SERIAL_PORT;
        self.bda_write_u16(BDA_EQUIPMENT, equipment);
        // AMI POST scratch bytes at 40:12 and 40:16 (captured).
        self.bda_write_u8(0x412, 0xBF);
        self.bda_write_u8(0x416, 0x18);
        self.bda_write_u16(BDA_MEMORY_SIZE, 640);

        self.bda_write_u8(BDA_KEYBOARD_FLAGS_1, 0x20);
        self.bda_write_u16(BDA_KEYBOARD_HEAD, KEYBOARD_BUFFER_START);
        self.bda_write_u16(BDA_KEYBOARD_TAIL, KEYBOARD_BUFFER_START);
        self.bda_write_u16(BDA_KEYBOARD_BUFFER_START, KEYBOARD_BUFFER_START);
        self.bda_write_u16(BDA_KEYBOARD_BUFFER_END, KEYBOARD_BUFFER_END);
        self.bda_write_u8(BDA_KEYBOARD_MODE, 0x10);
        self.bda_write_u8(BDA_KEYBOARD_LEDS, 0x12);

        self.bda_write_u8(BDA_FLOPPY_RECALIBRATE, 0);
        self.bda_write_u8(BDA_FLOPPY_MOTOR, 0);
        self.bda_write_u8(BDA_FLOPPY_MOTOR_COUNT, 0);
        self.bda_write_u8(BDA_FLOPPY_STATUS, 0);
        self.bda_write_u8(BDA_FLOPPY_MEDIA_CONTROL, 0);
        self.bda_write_u8(BDA_FLOPPY_DRIVE_INFO, 0x07);
        self.bda_write_u8(BDA_FLOPPY_MEDIA_STATE_0, 0);

        // Protected-mode return pointer scratch at 40:67 (AMI POST leftover,
        // captured).
        self.bda_write_u32(0x467, 0x0C00_0280);

        self.bda_write_u32(BDA_TIMER_COUNT, self.ticks_since_midnight());
        self.bda_write_u8(BDA_TIMER_OVERFLOW, 0);
        self.bda_write_u8(BDA_BREAK_FLAG, 0);
        self.bda_write_u16(
            BDA_RESET_FLAG,
            if warm_boot { WARM_BOOT_RESIDUE } else { 0 },
        );

        let disk_count = u8::from(self.ide.has_drive(0)) + u8::from(self.ide.has_drive(1));
        self.bda_write_u8(BDA_DISK_STATUS, 0);
        self.bda_write_u8(BDA_DISK_COUNT, disk_count);

        for lpt in 0..4u32 {
            self.bda_write_u8(BDA_LPT_TIMEOUT + lpt, 0x14);
        }
        for com in 0..4u32 {
            self.bda_write_u8(BDA_COM_TIMEOUT + com, 0x01);
        }

        // Dual PIC: full ICW sequence, then the post-POST interrupt masks.
        self.io_write(0x20, 0x11);
        self.io_write(0x21, MASTER_PIC_VECTOR_BASE);
        self.io_write(0x21, 0x04);
        self.io_write(0x21, 0x01);
        self.io_write(0xA0, 0x11);
        self.io_write(0xA1, SLAVE_PIC_VECTOR_BASE);
        self.io_write(0xA1, 0x02);
        self.io_write(0xA1, 0x01);
        self.io_write(0x21, MASTER_PIC_MASK);
        self.io_write(0xA1, SLAVE_PIC_MASK);

        // PIT: channel 0 mode 3 count 0 (the 18.2 Hz tick), channel 1 mode 2
        // refresh, channel 2 mode 3 gated off (beeper, captured count).
        self.io_write(0x43, 0x36);
        self.io_write(0x40, 0x00);
        self.io_write(0x40, 0x00);
        self.io_write(0x43, 0x54);
        self.io_write(0x41, 0x12);
        self.io_write(0x43, 0xB6);
        self.io_write(0x42, 0x05);
        self.io_write(0x42, 0x05);

        // DMA: master clear both 8237s (all channels masked), the standard
        // channel modes, cascade mode on channel 4, only the cascade channel
        // unmasked. The AMI BIOS leaves the DMA1 channels masked until a
        // transfer needs them.
        self.io_write(0x0D, 0x00);
        self.io_write(0xDA, 0x00);
        self.io_write(0x0B, 0x40);
        self.io_write(0x0B, 0x41);
        self.io_write(0x0B, 0x42);
        self.io_write(0x0B, 0x43);
        self.io_write(0xD6, 0xC0);
        self.io_write(0xD6, 0x41);
        self.io_write(0xD6, 0x42);
        self.io_write(0xD6, 0x43);
        self.io_write(0xD4, 0x00);

        // FDC: the POST pulses the controller reset, releases it with the
        // IRQ and DMA gates open, and drains the resulting sense interrupts,
        // so no polling interrupt is left pending afterwards.
        self.fdc_io_write(0x3F2, 0x08);
        self.fdc_io_write(0x3F2, FDC_DOR_POST);
        self.fdc_reset_poll_pending = false;
        self.scheduler.cancel(EventAt::FdcInterrupt);
        self.update_next_event_cycle();

        // KBC and keyboard: self test (sets the system flag), command byte,
        // typematic rate and LEDs, keyboard re-enabled last. Every response
        // is drained and consumed the way the polling real BIOS does. This
        // runs before the CS4031 register file: the self-test command drives
        // the chipset's emulated Gate A20 high, which must not latch (the
        // real POST programs register 0x1C only afterwards).
        self.io_write(0x64, 0xAA);
        self.drain_kbc_output();
        self.io_write(0x64, 0x60);
        self.io_write(0x60, KBC_COMMAND_BYTE);
        self.io_write(0x60, 0xF3);
        self.drain_kbc_output();
        self.io_write(0x60, KEYBOARD_TYPEMATIC);
        self.drain_kbc_output();
        self.io_write(0x60, 0xED);
        self.drain_kbc_output();
        self.io_write(0x60, KEYBOARD_LEDS);
        self.drain_kbc_output();
        self.io_write(0x64, 0xAE);

        // Fixed disk parameter tables: patched into the stub ROM image
        // before it is shadowed, so INT 41h/46h describe the mounted disks.
        self.patch_fixed_disk_parameter_tables();

        // Shadow the system and video BIOS like the real POST: enable shadow
        // writes for the C0000/C4000/F0000 regions, copy the ROM images into
        // DRAM, then let the captured register file below write-protect the
        // shadow again (0x19 keeps the shadow readable, 0x1A blocks writes).
        self.io_write(0x22, device::cs4031::CS4031_REG_SHADOW_WRITE);
        self.io_write(0x23, SHADOWED_REGIONS);
        for offset in 0..SYSTEM_BIOS_SHADOW_SIZE {
            let byte = self.memory.bios_byte(offset as usize);
            self.memory.write_physical(0xF0000 + offset, byte);
        }
        for offset in 0..VGA_BIOS_SHADOW_SIZE {
            let byte = self.memory.vga_bios_byte(offset as usize);
            self.memory.write_physical(0xC0000 + offset, byte);
        }

        // CS4031: the captured post-POST register file (DRAM configuration,
        // shadow map, A/B routing to the VGA). The A20 gate ends disabled
        // through the port 0x92 fast gate; the real BIOS never programs the
        // KBC output port. The real BIOS derives the DMA clock divider from
        // the CPU speed it measures during POST, so it follows the effective
        // clock, not the model: 0x09 only on the DX2-66 in high cpu-mode
        // (66 MHz), 0x0A everywhere else (verified against the real BIOS in
        // both cpu-modes on both models).
        let dma_clock_divider: u8 = if self.clocks.cpu_clock_hz > 60_000_000 {
            0x09
        } else {
            0x0A
        };
        self.io_write(0x22, device::cs4031::CS4031_REG_DMA_CLOCK);
        self.io_write(0x23, dma_clock_divider);
        for (register, value) in CS4031_POST_REGISTERS {
            self.io_write(0x22, register);
            self.io_write(0x23, value);
        }
        self.io_write(0x61, PORT_B_POST);
        self.io_write(0x92, 0x02);
        self.io_write(0x92, 0x00);
        // The last CMOS access leaves NMI disabled (port 0x70 bit 7).
        self.io_write(0x70, 0x8D);
        let _ = self.io_read(0x71);

        // Video: the mode set control byte and the save pointer table first, so
        // the very first mode set already sees a well-formed chain, then mode
        // 03h through the same routine INT 10h AH=00h uses, including the plane
        // 2 font upload, the INT 1Fh/43h font vectors and the BDA video block.
        self.initialize_video_bda_state();
        self.bios_set_video_mode(0x03);
    }
}
