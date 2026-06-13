//! Disk sub-CPU (PC80S31K) memory: a flat 64 KiB space.
//!
//! `disk.rom` at 0x0000-0x1FFF, a fixed init pattern at 0x2000-0x7FFF
//! (only 0x4000-0x7FFF is writable RAM), and 0xFF everywhere above 0x8000.
//! The init pattern is the power-on contents the disk ROM expects to find;
//! reproducing it keeps the sub-CPU boot deterministic.

/// Disk sub-CPU 64 KiB memory.
pub(crate) struct SubMemory {
    mem: Box<[u8; 0x1_0000]>,
}

/// Per-0x100-block XOR selector for the 0x2000-0x7FFF init pattern:
/// 0 -> 0xF0, 1 -> 0x0F, 2 -> 0xFF, else 0x00.
const INIT_TABLE: [u8; 0x60] = [
    0, 1, 0, 1, 0, 3, 0, 0, 0, 0, 1, 0, 1, 0, 1, 0, // 0x2000
    0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 0, // 0x3000
    1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, // 0x4000
    1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 3, 1, 0, 1, 0, 1, // 0x5000
    1, 0, 1, 0, 1, 2, 1, 1, 1, 1, 0, 1, 0, 1, 0, 1, // 0x6000
    1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 0, 1, // 0x7000
];

/// Base 16-byte fill pattern, XORed per block.
const INIT_PATTERN: [u8; 0x10] = [
    0x00, 0xFF, 0x00, 0xFF, 0xFF, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0x00, 0xFF, 0x00,
];

impl SubMemory {
    /// Creates the sub memory in its power-on state (no disk ROM yet).
    pub(crate) fn new() -> Self {
        let mut mem = Box::new([0u8; 0x1_0000]);
        // ROM region and high memory default to 0xFF.
        mem[0x0000..0x2000].fill(0xFF);
        mem[0x8000..0x1_0000].fill(0xFF);
        // JR * at the reset vector idles the sub CPU until disk.rom is loaded.
        mem[0x0000] = 0x18;
        mem[0x0001] = 0xFE;
        // 0x2000-0x7FFF init pattern.
        for (block, &selector) in INIT_TABLE.iter().enumerate() {
            let eor = match selector {
                0 => 0xF0,
                1 => 0x0F,
                2 => 0xFF,
                _ => 0x00,
            };
            let base = 0x2000 + block * 0x100;
            for (low, &pattern) in INIT_PATTERN.iter().enumerate() {
                let byte = pattern ^ eor;
                let start = base + low * 0x10;
                mem[start..start + 0x10].fill(byte);
            }
        }
        Self { mem }
    }

    /// Loads the disk sub-CPU ROM (8 KiB) at 0x0000-0x1FFF.
    pub(crate) fn load_disk_rom(&mut self, data: &[u8]) {
        let length = data.len().min(0x2000);
        self.mem[0..length].copy_from_slice(&data[..length]);
    }

    /// Reads a byte (the whole space is readable).
    pub(crate) fn read(&self, address: u16) -> u8 {
        self.mem[address as usize]
    }

    /// Writes a byte. Only 0x4000-0x7FFF is writable RAM; other regions ignore writes.
    pub(crate) fn write(&mut self, address: u16, value: u8) {
        if (0x4000..0x8000).contains(&address) {
            self.mem[address as usize] = value;
        }
    }
}
