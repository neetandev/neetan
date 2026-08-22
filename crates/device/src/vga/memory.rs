//! CPU access path into display memory: window decode, banking, chain
//! translation, plane latches, read modes 0-1 and write modes 0-3.

use super::{VGA_VRAM_SIZE, Vga};

/// Size of the CPU window at 0xA0000 in bytes.
pub const VGA_WINDOW_SIZE: u32 = 0x2_0000;

impl Vga {
    /// Reads a byte through the CPU window; `offset` is relative to 0xA0000.
    ///
    /// Returns `None` when the current memory map does not decode the offset.
    pub fn mem_read(&mut self, offset: u32) -> Option<u8> {
        let address = self.decode_window(offset & (VGA_WINDOW_SIZE - 1), false)?;

        let memory_mode = self.seq[4];
        if memory_mode & 0x08 != 0 {
            // Chain 4: the interleaved layout is byte-linear.
            if address >= VGA_VRAM_SIZE as u32 {
                return Some(0xFF);
            }
            let latch_base = (address & !3) as usize;
            self.latches
                .copy_from_slice(&self.vram[latch_base..latch_base + 4]);
            return Some(self.vram[address as usize]);
        }

        let mut read_plane = self.gc[4] & 0x03;
        let plane_address = if self.gc[5] & 0x10 != 0 {
            // Host odd/even: the address low bit selects the plane pair.
            read_plane = (read_plane & 0x02) | (address as u8 & 0x01);
            (address & !1) << 2
        } else {
            address << 2
        };
        if plane_address >= VGA_VRAM_SIZE as u32 {
            return Some(0xFF);
        }
        let latch_base = (plane_address & !3) as usize;
        self.latches
            .copy_from_slice(&self.vram[latch_base..latch_base + 4]);

        if self.gc[5] & 0x08 != 0 {
            Some(self.read_mode_1())
        } else {
            Some(self.vram[(plane_address | u32::from(read_plane)) as usize])
        }
    }

    /// Writes a byte through the CPU window; `offset` is relative to 0xA0000.
    pub fn mem_write(&mut self, offset: u32, value: u8) {
        let Some(address) = self.decode_window(offset & (VGA_WINDOW_SIZE - 1), true) else {
            return;
        };

        let memory_mode = self.seq[4];
        let mut write_mask = self.seq[2] & 0x0F;

        let plane_address = if memory_mode & 0x08 != 0 {
            write_mask &= 1 << (address & 3);
            address & !3
        } else if memory_mode & 0x04 == 0 {
            write_mask &= 0x05 << (address & 1);
            (address & !1) << 2
        } else {
            address << 2
        };
        if write_mask == 0 || plane_address >= VGA_VRAM_SIZE as u32 {
            return;
        }

        let rotate = u32::from(self.gc[3] & 0x07);
        let logical_op = (self.gc[3] >> 3) & 0x03;
        let write_mode = self.gc[5] & 0x03;

        let mut plane_values = [0u8; 4];

        let bit_mask = match write_mode {
            0 => {
                let rotated = value.rotate_right(rotate);
                for (plane, plane_value) in plane_values.iter_mut().enumerate() {
                    *plane_value = if self.gc[1] & (1 << plane) != 0 {
                        plane_fill(self.gc[0], plane)
                    } else {
                        rotated
                    };
                }
                self.gc[8]
            }
            1 => {
                for plane in 0..4 {
                    if write_mask & (1 << plane) != 0 {
                        self.vram[(plane_address as usize) | plane] = self.latches[plane];
                    }
                }
                return;
            }
            2 => {
                for (plane, plane_value) in plane_values.iter_mut().enumerate() {
                    *plane_value = plane_fill(value, plane);
                }
                self.gc[8]
            }
            _ => {
                let rotated = value.rotate_right(rotate);
                for (plane, plane_value) in plane_values.iter_mut().enumerate() {
                    *plane_value = plane_fill(self.gc[0], plane);
                }
                self.gc[8] & rotated
            }
        };

        for (plane, &plane_value) in plane_values.iter().enumerate() {
            if write_mask & (1 << plane) == 0 {
                continue;
            }
            let latch = self.latches[plane];
            let combined = match logical_op {
                0 => plane_value,
                1 => plane_value & latch,
                2 => plane_value | latch,
                _ => plane_value ^ latch,
            };
            self.vram[(plane_address as usize) | plane] =
                (combined & bit_mask) | (latch & !bit_mask);
        }
    }

    /// Resolves a window offset to a display memory address, applying the
    /// memory map select and the ET4000 segment pointers.
    fn decode_window(&self, offset: u32, write: bool) -> Option<u32> {
        let map_select = (self.gc[6] >> 2) & 0x03;
        let address = match map_select {
            0 => offset,
            1 => {
                if offset >= 0x1_0000 {
                    return None;
                }
                offset
            }
            2 => {
                if !(0x1_0000..0x1_8000).contains(&offset) {
                    return None;
                }
                offset - 0x1_0000
            }
            _ => {
                if offset < 0x1_8000 {
                    return None;
                }
                offset - 0x1_8000
            }
        };
        // The segment pointers only apply to the 0xA0000 windows.
        if map_select <= 1 {
            let bank = if write {
                self.write_bank_offset()
            } else {
                self.read_bank_offset()
            };
            Some(address + bank)
        } else {
            Some(address)
        }
    }

    /// Read mode 1: per-pixel color compare across the cared-about planes.
    fn read_mode_1(&self) -> u8 {
        let compare = self.gc[2];
        let care = self.gc[7];
        let mut result = 0xFF;
        for plane in 0..4 {
            if care & (1 << plane) == 0 {
                continue;
            }
            let expected = plane_fill(compare, plane);
            result &= !(self.latches[plane] ^ expected);
        }
        result
    }
}

/// Expands bit `plane` of `value` to a full byte of zeros or ones.
fn plane_fill(value: u8, plane: usize) -> u8 {
    if value & (1 << plane) != 0 {
        0xFF
    } else {
        0x00
    }
}
