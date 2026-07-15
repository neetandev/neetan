//! PC-8801 graphics palette.
//!
//! The PC-88 has eight programmable graphics pens plus a background/border pen,
//! each holding a 3-bit-per-channel GRB color. Pens are programmed at ports
//! 0x54-0x5B and the background at port 0x52. In analog mode (port 0x32 bit 5
//! set) each pen takes two writes selected by bit 6 of the value; in digital mode
//! a single write sets one bit per channel. The text layer uses a fixed 8-color
//! palette and is not affected by these registers.

use std::ops::{Deref, DerefMut};

/// Number of programmable pens plus the background pen (index 8).
pub const PEN_COUNT: usize = 9;
/// Index of the background/border pen.
pub const BACKGROUND_PEN: usize = 8;

save_state::runtime_state! {
/// A single palette pen with 3-bit-per-channel GRB intensity (0..=7).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pc88Pen {
    /// Red intensity (0..=7).
    pub red: u8,
    /// Green intensity (0..=7).
    pub green: u8,
    /// Blue intensity (0..=7).
    pub blue: u8,
}}

impl Pc88Pen {
    /// Expands the 3-bit channels to 8-bit RGB for the renderer.
    pub fn to_rgb(self) -> [u8; 3] {
        const LEVELS: [u8; 8] = [0, 36, 73, 109, 146, 182, 219, 255];
        [
            LEVELS[(self.red & 7) as usize],
            LEVELS[(self.green & 7) as usize],
            LEVELS[(self.blue & 7) as usize],
        ]
    }
}

save_state::runtime_state! {
/// Snapshot of the PC-88 palette for save/restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pc88PaletteState {
    /// Eight graphics pens plus the background pen at index 8.
    pub pens: [Pc88Pen; PEN_COUNT],
}}

/// PC-8801 graphics palette.
pub struct Pc88Palette {
    /// Embedded state for save/restore.
    pub state: Pc88PaletteState,
}

impl Deref for Pc88Palette {
    type Target = Pc88PaletteState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for Pc88Palette {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for Pc88Palette {
    fn default() -> Self {
        Self::new()
    }
}

impl Pc88Palette {
    /// Creates a palette in its power-on reset state (all pens black).
    pub fn new() -> Self {
        Self {
            state: Pc88PaletteState {
                pens: [Pc88Pen::default(); PEN_COUNT],
            },
        }
    }

    /// Writes a pen at port 0x54-0x5B. `pen` is `port - 0x54` (0..=7). In analog
    /// mode bit 7 of the value redirects to the background pen and bit 6 selects
    /// the green write versus the blue/red write.
    pub fn write_pen(&mut self, pen: usize, value: u8, analog_mode: bool) {
        if analog_mode {
            let index = if value & 0x80 != 0 {
                BACKGROUND_PEN
            } else {
                pen
            };
            if value & 0x40 != 0 {
                self.state.pens[index].green = value & 7;
            } else {
                self.state.pens[index].blue = value & 7;
                self.state.pens[index].red = (value >> 3) & 7;
            }
        } else {
            self.state.pens[pen].blue = if value & 1 != 0 { 7 } else { 0 };
            self.state.pens[pen].red = if value & 2 != 0 { 7 } else { 0 };
            self.state.pens[pen].green = if value & 4 != 0 { 7 } else { 0 };
        }
    }

    /// Writes the background/border pen at port 0x52 (1-bit-per-channel GRB in
    /// bits 4-6).
    pub fn write_background(&mut self, value: u8) {
        self.state.pens[BACKGROUND_PEN].blue = if value & 0x10 != 0 { 7 } else { 0 };
        self.state.pens[BACKGROUND_PEN].red = if value & 0x20 != 0 { 7 } else { 0 };
        self.state.pens[BACKGROUND_PEN].green = if value & 0x40 != 0 { 7 } else { 0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digital_mode_sets_full_channels() {
        let mut palette = Pc88Palette::new();
        palette.write_pen(3, 0b0000_0111, false); // blue+red+green
        assert_eq!(
            palette.pens[3],
            Pc88Pen {
                red: 7,
                green: 7,
                blue: 7
            }
        );
    }

    #[test]
    fn analog_mode_two_writes() {
        let mut palette = Pc88Palette::new();
        // First write (bit6 clear): blue = 5, red = 2.
        palette.write_pen(1, (2 << 3) | 5, true);
        // Second write (bit6 set): green = 4.
        palette.write_pen(1, 0x40 | 4, true);
        assert_eq!(
            palette.pens[1],
            Pc88Pen {
                red: 2,
                green: 4,
                blue: 5
            }
        );
    }

    #[test]
    fn analog_background_select() {
        let mut palette = Pc88Palette::new();
        palette.write_pen(0, 0x80 | 0x40 | 6, true); // bit7 -> background pen, green
        assert_eq!(palette.pens[BACKGROUND_PEN].green, 6);
    }

    #[test]
    fn background_port_52() {
        let mut palette = Pc88Palette::new();
        palette.write_background(0x10 | 0x40); // blue + green
        let pen = palette.pens[BACKGROUND_PEN];
        assert_eq!((pen.red, pen.green, pen.blue), (0, 7, 7));
    }
}
