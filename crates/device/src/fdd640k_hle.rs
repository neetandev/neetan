//! PC-9801-09-style 640KB FDD BIOS HLE extension. This is only used by the real PC-9801-F BIOS,
//! which expects to find an FDC extension BIOS.
//!
//! The controller exposes a generated expansion ROM at 0xD6000 and latches
//! trap writes from that ROM. Rust-side disk BIOS execution lives in the
//! machine bus, because it needs access to guest memory and the floppy image
//! store.

use std::cell::Cell;

save_state::runtime_state! {
/// Complete 640 KB floppy BIOS HLE trap state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fdd640kHleState {
    rom_installed: bool,
    hle_pending: bool,
    yield_requested: bool,
}}

/// Size of the 640KB FDD expansion ROM window mapped at 0xD6000.
pub const ROM_SIZE: usize = 4096;

/// I/O port used by the generated ROM to request Rust-side HLE dispatch.
pub const TRAP_PORT: u16 = 0x07ED;

/// 4096-byte PC-9801-09-style expansion ROM image.
static ROM_IMAGE: &[u8; ROM_SIZE] = include_bytes!("../../../utils/fdd_640k/fdd_640k.rom");

/// PC-9801-09-style 640KB FDD BIOS HLE controller.
#[derive(Debug)]
pub struct Fdd640kHle {
    rom_installed: bool,
    hle_pending: bool,
    yield_requested: Cell<bool>,
}

impl Default for Fdd640kHle {
    fn default() -> Self {
        Self::new()
    }
}

impl Fdd640kHle {
    /// Creates an idle controller with no expansion ROM installed.
    pub fn new() -> Self {
        Self {
            rom_installed: false,
            hle_pending: false,
            yield_requested: Cell::new(false),
        }
    }

    /// Captures the complete floppy HLE trap state.
    pub fn capture_state(&self) -> Fdd640kHleState {
        Fdd640kHleState {
            rom_installed: self.rom_installed,
            hle_pending: self.hle_pending,
            yield_requested: self.yield_requested.get(),
        }
    }

    /// Restores the complete floppy HLE trap state.
    pub fn restore_state(&mut self, state: Fdd640kHleState) {
        self.rom_installed = state.rom_installed;
        self.hle_pending = state.hle_pending;
        self.yield_requested.set(state.yield_requested);
    }

    /// Installs the generated expansion ROM.
    pub fn install_rom(&mut self) {
        self.rom_installed = true;
    }

    /// Returns whether the expansion ROM is installed.
    pub fn rom_installed(&self) -> bool {
        self.rom_installed
    }

    /// Reads a byte from the generated expansion ROM at the given offset.
    pub fn read_rom_byte(&self, offset: usize) -> u8 {
        if self.rom_installed {
            ROM_IMAGE[offset]
        } else {
            0xFF
        }
    }

    /// Writes a byte to the trap port. Any value requests HLE dispatch.
    pub fn write_trap_port(&mut self, _value: u8) {
        self.hle_pending = true;
        self.yield_requested.set(true);
    }

    /// Returns true if an HLE trap is pending.
    pub fn hle_pending(&self) -> bool {
        self.hle_pending
    }

    /// Clears the HLE pending flag after execution.
    pub fn clear_hle_pending(&mut self) {
        self.hle_pending = false;
    }

    /// Returns and clears the yield-requested flag.
    pub fn take_yield_requested(&self) -> bool {
        self.yield_requested.replace(false)
    }
}
