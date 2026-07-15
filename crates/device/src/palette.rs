//! Palette registers - digital (8-color) and analog (16-color) modes.
//!
//! In digital mode (mode2 bit 0 = 0), ports 0xA8/AA/AC/AE store
//! packed nibble pairs directly.
//!
//! In analog mode (mode2 bit 0 = 1), port 0xA8 selects the palette
//! index, and ports 0xAA/AC/AE set green/red/blue components.
//!
//! The bus checks `display_control.is_palette_analog_mode()` to route writes.

/// Default digital palette: packed nibble pairs mapping 8 digital colors.
/// Each byte encodes two 3-bit color indices (high nibble = color N+4, low nibble = color N).
const DEFAULT_DIGITAL_PALETTE: [u8; 4] = [0x04, 0x15, 0x26, 0x37];

/// Default analog palette: 16 entries of [green, red, blue] (4-bit each).
/// Forms a CGA-like color table with 8 normal + 8 bright colors (index 8 = dark gray).
///
/// Packed source values (GRB nibbles): 0x000, 0x007, 0x070, 0x077,
///   0x700, 0x707, 0x770, 0x777, 0x444, 0x00F, 0x0F0, 0x0FF,
///   0xF00, 0xF0F, 0xFF0, 0xFFF
const DEFAULT_ANALOG_PALETTE: [[u8; 3]; 16] = [
    [0x0, 0x0, 0x0], // 0: black
    [0x0, 0x0, 0x7], // 1: blue
    [0x0, 0x7, 0x0], // 2: red
    [0x0, 0x7, 0x7], // 3: magenta
    [0x7, 0x0, 0x0], // 4: green
    [0x7, 0x0, 0x7], // 5: cyan
    [0x7, 0x7, 0x0], // 6: yellow
    [0x7, 0x7, 0x7], // 7: white
    [0x4, 0x4, 0x4], // 8: dark gray
    [0x0, 0x0, 0xF], // 9: bright blue
    [0x0, 0xF, 0x0], // 10: bright red
    [0x0, 0xF, 0xF], // 11: bright magenta
    [0xF, 0x0, 0x0], // 12: bright green
    [0xF, 0x0, 0xF], // 13: bright cyan
    [0xF, 0xF, 0x0], // 14: bright yellow
    [0xF, 0xF, 0xF], // 15: bright white
];

save_state::runtime_state! {
/// Authoritative palette state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteState {
    /// Analog palette index register (write via port 0xA8 in analog mode).
    pub index: u8,
    /// 8-color digital palette (packed format, ports 0xA8/AA/AC/AE in digital mode).
    pub digital: [u8; 4],
    /// 16-color analog palette: 16 entries of [green, red, blue] (4-bit each).
    pub analog: [[u8; 3]; 16],
}}

/// Palette controller.
pub struct Palette {
    /// Embedded state for save/restore.
    pub state: PaletteState,
}

impl_simple_state_accessors!(Palette, PaletteState);

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}

impl Palette {
    /// Creates a new palette with default colors.
    pub fn new() -> Self {
        Self {
            state: PaletteState {
                index: 0,
                digital: DEFAULT_DIGITAL_PALETTE,
                analog: DEFAULT_ANALOG_PALETTE,
            },
        }
    }

    /// Writes the analog palette index register (port 0xA8 in analog mode).
    pub fn write_index(&mut self, value: u8) {
        self.state.index = value;
    }

    /// Writes an analog palette component for the currently selected index.
    ///
    /// `component`: 0=green (0xAA), 1=red (0xAC), 2=blue (0xAE).
    pub fn write_analog(&mut self, component: usize, value: u8) {
        let i = (self.state.index & 0x0F) as usize;
        self.state.analog[i][component] = value;
    }

    /// Writes a digital palette register.
    ///
    /// `index`: 0 (0xA8), 1 (0xAA), 2 (0xAC), 3 (0xAE).
    pub fn write_digital(&mut self, index: usize, value: u8) {
        self.state.digital[index] = value;
    }

    /// Reads the analog palette index register (port 0xA8 in analog mode).
    pub fn read_index(&self) -> u8 {
        self.state.index & 0x0F
    }

    /// Reads an analog palette component for the currently selected index.
    ///
    /// `component`: 0=green (0xAA), 1=red (0xAC), 2=blue (0xAE).
    pub fn read_analog(&self, component: usize) -> u8 {
        let i = (self.state.index & 0x0F) as usize;
        self.state.analog[i][component] & 0x0F
    }

    /// Reads a digital palette register.
    ///
    /// `index`: 0 (0xA8), 1 (0xAA), 2 (0xAC), 3 (0xAE).
    pub fn read_digital(&self, index: usize) -> u8 {
        self.state.digital[index]
    }
}
