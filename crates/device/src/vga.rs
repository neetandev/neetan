//! Tseng Labs ET4000AX SVGA display adapter.
//!
//! Models the full IBM VGA register file (sequencer, CRTC, graphics
//! controller, attribute controller, DAC) with the four-plane display memory
//! and its read/write modes, plus the ET4000AX extensions: the KEY protection
//! of the extended registers, the segment select register at 0x3CD, the
//! extended CRTC registers 0x31-0x37, the extended timing sequencer registers
//! 6 and 7, attribute controller register 0x16, the hidden DAC control
//! register, and 1 MiB of display memory.
//!
//! Display memory is stored dword-interleaved (plane `p` of plane offset `o`
//! lives at `vram[o * 4 + p]`), the layout the chain-4 packed modes address
//! linearly.

mod dac;
mod et4000;
mod io;
mod memory;
mod resolve;
mod timing;

pub use io::RetraceStatus;
pub use resolve::{ResolvedVgaFrame, VgaRenderMode};
pub use timing::VgaFrameTiming;

/// Monochrome CRTC index port (active when misc output bit 0 is clear).
pub const VGA_PORT_CRTC_INDEX_MONO: u16 = 0x03B4;
/// Monochrome CRTC data port.
pub const VGA_PORT_CRTC_DATA_MONO: u16 = 0x03B5;
/// Monochrome mode control port (second KEY write lands here in mono mode).
pub const VGA_PORT_MODE_CONTROL_MONO: u16 = 0x03B8;
/// Monochrome input status one / feature control port.
pub const VGA_PORT_STATUS_MONO: u16 = 0x03BA;
/// Hercules compatibility port (first KEY write lands here).
pub const VGA_PORT_HERCULES_COMPAT: u16 = 0x03BF;
/// Attribute controller index/data port (write flip-flop).
pub const VGA_PORT_ATC_WRITE: u16 = 0x03C0;
/// Attribute controller data read port.
pub const VGA_PORT_ATC_READ: u16 = 0x03C1;
/// Input status zero read / miscellaneous output write port.
pub const VGA_PORT_STATUS0_MISC_WRITE: u16 = 0x03C2;
/// Sequencer index port.
pub const VGA_PORT_SEQ_INDEX: u16 = 0x03C4;
/// Sequencer data port.
pub const VGA_PORT_SEQ_DATA: u16 = 0x03C5;
/// DAC pixel mask port (also the ET4000 hidden DAC control register).
pub const VGA_PORT_DAC_MASK: u16 = 0x03C6;
/// DAC read index write / DAC state read port.
pub const VGA_PORT_DAC_READ_INDEX: u16 = 0x03C7;
/// DAC write index port.
pub const VGA_PORT_DAC_WRITE_INDEX: u16 = 0x03C8;
/// DAC palette data port.
pub const VGA_PORT_DAC_DATA: u16 = 0x03C9;
/// Feature control read port.
pub const VGA_PORT_FEATURE_READ: u16 = 0x03CA;
/// Miscellaneous output read port.
pub const VGA_PORT_MISC_READ: u16 = 0x03CC;
/// ET4000 segment select port (KEY protected).
pub const VGA_PORT_SEGMENT_SELECT: u16 = 0x03CD;
/// Graphics controller index port.
pub const VGA_PORT_GC_INDEX: u16 = 0x03CE;
/// Graphics controller data port.
pub const VGA_PORT_GC_DATA: u16 = 0x03CF;
/// Color CRTC index port (active when misc output bit 0 is set).
pub const VGA_PORT_CRTC_INDEX_COLOR: u16 = 0x03D4;
/// Color CRTC data port.
pub const VGA_PORT_CRTC_DATA_COLOR: u16 = 0x03D5;
/// Color mode control port (second KEY write lands here in color mode).
pub const VGA_PORT_MODE_CONTROL_COLOR: u16 = 0x03D8;
/// Color input status one / feature control port.
pub const VGA_PORT_STATUS_COLOR: u16 = 0x03DA;

/// Display memory size in bytes (1 MiB on the ET4000AX).
pub const VGA_VRAM_SIZE: usize = 0x10_0000;

/// Number of CRTC registers stored (covers the ET4000 extended set).
const CRTC_REGISTER_COUNT: usize = 0x40;
/// Number of sequencer registers stored.
const SEQ_REGISTER_COUNT: usize = 8;
/// Number of graphics controller registers stored.
const GC_REGISTER_COUNT: usize = 9;
/// Number of attribute controller registers stored.
const ATC_REGISTER_COUNT: usize = 0x20;

/// Sequencer index: reset (bit 1 clear asserts synchronous reset).
pub const SEQ_INDEX_RESET: u8 = 0x00;
/// Sequencer index: clocking mode.
pub const SEQ_INDEX_CLOCKING: u8 = 0x01;
/// Sequencer index: plane write mask.
pub const SEQ_INDEX_MAP_MASK: u8 = 0x02;
/// Sequencer index: character map select.
pub const SEQ_INDEX_CHAR_MAP: u8 = 0x03;
/// Sequencer index: memory mode (odd/even, chain 4).
pub const SEQ_INDEX_MEMORY_MODE: u8 = 0x04;
/// Sequencer index: ET4000 timing sequencer state control (KEY protected).
pub const SEQ_INDEX_TS_STATE: u8 = 0x06;
/// Sequencer index: ET4000 timing sequencer auxiliary mode (KEY protected).
pub const SEQ_INDEX_TS_AUX_MODE: u8 = 0x07;

/// CRTC index: cursor start row and cursor disable bit.
pub const CRTC_INDEX_CURSOR_START: u8 = 0x0A;
/// CRTC index: cursor end row.
pub const CRTC_INDEX_CURSOR_END: u8 = 0x0B;
/// CRTC index: vertical retrace end and register write protection.
pub const CRTC_INDEX_VRETRACE_END: u8 = 0x11;
/// Highest CRTC index reachable without the ET4000 KEY.
const CRTC_LAST_UNPROTECTED_INDEX: u8 = 0x18;
/// CRTC index: ET4000 extended start address (not KEY protected).
pub const CRTC_INDEX_EXT_START: u8 = 0x33;
/// CRTC index: ET4000 auxiliary control (clock select bit 2).
pub const CRTC_INDEX_AUX_CONTROL: u8 = 0x34;
/// CRTC index: ET4000 overflow high (protected by CRTC 0x11 bit 7, not KEY).
pub const CRTC_INDEX_OVERFLOW_HIGH: u8 = 0x35;
/// CRTC index: ET4000 video system configuration 1.
pub const CRTC_INDEX_VSCONF1: u8 = 0x36;

/// Attribute controller index: mode control.
pub const ATC_INDEX_MODE_CONTROL: u8 = 0x10;
/// Attribute controller index: overscan color.
pub const ATC_INDEX_OVERSCAN: u8 = 0x11;
/// Attribute controller index: color plane enable.
pub const ATC_INDEX_PLANE_ENABLE: u8 = 0x12;
/// Attribute controller index: horizontal pel panning.
pub const ATC_INDEX_PEL_PAN: u8 = 0x13;
/// Attribute controller index: color select.
pub const ATC_INDEX_COLOR_SELECT: u8 = 0x14;
/// Attribute controller index: ET4000 miscellaneous (KEY protected).
pub const ATC_INDEX_ET4000_MISC: u8 = 0x16;

/// Value whose write to the Hercules compatibility port arms the KEY.
const KEY_PREFIX_VALUE: u8 = 0x03;
/// Value whose write to the mode control port completes the KEY sequence.
const KEY_COMPLETE_VALUE: u8 = 0xA0;

/// Tseng Labs ET4000AX display adapter state.
pub struct Vga {
    /// Miscellaneous output register (bit 0 selects color I/O decode).
    pub misc_output: u8,
    /// Feature control register.
    pub feature_control: u8,
    /// Current sequencer index.
    pub seq_index: u8,
    /// Sequencer registers.
    pub seq: [u8; SEQ_REGISTER_COUNT],
    /// Current CRTC index.
    pub crtc_index: u8,
    /// CRTC registers, including the ET4000 extended set.
    pub crtc: [u8; CRTC_REGISTER_COUNT],
    /// Current graphics controller index.
    pub gc_index: u8,
    /// Graphics controller registers.
    pub gc: [u8; GC_REGISTER_COUNT],
    /// Current attribute controller index (bit 5 is the palette address
    /// source; when clear the screen blanks and the palette is writable).
    pub atc_index: u8,
    /// Attribute controller flip-flop: next 0x3C0 write is data, not index.
    pub atc_data_phase: bool,
    /// Attribute controller registers.
    pub atc: [u8; ATC_REGISTER_COUNT],
    /// DAC pixel mask.
    pub dac_mask: u8,
    /// DAC write cycle index.
    pub dac_write_index: u8,
    /// DAC read cycle index.
    pub dac_read_index: u8,
    /// Position within the current three-byte DAC color cycle (0-2).
    pub dac_cycle: u8,
    /// Whether the DAC is in a read cycle (state register reads 3).
    pub dac_read_mode: bool,
    /// Bytes collected for the current DAC write cycle.
    pub dac_write_latch: [u8; 3],
    /// Consecutive DAC mask port reads (four unlock the hidden register).
    pub dac_hidden_counter: u8,
    /// ET4000 hidden DAC control register.
    pub dac_hidden_control: u8,
    /// DAC palette: 256 entries of 6-bit red, green, blue.
    pub dac: [[u8; 3]; 256],
    /// The four plane latches loaded by display memory reads.
    pub latches: [u8; 4],
    /// ET4000 segment select: write bank in the low nibble, read bank in the
    /// high nibble.
    pub segment_select: u8,
    /// Whether the KEY prefix (0x03 to the Hercules compatibility port) was
    /// the most recent write to that port.
    pub key_prefix_armed: bool,
    /// Whether the ET4000 extended registers are unlocked by the KEY.
    pub key_unlocked: bool,
    /// Raw Hercules compatibility register byte.
    pub hercules_compat: u8,
    /// Raw mode control register byte (ports 0x3B8/0x3D8).
    pub mode_control: u8,
    /// Vertical retrace interrupt latch (input status zero bit 7).
    pub vretrace_interrupt_latch: bool,
    /// Frame counter advanced at every vertical sync, drives blink phases.
    pub frame_counter: u32,
    /// Display memory, dword-interleaved across the four planes.
    pub vram: Box<[u8]>,
}

impl Default for Vga {
    fn default() -> Self {
        Self::new()
    }
}

impl Vga {
    /// Creates the adapter in its power-on state.
    ///
    /// The miscellaneous output register resets to zero, so the CRTC decodes
    /// at the monochrome ports until the VGA BIOS selects color decode. The
    /// graphics controller miscellaneous register resets to memory map 3
    /// (32 KiB at 0xB8000) and the DAC mask to 0xFF.
    pub fn new() -> Self {
        let mut gc = [0; GC_REGISTER_COUNT];
        gc[6] = 0x0C;
        let mut crtc = [0; CRTC_REGISTER_COUNT];
        crtc[0x07] = 0x10;
        crtc[0x09] = 0x40;
        crtc[0x18] = 0xFF;
        Self {
            misc_output: 0,
            feature_control: 0,
            seq_index: 0,
            seq: [0; SEQ_REGISTER_COUNT],
            crtc_index: 0,
            crtc,
            gc_index: 0,
            gc,
            atc_index: 0,
            atc_data_phase: false,
            atc: [0; ATC_REGISTER_COUNT],
            dac_mask: 0xFF,
            dac_write_index: 0,
            dac_read_index: 0,
            dac_cycle: 0,
            dac_read_mode: false,
            dac_write_latch: [0; 3],
            dac_hidden_counter: 0,
            dac_hidden_control: 0,
            dac: [[0; 3]; 256],
            latches: [0; 4],
            segment_select: 0,
            key_prefix_armed: false,
            key_unlocked: false,
            hercules_compat: 0,
            mode_control: 0,
            vretrace_interrupt_latch: false,
            frame_counter: 0,
            vram: vec![0; VGA_VRAM_SIZE].into_boxed_slice(),
        }
    }

    /// Returns the display memory slice for the renderer.
    pub fn vram(&self) -> &[u8] {
        &self.vram
    }

    /// Whether the color I/O decode is selected (misc output bit 0).
    pub fn color_decode(&self) -> bool {
        self.misc_output & 0x01 != 0
    }
}
