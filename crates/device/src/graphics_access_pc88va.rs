//! PC-88VA2 graphics access controller.
//!
//! Sits between the CPU and graphics VRAM. It carries two register
//! files selected by the GMSP bit (port `0x153` bit 4): a single-plane file
//! (write mode, pattern, ROP) and a multi-plane file (access mode, plane masks,
//! pattern/ROP registers, compare data). The `gvram_read`/`gvram_write` methods
//! apply the configured raster operation, pattern, plane mask, and compare-read
//! exactly as the controller does when the CPU touches the GVRAM window.

/// Single-plane register file.
#[derive(Default)]
struct SinglePlane {
    write_mode: u8,
    pattern: [u16; 2],
    rop: [u8; 2],
}

/// Multi-plane register file.
struct MultiPlane {
    access_mode: u8,
    access_block: u8,
    read_plane: u8,
    write_plane: u8,
    advanced_access_mode: u8,
    compare_data: [u8; 4],
    pattern: [[u8; 2]; 4],
    pattern_read_pointer: u8,
    pattern_write_pointer: u8,
    rop: [u8; 4],
}

impl Default for MultiPlane {
    fn default() -> Self {
        Self {
            access_mode: 0,
            access_block: 0,
            read_plane: 0xFF,
            write_plane: 0xFF,
            advanced_access_mode: 0,
            compare_data: [0; 4],
            pattern: [[0; 2]; 4],
            pattern_read_pointer: 0xF0,
            pattern_write_pointer: 0xF0,
            rop: [0; 4],
        }
    }
}

/// The graphics access controller.
pub struct GraphicsAccessVa {
    /// `true` when the GMSP bit selects single-plane access (port `0x153` bit 4).
    single_plane: bool,
    single: SinglePlane,
    multi: MultiPlane,
}

impl Default for GraphicsAccessVa {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphicsAccessVa {
    /// Creates a reset graphics access controller.
    pub fn new() -> Self {
        Self {
            single_plane: false,
            single: SinglePlane::default(),
            multi: MultiPlane::default(),
        }
    }

    /// Resets the controller.
    pub fn reset(&mut self) {
        let single_plane = self.single_plane;
        self.single = SinglePlane::default();
        self.multi = MultiPlane::default();
        self.single_plane = single_plane;
    }

    /// Updates the GMSP selection from a port `0x153` write.
    pub fn set_single_plane(&mut self, single_plane: bool) {
        self.single_plane = single_plane;
    }

    /// The 8-input raster-operation lookup.
    fn raster_op(rop: u8, pattern: u16, cpu: u16, mem: u16) -> u16 {
        let mut out = 0u16;
        if rop & 0x01 != 0 {
            out |= !pattern & !cpu & !mem;
        }
        if rop & 0x02 != 0 {
            out |= !pattern & !cpu & mem;
        }
        if rop & 0x04 != 0 {
            out |= !pattern & cpu & !mem;
        }
        if rop & 0x08 != 0 {
            out |= !pattern & cpu & mem;
        }
        if rop & 0x10 != 0 {
            out |= pattern & !cpu & !mem;
        }
        if rop & 0x20 != 0 {
            out |= pattern & !cpu & mem;
        }
        if rop & 0x40 != 0 {
            out |= pattern & cpu & !mem;
        }
        if rop & 0x80 != 0 {
            out |= pattern & cpu & mem;
        }
        out
    }

    fn write_value_single(&self, grph: &[u8], address: u32, value: u16) -> u16 {
        let index = address as usize;
        let memory = u16::from(grph.get(index).copied().unwrap_or(0))
            | (u16::from(grph.get(index + 1).copied().unwrap_or(0)) << 8);
        let page = ((address >> 17) & 1) as usize;
        let pattern = self.single.pattern[page];
        match (self.single.write_mode >> 3) & 3 {
            0 => Self::raster_op(self.single.rop[page], pattern, value, memory),
            1 => pattern,
            2 => value,
            _ => memory,
        }
    }

    fn write_value_multi(&self, grph: &[u8], address: u32, value: u8) -> u8 {
        let memory = grph.get(address as usize).copied().unwrap_or(0);
        let plane = (address >> 16) as usize;
        let pattern_index = ((self.multi.pattern_read_pointer >> plane) & 1) as usize;
        let pattern = self.multi.pattern[plane][pattern_index];
        match (self.multi.advanced_access_mode >> 3) & 3 {
            0 => Self::raster_op(
                self.multi.rop[plane],
                u16::from(pattern),
                u16::from(value),
                u16::from(memory),
            ) as u8,
            1 => pattern,
            2 => value,
            _ => memory,
        }
    }

    /// Writes one byte through the controller.
    pub fn gvram_write(&mut self, grph: &mut [u8], address: u32, value: u8) {
        if self.single_plane {
            if address & 1 != 0 {
                let out = self.write_value_single(grph, address & !1, u16::from(value) << 8);
                set(grph, address, (out >> 8) as u8);
            } else {
                let out = self.write_value_single(grph, address, u16::from(value));
                set(grph, address, out as u8);
            }
            return;
        }

        if self.multi.access_mode != 0 {
            let mut address = address & 0x7FFF;
            if self.multi.access_block != 0 {
                address |= 0x8000;
            }
            let mut mask = self.multi.write_plane;
            for plane in 0..4 {
                if mask & 1 == 0 {
                    let out = self.write_value_multi(grph, address, value);
                    let memory = grph.get(address as usize).copied().unwrap_or(0);
                    set(grph, address, out);
                    if self.multi.advanced_access_mode & 0x02 != 0 {
                        let pattern_index =
                            ((self.multi.pattern_write_pointer >> plane) & 1) as usize;
                        self.multi.pattern[plane][pattern_index] = memory;
                    }
                }
                mask >>= 1;
                address += 0x10000;
            }
            if self.multi.advanced_access_mode & 0x04 != 0 {
                self.multi.pattern_read_pointer ^= 0x0F;
                if self.multi.advanced_access_mode & 0x02 != 0 {
                    self.multi.pattern_write_pointer ^= 0x0F;
                }
            }
        } else {
            set(grph, address, value);
        }
    }

    /// Reads one byte through the controller.
    pub fn gvram_read(&mut self, grph: &[u8], address: u32) -> u8 {
        if self.single_plane || self.multi.access_mode == 0 {
            return grph.get(address as usize).copied().unwrap_or(0);
        }

        let mut address = address & 0x7FFF;
        if self.multi.access_block != 0 {
            address |= 0x8000;
        }
        let mut mask = self.multi.read_plane;
        let mut result = 0xFFu8;
        for plane in 0..4 {
            if mask & 1 == 0 {
                let data = grph.get(address as usize).copied().unwrap_or(0);
                if self.multi.advanced_access_mode & 0x20 != 0 {
                    result &= !(data ^ self.multi.compare_data[plane]);
                } else {
                    result &= data;
                }
                if self.multi.advanced_access_mode & 0x01 != 0 {
                    let pattern_index = ((self.multi.pattern_write_pointer >> plane) & 1) as usize;
                    self.multi.pattern[plane][pattern_index] = data;
                }
            }
            mask >>= 1;
            address += 0x10000;
        }
        if self.multi.advanced_access_mode & 0x04 != 0
            && self.multi.advanced_access_mode & 0x01 != 0
        {
            self.multi.pattern_write_pointer ^= 0x0F;
        }
        result
    }

    /// Writes a controller register (ports `0x510-0x5A2`).
    pub fn io_write(&mut self, port: u16, value: u8) {
        match port {
            0x510 => self.multi.access_mode = value & 0x01,
            0x512 => self.multi.access_block = value & 0x01,
            0x514 => self.multi.read_plane = value | 0xF0,
            0x516 => self.multi.write_plane = value | 0xF0,
            0x518 => {
                if (self.multi.advanced_access_mode ^ (value & 0x04)) != 0 && value & 0x04 == 0 {
                    self.multi.pattern_read_pointer = 0xF0;
                    self.multi.pattern_write_pointer = 0xF0;
                }
                self.multi.advanced_access_mode = value & 0x3F;
            }
            0x520 | 0x522 | 0x524 | 0x526 => {
                self.multi.compare_data[((port >> 1) & 3) as usize] = value;
            }
            0x528 => {
                let mut bits = value;
                for slot in &mut self.multi.compare_data {
                    *slot = if bits & 1 != 0 { 0xFF } else { 0 };
                    bits >>= 1;
                }
            }
            0x530 | 0x532 | 0x534 | 0x536 => {
                self.multi.pattern[((port >> 1) & 3) as usize][0] = value;
            }
            0x540 | 0x542 | 0x544 | 0x546 => {
                self.multi.pattern[((port >> 1) & 3) as usize][1] = value;
            }
            0x550 => {
                let value = if self.multi.advanced_access_mode & 0x04 == 0 {
                    0
                } else {
                    value
                };
                self.multi.pattern_read_pointer = (value & 0x0F) | 0xF0;
            }
            0x552 => {
                let value = if self.multi.advanced_access_mode & 0x04 == 0 {
                    0
                } else {
                    value
                };
                self.multi.pattern_write_pointer = (value & 0x0F) | 0xF0;
            }
            0x560 | 0x562 | 0x564 | 0x566 => {
                self.multi.rop[((port >> 1) & 3) as usize] = value;
            }
            0x580 => self.single.write_mode = value & 0x18,
            0x590 | 0x592 => set_low(&mut self.single.pattern[((port >> 1) & 1) as usize], value),
            0x591 | 0x593 => set_high(&mut self.single.pattern[((port >> 1) & 1) as usize], value),
            0x5A0 | 0x5A2 => self.single.rop[((port >> 1) & 1) as usize] = value,
            _ => {}
        }
    }

    /// Reads a controller register (ports `0x510-0x5A2`).
    pub fn io_read(&self, port: u16) -> u8 {
        let multi_active = !self.single_plane;
        let single_active = self.single_plane;
        match port {
            0x510 if multi_active => self.multi.access_mode,
            0x512 if multi_active => self.multi.access_block,
            0x514 if multi_active => self.multi.read_plane,
            0x516 if multi_active => self.multi.write_plane,
            0x518 if multi_active => self.multi.advanced_access_mode,
            0x520 | 0x522 | 0x524 | 0x526 if multi_active => {
                self.multi.compare_data[((port >> 1) & 3) as usize]
            }
            0x528 if multi_active => {
                let mut value = 0u8;
                for index in (0..4).rev() {
                    value <<= 1;
                    if self.multi.compare_data[index] == 0xFF {
                        value |= 1;
                    }
                }
                value | 0xF0
            }
            0x530 | 0x532 | 0x534 | 0x536 if multi_active => {
                self.multi.pattern[((port >> 1) & 3) as usize][0]
            }
            0x540 | 0x542 | 0x544 | 0x546 if multi_active => {
                self.multi.pattern[((port >> 1) & 3) as usize][1]
            }
            0x550 if multi_active => self.multi.pattern_read_pointer,
            0x552 if multi_active => self.multi.pattern_write_pointer,
            0x560 | 0x562 | 0x564 | 0x566 if multi_active => {
                self.multi.rop[((port >> 1) & 3) as usize]
            }
            0x580 if single_active => self.single.write_mode,
            0x590 | 0x592 if single_active => {
                (self.single.pattern[((port >> 1) & 1) as usize] & 0xFF) as u8
            }
            0x591 | 0x593 if single_active => {
                (self.single.pattern[((port >> 1) & 1) as usize] >> 8) as u8
            }
            0x5A0 | 0x5A2 if single_active => self.single.rop[((port >> 1) & 1) as usize],
            _ => not_active(port),
        }
    }
}

/// The open-bus value the controller returns for an inactive or unimplemented
/// register.
fn not_active(port: u16) -> u8 {
    if port & 1 != 0 {
        if port < 0x580 {
            if port & 0x02 != 0 { 0xFD } else { 0xFF }
        } else if port & 0x02 != 0 {
            0x7D
        } else {
            0x7F
        }
    } else if port & 0x0F == 0x0A {
        0xFA
    } else {
        0xFE
    }
}

fn set(grph: &mut [u8], address: u32, value: u8) {
    if let Some(slot) = grph.get_mut(address as usize) {
        *slot = value;
    }
}

fn set_low(register: &mut u16, value: u8) {
    *register = (*register & 0xFF00) | u16::from(value);
}

fn set_high(register: &mut u16, value: u8) {
    *register = (*register & 0x00FF) | (u16::from(value) << 8);
}
