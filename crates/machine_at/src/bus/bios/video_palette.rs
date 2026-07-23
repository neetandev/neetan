//! INT 10h palette services: the CGA palette interface (AH=0Bh) and the
//! ATC/DAC services (AH=10h).
//!
//! Register writes go through the I/O ports with the real attribute
//! controller flip-flop protocol; read-backs come from the device state
//! directly.

use common::{Cpu, TraceSink};
use device::vga::{
    VGA_PORT_ATC_WRITE, VGA_PORT_DAC_DATA, VGA_PORT_DAC_WRITE_INDEX, VGA_PORT_STATUS_COLOR,
    VGA_PORT_STATUS_MONO,
};

use super::{super::AtBus, video::BDA_CGA_PALETTE, video_modes::ModeFamily};

/// ATC register index of the CGA emulation mode control.
const ATC_MODE_CONTROL: usize = 0x10;
/// ATC register index of the overscan (border) color.
const ATC_OVERSCAN: usize = 0x11;
/// ATC register index of the color select (page) register.
const ATC_COLOR_SELECT: usize = 0x14;
/// Number of DAC palette entries.
pub(super) const DAC_ENTRIES: u16 = 256;
/// Largest value a six bit DAC component can hold.
const DAC_COMPONENT_MAX: u32 = 0x3F;
/// Gray-scale weight of the red component, 0.30 in 8.8 fixed point.
const GRAY_WEIGHT_RED: u32 = 77;
/// Gray-scale weight of the green component, 0.59 in 8.8 fixed point.
const GRAY_WEIGHT_GREEN: u32 = 151;
/// Gray-scale weight of the blue component, 0.11 in 8.8 fixed point.
const GRAY_WEIGHT_BLUE: u32 = 28;
/// Half of one in 8.8 fixed point, the round-to-nearest bias.
const GRAY_ROUNDING_BIAS: u32 = 0x80;

impl<T: TraceSink> AtBus<T> {
    /// Writes one attribute controller register with the flip-flop protocol
    /// and re-enables the palette address source.
    fn atc_register_write(&mut self, index: u8, value: u8) {
        let status_port = if self.vga.misc_output & 0x01 != 0 {
            VGA_PORT_STATUS_COLOR
        } else {
            VGA_PORT_STATUS_MONO
        };
        let _ = self.io_read(status_port);
        self.io_write(VGA_PORT_ATC_WRITE, index);
        self.io_write(VGA_PORT_ATC_WRITE, value);
        self.io_write(VGA_PORT_ATC_WRITE, 0x20);
    }

    /// Writes one DAC entry through the write index and data ports.
    fn dac_entry_write(&mut self, index: u8, red: u8, green: u8, blue: u8) {
        self.io_write(VGA_PORT_DAC_WRITE_INDEX, index);
        self.io_write(VGA_PORT_DAC_DATA, red);
        self.io_write(VGA_PORT_DAC_DATA, green);
        self.io_write(VGA_PORT_DAC_DATA, blue);
    }

    /// Replaces `count` DAC entries from `start` with their gray-scale sum,
    /// wrapping past entry 255. The weights are the 0.30/0.59/0.11 NTSC
    /// luminance triple in 8.8 fixed point, rounded to nearest, which matches
    /// the real BIOS byte for byte.
    pub(super) fn gray_scale_sum_dac(&mut self, start: u16, count: u16) {
        for index in 0..count {
            let entry = (start.wrapping_add(index) % DAC_ENTRIES) as u8;
            let [red, green, blue] = self.vga.dac[usize::from(entry)];
            let sum = (GRAY_WEIGHT_RED * u32::from(red)
                + GRAY_WEIGHT_GREEN * u32::from(green)
                + GRAY_WEIGHT_BLUE * u32::from(blue)
                + GRAY_ROUNDING_BIAS)
                >> 8;
            let gray = sum.min(DAC_COMPONENT_MAX) as u8;
            self.dac_entry_write(entry, gray, gray, gray);
        }
    }

    /// AH=0Bh: CGA compatibility palette interface for modes 04h/05h/06h.
    pub(super) fn int10h_cga_palette(&mut self, cpu: &mut impl Cpu) {
        let Some(entry) = self.active_mode_entry() else {
            return;
        };
        let value = cpu.bl();
        match cpu.bh() {
            0x00 => {
                // Background and border color. The border always tracks it;
                // the 320x200 modes also recolor palette entry zero.
                self.atc_register_write(ATC_OVERSCAN as u8, value & 0x0F);
                if entry.family == ModeFamily::Cga4 {
                    self.atc_register_write(0x00, value & 0x0F);
                }
                let palette = self.read_mem_byte(BDA_CGA_PALETTE);
                self.write_mem_byte(BDA_CGA_PALETTE, (palette & 0xE0) | (value & 0x1F));
            }
            0x01 => {
                // CGA palette select: entries 1-3 become the DAC indices of
                // the chosen palette (even = green/red/brown, odd = cyan/
                // magenta/white).
                let select = value & 0x01;
                for color in 1..=3u8 {
                    self.atc_register_write(color, 0x10 + color * 2 + select);
                }
                let palette = self.read_mem_byte(BDA_CGA_PALETTE);
                self.write_mem_byte(BDA_CGA_PALETTE, (palette & !0x20) | (select << 5));
            }
            _ => self.set_iret_cf(cpu, true),
        }
    }

    /// AH=10h: attribute controller and DAC services, dispatched on AL.
    pub(super) fn int10h_palette_services(&mut self, cpu: &mut impl Cpu) {
        match cpu.al() {
            0x00 => {
                // Set one ATC palette register.
                let index = cpu.bl();
                if index > 0x14 {
                    return;
                }
                self.atc_register_write(index, cpu.bh());
            }
            0x01 => self.atc_register_write(ATC_OVERSCAN as u8, cpu.bh()),
            0x02 => {
                // Load ATC 0-15 plus the overscan from the 17 bytes at ES:DX.
                let source = (u32::from(cpu.es()) << 4).wrapping_add(u32::from(cpu.dx()));
                for index in 0..16u8 {
                    let value = self.read_mem_byte(source.wrapping_add(u32::from(index)));
                    self.atc_register_write(index, value);
                }
                let overscan = self.read_mem_byte(source.wrapping_add(16));
                self.atc_register_write(ATC_OVERSCAN as u8, overscan);
            }
            0x03 => {
                // Blink versus background intensity.
                let control = self.vga.atc[ATC_MODE_CONTROL] & !0x08;
                let bit = if cpu.bl() & 0x01 != 0 { 0x08 } else { 0x00 };
                self.atc_register_write(ATC_MODE_CONTROL as u8, control | bit);
            }
            0x07 => {
                let index = cpu.bl();
                if index > 0x14 {
                    return;
                }
                cpu.set_bh(self.vga.atc[usize::from(index)]);
            }
            0x08 => cpu.set_bh(self.vga.atc[ATC_OVERSCAN]),
            0x09 => {
                // Store ATC 0-15 plus the overscan to the buffer at ES:DX.
                let target = (u32::from(cpu.es()) << 4).wrapping_add(u32::from(cpu.dx()));
                for index in 0..16u32 {
                    let value = self.vga.atc[index as usize];
                    self.write_mem_byte(target.wrapping_add(index), value);
                }
                let overscan = self.vga.atc[ATC_OVERSCAN];
                self.write_mem_byte(target.wrapping_add(16), overscan);
            }
            0x10 => self.dac_entry_write(cpu.bl(), cpu.dh(), cpu.ch(), cpu.cl()),
            0x12 => {
                // Load a DAC block from the RGB triples at ES:DX.
                let source = (u32::from(cpu.es()) << 4).wrapping_add(u32::from(cpu.dx()));
                let start = cpu.bx();
                let count = cpu.cx();
                self.io_write(VGA_PORT_DAC_WRITE_INDEX, start as u8);
                for index in 0..u32::from(count) * 3 {
                    let component = self.read_mem_byte(source.wrapping_add(index));
                    self.io_write(VGA_PORT_DAC_DATA, component);
                }
            }
            0x13 => match cpu.bl() {
                0x00 => {
                    // Select the paging mode (4 blocks of 64 or 16 of 16).
                    let control = self.vga.atc[ATC_MODE_CONTROL] & !0x80;
                    let bit = if cpu.bh() & 0x01 != 0 { 0x80 } else { 0x00 };
                    self.atc_register_write(ATC_MODE_CONTROL as u8, control | bit);
                }
                0x01 => self.atc_register_write(ATC_COLOR_SELECT as u8, cpu.bh() & 0x0F),
                _ => self.set_iret_cf(cpu, true),
            },
            0x15 => {
                let entry = self.vga.dac[usize::from(cpu.bl())];
                cpu.set_dh(entry[0]);
                cpu.set_ch(entry[1]);
                cpu.set_cl(entry[2]);
            }
            0x17 => {
                // Read a DAC block into the buffer at ES:DX.
                let target = (u32::from(cpu.es()) << 4).wrapping_add(u32::from(cpu.dx()));
                let start = usize::from(cpu.bx());
                let count = usize::from(cpu.cx());
                for index in 0..count {
                    let entry = self.vga.dac[(start + index) & 0xFF];
                    for (component, &value) in entry.iter().enumerate() {
                        self.write_mem_byte(
                            target.wrapping_add((index * 3 + component) as u32),
                            value,
                        );
                    }
                }
            }
            0x1A => {
                // Read the color page state.
                cpu.set_bl(self.vga.atc[ATC_MODE_CONTROL] >> 7);
                cpu.set_bh(self.vga.atc[ATC_COLOR_SELECT] & 0x0F);
            }
            0x1B => self.gray_scale_sum_dac(cpu.bx(), cpu.cx()),
            _ => self.set_iret_cf(cpu, true),
        }
    }
}
