//! Sony S1985 switched-I/O functions.

/// S1985 switched-I/O device identifier.
pub(super) const S1985_DEVICE_ID: u8 = 0xFE;
/// Number of bytes in the S1985 battery-backed RAM.
const BACKUP_RAM_SIZE: usize = 16;
/// Initial erased value of S1985 battery-backed RAM.
const BACKUP_RAM_ERASED: u8 = 0xFF;

/// Sony S1985 system-controller state.
pub(super) struct S1985 {
    backup_ram: [u8; BACKUP_RAM_SIZE],
    backup_address: u8,
    first_color: u8,
    second_color: u8,
    pattern: u8,
}

save_state::runtime_state! {
/// Mutable Sony S1985 controller state.
#[derive(Clone)]
pub(super) struct S1985State {
    backup_ram: [u8; BACKUP_RAM_SIZE],
    backup_address: u8,
    first_color: u8,
    second_color: u8,
    pattern: u8,
}}

impl S1985 {
    /// Creates an S1985 with erased battery-backed RAM.
    pub(super) const fn new() -> Self {
        let mut controller = Self {
            backup_ram: [BACKUP_RAM_ERASED; BACKUP_RAM_SIZE],
            backup_address: 0,
            first_color: 0,
            second_color: 0,
            pattern: 0,
        };
        controller.reset();
        controller
    }

    /// Resets volatile controller state without clearing backup RAM.
    const fn reset(&mut self) {
        self.backup_address = 0;
        self.first_color = 0;
        self.second_color = 0;
        self.pattern = 0;
    }

    /// Reads one selected switched-I/O register.
    pub(super) fn read(&mut self, offset: u8) -> u8 {
        match offset & 0x0F {
            0 => !S1985_DEVICE_ID,
            2 => self.backup_ram[usize::from(self.backup_address)],
            7 => {
                let value = if self.pattern & 0x80 != 0 {
                    self.second_color
                } else {
                    self.first_color
                };
                self.pattern = self.pattern.rotate_left(1);
                value
            }
            _ => 0xFF,
        }
    }

    /// Writes one selected switched-I/O register.
    pub(super) fn write(&mut self, offset: u8, value: u8) {
        match offset & 0x0F {
            1 => self.backup_address = value & 0x0F,
            2 => self.backup_ram[usize::from(self.backup_address)] = value,
            6 => {
                self.second_color = self.first_color;
                self.first_color = value;
            }
            7 => self.pattern = value,
            _ => {}
        }
    }

    /// Captures backup RAM and pattern registers.
    pub(super) const fn capture_state(&self) -> S1985State {
        S1985State {
            backup_ram: self.backup_ram,
            backup_address: self.backup_address,
            first_color: self.first_color,
            second_color: self.second_color,
            pattern: self.pattern,
        }
    }

    /// Restores backup RAM and pattern registers.
    pub(super) fn restore_state(
        &mut self,
        state: S1985State,
    ) -> Result<(), save_state::StateValidationError> {
        if state.backup_address as usize >= BACKUP_RAM_SIZE {
            return Err(save_state::StateValidationError::new(
                "S1985 backup address is invalid",
            ));
        }
        self.backup_ram = state.backup_ram;
        self.backup_address = state.backup_address;
        self.first_color = state.first_color;
        self.second_color = state.second_color;
        self.pattern = state.pattern;
        Ok(())
    }
}

impl Default for S1985 {
    /// Creates an S1985 with erased battery-backed RAM.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_backup_ram_and_unhandled_registers_read_back() {
        let mut controller = S1985::new();
        assert_eq!(controller.read(0), 0x01);
        assert_eq!(controller.read(2), 0xFF);
        assert_eq!(controller.read(3), 0xFF);
        controller.write(1, 0x13);
        controller.write(2, 0xA5);
        controller.write(1, 0x03);
        assert_eq!(controller.read(2), 0xA5);
    }

    #[test]
    fn pattern_reads_select_colors_and_rotate() {
        let mut controller = S1985::new();
        controller.write(6, 0x12);
        controller.write(6, 0x34);
        controller.write(7, 0x80);
        assert_eq!(controller.read(7), 0x12);
        assert_eq!(controller.read(7), 0x34);
    }

    #[test]
    fn reset_preserves_backup_ram_only() {
        let mut controller = S1985::new();
        controller.write(1, 4);
        controller.write(2, 0x5A);
        controller.write(6, 0x22);
        controller.write(7, 0x80);
        controller.reset();
        controller.write(1, 4);
        assert_eq!(controller.read(2), 0x5A);
        assert_eq!(controller.read(7), 0);
    }
}
