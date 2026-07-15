use super::{CTRL_RESET, UPD765_PLATFORM_X68K, Upd765aFdc};

/// X68000 auxiliary command that resets the command engine.
const AUXILIARY_RESET: u8 = 0x36;
/// X68000 auxiliary command that enters standby.
const AUXILIARY_SET_STANDBY: u8 = 0x35;
/// X68000 auxiliary command that leaves standby.
const AUXILIARY_RESET_STANDBY: u8 = 0x34;

impl Upd765aFdc<UPD765_PLATFORM_X68K> {
    /// Writes the X68000 auxiliary command register.
    pub fn write_auxiliary_command(&mut self, value: u8) {
        match value {
            AUXILIARY_RESET => {
                self.write_control(CTRL_RESET);
                self.write_control(0);
            }
            AUXILIARY_SET_STANDBY => self.standby = true,
            AUXILIARY_RESET_STANDBY => self.standby = false,
            _ => {}
        }
    }

    /// Returns whether the X68000 controller is in standby.
    pub fn standby(&self) -> bool {
        self.standby
    }

    /// Updates which X68000 drives report ready.
    pub fn set_drive_ready_mask(&mut self, mask: u8) {
        self.state.drive_has_disk = mask & 0x0F;
    }

    /// Updates which X68000 drives report write-protected media.
    pub fn set_drive_write_protected_mask(&mut self, mask: u8) {
        self.state.drive_write_protected = mask & 0x0F;
    }
}
