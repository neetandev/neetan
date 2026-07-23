//! INT 11h equipment list, INT 12h memory size, INT 15h system services.

use common::{Cpu, SegmentRegister, TraceSink};

use super::{AtBus, BIOS_CODE_SEGMENT, METADATA_CONFIG_TABLE, timer::RTC_REG_B_PIE};

/// BIOS data area: equipment word.
const BDA_EQUIPMENT: u32 = 0x410;
/// BIOS data area: base memory size in KiB (word).
const BDA_MEMORY_SIZE: u32 = 0x413;
/// CMOS register: extended memory KiB, AMI mirror low byte.
const CMOS_EXTENDED_MEMORY_LOW: usize = 0x30;
/// CMOS register: extended memory KiB, AMI mirror high byte.
const CMOS_EXTENDED_MEMORY_HIGH: usize = 0x31;
/// INT 15h error code: function not supported.
const INT15H_ERROR_UNSUPPORTED: u8 = 0x86;
/// System control port A with the fast A20 gate.
const FAST_GATE_PORT: u16 = 0x92;
/// Port 0x92 bit 1: fast A20 gate enable.
const FAST_GATE_A20_ENABLE: u8 = 0x02;
/// KBC command port.
const KBC_COMMAND_PORT: u16 = 0x64;
/// KBC data port.
const KBC_DATA_PORT: u16 = 0x60;
/// KBC command: write output port.
const KBC_WRITE_OUTPUT_PORT: u8 = 0xD1;
/// KBC output port value: A20 off, reset line high, keyboard lines idle.
const KBC_OUTPUT_A20_OFF: u8 = 0xDD;
/// INT 15h AH=24h AL=03h support bitmap: the KBC and port 0x92 both work.
const A20_SUPPORT_KBC_AND_FAST_GATE: u16 = 0x0003;
/// BIOS data area: event wait user flag pointer offset (word).
const BDA_WAIT_POINTER_OFFSET: u32 = 0x498;
/// BIOS data area: event wait user flag pointer segment (word).
const BDA_WAIT_POINTER_SEGMENT: u32 = 0x49A;
/// BIOS data area: event wait microsecond count (dword).
const BDA_WAIT_COUNT: u32 = 0x49C;
/// BIOS data area: event wait active flag.
const BDA_WAIT_ACTIVE: u32 = 0x4A0;
/// BDA 40:A0 bit 0: an event wait interval is armed.
const WAIT_ACTIVE_BIT: u8 = 0x01;

impl<T: TraceSink> AtBus<T> {
    /// INT 11h: returns the BDA equipment word in AX.
    pub(super) fn hle_int11h(&mut self, cpu: &mut impl Cpu) {
        let equipment = self.read_mem_word(BDA_EQUIPMENT);
        cpu.set_ax(equipment);
    }

    /// INT 12h: returns the BDA base memory size in KiB in AX.
    pub(super) fn hle_int12h(&mut self, cpu: &mut impl Cpu) {
        let memory_size = self.read_mem_word(BDA_MEMORY_SIZE);
        cpu.set_ax(memory_size);
    }

    /// INT 15h system services dispatch. AH=86h (wait) never arrives here:
    /// the ROM stub intercepts it and busy-waits on the refresh toggle.
    pub(super) fn hle_int15h(&mut self, cpu: &mut impl Cpu) {
        match cpu.ah() {
            0x24 => self.int15h_gate_a20(cpu),
            0x4F => self.int15h_keyboard_intercept(),
            0x83 => self.int15h_event_wait(cpu),
            0x87 => self.int15h_extended_memory_move(cpu),
            0x88 => self.int15h_extended_memory_size(cpu),
            0x90 | 0x91 => self.int15h_wait_hooks(cpu),
            0xC0 => self.int15h_configuration_table(cpu),
            // Explicitly unsupported: AH=49h (DBCS BIOS check, plain VGA
            // clone), AH=89h (switch to protected mode), AH=C1h (no EBDA),
            // AH=C2h (no PS/2 mouse; the serial mouse needs no BIOS).
            0x49 | 0x89 | 0xC1 | 0xC2 => self.int15h_unsupported(cpu),
            _ => self.int15h_unsupported(cpu),
        }
    }

    /// AH=83h: event wait. AL=00h arms an interval that sets bit 7 of the
    /// byte at ES:BX after CX:DX microseconds, driven by the RTC periodic
    /// interrupt through the INT 70h handler. AL=01h cancels it.
    fn int15h_event_wait(&mut self, cpu: &mut impl Cpu) {
        match cpu.al() {
            0x00 => {
                if self.read_mem_byte(BDA_WAIT_ACTIVE) & WAIT_ACTIVE_BIT != 0 {
                    self.set_iret_cf(cpu, true);
                    return;
                }
                self.write_mem_word(BDA_WAIT_POINTER_OFFSET, cpu.bx());
                self.write_mem_word(BDA_WAIT_POINTER_SEGMENT, cpu.es());
                let microseconds = (u32::from(cpu.cx()) << 16) | u32::from(cpu.dx());
                self.write_mem_dword(BDA_WAIT_COUNT, microseconds);
                self.write_mem_byte(BDA_WAIT_ACTIVE, WAIT_ACTIVE_BIT);
                self.rtc_select_periodic_rate_1024hz();
                self.rtc_update_reg_b(RTC_REG_B_PIE, 0);
                self.set_iret_cf(cpu, false);
            }
            0x01 => {
                self.write_mem_byte(BDA_WAIT_ACTIVE, 0);
                self.rtc_update_reg_b(0, RTC_REG_B_PIE);
                self.set_iret_cf(cpu, false);
            }
            _ => self.int15h_unsupported(cpu),
        }
    }

    /// AH=90h device busy / AH=91h interrupt complete: default hooks that
    /// report "not handled" with AH=00h and the carry flag clear.
    fn int15h_wait_hooks(&mut self, cpu: &mut impl Cpu) {
        cpu.set_ah(0x00);
        self.set_iret_cf(cpu, false);
    }

    /// AH=4Fh: keyboard intercept. The default handler returns with AL (the
    /// scancode) and the carry flag untouched. The INT 09h stub calls with
    /// CF set, so every key passes unless a guest hook clears CF.
    fn int15h_keyboard_intercept(&mut self) {}

    /// AH=24h: A20 gate control. The gates are driven through the guest
    /// visible I/O paths so the chipset and memory mask update exactly as a
    /// hardware access would.
    fn int15h_gate_a20(&mut self, cpu: &mut impl Cpu) {
        match cpu.al() {
            0x00 => {
                // The effective gate is fast gate OR KBC gate, so a robust
                // disable drops both.
                self.io_write(FAST_GATE_PORT, 0x00);
                self.io_write(KBC_COMMAND_PORT, KBC_WRITE_OUTPUT_PORT);
                self.io_write(KBC_DATA_PORT, KBC_OUTPUT_A20_OFF);
                cpu.set_ah(0x00);
                self.set_iret_cf(cpu, false);
            }
            0x01 => {
                self.io_write(FAST_GATE_PORT, FAST_GATE_A20_ENABLE);
                cpu.set_ah(0x00);
                self.set_iret_cf(cpu, false);
            }
            0x02 => {
                cpu.set_al(u8::from(self.chipset.a20_enabled()));
                cpu.set_ah(0x00);
                self.set_iret_cf(cpu, false);
            }
            0x03 => {
                cpu.set_bx(A20_SUPPORT_KBC_AND_FAST_GATE);
                cpu.set_ah(0x00);
                self.set_iret_cf(cpu, false);
            }
            _ => self.int15h_unsupported(cpu),
        }
    }

    /// Reads the 32-bit linear base of an AH=87h segment descriptor (bytes
    /// 2-4 hold bits 0-23, byte 7 holds bits 24-31 on the 386 and later).
    fn int15h_descriptor_base(&mut self, descriptor: u32) -> u32 {
        u32::from(self.read_mem_byte(descriptor + 2))
            | (u32::from(self.read_mem_byte(descriptor + 3)) << 8)
            | (u32::from(self.read_mem_byte(descriptor + 4)) << 16)
            | (u32::from(self.read_mem_byte(descriptor + 7)) << 24)
    }

    /// AH=87h: copies CX words between the linear addresses described by the
    /// source and target descriptors of the table at ES:SI. The A20 gate is
    /// enabled for the copy and restored afterwards, like the real BIOS.
    fn int15h_extended_memory_move(&mut self, cpu: &mut impl Cpu) {
        let table = (u32::from(cpu.es()) << 4).wrapping_add(u32::from(cpu.si()));
        let source = self.int15h_descriptor_base(table + 0x10);
        let target = self.int15h_descriptor_base(table + 0x18);
        let bytes = u32::from(cpu.cx()) * 2;

        let saved_fast_gate = self.io_read(FAST_GATE_PORT).0;
        self.io_write(FAST_GATE_PORT, saved_fast_gate | FAST_GATE_A20_ENABLE);
        for offset in 0..bytes {
            let value = self.hle_physical_read_byte(source.wrapping_add(offset));
            self.hle_physical_write_byte(target.wrapping_add(offset), value);
        }
        self.io_write(FAST_GATE_PORT, saved_fast_gate);

        cpu.set_ah(0x00);
        self.set_iret_cf(cpu, false);
        self.set_iret_zf(cpu, true);
    }

    /// AH=88h: returns the extended memory size in KiB from the CMOS mirror.
    fn int15h_extended_memory_size(&mut self, cpu: &mut impl Cpu) {
        let extended_kib = u16::from(self.rtc.cmos[CMOS_EXTENDED_MEMORY_LOW])
            | (u16::from(self.rtc.cmos[CMOS_EXTENDED_MEMORY_HIGH]) << 8);
        cpu.set_ax(extended_kib);
        self.set_iret_cf(cpu, false);
    }

    /// AH=C0h: returns the ROM configuration table at ES:BX.
    fn int15h_configuration_table(&mut self, cpu: &mut impl Cpu) {
        let table_offset = u16::from(self.memory.bios_byte(METADATA_CONFIG_TABLE))
            | (u16::from(self.memory.bios_byte(METADATA_CONFIG_TABLE + 1)) << 8);
        cpu.load_segment_real_mode(SegmentRegister::ES, BIOS_CODE_SEGMENT);
        cpu.set_bx(table_offset);
        cpu.set_ah(0x00);
        self.set_iret_cf(cpu, false);
    }

    /// Unsupported INT 15h function: AH=86h with the carry flag set.
    fn int15h_unsupported(&mut self, cpu: &mut impl Cpu) {
        cpu.set_ah(INT15H_ERROR_UNSUPPORTED);
        self.set_iret_cf(cpu, true);
    }
}
