//! Frame timing derived from the CRTC register file and the clock generator.

use super::{CRTC_INDEX_AUX_CONTROL, CRTC_INDEX_OVERFLOW_HIGH, Vga};
use crate::vga::io::RetraceStatus;

/// ICS2494AN-304 clock generator output frequencies, indexed by the ET4000AX
/// clock select (misc output bits 2-3, CRTC 0x34 bit 1, CRTC 0x31 bit 6).
const ICS2494AN_304_FREQUENCIES_HZ: [u32; 16] = [
    50_350_000, 56_644_000, 65_000_000, 72_000_000, 80_000_000, 89_800_000, 63_000_000, 75_000_000,
    25_175_000, 28_322_000, 31_500_000, 36_000_000, 40_000_000, 44_900_000, 50_000_000, 65_000_000,
];

/// Horizontal total below which the CRTC is treated as unprogrammed.
const MIN_PROGRAMMED_HTOTAL: u8 = 0x10;
/// Vertical total below which the CRTC is treated as unprogrammed.
const MIN_PROGRAMMED_VTOTAL: u32 = 0x40;

/// Standard 70 Hz 720x400 text timing used while the CRTC is unprogrammed.
const FALLBACK_TIMING: VgaFrameTiming = VgaFrameTiming {
    dot_clock_hz: 28_322_000,
    dots_per_scanline: 900,
    total_scanlines: 449,
    active_dots: 720,
    active_scanlines: 400,
    vsync_start_scanline: 412,
    vsync_scanlines: 2,
};

/// One frame of scan timing in dot clock units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VgaFrameTiming {
    /// Dot clock frequency in hertz.
    pub dot_clock_hz: u32,
    /// Total dots per scanline including blanking.
    pub dots_per_scanline: u32,
    /// Total scanlines per frame including blanking.
    pub total_scanlines: u32,
    /// Dots of active display per scanline.
    pub active_dots: u32,
    /// Scanlines of active display per frame.
    pub active_scanlines: u32,
    /// First scanline of vertical retrace.
    pub vsync_start_scanline: u32,
    /// Number of scanlines vertical retrace stays asserted.
    pub vsync_scanlines: u32,
}

impl VgaFrameTiming {
    /// Length of one frame in CPU cycles (at least one).
    pub fn frame_cycles(&self, cpu_clock_hz: u32) -> u64 {
        let dots_per_frame = u64::from(self.dots_per_scanline) * u64::from(self.total_scanlines);
        (u64::from(cpu_clock_hz) * dots_per_frame / u64::from(self.dot_clock_hz)).max(1)
    }

    /// Retrace state at the given CPU cycle offset from the frame start.
    ///
    /// The frame is anchored at the start of vertical retrace, matching the
    /// scheduler event that drives rendering.
    pub fn retrace_status(&self, cycles_into_frame: u64, cpu_clock_hz: u32) -> RetraceStatus {
        let dots =
            cycles_into_frame * u64::from(self.dot_clock_hz) / u64::from(cpu_clock_hz).max(1);
        let scanline_offset = (dots / u64::from(self.dots_per_scanline)) as u32;
        let dot_in_scanline = (dots % u64::from(self.dots_per_scanline)) as u32;
        let scanline = (self.vsync_start_scanline + scanline_offset % self.total_scanlines)
            % self.total_scanlines;
        let vertical_retrace = scanline >= self.vsync_start_scanline
            && scanline < self.vsync_start_scanline + self.vsync_scanlines;
        let display_disabled =
            scanline >= self.active_scanlines || dot_in_scanline >= self.active_dots;
        RetraceStatus {
            display_disabled,
            vertical_retrace,
        }
    }
}

impl Vga {
    /// Derives the frame timing from the CRTC, sequencer and clock select.
    ///
    /// Falls back to standard 70 Hz text timing while the CRTC still holds
    /// its unprogrammed power-on values, so the frame event keeps a sane
    /// period from reset onward.
    pub fn frame_timing(&self) -> VgaFrameTiming {
        let vertical_total = self.vertical_field(0x06, 0x01, 0x20, 0x02) + 2;
        if self.crtc[0x00] < MIN_PROGRAMMED_HTOTAL || vertical_total < MIN_PROGRAMMED_VTOTAL {
            return FALLBACK_TIMING;
        }

        let character_width: u32 = if self.seq[1] & 0x01 != 0 { 8 } else { 9 };
        let character_dots = if self.seq[1] & 0x08 != 0 {
            character_width * 2
        } else {
            character_width
        };
        let horizontal_total_characters = u32::from(self.crtc[0x00]) + 5;
        let horizontal_active_characters = u32::from(self.crtc[0x01]) + 1;

        let active_scanlines = self.vertical_field(0x12, 0x02, 0x40, 0x04) + 1;
        let vsync_start_scanline = self.vertical_field(0x10, 0x04, 0x80, 0x08);
        let vsync_end_low = u32::from(self.crtc[0x11] & 0x0F);
        let mut vsync_scanlines = vsync_end_low.wrapping_sub(vsync_start_scanline) & 0x0F;
        if vsync_scanlines == 0 {
            vsync_scanlines = 16;
        }

        VgaFrameTiming {
            dot_clock_hz: self.dot_clock_hz(),
            dots_per_scanline: horizontal_total_characters * character_dots,
            total_scanlines: vertical_total,
            active_dots: horizontal_active_characters * character_dots,
            active_scanlines,
            vsync_start_scanline,
            vsync_scanlines,
        }
    }

    /// Composes a 10/11-bit vertical CRTC value from its low register and the
    /// overflow bits (CRTC 0x07 and the ET4000 overflow high register).
    fn vertical_field(
        &self,
        low_index: usize,
        overflow_bit8: u8,
        overflow_bit9: u8,
        overflow_high_bit10: u8,
    ) -> u32 {
        let mut value = u32::from(self.crtc[low_index]);
        if self.crtc[0x07] & overflow_bit8 != 0 {
            value |= 0x100;
        }
        if self.crtc[0x07] & overflow_bit9 != 0 {
            value |= 0x200;
        }
        if self.crtc[usize::from(CRTC_INDEX_OVERFLOW_HIGH)] & overflow_high_bit10 != 0 {
            value |= 0x400;
        }
        value
    }

    /// Dot clock frequency from the clock select and the MCLK dividers.
    fn dot_clock_hz(&self) -> u32 {
        let select = usize::from(
            (self.misc_output >> 2) & 0x03
                | (self.crtc[usize::from(CRTC_INDEX_AUX_CONTROL)] & 0x02) << 1
                | (self.crtc[0x31] & 0x40) >> 3,
        );
        let mut frequency = ICS2494AN_304_FREQUENCIES_HZ[select];
        if self.seq[7] & 0x01 != 0 {
            frequency /= 4;
        } else if self.seq[7] & 0x40 != 0 {
            frequency /= 2;
        }
        frequency
    }
}
