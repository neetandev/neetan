//! Centronics printer interface (i8255 PPI at ports 0x40/0x42/0x44/0x46).
//!
//! Port A (0x40, R/W): printer data latch.
//! Port C (0x44, R/W): printer control - bit 7 is PSTB# (active-high strobe).
//! Control (0x46, W): i8255 mode set / BSR register.
//!
//! Ref: undoc98 `io_prn.txt`.

use std::io::Write;

save_state::runtime_state! {
/// Authoritative printer electronics state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterState {
    /// Last byte written to port 0x40 (data latch).
    pub data: u8,
    /// Port C register - bit 7 is PSTB# strobe signal.
    pub port_c: u8,
    /// Whether a printer output is attached.
    pub attached: bool,
}}

/// Centronics printer device.
pub struct Printer {
    /// Embedded state for save/restore.
    pub state: PrinterState,
    output: Option<std::fs::File>,
}

impl Default for Printer {
    fn default() -> Self {
        Self::new()
    }
}

impl Printer {
    /// Creates a new printer with no output attached.
    pub fn new() -> Self {
        Self {
            state: PrinterState {
                data: 0x00,
                port_c: 0x00,
                attached: false,
            },
            output: None,
        }
    }

    /// Captures the printer electronics state.
    pub fn capture_state(&self) -> PrinterState {
        self.state.clone()
    }

    /// Restores electronics while retaining the output file resource.
    pub fn restore_state(
        &mut self,
        state: PrinterState,
    ) -> Result<(), save_state::StateValidationError> {
        if state.attached != self.output.is_some() {
            return Err(save_state::StateValidationError::new(
                "printer attachment differs",
            ));
        }
        self.state = state;
        Ok(())
    }

    /// Attaches a file handle for printer output.
    pub fn attach(&mut self, file: std::fs::File) {
        self.output = Some(file);
        self.state.attached = true;
    }

    /// Returns `true` if the printer port reports ready.
    ///
    /// On real PC-98 hardware, the BUSY# line is high (not busy) when no
    /// printer is connected, so the port always reports ready.
    pub fn is_ready(&self) -> bool {
        true
    }

    /// Writes the data latch (port 0x40 write).
    pub fn write_data(&mut self, value: u8) {
        self.state.data = value;
    }

    /// Reads the data latch (port 0x40 read).
    pub fn read_data(&self) -> u8 {
        self.state.data
    }

    /// Writes port C (port 0x44 write).
    ///
    /// Detects rising edge on bit 7 (PSTB#): when old=0 and new=1,
    /// the latched data byte is written to the output file.
    pub fn write_port_c(&mut self, value: u8) {
        let old = self.state.port_c;
        self.state.port_c = value;
        self.check_strobe(old, value);
    }

    /// Reads port C (port 0x44 read).
    pub fn read_port_c(&self) -> u8 {
        self.state.port_c
    }

    /// Writes the i8255 control register (port 0x46 write).
    ///
    /// Bit 7=1: mode set command - resets port C to 0.
    /// Bit 7=0: BSR (bit set/reset) on port C.
    pub fn write_control(&mut self, value: u8) {
        let old = self.state.port_c;
        if value & 0x80 != 0 {
            self.state.port_c = 0x00;
        } else {
            let bit = 1u8 << ((value >> 1) & 0x07);
            if value & 1 != 0 {
                self.state.port_c |= bit;
            } else {
                self.state.port_c &= !bit;
            }
        }
        self.check_strobe(old, self.state.port_c);
    }

    /// Flushes the output file if attached.
    pub fn flush(&mut self) {
        if let Some(ref mut file) = self.output {
            let _ = file.flush();
        }
    }

    /// Detects rising edge on bit 7 and outputs the data byte.
    fn check_strobe(&mut self, old: u8, new: u8) {
        if (old & 0x80 == 0)
            && (new & 0x80 != 0)
            && let Some(ref mut file) = self.output
        {
            let _ = file.write_all(&[self.state.data]);
        }
    }
}
