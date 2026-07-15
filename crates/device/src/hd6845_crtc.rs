//! HD6845 (MC6845-family) CRT controller.
//!
//! The chip is programmed through an address latch and a data register: the machine
//! writes a register index to the address port, then reads or writes that register
//! through the data port. This device is a passive register shadow. It holds the
//! programmed geometry and cursor state and derives the frame dimensions from them.
//! The machine layer owns the port decode and drives raster timing from the derived
//! geometry.

/// Number of addressable CRTC registers (R0..R17).
pub const REGISTER_COUNT: usize = 18;

const R_HORIZONTAL_TOTAL: usize = 0;
const R_HORIZONTAL_DISPLAYED: usize = 1;
const R_VERTICAL_TOTAL: usize = 4;
const R_VERTICAL_TOTAL_ADJUST: usize = 5;
const R_VERTICAL_DISPLAYED: usize = 6;
const R_MAX_SCANLINE: usize = 9;
const R_CURSOR_START: usize = 10;
const R_CURSOR_END: usize = 11;
const R_START_ADDRESS_HIGH: usize = 12;
const R_START_ADDRESS_LOW: usize = 13;
const R_CURSOR_ADDRESS_HIGH: usize = 14;
const R_CURSOR_ADDRESS_LOW: usize = 15;
const R_LIGHT_PEN_HIGH: usize = 16;
const R_LIGHT_PEN_LOW: usize = 17;

/// Display-memory address mask (14-bit refresh address).
const ADDRESS_MASK: u16 = 0x3FFF;

/// Cursor-mode field of R10 (bits 5..6): 0 no-blink, 1 off, 2 slow, 3 fast blink.
const CURSOR_MODE_MASK: u8 = 0x60;
const CURSOR_MODE_OFF: u8 = 0x20;

save_state::runtime_state! {
/// Snapshot of the HD6845 CRTC state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hd6845State {
    /// Register file R0..R17.
    pub regs: [u8; REGISTER_COUNT],
    /// Latched register index selected through the address port.
    pub address: u8,
}}

/// HD6845 CRT controller.
pub struct Hd6845 {
    /// Embedded state for save/restore.
    pub state: Hd6845State,
}

impl Default for Hd6845 {
    fn default() -> Self {
        Self::new()
    }
}

impl Hd6845 {
    /// Creates a CRTC with a zeroed register file.
    pub fn new() -> Self {
        Self {
            state: Hd6845State {
                regs: [0; REGISTER_COUNT],
                address: 0,
            },
        }
    }

    /// Latches the register index selected through the address port.
    pub fn write_address(&mut self, value: u8) {
        self.state.address = value;
    }

    /// Writes the currently selected register through the data port. Indices
    /// beyond the register file are ignored.
    pub fn write_data(&mut self, value: u8) {
        let index = self.state.address as usize;
        if index < REGISTER_COUNT {
            self.state.regs[index] = value;
        }
    }

    /// Reads the currently selected register through the data port. Only the
    /// cursor and light-pen registers (R14..R17) are readable; other indices
    /// return zero, matching the 6845.
    pub fn read_data(&self) -> u8 {
        let index = self.state.address as usize;
        match index {
            R_CURSOR_ADDRESS_HIGH | R_CURSOR_ADDRESS_LOW | R_LIGHT_PEN_HIGH | R_LIGHT_PEN_LOW => {
                self.state.regs[index]
            }
            _ => 0,
        }
    }

    /// Displayed character columns per row (R1).
    pub fn display_width_chars(&self) -> u16 {
        u16::from(self.state.regs[R_HORIZONTAL_DISPLAYED])
    }

    /// Displayed character rows (R6).
    pub fn display_height_rows(&self) -> u16 {
        u16::from(self.state.regs[R_VERTICAL_DISPLAYED])
    }

    /// Scanlines per character row (R9 + 1).
    pub fn char_height(&self) -> u16 {
        u16::from(self.state.regs[R_MAX_SCANLINE] & 0x1F) + 1
    }

    /// Displayed scanlines (rows times character height).
    pub fn display_height_lines(&self) -> u16 {
        self.display_height_rows() * self.char_height()
    }

    /// Total scanlines per frame including the vertical adjust (R4/R5/R9).
    pub fn total_scanlines(&self) -> u16 {
        let vertical_total = u16::from(self.state.regs[R_VERTICAL_TOTAL] & 0x7F) + 1;
        let adjust = u16::from(self.state.regs[R_VERTICAL_TOTAL_ADJUST] & 0x1F);
        vertical_total * self.char_height() + adjust
    }

    /// Total character columns per scanline (R0 + 1).
    pub fn horizontal_total(&self) -> u16 {
        u16::from(self.state.regs[R_HORIZONTAL_TOTAL]) + 1
    }

    /// Display-memory start address (R12/R13, 14-bit).
    pub fn start_address(&self) -> u16 {
        ((u16::from(self.state.regs[R_START_ADDRESS_HIGH]) << 8)
            | u16::from(self.state.regs[R_START_ADDRESS_LOW]))
            & ADDRESS_MASK
    }

    /// Cursor display-memory address (R14/R15, 14-bit).
    pub fn cursor_address(&self) -> u16 {
        ((u16::from(self.state.regs[R_CURSOR_ADDRESS_HIGH]) << 8)
            | u16::from(self.state.regs[R_CURSOR_ADDRESS_LOW]))
            & ADDRESS_MASK
    }

    /// Whether the cursor is currently enabled (R10 cursor mode not "off").
    pub fn cursor_enabled(&self) -> bool {
        (self.state.regs[R_CURSOR_START] & CURSOR_MODE_MASK) != CURSOR_MODE_OFF
    }

    /// First scanline of the cursor within a character row (R10 low bits).
    pub fn cursor_start_line(&self) -> u8 {
        self.state.regs[R_CURSOR_START] & 0x1F
    }

    /// Last scanline of the cursor within a character row (R11 low bits).
    pub fn cursor_end_line(&self) -> u8 {
        self.state.regs[R_CURSOR_END] & 0x1F
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_latch_selects_the_written_register() {
        let mut crtc = Hd6845::new();
        crtc.write_address(R_HORIZONTAL_DISPLAYED as u8);
        crtc.write_data(80);
        assert_eq!(crtc.display_width_chars(), 80);
    }

    #[test]
    fn geometry_derives_from_registers() {
        let mut crtc = Hd6845::new();
        // 40x25 text, 8 scanlines per row.
        crtc.write_address(R_HORIZONTAL_DISPLAYED as u8);
        crtc.write_data(40);
        crtc.write_address(R_VERTICAL_DISPLAYED as u8);
        crtc.write_data(25);
        crtc.write_address(R_MAX_SCANLINE as u8);
        crtc.write_data(7);
        assert_eq!(crtc.display_width_chars(), 40);
        assert_eq!(crtc.display_height_rows(), 25);
        assert_eq!(crtc.char_height(), 8);
        assert_eq!(crtc.display_height_lines(), 200);
    }

    #[test]
    fn start_address_combines_high_and_low_registers() {
        let mut crtc = Hd6845::new();
        crtc.write_address(R_START_ADDRESS_HIGH as u8);
        crtc.write_data(0x12);
        crtc.write_address(R_START_ADDRESS_LOW as u8);
        crtc.write_data(0x34);
        assert_eq!(crtc.start_address(), 0x1234);
    }

    #[test]
    fn only_cursor_and_light_pen_registers_read_back() {
        let mut crtc = Hd6845::new();
        crtc.write_address(R_CURSOR_ADDRESS_HIGH as u8);
        crtc.write_data(0x2A);
        assert_eq!(crtc.read_data(), 0x2A);

        crtc.write_address(R_HORIZONTAL_DISPLAYED as u8);
        crtc.write_data(80);
        assert_eq!(crtc.read_data(), 0);
    }

    #[test]
    fn cursor_enable_follows_the_mode_field() {
        let mut crtc = Hd6845::new();
        crtc.write_address(R_CURSOR_START as u8);
        crtc.write_data(CURSOR_MODE_OFF);
        assert!(!crtc.cursor_enabled());
        crtc.write_data(0x00);
        assert!(crtc.cursor_enabled());
    }

    #[test]
    fn writes_past_the_register_file_are_ignored() {
        let mut crtc = Hd6845::new();
        crtc.write_address(0x20);
        crtc.write_data(0xFF);
        assert_eq!(crtc.state.regs, [0; REGISTER_COUNT]);
    }
}
