//! I/O port reads, dispatched per machine generation.

use common::Tracing;

use super::{OPEN_BUS, Pc6000Bus};
use crate::config::Pc6000Model;

impl<T: Tracing> Pc6000Bus<T> {
    pub fn io_read(&mut self, port: u16) -> u8 {
        match port & 0xF0 {
            0x90 => self
                .ppi
                .read((port & 0x03) as u8, self.sub.current_keycode()),
            0xA0 => self.psg_read(port),
            0xB0 if !self.model.is_sr() && self.model.has_builtin_fdd() && port & 0xFF == 0xB2 => {
                self.fdc_read(port)
            }
            0xD0 if self.model.has_builtin_fdd() => self.fdc_read(port),
            _ => self.io_read_extended(port),
        }
    }

    /// Reads the generation-specific ports. The base PC-6001 leaves them as
    /// open bus.
    fn io_read_extended(&mut self, port: u16) -> u8 {
        match self.model {
            Pc6000Model::Pc6001 => OPEN_BUS,
            Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6601 => self.io_read_mk2(port),
            Pc6000Model::Pc6001Mk2Sr | Pc6000Model::Pc6601Sr => self.io_read_sr(port),
        }
    }

    /// Reads the PC-6001mkII / PC-6601 ports.
    fn io_read_mk2(&mut self, port: u16) -> u8 {
        match port & 0xFF {
            0xE0..=0xEF => self.voice.read((port & 0x03) as u8),
            0xF0 => self
                .memory
                .banked()
                .map_or(OPEN_BUS, |memory| memory.read_bank_low()),
            0xF1 => self
                .memory
                .banked()
                .map_or(OPEN_BUS, |memory| memory.read_bank_high()),
            0xF2 => self
                .memory
                .banked()
                .map_or(OPEN_BUS, |memory| memory.write_bank()),
            _ => OPEN_BUS,
        }
    }

    /// Reads the PC-6001mkIISR / PC-6601SR ports.
    fn io_read_sr(&mut self, port: u16) -> u8 {
        match port & 0xFF {
            0x60..=0x67 => self
                .memory
                .sr()
                .map_or(OPEN_BUS, |memory| memory.read_page((port & 0x07) as usize)),
            0x68..=0x6F => self
                .memory
                .sr()
                .map_or(OPEN_BUS, |memory| memory.write_page((port & 0x07) as usize)),
            0x80 => self.serial.read_data().0,
            0x81 => self.serial.read_status(),
            0xB2 => {
                if self.model == Pc6000Model::Pc6601Sr {
                    0x03
                } else {
                    0x01
                }
            }
            0xB8..=0xBF => self.interrupt.sr_vector((port & 0x07) as usize),
            0xE0..=0xEF => self.voice.read((port & 0x03) as u8),
            0xF0 => self
                .memory
                .sr()
                .map_or(OPEN_BUS, |memory| memory.compat_read_bank_low()),
            0xF1 => self
                .memory
                .sr()
                .map_or(OPEN_BUS, |memory| memory.compat_read_bank_high()),
            0xF2 => self
                .memory
                .sr()
                .map_or(OPEN_BUS, |memory| memory.compat_write_bank()),
            _ => OPEN_BUS,
        }
    }
}
