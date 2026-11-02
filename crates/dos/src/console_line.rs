//! Cooked line input of `INT 21h AH=3Fh` from CON.
//!
//! A read from a handle on the standard input device reads one line with
//! echo and backspace editing like real DOS in cooked mode. Enter ends the
//! line. The line and CR LF go to a pending buffer. Each read takes up to
//! `CX` bytes from it. The next read waits for a new line only when the
//! buffer is empty. A handle in raw mode (device information bit 5) reads
//! `CX` keys without echo and without line editing.

use crate::{CpuAccess, MemoryAccess, NeetanDos, SegmentRegister, tables};

/// Longest line without the CR LF.
const LINE_LIMIT: usize = 127;

/// Carriage return.
const CARRIAGE_RETURN: u8 = 0x0D;

/// Line feed.
const LINE_FEED: u8 = 0x0A;

/// Backspace.
const BACKSPACE: u8 = 0x08;

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
                match byte {
                    CARRIAGE_RETURN => {
                        self.console_output().write_byte(memory, CARRIAGE_RETURN);
                        self.console_output().write_byte(memory, LINE_FEED);
                        let line = std::mem::take(&mut self.state.console_line.edited);
                        let pending = &mut self.state.console_line.pending;
                        pending.extend(line);
                        pending.extend([CARRIAGE_RETURN, LINE_FEED]);
                        break;
                    }
                    BACKSPACE => {
                        if self.state.console_line.edited.pop().is_some() {
                            for echo in [BACKSPACE, b' ', BACKSPACE] {
                                self.console_output().write_byte(memory, echo);
                            }
                        }
                    }
                    _ => {
                        if self.state.console_line.edited.len() < LINE_LIMIT {
                            self.state.console_line.edited.push(byte);
                            self.console_output().write_byte(memory, byte);
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
