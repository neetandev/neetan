//! I/O port write dispatch for the X1.
//!
//! Decode mirrors [`super::io_read`]. The bitmap VRAM window is handled first
//! (and also captures writes below `0x4000` while the VRAM multi-plane latch is
//! set); the remaining device blocks live below `0x4000`.

use common::TraceSink;

use super::{SUB_CPU_PSG_WAIT_CYCLES, X1Bus, ppi_link::PpiEffect};

impl<T: TraceSink> X1Bus<T> {
    /// Writes a byte and reports whether the hardware decoded the port.
    pub fn io_write(&mut self, port: u16, value: u8) -> bool {
        match self.model {
            crate::config::X1Model::X1 | crate::config::X1Model::X1Turbo => {
                self.io_write_common(port, value)
            }
        }
    }

    fn io_write_common(&mut self, port: u16, value: u8) -> bool {
        if self.video.is_bitmap_write(port) {
            self.charge_vram_access_wait();
            self.video.write_bitmap(port, value);
            return true;
        }

        let handled = match port & 0xF000 {
            0x0000 => self.io_write_low(port, value),
            0x1000 => self.io_write_devices(port, value),
            0x2000 => {
                self.video.write_attr(port, value);
                true
            }
            0x3000 => {
                if self.model.has_kanji() && (port & 0x0800) != 0 {
                    self.video.write_kvram(port, value);
                } else {
                    self.video.write_text(port, value);
                }
                true
            }
            _ => false,
        };
        if matches!(port & 0xFF00, 0x1900 | 0x1B00 | 0x1C00) {
            self.add_wait_cycles(SUB_CPU_PSG_WAIT_CYCLES);
        }
        handled
    }

    /// Writes the 0x0000-0x0FFF block (ROM/CG window, bank register and FDC).
    fn io_write_low(&mut self, port: u16, value: u8) -> bool {
        match port {
            0x0700..=0x0707 if self.model.has_fm() => return self.fm_write(port, value),
            0x0B00 if self.model.is_turbo() => self.memory.set_ex_bank(value),
            0x0E80..=0x0E82 if self.model.has_kanji() => self.kanji_write(port & 0x03, value),
            0x0E00..=0x0E02 => {} // cartridge ROM address latch; no cartridge here
            0x0FF8..=0x0FFF => self.fdc_write(port, value),
            _ => return false,
        }
        true
    }

    /// Writes the CZ-8BS1 FM sound board block (`0x0700-0x0707`): the OPM address
    /// port `0x0700`, the OPM data port `0x0701`, and the paired sound-board CTC
    /// at `0x0704-0x0707`.
    fn fm_write(&mut self, port: u16, value: u8) -> bool {
        let now = self.current_cycle;
        match port {
            0x0700 => {
                if let Some(fm) = &mut self.fm {
                    fm.write_address(value, now);
                }
            }
            0x0701 => {
                if let Some(fm) = &mut self.fm {
                    fm.write_data(value, now);
                }
                self.apply_fm_timers();
            }
            0x0704..=0x0707 => {
                self.sound_ctc.write((port - 0x0704) as usize, value, now);
                self.sync_sound_ctc_schedule();
            }
            _ => return false,
        }
        true
    }

    /// Writes the 0x1000-0x1FFF device block.
    fn io_write_devices(&mut self, port: u16, value: u8) -> bool {
        match port & 0xFF00 {
            0x1000 => self.video.set_palette_blue(value),
            0x1100 => self.video.set_palette_red(value),
            0x1200 => self.video.set_palette_green(value),
            0x1300 => self.video.set_priority(value),
            // The CRT font page 0x1400 is read-only; only the PCG planes at
            // 0x1500-0x17FF accept writes.
            0x1400 => {}
            0x1500 | 0x1600 | 0x1700 => {
                let plane = ((port & 0x300) >> 8) as u8;
                if self.video.pcg_direct() {
                    // Turbo hi-speed PCG define: glyph row from the port address,
                    // character code from the PCG-select staging cell.
                    self.video.write_pcg_hispeed(port, plane, value);
                } else {
                    let (code, line) = self.beam_code_line();
                    self.video.write_pcg(code, line, plane, value);
                }
            }
            // The CRTC register pair mirrors every 0x10 ports through 0x18FF.
            0x1800 => match port & 0xFF0F {
                0x1800 => self.crtc.write_address(value),
                0x1801 => {
                    let register = self.crtc.state.address;
                    self.crtc.write_data(value);
                    self.on_crtc_register_write(register);
                }
                _ => return false,
            },
            0x1900 => self.sub.write_mailbox(value),
            0x1A00 => {
                let effect = self.ppi.write((port & 0x03) as u8, value);
                self.apply_ppi_effect(effect);
            }
            0x1B00 => self.psg.data_w_at(value, self.current_cycle),
            0x1C00 => self.psg.address_w(value),
            0x1D00 => self.memory.select_rom(),
            0x1E00 => self.memory.select_ram(),
            0x1F00 => return self.io_write_1f00(port, value),
            _ => return false,
        }
        true
    }

    /// Writes the 0x1F00-0x1FFF sub-block (CTC, and on turbo the DMA and SIO).
    fn io_write_1f00(&mut self, port: u16, value: u8) -> bool {
        let low = port & 0x00FF;
        match low {
            0xA0..=0xA3 | 0xA8..=0xAB => {
                if self.ctc_port_channel(port).is_none() {
                    return false;
                }
                self.ctc_write(port, value);
            }
            0xD0 if self.model.is_turbo() => self.video.write_mode1(value),
            0xE0 if self.model.is_turbo() => self.video.write_mode2(value),
            0x80..=0x8F if self.model.has_dma() => {
                // Control writes re-check the level-sensed ready line.
                self.refresh_dma_ready_line();
                self.dma.write(value);
                self.sync_interrupts();
                self.sync_dma_tick();
            }
            0x90..=0x93 if self.model.has_sio() => {
                let channel = ((port >> 1) & 1) as usize;
                if port & 1 == 0 {
                    self.sio.write_data(channel, value);
                } else {
                    self.sio.write_control(channel, value);
                    self.poll_mouse_rts();
                }
                self.sync_interrupts();
            }
            _ => return false,
        }
        true
    }

    fn apply_ppi_effect(&mut self, effect: PpiEffect) {
        if let PpiEffect::PortC {
            column40,
            vram_mode_latch,
            cassette_out: _,
        } = effect
        {
            self.column40 = column40;
            self.schedule_next_scanline_after(self.current_cycle);
            if vram_mode_latch {
                self.video.latch_vram_mode();
            }
        }
    }

    fn on_crtc_register_write(&mut self, register: u8) {
        match register {
            0 | 1 | 4 | 5 | 6 | 9 => self.reset_display_timing(),
            _ => {}
        }
    }

    fn ctc_write(&mut self, port: u16, value: u8) {
        if let Some(channel) = self.ctc_port_channel(port) {
            let now = self.current_cycle;
            self.ctc.write(channel, value, now);
            self.sync_ctc_schedule();
        }
    }
}
