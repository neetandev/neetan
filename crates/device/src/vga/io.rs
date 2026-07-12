//! I/O port decode for the VGA register file.

use super::{
    ATC_INDEX_ET4000_MISC, CRTC_INDEX_EXT_START, CRTC_INDEX_OVERFLOW_HIGH, CRTC_INDEX_VRETRACE_END,
    CRTC_LAST_UNPROTECTED_INDEX, CRTC_REGISTER_COUNT, GC_REGISTER_COUNT, KEY_COMPLETE_VALUE,
    KEY_PREFIX_VALUE, SEQ_INDEX_RESET, SEQ_INDEX_TS_AUX_MODE, SEQ_INDEX_TS_STATE,
    VGA_PORT_ATC_READ, VGA_PORT_ATC_WRITE, VGA_PORT_CRTC_DATA_COLOR, VGA_PORT_CRTC_DATA_MONO,
    VGA_PORT_CRTC_INDEX_COLOR, VGA_PORT_CRTC_INDEX_MONO, VGA_PORT_DAC_DATA, VGA_PORT_DAC_MASK,
    VGA_PORT_DAC_READ_INDEX, VGA_PORT_DAC_WRITE_INDEX, VGA_PORT_FEATURE_READ, VGA_PORT_GC_DATA,
    VGA_PORT_GC_INDEX, VGA_PORT_HERCULES_COMPAT, VGA_PORT_MISC_READ, VGA_PORT_MODE_CONTROL_COLOR,
    VGA_PORT_MODE_CONTROL_MONO, VGA_PORT_SEGMENT_SELECT, VGA_PORT_SEQ_DATA, VGA_PORT_SEQ_INDEX,
    VGA_PORT_STATUS_COLOR, VGA_PORT_STATUS_MONO, VGA_PORT_STATUS0_MISC_WRITE, Vga,
};

/// Live retrace state computed by the bus for input status one reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetraceStatus {
    /// The display enable signal is inactive (any horizontal or vertical
    /// blanking or border time).
    pub display_disabled: bool,
    /// Vertical retrace is in progress.
    pub vertical_retrace: bool,
}

/// Monitor sense threshold: the summed 6-bit DAC components of palette entry
/// zero above which the analog sense line reads low.
const MONITOR_SENSE_SUM_THRESHOLD: u16 = 0x4E;

impl Vga {
    /// Reads a VGA I/O port; `None` means the port does not decode.
    pub fn io_read(&mut self, port: u16, retrace: RetraceStatus) -> Option<u8> {
        match port {
            VGA_PORT_CRTC_INDEX_MONO | VGA_PORT_CRTC_INDEX_COLOR => {
                if self.crtc_port_active(port) {
                    Some(self.crtc_index)
                } else {
                    None
                }
            }
            VGA_PORT_CRTC_DATA_MONO | VGA_PORT_CRTC_DATA_COLOR => {
                if self.crtc_port_active(port) {
                    Some(self.crtc[usize::from(self.crtc_index) % CRTC_REGISTER_COUNT])
                } else {
                    None
                }
            }
            VGA_PORT_MODE_CONTROL_MONO | VGA_PORT_MODE_CONTROL_COLOR => {
                if self.crtc_port_active(port) {
                    // Bit 6 reads back the Hercules compatibility second-page
                    // enable per the ET4000 databook.
                    let second_page = (self.hercules_compat & 0x02) << 5;
                    Some((self.mode_control & !0x40) | second_page)
                } else {
                    None
                }
            }
            VGA_PORT_STATUS_MONO | VGA_PORT_STATUS_COLOR => {
                if self.crtc_port_active(port) {
                    Some(self.read_input_status_1(retrace))
                } else {
                    None
                }
            }
            VGA_PORT_HERCULES_COMPAT => None,
            VGA_PORT_ATC_WRITE => Some(self.atc_index),
            VGA_PORT_ATC_READ => Some(self.atc[usize::from(self.atc_index & 0x1F)]),
            VGA_PORT_STATUS0_MISC_WRITE => Some(self.read_input_status_0()),
            VGA_PORT_SEQ_INDEX => Some(self.seq_index),
            VGA_PORT_SEQ_DATA => Some(self.seq[usize::from(self.seq_index & 0x07)]),
            VGA_PORT_DAC_MASK => Some(self.dac_mask_read()),
            VGA_PORT_DAC_READ_INDEX => Some(self.dac_state_read()),
            VGA_PORT_DAC_WRITE_INDEX => Some(self.dac_write_index_read()),
            VGA_PORT_DAC_DATA => Some(self.dac_data_read()),
            VGA_PORT_FEATURE_READ => Some(self.feature_control),
            VGA_PORT_MISC_READ => Some(self.misc_output),
            VGA_PORT_SEGMENT_SELECT => {
                if self.key_unlocked {
                    Some(self.segment_select)
                } else {
                    None
                }
            }
            VGA_PORT_GC_INDEX => Some(self.gc_index),
            VGA_PORT_GC_DATA => {
                let index = usize::from(self.gc_index & 0x0F);
                if index < GC_REGISTER_COUNT {
                    Some(self.gc[index])
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Writes a VGA I/O port; writes to ports that do not decode are dropped.
    pub fn io_write(&mut self, port: u16, value: u8) {
        match port {
            VGA_PORT_CRTC_INDEX_MONO | VGA_PORT_CRTC_INDEX_COLOR => {
                if self.crtc_port_active(port) {
                    self.crtc_index = value & 0x3F;
                }
            }
            VGA_PORT_CRTC_DATA_MONO | VGA_PORT_CRTC_DATA_COLOR => {
                if self.crtc_port_active(port) {
                    self.crtc_data_write(value);
                }
            }
            VGA_PORT_MODE_CONTROL_MONO | VGA_PORT_MODE_CONTROL_COLOR => {
                if self.crtc_port_active(port) {
                    self.mode_control = value;
                    if value == KEY_COMPLETE_VALUE && self.key_prefix_armed {
                        self.key_unlocked = true;
                    }
                }
            }
            VGA_PORT_STATUS_MONO | VGA_PORT_STATUS_COLOR => {
                if self.crtc_port_active(port) {
                    self.feature_control = value;
                }
            }
            VGA_PORT_HERCULES_COMPAT => {
                self.hercules_compat = value;
                self.key_prefix_armed = value == KEY_PREFIX_VALUE;
            }
            VGA_PORT_ATC_WRITE => self.atc_write(value),
            VGA_PORT_ATC_READ => {}
            VGA_PORT_STATUS0_MISC_WRITE => self.misc_output = value,
            VGA_PORT_SEQ_INDEX => self.seq_index = value,
            VGA_PORT_SEQ_DATA => self.seq_data_write(value),
            VGA_PORT_DAC_MASK => self.dac_mask_write(value),
            VGA_PORT_DAC_READ_INDEX => self.dac_read_index_write(value),
            VGA_PORT_DAC_WRITE_INDEX => self.dac_write_index_write(value),
            VGA_PORT_DAC_DATA => self.dac_data_write(value),
            VGA_PORT_MISC_READ => {}
            VGA_PORT_SEGMENT_SELECT => {
                if self.key_unlocked {
                    self.segment_select = value;
                }
            }
            VGA_PORT_GC_INDEX => self.gc_index = value & 0x0F,
            VGA_PORT_GC_DATA => {
                let index = usize::from(self.gc_index & 0x0F);
                if index < GC_REGISTER_COUNT {
                    self.gc[index] = value;
                }
            }
            _ => {}
        }
    }

    /// Whether a CRTC-side port decodes under the current misc output bit 0.
    fn crtc_port_active(&self, port: u16) -> bool {
        let color_port = (port & 0x00F0) == 0x00D0;
        color_port == self.color_decode()
    }

    /// Input status one: display enable complement and vertical retrace.
    /// Reading it resets the attribute controller flip-flop to index phase.
    fn read_input_status_1(&mut self, retrace: RetraceStatus) -> u8 {
        self.atc_data_phase = false;
        let mut status = 0;
        if retrace.display_disabled {
            status |= 0x01;
        }
        if retrace.vertical_retrace {
            status |= 0x08;
        } else {
            status |= 0x80;
        }
        status
    }

    /// Input status zero: monitor sense, feature codes and the vertical
    /// retrace interrupt latch.
    fn read_input_status_0(&self) -> u8 {
        let mut status = 0;
        let sense_sum =
            u16::from(self.dac[0][0]) + u16::from(self.dac[0][1]) + u16::from(self.dac[0][2]);
        if sense_sum < MONITOR_SENSE_SUM_THRESHOLD {
            status |= 0x10;
        }
        if self.key_unlocked {
            // Feature code inputs default high per the ET4000 databook and
            // read as zero until the KEY is set.
            status |= 0x60;
        }
        if self.vretrace_interrupt_latch {
            status |= 0x80;
        }
        status
    }

    /// Attribute controller write through the index/data flip-flop.
    fn atc_write(&mut self, value: u8) {
        if self.atc_data_phase {
            let index = self.atc_index & 0x1F;
            let palette_locked = self.atc_index & 0x20 != 0 && index < 0x10;
            let et4000_locked = index == ATC_INDEX_ET4000_MISC && !self.key_unlocked;
            if !palette_locked && !et4000_locked {
                self.atc[usize::from(index)] = value;
            }
        } else {
            self.atc_index = value & 0x3F;
        }
        self.atc_data_phase = !self.atc_data_phase;
    }

    /// Sequencer data write with synchronous reset and KEY handling.
    fn seq_data_write(&mut self, value: u8) {
        let index = self.seq_index & 0x07;
        let et4000_register = index == SEQ_INDEX_TS_STATE || index == SEQ_INDEX_TS_AUX_MODE;
        if et4000_register && !self.key_unlocked {
            return;
        }
        if index == SEQ_INDEX_RESET && value & 0x02 == 0 {
            // A synchronous reset clears the KEY per the ET4000 databook.
            self.key_unlocked = false;
            self.key_prefix_armed = false;
        }
        self.seq[usize::from(index)] = value;
    }

    /// CRTC data write honoring the write protection and KEY rules.
    fn crtc_data_write(&mut self, value: u8) {
        let index = self.crtc_index;
        let protect = self.crtc[usize::from(CRTC_INDEX_VRETRACE_END)] & 0x80 != 0;
        if index <= 0x07 && protect {
            if index == 0x07 {
                // Line compare bit 8 stays writable under protection.
                let register = &mut self.crtc[usize::from(index)];
                *register = (*register & !0x10) | (value & 0x10);
            }
            return;
        }
        if index == CRTC_INDEX_OVERFLOW_HIGH {
            if protect {
                return;
            }
        } else if index > CRTC_LAST_UNPROTECTED_INDEX
            && index != CRTC_INDEX_EXT_START
            && !self.key_unlocked
        {
            return;
        }
        if index == CRTC_INDEX_VRETRACE_END && value & 0x10 == 0 {
            self.vretrace_interrupt_latch = false;
        }
        self.crtc[usize::from(index) % CRTC_REGISTER_COUNT] = value;
    }
}
