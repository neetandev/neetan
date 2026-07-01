//! I/O port writes, dispatched per machine generation.

use common::Tracing;

use super::Pc6000Bus;
use crate::config::Pc6000Model;

impl<T: Tracing> Pc6000Bus<T> {
    pub fn io_write(&mut self, port: u16, value: u8) {
        match port & 0xF0 {
            0x90 => {
                let effect = self.ppi.write((port & 0x03) as u8, value);
                self.apply_ppi_effect(effect);
                // Blanking the CRT releases the video bus-request immediately.
                if !self.ppi.crt_enabled() {
                    self.release_busreq();
                }
            }
            0xA0 => self.psg_write(port, value),
            0xB0 => match port & 0xFF {
                // On machines with the built-in drive the 0xB1-0xB3 block is the
                // FDD subsystem (interface select at 0xB1, controller status at
                // 0xB2/0xB3), not a mirror of the system latch. The disk-boot
                // routine drives those ports with bit 3 set; routing them to the
                // latch would spuriously spin the cassette motor and run the tape
                // off its leader before CLOAD ever starts.
                0xB1 if self.model.has_builtin_fdd() => self.fdc_write(port, value),
                0xB2 | 0xB3 if self.model.has_builtin_fdd() => {}
                0xB8..=0xBF if self.model.is_sr() => {
                    self.interrupt.set_sr_vector((port & 0x07) as usize, value);
                }
                _ => self.system_latch_write(value),
            },
            0xD0 if self.model.has_builtin_fdd() => self.fdc_write(port, value),
            _ => self.io_write_extended(port, value),
        }
    }

    /// Writes the generation-specific ports. The base PC-6001 ignores them.
    fn io_write_extended(&mut self, port: u16, value: u8) {
        match self.model {
            Pc6000Model::Pc6001 => {}
            Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6601 => self.io_write_mk2(port, value),
            Pc6000Model::Pc6001Mk2Sr | Pc6000Model::Pc6601Sr => self.io_write_sr(port, value),
        }
    }

    /// Writes the PC-6001mkIISR / PC-6601SR ports.
    fn io_write_sr(&mut self, port: u16, value: u8) {
        match port & 0xFF {
            // Palette CLUT ports exist but the SR palette is fixed in hardware.
            0x40..=0x43 => {}
            0x80 => self.serial.write_data(value),
            0x81 => self.serial.write_command(value),
            0x60..=0x67 => {
                if let Some(memory) = self.memory.sr_mut() {
                    memory.set_read_page((port & 0x07) as usize, value);
                }
            }
            0x68..=0x6F => {
                if let Some(memory) = self.memory.sr_mut() {
                    memory.set_write_page((port & 0x07) as usize, value);
                }
            }
            0xC0 => self.bgcol_bank = value & 0x07,
            0xC1 => {
                self.sr_width80 = value & 0x02 == 0;
                if self.sr_compat {
                    self.set_video_mode_register(value);
                }
            }
            0xC2 => {
                if let Some(memory) = self.memory.sr_mut() {
                    memory.set_compat_opt_bank(value);
                }
            }
            0xC8 => self.set_sr_mode_register(value),
            0xC9 => {
                if let Some(memory) = self.memory.sr_mut() {
                    memory.set_text_bank(value);
                }
            }
            0xCA..=0xCC => self.set_sr_scroll(port, value),
            0xCE | 0xCF => self.set_sr_bitmap_offset(port, value),
            0xE0..=0xEF => self.voice_write((port & 0x03) as u8, value),
            0xF0 => {
                if let Some(memory) = self.memory.sr_mut() {
                    memory.set_compat_read_bank_low(value);
                }
            }
            0xF1 => {
                if let Some(memory) = self.memory.sr_mut() {
                    memory.set_compat_read_bank_high(value);
                }
            }
            0xF2 => {
                if let Some(memory) = self.memory.sr_mut() {
                    memory.set_compat_write_bank(value);
                }
            }
            0xF3 => self.timer_irq_masked = value & 0x04 != 0,
            0xF6 => self.set_timer_divider(value),
            0xF7 => self.interrupt.set_timer_vector(value),
            _ => {}
        }
    }

    fn io_write_mk2(&mut self, port: u16, value: u8) {
        match port & 0xFF {
            0xC0 => self.bgcol_bank = value & 0x07,
            0xC1 => self.set_video_mode_register(value),
            0xC2 => {
                if let Some(memory) = self.memory.banked_mut() {
                    memory.set_opt_bank(value);
                }
            }
            0xE0..=0xEF => self.voice_write((port & 0x03) as u8, value),
            0xF0 => {
                if let Some(memory) = self.memory.banked_mut() {
                    memory.set_read_bank_low(value);
                }
            }
            0xF1 => {
                if let Some(memory) = self.memory.banked_mut() {
                    memory.set_read_bank_high(value);
                }
            }
            0xF2 => {
                if let Some(memory) = self.memory.banked_mut() {
                    memory.set_write_bank(value);
                }
            }
            0xF3 => self.timer_irq_masked = value & 0x04 != 0,
            0xF6 => self.set_timer_divider(value),
            0xF7 => self.interrupt.set_timer_vector(value),
            _ => {}
        }
    }
}
