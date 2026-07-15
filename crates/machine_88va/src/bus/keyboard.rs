//! PC-88VA keyboard bus and interrupt wiring.

use super::Pc88VaBus;

impl<T: common::TraceSink> Pc88VaBus<T> {
    /// Reports one host key event and asserts keyboard IRQ 1.
    pub(crate) fn push_key_scancode(&mut self, code: u8) {
        self.keyboard.push_scancode(code);
        self.pic.set_irq(1);
    }

    /// Reads the keyboard data port and synchronizes its IRQ line.
    pub(crate) fn read_keyboard_data(&mut self) -> u8 {
        let code = self.keyboard.read_data();
        if self.keyboard.has_data() {
            self.pic.set_irq(1);
        } else {
            self.pic.clear_irq(1);
        }
        code
    }
}
