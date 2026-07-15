//! NEC PC80S31K floppy sub-system memory and PPI mailbox.

use crate::i8255::{I8255, I8255Write};

save_state::runtime_state! {
/// Mutable RAM owned by a PC80S31K disk subsystem.
#[derive(Clone)]
pub struct Pc80s31kMemoryState {
    ram: Vec<u8>,
}}

save_state::runtime_state! {
/// State of the paired PPI mailbox.
#[derive(Clone)]
pub struct Pc80s31kPpiLinkState {
    main: crate::i8255::I8255State,
    sub: crate::i8255::I8255State,
}}

/// Disk sub-CPU 64 KiB memory.
pub struct Pc80s31kMemory {
    memory: Box<[u8; 0x1_0000]>,
}

/// Per-block selector for the power-on memory pattern.
const INITIAL_PATTERN_SELECTOR: [u8; 0x60] = [
    0, 1, 0, 1, 0, 3, 0, 0, 0, 0, 1, 0, 1, 0, 1, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 0,
    1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 3, 1, 0, 1, 0, 1,
    1, 0, 1, 0, 1, 2, 1, 1, 1, 1, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 0, 1,
];

/// Base 16-byte fill pattern, XORed per block.
const INITIAL_PATTERN: [u8; 0x10] = [
    0x00, 0xFF, 0x00, 0xFF, 0xFF, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0x00, 0xFF, 0x00,
];

impl Pc80s31kMemory {
    /// Creates the sub-system memory in its power-on state.
    pub fn new() -> Self {
        let mut memory = Box::new([0u8; 0x1_0000]);
        memory[0x0000..0x2000].fill(0xFF);
        memory[0x8000..0x1_0000].fill(0xFF);
        memory[0x0000] = 0x18;
        memory[0x0001] = 0xFE;
        for (block, &selector) in INITIAL_PATTERN_SELECTOR.iter().enumerate() {
            let exclusive_or = match selector {
                0 => 0xF0,
                1 => 0x0F,
                2 => 0xFF,
                _ => 0x00,
            };
            let base = 0x2000 + block * 0x100;
            for (low, &pattern) in INITIAL_PATTERN.iter().enumerate() {
                let byte = pattern ^ exclusive_or;
                let start = base + low * 0x10;
                memory[start..start + 0x10].fill(byte);
            }
        }
        Self { memory }
    }

    /// Loads up to 8 KiB of sub-CPU ROM at address zero.
    pub fn load_rom(&mut self, data: &[u8]) {
        let length = data.len().min(0x2000);
        self.memory[..length].copy_from_slice(&data[..length]);
    }

    /// Reads one byte from the sub-CPU address space.
    pub fn read(&self, address: u16) -> u8 {
        self.memory[usize::from(address)]
    }

    /// Writes one byte when the address selects writable RAM.
    pub fn write(&mut self, address: u16, value: u8) {
        if (0x4000..0x8000).contains(&address) {
            self.memory[usize::from(address)] = value;
        }
    }

    /// Captures writable sub-system RAM without the disk ROM.
    pub fn capture_state(&self) -> Pc80s31kMemoryState {
        Pc80s31kMemoryState {
            ram: self.memory[0x4000..0x8000].to_vec(),
        }
    }

    /// Restores writable sub-system RAM without changing the disk ROM.
    pub fn restore_state(
        &mut self,
        state: Pc80s31kMemoryState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.ram.len() != 0x4000 {
            return Err(save_state::StateValidationError::new(
                "PC80S31K RAM length is invalid",
            ));
        }
        self.memory[0x4000..0x8000].copy_from_slice(&state.ram);
        Ok(())
    }
}

impl Default for Pc80s31kMemory {
    fn default() -> Self {
        Self::new()
    }
}

/// Two Intel 8255 PPIs wired as the PC80S31K mailbox.
pub struct Pc80s31kPpiLink {
    main: I8255,
    sub: I8255,
}

impl Pc80s31kPpiLink {
    /// Creates both PPIs in their power-on state.
    pub fn new() -> Self {
        Self {
            main: I8255::new(),
            sub: I8255::new(),
        }
    }

    /// Reads one main-side PPI register.
    pub fn read_main(&self, register: u8) -> u8 {
        self.main.read(register)
    }

    /// Reads one sub-side PPI register.
    pub fn read_sub(&self, register: u8) -> u8 {
        self.sub.read(register)
    }

    /// Writes one main-side register and reports a handshake change.
    pub fn write_main(&mut self, register: u8, value: u8) -> bool {
        let changed = self.main.write(register, value);
        Self::propagate(&self.main, &mut self.sub, changed)
    }

    /// Writes one sub-side register and reports a handshake change.
    pub fn write_sub(&mut self, register: u8, value: u8) -> bool {
        let changed = self.sub.write(register, value);
        Self::propagate(&self.sub, &mut self.main, changed)
    }

    fn propagate(source: &I8255, destination: &mut I8255, changed: I8255Write) -> bool {
        match changed {
            I8255Write::PortA => destination.set_port_b(source.port_a()),
            I8255Write::PortB => destination.set_port_a(source.port_b()),
            I8255Write::PortC => destination.set_port_c(source.port_c().rotate_left(4)),
            I8255Write::Mode => {
                destination.set_port_b(source.port_a());
                destination.set_port_a(source.port_b());
                destination.set_port_c(source.port_c().rotate_left(4));
            }
            I8255Write::None => {}
        }
        matches!(changed, I8255Write::PortC | I8255Write::Mode)
    }

    /// Captures both sides of the mailbox.
    pub fn capture_state(&self) -> Pc80s31kPpiLinkState {
        Pc80s31kPpiLinkState {
            main: self.main.state.clone(),
            sub: self.sub.state.clone(),
        }
    }

    /// Restores both sides of the mailbox.
    pub fn restore_state(&mut self, state: Pc80s31kPpiLinkState) {
        self.main.state = state.main;
        self.sub.state = state.sub;
    }
}

impl Default for Pc80s31kPpiLink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_only_writes_ram_window() {
        let mut memory = Pc80s31kMemory::new();
        let rom = memory.read(0);
        memory.write(0, 0x55);
        memory.write(0x4000, 0xAA);
        assert_eq!(memory.read(0), rom);
        assert_eq!(memory.read(0x4000), 0xAA);
    }

    #[test]
    fn mailbox_crosses_data_ports() {
        let mut link = Pc80s31kPpiLink::new();
        link.write_main(0, 0x5A);
        link.write_sub(0, 0xA5);
        assert_eq!(link.read_sub(1), 0x5A);
        assert_eq!(link.read_main(1), 0xA5);
    }

    #[test]
    fn mailbox_swaps_port_c_nibbles_and_requests_resync() {
        let mut link = Pc80s31kPpiLink::new();
        assert!(link.write_main(3, (4 << 1) | 1));
        assert_eq!(link.read_sub(2) & 0x0F, 0x01);
    }
}
