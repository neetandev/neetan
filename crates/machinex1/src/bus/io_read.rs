//! I/O port read dispatch for the X1.
//!
//! The Z80 I/O space is decoded on the full 16-bit port. The bitmap VRAM window
//! (`0x4000-0xFFFF`) is handled first; the remaining device blocks live below
//! `0x4000`. Any read clears the VRAM multi-plane latch.

use common::Tracing;

use super::{OPEN_BUS, SUB_CPU_PSG_WAIT_CYCLES, X1Bus};
use crate::interrupt::IrqSource;

impl<T: Tracing> X1Bus<T> {
    /// Reads a byte from an I/O port.
    pub fn io_read(&mut self, port: u16) -> u8 {
        match self.model {
            crate::config::X1Model::X1 | crate::config::X1Model::X1Turbo => {
                self.io_read_common(port)
            }
        }
    }

    fn io_read_common(&mut self, port: u16) -> u8 {
        self.video.clear_vram_mode();
        if self.video.is_bitmap_read(port) {
            self.charge_vram_access_wait();
            return self.video.read_bitmap(port);
        }

        let value = match port & 0xF000 {
            0x0000 => self.io_read_low(port),
            0x1000 => self.io_read_devices(port),
            0x2000 => self.video.read_attr(port),
            0x3000 => {
                // On turbo the upper half of the text window is the kanji plane
                // (kvram); on the base X1 it mirrors text VRAM.
                if self.model.has_kanji() && (port & 0x0800) != 0 {
                    self.video.read_kvram(port)
                } else {
                    self.video.read_text(port)
                }
            }
            _ => OPEN_BUS,
        };
        if matches!(port & 0xFF00, 0x1900 | 0x1B00 | 0x1C00) {
            self.add_wait_cycles(SUB_CPU_PSG_WAIT_CYCLES);
        }
        value
    }

    /// Reads the 0x0000-0x0FFF block (ROM/CG window, bank register and FDC).
    fn io_read_low(&mut self, port: u16) -> u8 {
        match port {
            0x0700..=0x0707 if self.model.has_fm() => self.fm_read(port),
            0x0B00 if self.model.is_turbo() => self.memory.ex_bank(),
            0x0E80 | 0x0E81 if self.model.has_kanji() => self.kanji_data_read(port & 0x01),
            0x0E03 => 0x00, // cartridge ROM read; no cartridge on the base machine
            0x0FF8..=0x0FFF => self.fdc_read(port),
            _ => OPEN_BUS,
        }
    }

    /// Reads the CZ-8BS1 FM sound board block (`0x0700-0x0707`): the OPM address
    /// port `0x0700` (FM detection, reads back 0x00), the OPM status port
    /// `0x0701`, and the paired sound-board CTC at `0x0704-0x0707`.
    fn fm_read(&mut self, port: u16) -> u8 {
        let now = self.current_cycle;
        match port {
            0x0700 => 0x00,
            0x0701 => match &mut self.fm {
                Some(fm) => fm.read_status(now),
                None => OPEN_BUS,
            },
            0x0704..=0x0707 => self.sound_ctc.read((port - 0x0704) as usize, now),
            _ => OPEN_BUS,
        }
    }

    /// Reads the 0x1000-0x1FFF device block.
    fn io_read_devices(&mut self, port: u16) -> u8 {
        match port & 0xFF00 {
            0x1400 | 0x1500 | 0x1600 | 0x1700 => {
                let plane = ((port & 0x300) >> 8) as u8;
                if self.model.is_turbo() && self.video.pcg_direct() {
                    if plane == 0 {
                        self.pcg_direct_font_read(port)
                    } else {
                        self.video.read_pcg_hispeed(port, plane)
                    }
                } else {
                    let (code, line) = self.beam_code_line();
                    self.video.read_pcg(code, line, plane, &self.cg_rom)
                }
            }
            // The CRTC register pair mirrors every 0x10 ports through 0x18FF.
            0x1800 => match port & 0xFF0F {
                0x1801 => self.crtc.read_data(),
                _ => OPEN_BUS,
            },
            0x1900 => {
                let value = self.sub.read_mailbox();
                // A mailbox read consumes a pending interrupt's vector byte, so
                // reflect the dropped request into the daisy chain right away.
                self.interrupt
                    .set(IrqSource::Keyboard, self.sub.key_irq_pending());
                value
            }
            0x1A00 => {
                self.advance_cassette();
                let port_b = self.port_b();
                let value = self.ppi.read((port & 0x03) as u8, port_b);
                if port & 0x03 == 1 {
                    self.detect_vblank_poll(value);
                }
                value
            }
            0x1B00 => {
                self.psg.set_port_a_input(self.joystick_p1);
                self.psg.set_port_b_input(self.joystick_p2);
                self.psg.data_r()
            }
            0x1F00 => self.io_read_1f00(port),
            _ => OPEN_BUS,
        }
    }

    /// Reads the 0x1F00-0x1FFF sub-block (CTC, the turbo DMA and SIO, and the DIP
    /// switch).
    fn io_read_1f00(&mut self, port: u16) -> u8 {
        let low = port & 0x00FF;
        match low {
            0xA0..=0xA3 | 0xA8..=0xAB => self.ctc_read(port),
            0x80..=0x8F if self.model.has_dma() => {
                // The live status byte reports the level-sensed ready line.
                self.refresh_dma_ready_line();
                self.dma.read()
            }
            0x90..=0x93 if self.model.has_sio() => {
                let channel = ((port >> 1) & 1) as usize;
                if port & 1 == 0 {
                    // Reading the data register drains the receive FIFO, which may
                    // clear the receive interrupt; reflect that into the daisy chain.
                    let value = self.sio.read_data(channel);
                    self.sync_interrupts();
                    value
                } else {
                    self.sio.read_control(channel)
                }
            }
            0xF0 if self.model.is_turbo() => self.dip_switch(),
            _ => OPEN_BUS,
        }
    }

    /// The CTC channel a `0x1Fxx` port selects. The turbo wires the CTC at
    /// `0x1FA0-0x1FA3`; the base X1 (CZ-8BC1 expansion) at `0x1FA8-0x1FAB`.
    pub(super) fn ctc_port_channel(&self, port: u16) -> Option<usize> {
        let low = port & 0x00FF;
        let base = match self.model {
            crate::config::X1Model::X1 => 0xA8,
            crate::config::X1Model::X1Turbo => 0xA0,
        };
        (base..base + 4)
            .contains(&low)
            .then(|| usize::from(low - base))
    }

    fn ctc_read(&self, port: u16) -> u8 {
        match self.ctc_port_channel(port) {
            Some(channel) => self.ctc.read(channel, self.current_cycle),
            None => OPEN_BUS,
        }
    }
}
