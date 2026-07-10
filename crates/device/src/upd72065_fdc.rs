//! NEC uPD72065 Floppy Disk Controller as wired on the Sharp X68000.
//!
//! The uPD72065 executes the uPD765A command set and adds an auxiliary
//! command register written at the main-status address, a standby state,
//! SCAN command execution, and a 255-pulse RECALIBRATE step limit. The
//! command engine is the shared [`Upd765aFdc`]; this wrapper configures it
//! and models the auxiliary register.

use std::ops::{Deref, DerefMut};

use crate::upd765a_fdc::{FdcAction, Upd765aFdc};

/// Auxiliary command: software reset.
const AUXILIARY_RESET: u8 = 0x36;

/// Auxiliary command: enter standby.
const AUXILIARY_SET_STANDBY: u8 = 0x35;

/// Auxiliary command: leave standby.
const AUXILIARY_RESET_STANDBY: u8 = 0x34;

/// NEC uPD72065 FDC: the uPD765A command engine with X68000 configuration.
pub struct Upd72065Fdc {
    /// Embedded uPD765A command engine.
    core: Upd765aFdc,
    /// Whether the controller clock is gated off by SET STANDBY.
    standby: bool,
}

impl Deref for Upd72065Fdc {
    type Target = Upd765aFdc;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl DerefMut for Upd72065Fdc {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl Default for Upd72065Fdc {
    fn default() -> Self {
        Self::new()
    }
}

impl Upd72065Fdc {
    /// Creates a uPD72065 with SCAN support and the 255-track step limit.
    pub fn new() -> Self {
        let mut core = Upd765aFdc::new();
        core.state.scan_enabled = true;
        core.state.recalibrate_step_limit = 255;
        core.state.report_two_side = false;
        core.state.sense_reports_command_head = false;
        Self {
            core,
            standby: false,
        }
    }

    /// Reads the main status register.
    pub fn read_status(&self) -> u8 {
        self.core.read_status()
    }

    /// Writes the auxiliary command register at the main-status address.
    /// Only RESET, SET STANDBY, and RESET STANDBY are documented; other
    /// values are ignored.
    pub fn write_auxiliary_command(&mut self, value: u8) {
        match value {
            AUXILIARY_RESET => {
                // Software reset through the shared control-register edge:
                // head positions survive, the state machine returns to idle.
                self.core.write_control(0x80);
                self.core.write_control(0x00);
            }
            AUXILIARY_SET_STANDBY => self.standby = true,
            AUXILIARY_RESET_STANDBY => self.standby = false,
            _ => {}
        }
    }

    /// Returns whether the controller is in standby.
    pub fn standby(&self) -> bool {
        self.standby
    }

    /// Reads the data register.
    pub fn read_data(&mut self) -> u8 {
        self.core.read_data()
    }

    /// Writes the data register; command bytes are ignored during standby.
    pub fn write_data(&mut self, value: u8) -> FdcAction {
        if self.standby {
            return FdcAction::None;
        }
        self.core.write_data(value)
    }

    /// Updates which drives report ready (inserted media with the motor on).
    pub fn set_drive_ready_mask(&mut self, mask: u8) {
        self.core.state.drive_has_disk = mask & 0x0F;
    }

    /// Updates which drives report write-protected media.
    pub fn set_drive_write_protected_mask(&mut self, mask: u8) {
        self.core.state.drive_write_protected = mask & 0x0F;
    }

    /// Returns the embedded uPD765A command engine.
    pub fn core(&self) -> &Upd765aFdc {
        &self.core
    }

    /// Returns the embedded uPD765A command engine mutably.
    pub fn core_mut(&mut self) -> &mut Upd765aFdc {
        &mut self.core
    }
}
