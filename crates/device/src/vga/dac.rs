//! DAC palette port state machine, including the ET4000 hidden control
//! register reached by four consecutive reads of the pixel mask port.

use super::Vga;

/// Consecutive pixel mask reads needed to reach the hidden DAC register.
const HIDDEN_DAC_UNLOCK_READS: u8 = 4;

impl Vga {
    /// Reads the pixel mask port; the fifth consecutive read returns the
    /// hidden DAC control register.
    pub(super) fn dac_mask_read(&mut self) -> u8 {
        if self.dac_hidden_counter >= HIDDEN_DAC_UNLOCK_READS {
            self.dac_hidden_counter = 0;
            return self.dac_hidden_control;
        }
        self.dac_hidden_counter += 1;
        self.dac_mask
    }

    /// Writes the pixel mask port; after four consecutive mask reads the
    /// write lands in the hidden DAC control register instead.
    pub(super) fn dac_mask_write(&mut self, value: u8) {
        if self.dac_hidden_counter >= HIDDEN_DAC_UNLOCK_READS {
            self.dac_hidden_control = value;
        } else {
            self.dac_mask = value;
        }
        self.dac_hidden_counter = 0;
    }

    /// Reads the DAC state register (3 during a read cycle, 0 otherwise).
    pub(super) fn dac_state_read(&mut self) -> u8 {
        self.dac_hidden_counter = 0;
        if self.dac_read_mode { 0x03 } else { 0x00 }
    }

    /// Starts a DAC read cycle at the given palette index.
    pub(super) fn dac_read_index_write(&mut self, value: u8) {
        self.dac_read_index = value;
        self.dac_cycle = 0;
        self.dac_read_mode = true;
        self.dac_hidden_counter = 0;
    }

    /// Reads back the write index register.
    pub(super) fn dac_write_index_read(&mut self) -> u8 {
        self.dac_hidden_counter = 0;
        self.dac_write_index
    }

    /// Starts a DAC write cycle at the given palette index.
    pub(super) fn dac_write_index_write(&mut self, value: u8) {
        self.dac_write_index = value;
        self.dac_cycle = 0;
        self.dac_read_mode = false;
        self.dac_hidden_counter = 0;
    }

    /// Writes one 6-bit component; the third write commits the palette entry
    /// and advances the write index.
    pub(super) fn dac_data_write(&mut self, value: u8) {
        self.dac_hidden_counter = 0;
        self.dac_write_latch[usize::from(self.dac_cycle)] = value & 0x3F;
        self.dac_cycle += 1;
        if self.dac_cycle == 3 {
            self.dac[usize::from(self.dac_write_index)] = self.dac_write_latch;
            self.dac_write_index = self.dac_write_index.wrapping_add(1);
            self.dac_cycle = 0;
        }
    }

    /// Reads one 6-bit component; the third read advances the read index.
    pub(super) fn dac_data_read(&mut self) -> u8 {
        self.dac_hidden_counter = 0;
        let component = self.dac[usize::from(self.dac_read_index)][usize::from(self.dac_cycle)];
        self.dac_cycle += 1;
        if self.dac_cycle == 3 {
            self.dac_read_index = self.dac_read_index.wrapping_add(1);
            self.dac_cycle = 0;
        }
        component
    }
}

/// Expands a 6-bit DAC component to 8 bits.
pub(super) fn expand_6bit_component(component: u8) -> u8 {
    (component << 2) | (component >> 4)
}
