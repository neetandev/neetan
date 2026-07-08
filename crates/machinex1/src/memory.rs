//! Sharp X1 memory map: base X1 IPL ROM + work RAM + ROM/RAM toggle, plus the
//! turbo 64 KiB bank register.
//!
//! The base X1 maps the IPL ROM over the bottom 32 KiB of the address space while
//! the ROM/RAM latch selects ROM; the upper 32 KiB is always work RAM and all
//! writes go to RAM. A write to I/O `0x1D00` selects ROM in the bottom half, a
//! write to `0x1E00` selects RAM.
//!
//! On turbo machines the bank register (I/O `0x0B00`) selects one of sixteen flat
//! 64 KiB RAM banks: when its bit 4 (BMCS) is clear the whole address space maps
//! to `bank * 0x1_0000` with no ROM overlay, and when it is set the machine falls
//! back to the base ROM/RAM toggle (which addresses bank 0). Bank 0's lower half is
//! therefore the same storage as the base RAM.

use crate::config::X1Model;

/// The IPL ROM occupies the bottom 32 KiB of the address space when selected. The
/// base X1 dump is only 4 KiB; the remainder of the region reads back as 0xFF.
const IPL_WINDOW_SIZE: usize = 0x8000;

/// One flat turbo RAM bank spans the whole 64 KiB address space.
const BANK_SIZE: usize = 0x1_0000;

/// Bank register bit 4 (BMCS): when clear a flat RAM bank is selected, when set the
/// base ROM/RAM map applies. Reset selects the base map so the IPL is visible.
const BANK_SELECT_BASE_MAP: u8 = 0x10;

/// Only the low six bits of the bank register are stored.
const BANK_REGISTER_MASK: u8 = 0x3F;

/// Low nibble of the bank register: the selected flat RAM bank (0..=15).
const BANK_INDEX_MASK: u8 = 0x0F;

/// Base X1 memory: an IPL ROM shadow, the work-RAM banks, the ROM/RAM latch and the
/// turbo bank register. The work-RAM buffer is sized per model (one bank for the
/// base X1, sixteen for turbo).
pub struct X1Memory {
    ipl_rom: Vec<u8>,
    work_ram: Vec<u8>,
    rom_selected: bool,
    is_turbo: bool,
    ex_bank: u8,
}

impl X1Memory {
    /// Creates the memory for `model` with the IPL selected (the reset state).
    pub fn new(model: X1Model) -> Self {
        Self {
            ipl_rom: vec![0xFFu8; IPL_WINDOW_SIZE],
            work_ram: vec![0u8; model.work_ram_size()],
            rom_selected: true,
            is_turbo: model.is_turbo(),
            ex_bank: BANK_SELECT_BASE_MAP,
        }
    }

    /// Loads the IPL ROM image, padding the shadow window with 0xFF beyond it.
    pub fn load_ipl(&mut self, data: &[u8]) {
        let length = data.len().min(self.ipl_rom.len());
        self.ipl_rom[..length].copy_from_slice(&data[..length]);
        for byte in &mut self.ipl_rom[length..] {
            *byte = 0xFF;
        }
    }

    /// Selects the IPL ROM in the bottom 32 KiB (I/O `0x1D00`).
    pub fn select_rom(&mut self) {
        self.rom_selected = true;
    }

    /// Selects work RAM in the bottom 32 KiB (I/O `0x1E00`).
    pub fn select_ram(&mut self) {
        self.rom_selected = false;
    }

    /// Whether the IPL ROM is currently mapped in the bottom 32 KiB.
    pub fn rom_selected(&self) -> bool {
        self.rom_selected
    }

    /// Sets the turbo bank register (I/O `0x0B00`); only the low six bits are kept.
    pub fn set_ex_bank(&mut self, value: u8) {
        self.ex_bank = value & BANK_REGISTER_MASK;
    }

    /// The current turbo bank-register value.
    pub fn ex_bank(&self) -> u8 {
        self.ex_bank
    }

    /// Whether a flat turbo RAM bank is currently selected (BMCS clear).
    fn flat_bank_selected(&self) -> bool {
        self.is_turbo && (self.ex_bank & BANK_SELECT_BASE_MAP) == 0
    }

    /// The offset into `work_ram` of the currently selected flat RAM bank.
    fn flat_bank_offset(&self) -> usize {
        (self.ex_bank & BANK_INDEX_MASK) as usize * BANK_SIZE
    }

    /// Reads a byte from the CPU address space.
    pub fn read(&self, address: u16) -> u8 {
        if self.flat_bank_selected() {
            self.work_ram[self.flat_bank_offset() + address as usize]
        } else if address < IPL_WINDOW_SIZE as u16 && self.rom_selected {
            self.ipl_rom[address as usize]
        } else {
            self.work_ram[address as usize]
        }
    }

    /// Writes a byte to the CPU address space. Flat-bank mode writes to the selected
    /// bank; otherwise all writes go to work RAM (the ROM window is never written).
    pub fn write(&mut self, address: u16, value: u8) {
        if self.flat_bank_selected() {
            let offset = self.flat_bank_offset() + address as usize;
            self.work_ram[offset] = value;
        } else {
            self.work_ram[address as usize] = value;
        }
    }
}
