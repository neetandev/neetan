//! INT 17h printer services and INT 05h print screen.
//!
//! The machine has no parallel port hardware: the equipment word reports
//! zero printers and every BDA LPT base word at 40:08 is 0. The real AMI
//! BIOS (probed on the same LPT-less ct486 machine) exits INT 17h without
//! touching any register when the port base is 0, and INT 05h leaves 0xFF
//! (error, no printer) in the print screen status byte at 50:00.

use common::TraceSink;

use super::AtBus;

/// Print screen status byte at 50:00.
const PRINT_SCREEN_STATUS: u32 = 0x500;
/// Print screen status: the last print screen failed (no printer).
const PRINT_SCREEN_FAILED: u8 = 0xFF;

impl<T: TraceSink> AtBus<T> {
    /// INT 17h printer services: every BDA LPT base word is 0 on this
    /// machine, so every function returns with all registers and flags
    /// untouched, matching the probed real AMI BIOS early exit.
    pub(super) fn hle_int17h(&mut self) {}

    /// INT 05h print screen: no printer is attached, so the screen dump
    /// fails immediately with the error status at 50:00, matching the
    /// probed real AMI BIOS.
    pub(super) fn hle_int05h(&mut self) {
        self.write_mem_byte(PRINT_SCREEN_STATUS, PRINT_SCREEN_FAILED);
    }
}
