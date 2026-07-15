//! Software DIP Switch (SDIP) - battery-backed configuration storage.
//!
//! PC-9821 and late PC-9801 machines replace physical DIP switches with
//! software-configurable switches stored in NVRAM. The SDIP has two banks
//! (front and back) of 12 bytes each, accessed via I/O ports 0x841E–0x8F1E.
//!
//! Bank selection varies by model:
//! - PC-9821 first-gen, Ce: port 0x00F6 (0xA0 = front, 0xE0 = back)
//! - Later PC-9821 desktop: port 0x8F1F (0x80 = front, 0xC0 = back)
//!
//! Each byte carries an odd-parity bit. If the BIOS detects bad parity on
//! any SDIP byte, it shows "SET THE SOFTWARE DIP SWITCH" and refuses to boot.
//!
//! Ref: undoc98 `io_sdip.txt`

/// Number of registers per bank.
pub const BANK_SIZE: usize = 12;

/// Total storage: front bank + back bank.
const TOTAL_SIZE: usize = BANK_SIZE * 2;

/// Default front bank values for a PC-9821 desktop.
///
/// Each byte includes correct odd parity. Values configure a standard
/// PC-9821 desktop: 640 KB main RAM, 512 B HDD sectors, 80×25 text,
/// 2.5 MHz GDC clock, auto-switch FDD, sound enabled.
///
/// Ref: undoc98 `io_sdip.txt` (ports 0x841E–0x8F1E front bank)
const FRONT_BANK_DEFAULTS: [u8; BANK_SIZE] = [
    // 0x841E - DSW1: GRPH ext, 512 B HDD, RS-232C async, FDD 1/2, CRT
    //   bits 7-1 = 1111_100 -> 5 ones (odd) -> parity bit 0 = 0
    0xF8,
    // 0x851E - DSW2: GDC 2.5 MHz, HDD connected, 25 lines, 80 cols, no terminal
    //   bits {7,6,5,3,2,1,0} = 1,1,1,0,0,1,1 -> 5 ones (odd) -> parity bit 4 = 0
    0xE3,
    // 0x861E - DSW3: 640 KB, DMA compat, FDD motor ctrl, auto-switch FDD
    //   bits 6-0 = 011_1001 -> 4 ones (even) -> parity bit 7 = 1
    0xB9,
    // 0x871E - MEMSW init yes, BEEP loud
    //   bits 6-0 = 010_1100 -> 3 ones (odd) -> parity bit 7 = 0
    0x2C,
    // 0x881E - sound enabled
    //   bits 6-0 = 010_0000 -> 1 one (odd) -> parity bit 7 = 0
    0x20, // 0x891E - modem defaults (no modem); parity shared with 0x8A1E
    0xFC,
    // 0x8A1E - modem defaults (no modem); bit 7 = combined parity with 0x891E
    //   0xFC (6 ones) + 0xBF (7 ones) = 13 total -> odd ✓
    0xBF,
    // 0x8B1E - unused on desktop
    //   bits 6-0 = 000_0000 -> 0 ones (even) -> parity bit 7 = 1
    0x80,
    // 0x8C1E - no auto power-off
    //   bits 6-0 = 000_1111 -> 4 ones (even) -> parity bit 7 = 1
    0x8F,
    // 0x8D1E - no RAM drive, FDD boot, FDD first drive
    //   bits 6-0 = 101_1111 -> 6 ones (even) -> parity bit 7 = 1
    0xDF,
    // 0x8E1E - High CPU mode, SDIP active (bit 2 = 1)
    //   On PC-9821 (no hardware DIP switches), bit 2 must be 1 to indicate
    //   software DIP switches are in use. If 0, the BIOS enters setup mode.
    //   bits 6-0 = 000_0100 -> 1 one (odd) -> parity bit 7 = 0
    0x04,
    // 0x8F1E - FDD drives 1/2, High CPU mode
    //   bits 6-0 = 000_1000 -> 1 one (odd) -> parity bit 7 = 0
    0x08,
];

/// Default back bank values (mostly unused, parity-correct).
///
/// Ref: undoc98 `io_sdip.txt` (ports 0x841E–0x8F1E back bank)
const BACK_BANK_DEFAULTS: [u8; BANK_SIZE] = [
    // 0x841E back - HDD motor never stops (bits 3-0 = 1111)
    //   bits 6-0 = 000_1111 -> 4 ones (even) -> parity bit 7 = 1
    0x8F,
    // 0x851E back - unused on desktop
    //   bits 6-0 = 000_0000 -> 0 ones (even) -> parity bit 7 = 1
    0x80, // 0x861E–0x8F1E back - unused, parity-only
    0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80,
];

/// Returns the front bank values as they should appear in the CGROM.
///
/// The PC-9821 BIOS reads SDIP defaults from CGROM character 0x0156. For
/// most registers the CGROM value is written directly to the SDIP. Registers
/// 7 (0x8B1E) and 10 (0x8E1E) receive special processing by the BIOS:
/// - Register 10: BIOS ORs with 0x04 and saves separately.
/// - Register 7: BIOS conditionally ORs with 0x40 and fixes parity.
///
/// This function returns values that produce valid SDIP state after the
/// BIOS applies its transformations.
pub fn front_bank_cgrom_defaults() -> [u8; BANK_SIZE] {
    let mut values = FRONT_BANK_DEFAULTS;
    // Register 10: BIOS will OR with 0x04, so store 0x00.
    values[10] = 0x00;
    values
}

/// Returns the back bank values as they should appear in the CGROM.
///
/// The BIOS writes back bank values directly without transformation.
pub fn back_bank_cgrom_defaults() -> [u8; BANK_SIZE] {
    BACK_BANK_DEFAULTS
}

save_state::runtime_state! {
/// Authoritative SDIP state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdipState {
    /// 24-byte storage: `[0..12)` = front bank, `[12..24)` = back bank.
    pub ram: [u8; TOTAL_SIZE],
    /// Currently selected bank (`false` = front, `true` = back).
    pub bank: bool,
}}

/// Software DIP Switch controller.
pub struct Sdip {
    /// Embedded state for save/restore.
    pub state: SdipState,
}

impl_simple_state_accessors!(Sdip, SdipState);

impl Default for Sdip {
    fn default() -> Self {
        Self::new()
    }
}

impl Sdip {
    /// Creates a new SDIP with default PC-9821 desktop values.
    pub fn new() -> Self {
        let mut ram = [0u8; TOTAL_SIZE];
        ram[..BANK_SIZE].copy_from_slice(&FRONT_BANK_DEFAULTS);
        ram[BANK_SIZE..].copy_from_slice(&BACK_BANK_DEFAULTS);
        Self {
            state: SdipState { ram, bank: false },
        }
    }

    /// Reads an SDIP register (offset 0–11 within the current bank).
    pub fn read(&self, offset: usize) -> u8 {
        let index = offset + if self.state.bank { BANK_SIZE } else { 0 };
        self.state.ram[index]
    }

    /// Reads an SDIP register from the front bank, regardless of bank selection.
    ///
    /// System ports like 0x31 (DIP switch 2) always reflect front bank values.
    pub fn read_front_bank(&self, offset: usize) -> u8 {
        self.state.ram[offset]
    }

    /// Writes an SDIP register (offset 0–11 within the current bank).
    pub fn write(&mut self, offset: usize, value: u8) {
        let index = offset + if self.state.bank { BANK_SIZE } else { 0 };
        self.state.ram[index] = value;
    }

    /// Modifies a data bit in a front bank register and recomputes odd parity.
    ///
    /// Parity bit positions per register:
    /// - Register 0: bit 0
    /// - Register 1: bit 4
    /// - Registers 2-4, 7-11: bit 7
    /// - Registers 5-6: combined parity (not supported, will panic)
    pub fn set_front_bank_bit(&mut self, register: usize, bit: u8, value: bool) {
        debug_assert!(register < BANK_SIZE, "register out of range");
        debug_assert!(bit < 8, "bit out of range");

        let parity_bit = parity_bit_position(register);
        debug_assert_ne!(bit, parity_bit, "cannot set the parity bit directly");

        if value {
            self.state.ram[register] |= 1 << bit;
        } else {
            self.state.ram[register] &= !(1 << bit);
        }

        // Recompute odd parity: total ones (data + parity) must be odd.
        let data = self.state.ram[register] & !(1 << parity_bit);
        if data.count_ones().is_multiple_of(2) {
            self.state.ram[register] |= 1 << parity_bit;
        } else {
            self.state.ram[register] &= !(1 << parity_bit);
        }
    }

    /// Selects the SDIP bank from bit 6 of the written value.
    ///
    /// Bit 6 = 0 -> front bank, bit 6 = 1 -> back bank.
    /// Called on writes to port 0x00F6 (0xA0/0xE0) or port 0x8F1F (0x80/0xC0).
    pub fn select_bank_from_bit6(&mut self, value: u8) {
        self.state.bank = value & 0x40 != 0;
    }
}

/// Returns the parity bit position for the given SDIP register index.
fn parity_bit_position(register: usize) -> u8 {
    match register {
        0 => 0,
        1 => 4,
        2..=4 | 7..=11 => 7,
        5 | 6 => panic!("registers 5-6 use combined parity across both bytes"),
        _ => panic!("invalid SDIP register index"),
    }
}
