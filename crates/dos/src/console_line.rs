//! Line editing of the console input and the cooked line input of
//! `INT 21h AH=3Fh` from CON.
//!
//! A read from a handle on the standard input device reads one line with
//! echo and backspace editing like real DOS in cooked mode. A control byte
//! of `echoes_as_caret` echoes as `^` and its letter. Enter ends the line.
//! The line and CR LF go to a pending buffer. Each read takes up to `CX`
//! bytes from it. The next read waits for a new line only when the buffer is
//! empty. A handle in raw mode (device information bit 5) reads `CX` keys
//! without echo and without line editing.

use crate::{CpuAccess, MemoryAccess, NeetanDos, SegmentRegister, tables};

/// Longest line without the CR LF.
const LINE_LIMIT: usize = 127;

/// Carriage return.
const CARRIAGE_RETURN: u8 = 0x0D;

/// Line feed.
const LINE_FEED: u8 = 0x0A;

/// Backspace.
const BACKSPACE: u8 = 0x08;

/// Tab.
const TAB: u8 = 0x09;

/// Ctrl+F. The line editors drop it.
const CONTROL_F: u8 = 0x06;

/// Ctrl+U. The line editors erase the whole line on it.
const CONTROL_U: u8 = 0x15;

/// Caret of the echo of a control byte.
const CARET: u8 = b'^';

/// Distance from a control byte to the letter of its caret echo.
const CARET_LETTER_OFFSET: u8 = 0x40;

/// Returns true when the line editors echo `byte` as a caret and the letter
/// `byte + 40h` in two cells. The buffer keeps the byte itself.
pub(crate) const fn echoes_as_caret(byte: u8) -> bool {
    matches!(
        byte,
        0x01 | 0x02 | 0x04 | 0x05 | 0x07 | 0x0B | 0x0C | 0x1A | 0x1E
    )
}

/// Line editor action of one input byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKey {
    /// Enter ends the line.
    Enter,
    /// Line feed: echo CR LF, the line keeps nothing.
    LineFeed,
    /// Backspace erases the last byte.
    Backspace,
    /// Ctrl+U erases the whole line.
    EraseLine,
    /// The line editor drops the byte.
    Drop,
    /// The line keeps the byte.
    Store(u8),
}

impl LineKey {
    /// Returns the line editor action of `byte`.
    pub(crate) const fn of(byte: u8) -> Self {
        match byte {
            CARRIAGE_RETURN => Self::Enter,
            LINE_FEED => Self::LineFeed,
            BACKSPACE => Self::Backspace,
            CONTROL_U => Self::EraseLine,
            CONTROL_F => Self::Drop,
            _ => Self::Store(byte),
        }
    }
}

/// Returns true when the device information word names the standard input
/// device.
pub(crate) const fn is_console_input(device_info: u16) -> bool {
    device_info & tables::SFT_DEVINFO_CHAR != 0 && device_info & tables::SFT_DEVINFO_STDIN != 0
}

impl NeetanDos {
    /// `INT 21h AH=3Fh` from CON: returns up to `CX` bytes of the pending
    /// line in `AX`, after it reads a new line when none is pending. Waits
    /// for keys through the console backend.
    pub(crate) fn read_console_line(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let count = usize::from(cpu.cx());
        if count > 0 && self.state.console_line.pending.is_empty() {
            loop {
                let Some(byte) = self.read_input_byte(memory) else {
                    self.wait_for_console_input(cpu, memory);
                    return;
                };
                match LineKey::of(byte) {
                    LineKey::Enter => {
                        self.echo_new_line(memory);
                        let line = std::mem::take(&mut self.state.console_line.edited);
                        let pending = &mut self.state.console_line.pending;
                        pending.extend(line);
                        pending.extend([CARRIAGE_RETURN, LINE_FEED]);
                        break;
                    }
                    LineKey::LineFeed => self.echo_new_line(memory),
                    LineKey::Backspace => {
                        if let Some(erased) = self.state.console_line.edited.pop() {
                            self.erase_line_byte(memory, erased);
                        }
                    }
                    LineKey::EraseLine => {
                        while let Some(erased) = self.state.console_line.edited.pop() {
                            self.erase_line_byte(memory, erased);
                        }
                    }
                    LineKey::Drop => {}
                    LineKey::Store(byte) => {
                        if self.state.console_line.edited.len() < LINE_LIMIT {
                            self.state.console_line.edited.push(byte);
                            self.echo_line_byte(memory, byte);
                        }
                    }
                }
            }
        }

        let buffer = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        let pending = &mut self.state.console_line.pending;
        let taken: Vec<u8> = pending.drain(..count.min(pending.len())).collect();
        memory.write_block(buffer, &taken);
        cpu.set_ax(taken.len() as u16);
        self.state.set_iret_carry(cpu, memory, false);
    }
}

impl NeetanDos {
    /// Echoes one byte of a DOS kernel input function on the console. The
    /// text VRAM console gets a tab as spaces up to the next tab stop.
    pub(crate) fn echo_kernel_byte(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        if byte == TAB && self.console_device.is_text_vram() {
            for _ in 0..self.console.kernel_tab_spaces(memory) {
                self.console_output().write_byte(memory, b' ');
            }
        } else {
            self.console_output().write_byte(memory, byte);
        }
    }

    /// Echoes CR LF.
    pub(crate) fn echo_new_line(&mut self, memory: &mut dyn MemoryAccess) {
        self.console_output().write_byte(memory, CARRIAGE_RETURN);
        self.console_output().write_byte(memory, LINE_FEED);
    }

    /// Echoes a byte that a line editor keeps: a caret byte as `^` and its
    /// letter, every other byte as itself.
    pub(crate) fn echo_line_byte(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        if echoes_as_caret(byte) {
            self.console_output().write_byte(memory, CARET);
            self.console_output()
                .write_byte(memory, byte + CARET_LETTER_OFFSET);
        } else {
            self.echo_kernel_byte(memory, byte);
        }
    }

    /// Erases the echo of a byte that a line editor drops from its line with
    /// BS, space, BS for each cell of the echo.
    pub(crate) fn erase_line_byte(&mut self, memory: &mut dyn MemoryAccess, byte: u8) {
        let cells = if echoes_as_caret(byte) { 2 } else { 1 };
        for _ in 0..cells {
            for echo in [BACKSPACE, b' ', BACKSPACE] {
                self.console_output().write_byte(memory, echo);
            }
        }
    }

    /// `INT 21h AH=3Fh` from CON in raw mode: waits for `CX` keys and
    /// returns them without echo and without line editing.
    pub(crate) fn read_console_raw(
        &mut self,
        cpu: &mut dyn CpuAccess,
        memory: &mut dyn MemoryAccess,
    ) {
        let count = usize::from(cpu.cx());
        while self.state.console_line.raw.len() < count {
            let Some(byte) = self.read_input_byte(memory) else {
                self.wait_for_console_input(cpu, memory);
                return;
            };
            self.state.console_line.raw.push(byte);
        }
        let taken = std::mem::take(&mut self.state.console_line.raw);
        let buffer = cpu.linear_address(SegmentRegister::DS, cpu.dx());
        memory.write_block(buffer, &taken);
        cpu.set_ax(taken.len() as u16);
        self.state.set_iret_carry(cpu, memory, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_input_needs_a_character_device_with_the_stdin_bit() {
        assert!(is_console_input(
            tables::SFT_DEVINFO_CHAR | tables::SFT_DEVINFO_STDIN | tables::SFT_DEVINFO_STDOUT
        ));
        assert!(!is_console_input(
            tables::SFT_DEVINFO_CHAR | tables::SFT_DEVINFO_STDOUT
        ));
        assert!(!is_console_input(tables::SFT_DEVINFO_STDIN));
    }
}
