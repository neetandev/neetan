//! INT 1Ah CMT (cassette tape) stubs and printer service.

use common::{Cpu, MachineModel};

use super::{super::Pc9801Bus, PIT_CLOCK_8MHZ_LINEAGE};
use crate::Tracing;

impl<T: Tracing> Pc9801Bus<T> {
    pub(super) fn hle_int1ah(&mut self, cpu: &mut impl Cpu) {
        let result_ah = match cpu.ah() {
            // CMT functions (0x00..0x05): stubs.
            // VM/VX have no CMT hardware - AH=04h returns 0x00, AH=05h returns 0x27.
            // RA returns 0x02 for both (unsupported device).
            0x00 | 0x01 => 0x00,
            0x02 | 0x03 => 0x00,
            0x04 if self.machine_model == MachineModel::PC9801F => 0x00,
            0x04 if self.clocks.pit_clock_hz == PIT_CLOCK_8MHZ_LINEAGE => 0x02,
            0x04 => 0x00,
            0x05 if self.machine_model == MachineModel::PC9801F => 0x27,
            0x05 if self.clocks.pit_clock_hz == PIT_CLOCK_8MHZ_LINEAGE => 0x02,
            0x05 => 0x27,
            // Printer functions.
            0x10 => {
                self.system_ppi.write_control(0x0D);
                self.printer.write_control(0x82);
                self.printer.write_control(0x0F);
                self.system_ppi.write_control(0x0C);
                u8::from(self.printer.is_ready())
            }
            0x11 if self.printer.is_ready() => {
                self.printer.write_data(cpu.al());
                let old_c = self.printer.read_port_c();
                self.printer.write_port_c(old_c | 0x80);
                self.printer.write_port_c(old_c & !0x80);
                0x01
            }
            0x11 => 0x00,
            0x12 if self.printer.is_ready() => 0x01,
            0x12 => 0x00,
            0x30 => {
                let count = cpu.cx();
                if count == 0 {
                    0x00
                } else {
                    let src_seg = cpu.es();
                    let mut src_off = cpu.bx();
                    let src_base = (u32::from(src_seg) << 4).wrapping_add(u32::from(src_off));
                    let mut remaining = count;
                    for i in 0..count {
                        if !self.printer.is_ready() {
                            cpu.set_ah(0x00);
                            cpu.set_cx(remaining);
                            cpu.set_bx(src_off);
                            return;
                        }
                        let byte = self.read_mem_byte(src_base + u32::from(i));
                        self.printer.write_data(byte);
                        let old_c = self.printer.read_port_c();
                        self.printer.write_port_c(old_c | 0x80);
                        self.printer.write_port_c(old_c & !0x80);
                        src_off = src_off.wrapping_add(1);
                        remaining -= 1;
                    }
                    cpu.set_cx(0x0000);
                    cpu.set_bx(src_off);
                    0x00
                }
            }
            _ => 0x00,
        };

        cpu.set_ah(result_ah);
    }
}
