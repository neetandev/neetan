//! Mirrored system-port register block.

use common::{CpuMode, TraceSink};

use super::X68kBus;
use crate::{InterruptSource, X68kModel};

impl<T: TraceSink> X68kBus<T> {
    /// Reads a system-port register.
    pub(super) fn read_system_port(&self, address: u32) -> u8 {
        match address & 0x0F {
            0x01 => 0xF0 | self.contrast,
            0x03 => self.monitor_control,
            0x05 => self.color_latch,
            0x07 => 0xF4 | self.key_control & 0x0A,
            0x0B if self.model == X68kModel::X68000Xvi && self.cpu_mode == CpuMode::High => 0xFE,
            0x0B => 0xFF,
            _ => 0xFF,
        }
    }

    /// Writes a system-port register.
    pub(super) fn write_system_port(&mut self, address: u32, value: u8) {
        match address & 0x0F {
            0x01 => {
                self.catch_up_video();
                self.contrast = value & 0x0F;
            }
            0x03 => {
                self.catch_up_video();
                self.monitor_control = value;
            }
            0x05 => self.color_latch = value,
            0x07 => {
                self.catch_up_video();
                let change = self.crtc.set_hrl(value & 0x02 != 0);
                if change.clock {
                    self.crtc_remainder = 0;
                }
                self.key_control = value & 0x0A;
                self.keyboard.set_system_transmit_enabled(value & 0x08 != 0);
                if value & 0x04 != 0 {
                    self.interrupts.clear(InterruptSource::Nmi);
                }
                self.shuttle_keyboard_serial();
                self.schedule_events();
            }
            0x0D => self.sram_write_enabled = value == 0x31,
            0x0F => self.advance_shutdown_sequence(value),
            _ => {}
        }
    }

    /// Advances the shutdown command sequence.
    fn advance_shutdown_sequence(&mut self, value: u8) {
        self.shutdown_sequence = match (self.shutdown_sequence, value) {
            (0, 0x00) => 1,
            (1, 0x0F) => 2,
            (2, 0x0F) => {
                self.shutdown_requested = true;
                0
            }
            _ => 0,
        };
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, M68000AccessSize, M68000FunctionCode};

    use crate::{
        X68kModel,
        bus::test_support::{access, bus},
    };

    #[test]
    fn system_ports_gate_sram_and_shutdown() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        let sram_write = access(0xED0100, M68000AccessSize::Byte, supervisor);
        bus.m68000_write(sram_write, 0xA5).unwrap();
        assert_ne!(bus.sram_data()[0x100], 0xA5);
        bus.m68000_write(access(0xE8E00D, M68000AccessSize::Byte, supervisor), 0x31)
            .unwrap();
        bus.m68000_write(sram_write, 0xA5).unwrap();
        assert_eq!(bus.sram_data()[0x100], 0xA5);
        for value in [0, 0x0F, 0x0F] {
            bus.m68000_write(access(0xE8E00F, M68000AccessSize::Byte, supervisor), value)
                .unwrap();
        }
        assert!(bus.shutdown_requested());
    }

    #[test]
    fn register_word_access_uses_only_the_lower_lane_and_mirrors_system_ports() {
        let mut bus = bus(X68kModel::X68000Xvi);
        let supervisor = M68000FunctionCode::SupervisorData;
        assert_eq!(
            bus.m68000_read(access(0xE8E00A, M68000AccessSize::Word, supervisor)),
            Ok(0xFFFE)
        );
        bus.m68000_write(access(0xE8E01C, M68000AccessSize::Word, supervisor), 0xAA31)
            .unwrap();
        bus.m68000_write(access(0xED0100, M68000AccessSize::Word, supervisor), 0x1234)
            .unwrap();
        assert_eq!(&bus.sram_data()[0x100..0x102], &[0x12, 0x34]);
    }
}
