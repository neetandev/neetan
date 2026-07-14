use crate::{
    Pc9801Bus, Tracing,
    bus::{E_PLANE_PAGE_SIZE_BYTES, GRAPHICS_PAGE_SIZE_BYTES},
};

impl<T: Tracing> Pc9801Bus<T> {
    pub(super) fn access_page_index(&self) -> usize {
        usize::from(self.display_control.state.access_page & 1)
    }

    pub(super) fn display_page_index(&self) -> usize {
        usize::from(self.display_control.state.display_page & 1)
    }

    pub(super) fn graphics_plane_read_byte_from_page(
        &self,
        page: usize,
        plane: usize,
        offset: usize,
    ) -> u8 {
        let page_base = page * GRAPHICS_PAGE_SIZE_BYTES;
        match plane {
            0..=2 => self.memory.state.graphics_vram[page_base + plane * 0x8000 + offset],
            3 => {
                let e_page_base = page * E_PLANE_PAGE_SIZE_BYTES;
                self.memory.state.e_plane_vram[e_page_base + offset]
            }
            _ => unreachable!("graphics plane index out of range: {plane}"),
        }
    }

    pub(super) fn graphics_plane_write_byte_to_page(
        &mut self,
        page: usize,
        plane: usize,
        offset: usize,
        value: u8,
    ) {
        let page_base = page * GRAPHICS_PAGE_SIZE_BYTES;
        match plane {
            0..=2 => self.memory.state.graphics_vram[page_base + plane * 0x8000 + offset] = value,
            3 => {
                let e_page_base = page * E_PLANE_PAGE_SIZE_BYTES;
                self.memory.state.e_plane_vram[e_page_base + offset] = value;
            }
            _ => unreachable!("graphics plane index out of range: {plane}"),
        }
    }

    pub(super) fn graphics_plane_read_byte(&self, plane: usize, offset: usize) -> u8 {
        self.graphics_plane_read_byte_from_page(self.access_page_index(), plane, offset)
    }

    pub(super) fn graphics_plane_write_byte(&mut self, plane: usize, offset: usize, value: u8) {
        self.graphics_plane_write_byte_to_page(self.access_page_index(), plane, offset, value);
    }

    pub(super) fn graphics_plane_read_word(&self, plane: usize, offset: usize) -> u16 {
        let lo = self.graphics_plane_read_byte(plane, offset) as u16;
        let hi = self.graphics_plane_read_byte(plane, offset + 1) as u16;
        lo | (hi << 8)
    }

    pub(super) fn graphics_plane_write_word(&mut self, plane: usize, offset: usize, value: u16) {
        self.graphics_plane_write_byte(plane, offset, value as u8);
        self.graphics_plane_write_byte(plane, offset + 1, (value >> 8) as u8);
    }

    /// Converts a CPU address in the graphics VRAM range to a byte offset (0..0x7FFF).
    /// Works for both 0xA8000-0xBFFFF (B/R/G planes) and 0xE0000-0xE7FFF (E plane).
    pub(super) fn graphics_vram_offset(address: u32) -> usize {
        if address >= 0xE0000 {
            (address - 0xE0000) as usize & 0x7FFF
        } else {
            (address - 0xA8000) as usize & 0x7FFF
        }
    }

    /// Returns true if EGC mode is currently effective (EGC mode enabled + GRCG active).
    pub(super) fn is_egc_effective(&self) -> bool {
        self.display_control.is_egc_extended_mode_effective() && self.grcg.is_active()
    }

    pub(super) fn grcg_write_byte(&mut self, address: u32, value: u8) {
        self.pending_wait_cycles += self.grcg_wait;
        let offset = Self::graphics_vram_offset(address);
        if !self.grcg.is_rmw() {
            // TDW: write tile to each enabled plane, ignore CPU data
            for p in 0..4 {
                if p == 3 && !self.graphics_extension_enabled {
                    continue;
                }
                if self.grcg.plane_enabled(p) {
                    self.graphics_plane_write_byte(p, offset, self.grcg.state.tile[p]);
                }
            }
        } else {
            // RMW: bit-select between tile and existing VRAM
            for p in 0..4 {
                if p == 3 && !self.graphics_extension_enabled {
                    continue;
                }
                if self.grcg.plane_enabled(p) {
                    let current = self.graphics_plane_read_byte(p, offset);
                    let next = (value & self.grcg.state.tile[p]) | (!value & current);
                    self.graphics_plane_write_byte(p, offset, next);
                }
            }
        }
    }

    pub(super) fn grcg_read_byte(&mut self, address: u32) -> u8 {
        if self.grcg.is_rmw() {
            // RMW reads use standard VRAM wait.
            self.pending_wait_cycles += self.vram_wait;
            return self.read_byte_with_access_page(address);
        }
        // TCR reads use GRCG wait.
        self.pending_wait_cycles += self.grcg_wait;
        // TCR: compare VRAM against tiles, return match bitmask
        let offset = Self::graphics_vram_offset(address);
        let mut result = 0xFF;
        for p in 0..4 {
            if p == 3 && !self.graphics_extension_enabled {
                continue;
            }
            if self.grcg.plane_enabled(p) {
                result &= !(self.graphics_plane_read_byte(p, offset) ^ self.grcg.state.tile[p]);
            }
        }
        result
    }

    pub(super) fn grcg_write_word(&mut self, address: u32, value: u16) {
        self.pending_wait_cycles += self.grcg_wait;
        let offset = Self::graphics_vram_offset(address);
        let low = value as u8;
        let high = (value >> 8) as u8;
        if !self.grcg.is_rmw() {
            for p in 0..4 {
                if p == 3 && !self.graphics_extension_enabled {
                    continue;
                }
                if self.grcg.plane_enabled(p) {
                    self.graphics_plane_write_byte(p, offset, self.grcg.state.tile[p]);
                    self.graphics_plane_write_byte(p, offset + 1, self.grcg.state.tile[p]);
                }
            }
        } else {
            for p in 0..4 {
                if p == 3 && !self.graphics_extension_enabled {
                    continue;
                }
                if self.grcg.plane_enabled(p) {
                    let cur_lo = self.graphics_plane_read_byte(p, offset);
                    let cur_hi = self.graphics_plane_read_byte(p, offset + 1);
                    let next_lo = (low & self.grcg.state.tile[p]) | (!low & cur_lo);
                    let next_hi = (high & self.grcg.state.tile[p]) | (!high & cur_hi);
                    self.graphics_plane_write_byte(p, offset, next_lo);
                    self.graphics_plane_write_byte(p, offset + 1, next_hi);
                }
            }
        }
    }

    pub(super) fn grcg_read_word(&mut self, address: u32) -> u16 {
        if self.grcg.is_rmw() {
            self.pending_wait_cycles += self.vram_wait;
            let low = self.read_byte_with_access_page(address) as u16;
            let high = self.read_byte_with_access_page(address.wrapping_add(1)) as u16;
            return low | (high << 8);
        }
        self.pending_wait_cycles += self.grcg_wait;
        let offset = Self::graphics_vram_offset(address);
        let mut result_lo: u8 = 0xFF;
        let mut result_hi: u8 = 0xFF;
        for p in 0..4 {
            if p == 3 && !self.graphics_extension_enabled {
                continue;
            }
            if self.grcg.plane_enabled(p) {
                result_lo &= !(self.graphics_plane_read_byte(p, offset) ^ self.grcg.state.tile[p]);
                result_hi &=
                    !(self.graphics_plane_read_byte(p, offset + 1) ^ self.grcg.state.tile[p]);
            }
        }
        u16::from(result_lo) | (u16::from(result_hi) << 8)
    }

    pub(super) fn egc_read_byte(&mut self, address: u32) -> u8 {
        self.pending_wait_cycles += self.grcg_wait;
        self.egc_read_byte_inner(address)
    }

    pub(super) fn egc_read_byte_inner(&mut self, address: u32) -> u8 {
        let offset = Self::graphics_vram_offset(address);
        let vram = [
            self.graphics_plane_read_byte(0, offset),
            self.graphics_plane_read_byte(1, offset),
            self.graphics_plane_read_byte(2, offset),
            if self.graphics_extension_enabled {
                self.graphics_plane_read_byte(3, offset)
            } else {
                0
            },
        ];
        self.egc.read_byte(address, vram)
    }

    pub(super) fn egc_write_byte(&mut self, address: u32, value: u8) {
        self.pending_wait_cycles += self.grcg_wait;
        self.egc_write_byte_inner(address, value);
    }

    pub(super) fn egc_write_byte_inner(&mut self, address: u32, value: u8) {
        let offset = Self::graphics_vram_offset(address);
        let vram = [
            self.graphics_plane_read_byte(0, offset),
            self.graphics_plane_read_byte(1, offset),
            self.graphics_plane_read_byte(2, offset),
            if self.graphics_extension_enabled {
                self.graphics_plane_read_byte(3, offset)
            } else {
                0
            },
        ];
        let (data, mask) = self.egc.write_byte(address, value, vram);
        if mask != 0 {
            for (p, &plane_data) in data.iter().enumerate() {
                if p == 3 && !self.graphics_extension_enabled {
                    continue;
                }
                if self.egc.plane_write_enabled(p) {
                    let current = self.graphics_plane_read_byte(p, offset);
                    let result = (current & !mask) | (plane_data & mask);
                    self.graphics_plane_write_byte(p, offset, result);
                }
            }
        }
    }

    pub(super) fn egc_read_word(&mut self, address: u32) -> u16 {
        self.pending_wait_cycles += self.grcg_wait;
        if address & 1 != 0 {
            // Misaligned: decompose into two byte operations (no extra wait charge).
            return if !self.egc.is_descending() {
                let lo = self.egc_read_byte_inner(address) as u16;
                let hi = self.egc_read_byte_inner(address + 1) as u16;
                lo | (hi << 8)
            } else {
                let hi = self.egc_read_byte_inner(address + 1) as u16;
                let lo = self.egc_read_byte_inner(address) as u16;
                lo | (hi << 8)
            };
        }
        let offset = Self::graphics_vram_offset(address);
        let vram = [
            self.graphics_plane_read_word(0, offset),
            self.graphics_plane_read_word(1, offset),
            self.graphics_plane_read_word(2, offset),
            if self.graphics_extension_enabled {
                self.graphics_plane_read_word(3, offset)
            } else {
                0
            },
        ];
        self.egc.read_word(address, vram)
    }

    pub(super) fn egc_write_word(&mut self, address: u32, value: u16) {
        self.pending_wait_cycles += self.grcg_wait;
        self.egc_write_word_inner(address, value);
    }

    pub(super) fn egc_write_word_inner(&mut self, address: u32, value: u16) {
        if address & 1 != 0 {
            // Misaligned: decompose into two byte operations (no extra wait charge).
            if !self.egc.is_descending() {
                self.egc_write_byte_inner(address, value as u8);
                self.egc_write_byte_inner(address + 1, (value >> 8) as u8);
            } else {
                self.egc_write_byte_inner(address + 1, (value >> 8) as u8);
                self.egc_write_byte_inner(address, value as u8);
            }
            return;
        }
        let offset = Self::graphics_vram_offset(address);
        let vram = [
            self.graphics_plane_read_word(0, offset),
            self.graphics_plane_read_word(1, offset),
            self.graphics_plane_read_word(2, offset),
            if self.graphics_extension_enabled {
                self.graphics_plane_read_word(3, offset)
            } else {
                0
            },
        ];
        let (data, mask) = self.egc.write_word(address, value, vram);
        if mask != 0 {
            for (p, &plane_data) in data.iter().enumerate() {
                if p == 3 && !self.graphics_extension_enabled {
                    continue;
                }
                if self.egc.plane_write_enabled(p) {
                    let current = self.graphics_plane_read_word(p, offset);
                    let result = (current & !mask) | (plane_data & mask);
                    self.graphics_plane_write_word(p, offset, result);
                }
            }
        }
    }

    pub(super) fn egc_write_dword(&mut self, address: u32, value: u32) {
        self.pending_wait_cycles += self.grcg_wait;
        if !self.egc.is_descending() {
            self.egc_write_word_inner(address, value as u16);
            self.egc_write_word_inner(address.wrapping_add(2), (value >> 16) as u16);
        } else {
            self.egc_write_word_inner(address.wrapping_add(2), (value >> 16) as u16);
            self.egc_write_word_inner(address, value as u16);
        }
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, CpuMode, MachineModel};

    use crate::bus::{GRAPHICS_PAGE_SIZE_BYTES, NoTracing, Pc9801Bus};

    fn enable_egc_mode(bus: &mut Pc9801Bus<NoTracing>) {
        bus.io_write_byte(0x6A, 0x07);
        bus.io_write_byte(0x6A, 0x05);
    }

    fn configure_egc_mono(bus: &mut Pc9801Bus<NoTracing>, color: u16, rop: u16, shift: u16) {
        bus.io_write_word(0x04A0, 0xFFF0);
        bus.io_write_word(0x04A2, 0x00FF);
        bus.io_write_word(0x04A8, 0xFFFF);
        bus.io_write_word(0x04A6, color);
        bus.io_write_word(0x04A2, 0x4000);
        bus.io_write_word(0x04A4, 0x0C00 | rop);
        bus.io_write_word(0x04AC, shift);
        bus.io_write_word(0x04AE, 31);
    }

    fn seed_background_color5(bus: &mut Pc9801Bus<NoTracing>, offset: usize, len: usize) {
        for byte in offset..offset + len {
            bus.memory.state.graphics_vram[byte] = 0xFF;
            bus.memory.state.graphics_vram[0x8000 + byte] = 0x00;
            bus.memory.state.graphics_vram[0x10000 + byte] = 0xFF;
            bus.memory.state.e_plane_vram[byte] = 0x00;
        }
    }

    fn graphics_pixel(bus: &Pc9801Bus<NoTracing>, x: usize) -> u8 {
        let offset = x / 8;
        let mask = 0x80 >> (x & 7);
        let mut color = 0;
        if bus.memory.state.graphics_vram[offset] & mask != 0 {
            color |= 1;
        }
        if bus.memory.state.graphics_vram[0x8000 + offset] & mask != 0 {
            color |= 2;
        }
        if bus.memory.state.graphics_vram[0x10000 + offset] & mask != 0 {
            color |= 4;
        }
        if bus.memory.state.e_plane_vram[offset] & mask != 0 {
            color |= 8;
        }
        color
    }

    #[test]
    fn grcg_tdw_write_byte_intercepts_e_plane() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
        bus.set_graphics_extension_enabled(true);
        bus.grcg.write_mode(0x80); // TDW mode, all planes enabled

        bus.grcg.state.tile = [0x5A, 0xA5, 0x3C, 0xC3];

        // Write to E-plane address 0xE0000 (offset 0).
        bus.write_byte(0xE0000, 0xFF);

        // GRCG TDW should write tile values to all 4 planes at offset 0.
        assert_eq!(bus.memory.state.graphics_vram[0], 0x5A); // B-plane
        assert_eq!(bus.memory.state.graphics_vram[0x8000], 0xA5); // R-plane
        assert_eq!(bus.memory.state.graphics_vram[0x10000], 0x3C); // G-plane
        assert_eq!(bus.memory.state.e_plane_vram[0], 0xC3); // E-plane
    }

    #[test]
    fn grcg_tcr_read_byte_intercepts_e_plane() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
        bus.set_graphics_extension_enabled(true);
        bus.grcg.write_mode(0x80); // TDW/TCR mode, all planes enabled

        // Set tile (compare) values and VRAM content to match.
        bus.grcg.state.tile = [0xAA, 0xBB, 0xCC, 0xDD];
        bus.memory.state.graphics_vram[0x100] = 0xAA; // B
        bus.memory.state.graphics_vram[0x8100] = 0xBB; // R
        bus.memory.state.graphics_vram[0x10100] = 0xCC; // G
        bus.memory.state.e_plane_vram[0x100] = 0xDD; // E

        // TCR read from E-plane address at offset 0x100.
        let result = bus.read_byte(0xE0100);
        assert_eq!(result, 0xFF, "all bits match tile values");
    }

    #[test]
    fn grcg_tdw_write_word_intercepts_e_plane() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
        bus.set_graphics_extension_enabled(true);
        bus.grcg.write_mode(0x80); // TDW mode
        bus.grcg.state.tile = [0x11, 0x22, 0x33, 0x44];

        bus.write_word(0xE0000, 0xFFFF);

        assert_eq!(bus.memory.state.graphics_vram[0], 0x11);
        assert_eq!(bus.memory.state.graphics_vram[1], 0x11);
        assert_eq!(bus.memory.state.graphics_vram[0x8000], 0x22);
        assert_eq!(bus.memory.state.graphics_vram[0x8001], 0x22);
        assert_eq!(bus.memory.state.graphics_vram[0x10000], 0x33);
        assert_eq!(bus.memory.state.graphics_vram[0x10001], 0x33);
        assert_eq!(bus.memory.state.e_plane_vram[0], 0x44);
        assert_eq!(bus.memory.state.e_plane_vram[1], 0x44);
    }

    #[test]
    fn egc_aligned_word_write_charges_one_grcg_wait() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
        enable_egc_mode(&mut bus);
        bus.grcg.write_mode(0x80);

        bus.pending_wait_cycles = 0;
        bus.egc_write_word(0xA8000, 0x1234);
        assert_eq!(bus.pending_wait_cycles, 8);
    }

    #[test]
    fn egc_misaligned_word_write_charges_one_grcg_wait() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
        enable_egc_mode(&mut bus);
        bus.grcg.write_mode(0x80);

        bus.pending_wait_cycles = 0;
        bus.egc_write_word(0xA8001, 0x1234); // misaligned
        assert_eq!(
            bus.pending_wait_cycles, 8,
            "misaligned should charge exactly 1x grcg_wait"
        );
    }

    #[test]
    fn egc_mono_dword_write_draws_color_and_preserves_zero_bits() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
        bus.set_graphics_extension_enabled(true);
        enable_egc_mode(&mut bus);
        bus.grcg.write_mode(0x80);
        configure_egc_mono(&mut bus, 10, 0xAC, 0x0000);
        seed_background_color5(&mut bus, 0, 4);

        bus.pending_wait_cycles = 0;
        bus.write_dword(0xA8000, 0x81CC_AAF0);

        assert_eq!(bus.pending_wait_cycles, 8);
        assert_eq!(graphics_pixel(&bus, 0), 10);
        assert_eq!(graphics_pixel(&bus, 4), 5);
        assert_eq!(graphics_pixel(&bus, 8), 10);
        assert_eq!(graphics_pixel(&bus, 9), 5);
        assert_eq!(graphics_pixel(&bus, 31), 10);
    }

    #[test]
    fn egc_shifted_mono_dword_with_flush_preserves_edges() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
        bus.set_graphics_extension_enabled(true);
        enable_egc_mode(&mut bus);
        bus.grcg.write_mode(0x80);
        configure_egc_mono(&mut bus, 10, 0xAC, 0x0010);
        seed_background_color5(&mut bus, 0, 6);

        bus.write_dword(0xA8000, 0xFFFF_FFFF);
        bus.write_word(0xA8004, 0);

        assert_eq!(graphics_pixel(&bus, 0), 5);
        assert_eq!(graphics_pixel(&bus, 1), 10);
        assert_eq!(graphics_pixel(&bus, 16), 10);
        assert_eq!(graphics_pixel(&bus, 32), 10);
        assert_eq!(graphics_pixel(&bus, 33), 5);
    }

    #[test]
    fn egc_misaligned_word_read_charges_one_grcg_wait() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
        enable_egc_mode(&mut bus);
        bus.grcg.write_mode(0x80);

        bus.pending_wait_cycles = 0;
        let _ = bus.egc_read_word(0xA8001); // misaligned
        assert_eq!(
            bus.pending_wait_cycles, 8,
            "misaligned should charge exactly 1x grcg_wait"
        );
    }

    #[test]
    fn grcg_tdw_operates_on_access_page() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);

        // Seed page 0 B-plane with a known value before enabling GRCG.
        bus.memory.state.graphics_vram[0] = 0x42;

        bus.grcg.write_mode(0x80); // TDW, all planes enabled
        bus.grcg.state.tile = [0x5A, 0xA5, 0x3C, 0x00];

        // Switch to page 1 and write through GRCG.
        bus.io_write_byte(0xA6, 0x01);
        bus.write_byte(0xA8000, 0xFF);

        // Page 0 B-plane untouched.
        assert_eq!(bus.memory.state.graphics_vram[0], 0x42);

        // Page 1 has tile values.
        let page1_base = GRAPHICS_PAGE_SIZE_BYTES;
        assert_eq!(bus.memory.state.graphics_vram[page1_base], 0x5A); // B
        assert_eq!(bus.memory.state.graphics_vram[page1_base + 0x8000], 0xA5); // R
        assert_eq!(bus.memory.state.graphics_vram[page1_base + 0x10000], 0x3C); // G
    }

    #[test]
    fn grcg_tcr_reads_from_access_page() {
        let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
        bus.grcg.write_mode(0x80); // TCR mode, all planes enabled
        bus.grcg.state.tile = [0xAA, 0xBB, 0xCC, 0x00];

        // Place matching data on page 1 only.
        let page1_base = GRAPHICS_PAGE_SIZE_BYTES;
        bus.memory.state.graphics_vram[page1_base] = 0xAA; // B
        bus.memory.state.graphics_vram[page1_base + 0x8000] = 0xBB; // R
        bus.memory.state.graphics_vram[page1_base + 0x10000] = 0xCC; // G

        // TCR read from page 0: no match (all zeros vs tile).
        bus.io_write_byte(0xA6, 0x00);
        let result_page0 = bus.read_byte(0xA8000);
        assert_eq!(result_page0, !0xAA & !0xBB & !0xCC);

        // TCR read from page 1: all match.
        bus.io_write_byte(0xA6, 0x01);
        let result_page1 = bus.read_byte(0xA8000);
        assert_eq!(result_page1, 0xFF);
    }
}
