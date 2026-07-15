//! PC-88VA CGROM memory wiring.

use super::Pc88VaBus;

impl<T: common::TraceSink> Pc88VaBus<T> {
    fn cgrom_font8(&self) -> bool {
        self.video.txtmode & 0x04 != 0
    }

    /// Reads the CGROM data window.
    pub(crate) fn read_cgrom_data(&self) -> u8 {
        self.cgrom.read_data(
            self.memory.font_rom(),
            self.memory.backup_ram(),
            self.cgrom_font8(),
        )
    }

    /// Writes the writable CGROM data window.
    pub(crate) fn write_cgrom_data(&mut self, value: u8) {
        let font8 = self.cgrom_font8();
        self.cgrom
            .write_data(value, self.memory.backup_ram_mut(), font8);
    }
}
