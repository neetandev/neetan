//! Sharp X1 memory map: base X1 IPL ROM + work RAM + ROM/RAM toggle, plus the
//! turbo lower-window RAM banking.
//!
//! The base X1 maps the IPL ROM over the bottom 32 KiB of the address space while
//! the ROM/RAM latch selects ROM; the upper 32 KiB is always work RAM and all
//! writes go to RAM. A write to I/O `0x1D00` selects ROM in the bottom half, a
//! write to `0x1E00` selects RAM.
//!
//! On turbo machines, clearing BMCS in the bank register (I/O `0x0B00`) maps
//! bank RAM over the bottom half of the address space. The register stores the
//! full byte, including the inert BML5 latch in bit 5 and the 4-bit BMNO field
//! in bits 3..0, but known implementations provide only physical bank 0/1, so
//! BMNO aliases by parity. Setting BMCS in bit 4 falls back to the base ROM/RAM
//! toggle. The upper half remains normal work RAM.
//!
//! X1center references used for this behavior:
//! - I/O map and memory banking: <http://x1center.org/sdx1/sdx1_0.html>
//! - External EMM boards, separate from `0x0B00`: <http://x1center.org/resource/x1emm.html>

use crate::config::X1Model;

/// The IPL ROM occupies the bottom 32 KiB of the address space when selected. The
/// base X1 dump is only 4 KiB; the remainder of the region reads back as 0xFF.
const IPL_WINDOW_SIZE: usize = 0x8000;

/// The turbo banked window covers the bottom 32 KiB of the CPU address space.
const TURBO_BANK_WINDOW_SIZE: usize = 0x8000;

/// Physical turbo lower-window bank count documented by X1center.
const TURBO_BANK_COUNT: usize = 2;

/// Bank register bit 4 (BMCS): when clear a lower-window RAM bank is selected,
/// when set the base ROM/RAM map applies. Reset selects the base map so the IPL is
/// visible.
const BANK_SELECT_BASE_MAP: u8 = 0x10;

/// Only BMNO bit 0 selects physical storage; the full register still reads back.
const PHYSICAL_BANK_INDEX_MASK: u8 = 0x01;

/// Base X1 memory: an IPL ROM shadow, work RAM, the ROM/RAM latch and the
/// turbo bank register.
pub struct X1Memory {
    ipl_rom: Vec<u8>,
    work_ram: Vec<u8>,
    turbo_lower_banks: Vec<u8>,
    rom_selected: bool,
    is_turbo: bool,
    ex_bank: u8,
}

save_state::runtime_state! {
/// Authoritative Sharp X1 memory contents and banking state.
#[derive(Clone)]
pub(crate) struct X1MemoryState {
    work_ram: Vec<u8>,
    turbo_lower_banks: Vec<u8>,
    rom_selected: bool,
    extended_bank: u8,
}}

impl X1Memory {
    /// Creates the memory for `model` with the IPL selected (the reset state).
    pub fn new(model: X1Model) -> Self {
        Self {
            ipl_rom: vec![0xFFu8; IPL_WINDOW_SIZE],
            work_ram: vec![0u8; model.work_ram_size()],
            turbo_lower_banks: if model.is_turbo() {
                vec![0u8; TURBO_BANK_COUNT * TURBO_BANK_WINDOW_SIZE]
            } else {
                Vec::new()
            },
            rom_selected: true,
            is_turbo: model.is_turbo(),
            ex_bank: BANK_SELECT_BASE_MAP,
        }
    }

    pub(crate) fn capture_state(&self) -> X1MemoryState {
        X1MemoryState {
            work_ram: self.work_ram.clone(),
            turbo_lower_banks: self.turbo_lower_banks.clone(),
            rom_selected: self.rom_selected,
            extended_bank: self.ex_bank,
        }
    }

    pub(crate) fn restore_state(
        &mut self,
        state: X1MemoryState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.work_ram.len() != self.work_ram.len()
            || state.turbo_lower_banks.len() != self.turbo_lower_banks.len()
        {
            return Err(save_state::StateValidationError::new(
                "X1 memory size differs from the active model",
            ));
        }
        self.work_ram = state.work_ram;
        self.turbo_lower_banks = state.turbo_lower_banks;
        self.rom_selected = state.rom_selected;
        self.ex_bank = state.extended_bank;
        Ok(())
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

    /// Sets the turbo bank register (I/O `0x0B00`).
    pub fn set_ex_bank(&mut self, value: u8) {
        self.ex_bank = value;
    }

    /// The current turbo bank-register value.
    pub fn ex_bank(&self) -> u8 {
        self.ex_bank
    }

    /// Whether a turbo lower-window RAM bank is currently selected (BMCS clear).
    fn turbo_lower_bank_selected(&self) -> bool {
        self.is_turbo && (self.ex_bank & BANK_SELECT_BASE_MAP) == 0
    }

    /// The offset into `turbo_lower_banks` of the currently selected bank.
    fn turbo_lower_bank_offset(&self) -> usize {
        (self.ex_bank & PHYSICAL_BANK_INDEX_MASK) as usize * TURBO_BANK_WINDOW_SIZE
    }

    /// Reads a byte from the CPU address space.
    pub fn read(&self, address: u16) -> u8 {
        if self.turbo_lower_bank_selected() && (address as usize) < TURBO_BANK_WINDOW_SIZE {
            self.turbo_lower_banks[self.turbo_lower_bank_offset() + address as usize]
        } else if address < IPL_WINDOW_SIZE as u16 && self.rom_selected {
            self.ipl_rom[address as usize]
        } else {
            self.work_ram[address as usize]
        }
    }

    /// Writes a byte to the CPU address space. Turbo lower-bank mode writes the
    /// bottom half to the selected slot; all other writes go to work RAM.
    pub fn write(&mut self, address: u16, value: u8) {
        if self.turbo_lower_bank_selected() && (address as usize) < TURBO_BANK_WINDOW_SIZE {
            let offset = self.turbo_lower_bank_offset() + address as usize;
            self.turbo_lower_banks[offset] = value;
        } else {
            self.work_ram[address as usize] = value;
        }
    }
}
