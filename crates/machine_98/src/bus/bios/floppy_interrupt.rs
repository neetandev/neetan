//! INT 12h (640K FDC) and INT 13h (1MB FDC) result-drain interrupts.

use common::{Cpu, MachineModel};

use super::super::Pc9801Bus;
use crate::Tracing;

impl<T: Tracing> Pc9801Bus<T> {
    pub(super) fn hle_int12h(&mut self, _cpu: &mut impl Cpu) {
        // ISR-aware EOI: always EOI slave, only EOI master if slave ISR is clear.
        self.pic.write_port0(1, 0x20);
        if self.pic.state.chips[1].isr == 0 {
            self.pic.write_port0(0, 0x20);
        }

        if self.machine_model == MachineModel::PC9801F {
            loop {
                let fdc = self.floppy.fdc_640k_mut();
                let status = fdc.read_status();
                if status & 0x10 == 0 {
                    if status & 0xC0 != 0x80 {
                        return;
                    }
                    fdc.write_data(0x08);
                }

                let fdc_status = self.floppy.fdc_640k_mut().read_status();
                if fdc_status & 0xD0 != 0xD0 {
                    return;
                }
                let st0 = self.floppy.fdc_640k_mut().read_data();
                if st0 == 0x80 {
                    return;
                }

                let mut buf = [0u8; 7];
                let _ = Self::fdc_drain_results(self.floppy.fdc_640k_mut(), &mut buf);
            }
        }

        // Loop to drain all pending FDC results.
        loop {
            let fdc = self.floppy.fdc_640k_mut();
            let status = fdc.read_status();

            // If FDC is not busy (CB clear), issue SENSE INTERRUPT STATUS.
            if status & 0x10 == 0 {
                if status & 0xC0 != 0x80 {
                    return;
                }
                fdc.write_data(0x08);
            }

            // Read first result byte (ST0).
            let fdc_status = self.floppy.fdc_640k_mut().read_status();
            if fdc_status & 0xD0 != 0xD0 {
                return;
            }
            let st0 = self.floppy.fdc_640k_mut().read_data();

            if st0 == 0x80 {
                if self.memory.state.ram[0x05D7] > 0 {
                    self.memory.state.ram[0x05D7] -= 1;
                }
                return;
            }

            let drive = (st0 & 0x03) as usize;
            let flag_bit = 0x10u8 << drive;

            let result_base = if st0 & 0xA0 != 0 {
                0x05D8 + drive * 2
            } else {
                0x05D0
            };

            self.memory.state.ram[result_base] = st0;
            let mut buf = [0u8; 7];
            let extra = Self::fdc_drain_results(self.floppy.fdc_640k_mut(), &mut buf);
            self.memory.state.ram[result_base + 1..result_base + 1 + extra]
                .copy_from_slice(&buf[..extra]);

            self.memory.state.ram[0x055F] |= flag_bit;
        }
    }

    pub(super) fn hle_int13h(&mut self, _cpu: &mut impl Cpu) {
        // ISR-aware EOI: always EOI slave, only EOI master if slave ISR is clear.
        self.pic.write_port0(1, 0x20);
        if self.pic.state.chips[1].isr == 0 {
            self.pic.write_port0(0, 0x20);
        }

        // Loop to drain all pending FDC results.
        loop {
            let fdc = self.floppy.fdc_1mb_mut();
            let status = fdc.read_status();

            if status & 0x10 == 0 {
                if status & 0xC0 != 0x80 {
                    break;
                }
                fdc.write_data(0x08);
            }

            let fdc_status = self.floppy.fdc_1mb_mut().read_status();
            if fdc_status & 0xD0 != 0xD0 {
                break;
            }
            let st0 = self.floppy.fdc_1mb_mut().read_data();

            if st0 == 0x80 {
                break;
            }

            let drive = (st0 & 0x03) as usize;
            let flag_bit = 1u8 << drive;

            let result_base = 0x0564 + drive * 8;
            self.memory.state.ram[result_base] = st0;

            let mut buf = [0u8; 7];
            let extra = Self::fdc_drain_results(self.floppy.fdc_1mb_mut(), &mut buf);
            self.memory.state.ram[result_base + 1..result_base + 1 + extra]
                .copy_from_slice(&buf[..extra]);

            self.memory.state.ram[0x055E] |= flag_bit;
        }

        // Motor-off timer: decrement counter, mark drives for motor-off at zero.
        if self.memory.state.ram[0x0480] & 0x10 != 0 && self.memory.state.ram[0x0485] > 0 {
            self.memory.state.ram[0x0485] -= 1;
            if self.memory.state.ram[0x0485] == 0 {
                self.memory.state.ram[0x05A4] |= 0x0F;
            }
        }
    }
}
