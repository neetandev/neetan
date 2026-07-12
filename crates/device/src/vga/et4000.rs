//! ET4000AX segment select banking helpers.

use super::{CRTC_INDEX_VSCONF1, Vga};

/// Size of one display memory segment selected by the segment pointers.
const SEGMENT_SIZE: u32 = 0x1_0000;

impl Vga {
    /// Whether the segment pointers apply (video system configuration 1
    /// bit 4 disables them).
    pub(super) fn banking_enabled(&self) -> bool {
        self.crtc[usize::from(CRTC_INDEX_VSCONF1)] & 0x10 == 0
    }

    /// Display memory offset added to CPU writes by the write segment pointer.
    pub(super) fn write_bank_offset(&self) -> u32 {
        if self.banking_enabled() {
            u32::from(self.segment_select & 0x0F) * SEGMENT_SIZE
        } else {
            0
        }
    }

    /// Display memory offset added to CPU reads by the read segment pointer.
    pub(super) fn read_bank_offset(&self) -> u32 {
        if self.banking_enabled() {
            u32::from(self.segment_select >> 4) * SEGMENT_SIZE
        } else {
            0
        }
    }
}
