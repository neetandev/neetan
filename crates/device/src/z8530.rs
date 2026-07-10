//! Zilog Z8530 serial communications controller.
//!
//! Behavioral subset used by the X68000: channel B carries the Sharp mouse
//! protocol behind the MSCTRL handshake, channel A is an RS-232C stub whose
//! transmitter is always empty. The controller latches a three byte mouse
//! packet supplied by the machine glue, serializes it at the channel B baud
//! rate, and delivers one vectored receive interrupt per packet byte.

/// Status code placed in modified vectors for a channel B receive interrupt.
const B_RECEIVE_STATUS_CODE: u8 = 2;
/// Status code placed in modified vectors for a channel A transmit interrupt.
const A_SEND_STATUS_CODE: u8 = 4;
/// Status code placed in modified vectors for a channel A receive interrupt.
const A_RECEIVE_STATUS_CODE: u8 = 6;

/// RR3 pending bit for the channel A receive interrupt.
const RR3_A_RECEIVE_PENDING: u8 = 0x20;
/// RR3 pending bit for the channel A transmit interrupt.
const RR3_A_SEND_PENDING: u8 = 0x10;
/// RR3 pending bit for the channel B receive interrupt.
const RR3_B_RECEIVE_PENDING: u8 = 0x04;

/// RR0 bit reporting an empty transmit buffer.
const RR0_TX_BUFFER_EMPTY: u8 = 0x04;
/// RR0 bit reporting a received character waiting in the buffer.
const RR0_RX_CHARACTER_AVAILABLE: u8 = 0x01;

/// WR0 command resetting the highest in-service interrupt.
const WR0_RESET_HIGHEST_IUS: u8 = 0x38;
/// WR1 mask selecting any receive interrupt mode.
const WR1_RECEIVE_INTERRUPT_MODE: u8 = 0x18;
/// WR1 bit enabling the transmit interrupt.
const WR1_TRANSMIT_INTERRUPT_ENABLE: u8 = 0x02;
/// WR5 bit driving the RTS output.
const WR5_REQUEST_TO_SEND: u8 = 0x02;
/// WR9 bit enabling interrupts globally.
const WR9_MASTER_INTERRUPT_ENABLE: u8 = 0x08;
/// WR9 mask selecting the vector modification mode.
const WR9_VECTOR_MODE_MASK: u8 = 0x11;
/// WR9 reset command field.
const WR9_RESET_MASK: u8 = 0xC0;
/// WR9 command resetting channel B.
const WR9_CHANNEL_RESET_B: u8 = 0x40;
/// WR9 command resetting channel A.
const WR9_CHANNEL_RESET_A: u8 = 0x80;
/// WR9 command forcing a hardware reset.
const WR9_FORCE_HARDWARE_RESET: u8 = 0xC0;

/// Reset value of the channel B baud rate time constant.
const RESET_BAUD_RATE_B: u16 = 31;
/// Reset value of the channel A baud rate time constant.
const RESET_BAUD_RATE_A: u16 = 14;

/// Number of bytes in a Sharp mouse packet.
const MOUSE_PACKET_LENGTH: u8 = 3;

/// Serial bits per mouse byte: one start, eight data, and two stop bits.
const MOUSE_FRAME_BITS: u64 = 11;

/// PCLK ticks per baud-rate-generator bit at the x16 clock mode.
const BAUD_TICKS_PER_BIT_UNIT: u64 = 2 * 16;

/// SCC channel selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SccChannel {
    /// Channel A, the X68000 RS-232C port.
    A,
    /// Channel B, the X68000 mouse port.
    B,
}

/// Externally visible side effect of a control register write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SccWriteEffect {
    /// No side effect beyond internal state.
    None,
    /// Channel B RTS rose, pulsing MSCTRL and requesting a mouse packet.
    ///
    /// The machine glue must latch a fresh packet with
    /// [`Z8530::load_mouse_packet`] when it observes this effect.
    MouseRequestEdge,
}

/// Zilog Z8530 serial communications controller.
pub struct Z8530 {
    interrupt_vector: u8,
    vector_mode: u8,
    master_interrupt_enable: bool,
    register_pointer_a: u8,
    register_pointer_b: u8,
    b_receive_interrupt_enabled: bool,
    a_receive_interrupt_enabled: bool,
    a_send_interrupt_enabled: bool,
    b_receive_pending: bool,
    b_receive_request: bool,
    a_receive_pending: bool,
    a_receive_request: bool,
    a_send_pending: bool,
    a_send_request: bool,
    b_request_to_send: bool,
    baud_rate_a: u16,
    baud_rate_b: u16,
    mouse_packet: [u8; 3],
    mouse_read_count: u8,
    mouse_released_count: u8,
    mouse_release_tick: Option<u64>,
}

impl Z8530 {
    /// Builds a controller in the power-on state.
    pub fn new() -> Self {
        Self {
            interrupt_vector: 0,
            vector_mode: 0,
            master_interrupt_enable: false,
            register_pointer_a: 0,
            register_pointer_b: 0,
            b_receive_interrupt_enabled: false,
            a_receive_interrupt_enabled: false,
            a_send_interrupt_enabled: false,
            b_receive_pending: false,
            b_receive_request: false,
            a_receive_pending: false,
            a_receive_request: false,
            a_send_pending: false,
            a_send_request: false,
            b_request_to_send: false,
            baud_rate_a: RESET_BAUD_RATE_A,
            baud_rate_b: RESET_BAUD_RATE_B,
            mouse_packet: [0; 3],
            mouse_read_count: 0,
            mouse_released_count: 0,
            mouse_release_tick: None,
        }
    }

    /// Returns the controller to the power-on state.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Reads the control register selected by the channel pointer.
    pub fn read_control(&mut self, channel: SccChannel) -> u8 {
        match channel {
            SccChannel::B => {
                let register = self.register_pointer_b;
                self.register_pointer_b = 0;
                match register {
                    0 => {
                        RR0_TX_BUFFER_EMPTY
                            | if self.mouse_read_count < self.mouse_released_count {
                                RR0_RX_CHARACTER_AVAILABLE
                            } else {
                                0
                            }
                    }
                    2 => self.pending_modified_vector(),
                    12 => self.baud_rate_b as u8,
                    13 => (self.baud_rate_b >> 8) as u8,
                    _ => 0,
                }
            }
            SccChannel::A => {
                let register = self.register_pointer_a;
                self.register_pointer_a = 0;
                match register {
                    0 => RR0_TX_BUFFER_EMPTY,
                    2 => self.interrupt_vector,
                    3 => self.pending_register(),
                    12 => self.baud_rate_a as u8,
                    13 => (self.baud_rate_a >> 8) as u8,
                    _ => 0,
                }
            }
        }
    }

    /// Writes the control register selected by the channel pointer.
    pub fn write_control(&mut self, channel: SccChannel, value: u8) -> SccWriteEffect {
        let register = match channel {
            SccChannel::B => self.register_pointer_b,
            SccChannel::A => self.register_pointer_a,
        };
        if register == 0 {
            if value & 0xF0 == 0 {
                match channel {
                    SccChannel::B => self.register_pointer_b = value & 0x0F,
                    SccChannel::A => self.register_pointer_a = value & 0x0F,
                }
            } else if value == WR0_RESET_HIGHEST_IUS {
                self.reset_highest_in_service(channel);
            }
            return SccWriteEffect::None;
        }
        let mut effect = SccWriteEffect::None;
        match channel {
            SccChannel::B => {
                match register {
                    1 => {
                        self.b_receive_interrupt_enabled = value & WR1_RECEIVE_INTERRUPT_MODE != 0;
                    }
                    2 => self.interrupt_vector = value,
                    5 => {
                        let request_to_send = value & WR5_REQUEST_TO_SEND != 0;
                        if request_to_send && !self.b_request_to_send {
                            self.mouse_read_count = 0;
                            self.mouse_released_count = 0;
                            self.mouse_release_tick = None;
                            effect = SccWriteEffect::MouseRequestEdge;
                        }
                        self.b_request_to_send = request_to_send;
                    }
                    9 => self.write_wr9(value),
                    12 => self.baud_rate_b = (self.baud_rate_b & 0xFF00) | u16::from(value),
                    13 => self.baud_rate_b = (u16::from(value) << 8) | (self.baud_rate_b & 0xFF),
                    _ => {}
                }
                self.register_pointer_b = 0;
            }
            SccChannel::A => {
                match register {
                    1 => {
                        self.a_receive_interrupt_enabled = value & WR1_RECEIVE_INTERRUPT_MODE != 0;
                        self.a_send_interrupt_enabled = value & WR1_TRANSMIT_INTERRUPT_ENABLE != 0;
                    }
                    2 => self.interrupt_vector = value,
                    9 => self.write_wr9(value),
                    12 => self.baud_rate_a = (self.baud_rate_a & 0xFF00) | u16::from(value),
                    13 => self.baud_rate_a = (u16::from(value) << 8) | (self.baud_rate_a & 0xFF),
                    _ => {}
                }
                self.register_pointer_a = 0;
            }
        }
        effect
    }

    /// Reads the data register of a channel.
    pub fn read_data(&mut self, channel: SccChannel) -> u8 {
        match channel {
            SccChannel::B => {
                if self.mouse_read_count < self.mouse_released_count {
                    let byte = self.mouse_packet[usize::from(self.mouse_read_count)];
                    self.mouse_read_count += 1;
                    byte
                } else {
                    0
                }
            }
            SccChannel::A => 0,
        }
    }

    /// Writes the data register of a channel.
    ///
    /// The channel A transmitter is a stub that discards the byte and raises
    /// the transmit-buffer-empty interrupt immediately when enabled.
    pub fn write_data(&mut self, channel: SccChannel, value: u8) {
        let _ = value;
        match channel {
            SccChannel::B => {}
            SccChannel::A => {
                if self.master_interrupt_enable && self.a_send_interrupt_enabled {
                    self.a_send_pending = true;
                    self.a_send_request = true;
                }
            }
        }
    }

    /// Latches a fresh three byte mouse packet for channel B reads. The bytes
    /// become readable one serial byte time apart, starting at `tick`.
    pub fn load_mouse_packet(&mut self, packet: [u8; 3], tick: u64) {
        self.mouse_packet = packet;
        self.mouse_read_count = 0;
        self.mouse_released_count = 0;
        self.mouse_release_tick = Some(tick + self.mouse_byte_duration_ticks());
    }

    /// Returns the serial transmission time of one mouse byte in PCLK ticks.
    pub fn mouse_byte_duration_ticks(&self) -> u64 {
        MOUSE_FRAME_BITS * BAUD_TICKS_PER_BIT_UNIT * (u64::from(self.baud_rate_b) + 2)
    }

    /// Advances the mouse serializer to the absolute PCLK tick, releasing
    /// packet bytes that finished transmission.
    pub fn advance_to(&mut self, tick: u64) {
        while let Some(due) = self.mouse_release_tick {
            if due > tick {
                break;
            }
            self.mouse_released_count += 1;
            if self.master_interrupt_enable && self.b_receive_interrupt_enabled {
                self.b_receive_pending = true;
                self.b_receive_request = true;
            }
            self.mouse_release_tick = if self.mouse_released_count < MOUSE_PACKET_LENGTH {
                Some(due + self.mouse_byte_duration_ticks())
            } else {
                None
            };
        }
    }

    /// Returns the PCLK tick of the next pending byte release, if any.
    pub fn next_event_tick(&self) -> Option<u64> {
        self.mouse_release_tick
    }

    /// Reports whether the interrupt output is asserted.
    pub fn irq_asserted(&self) -> bool {
        self.a_receive_request || self.a_send_request || self.b_receive_request
    }

    /// Acknowledges the highest priority interrupt, returning its vector.
    pub fn acknowledge_interrupt(&mut self) -> Option<u8> {
        if self.a_receive_request {
            self.a_receive_request = false;
            Some(self.modified_vector(A_RECEIVE_STATUS_CODE))
        } else if self.a_send_request {
            self.a_send_request = false;
            Some(self.modified_vector(A_SEND_STATUS_CODE))
        } else if self.b_receive_request {
            self.b_receive_request = false;
            Some(self.modified_vector(B_RECEIVE_STATUS_CODE))
        } else {
            None
        }
    }

    /// Handles the shared WR9 master interrupt control register.
    fn write_wr9(&mut self, value: u8) {
        match value & WR9_RESET_MASK {
            WR9_CHANNEL_RESET_B => self.b_request_to_send = false,
            WR9_CHANNEL_RESET_A => {}
            WR9_FORCE_HARDWARE_RESET => self.b_request_to_send = false,
            _ => {}
        }
        self.vector_mode = value & WR9_VECTOR_MODE_MASK;
        self.master_interrupt_enable = value & WR9_MASTER_INTERRUPT_ENABLE != 0;
    }

    /// Clears the highest in-service interrupt of a channel.
    ///
    /// Channel B re-raises its receive interrupt while packet bytes remain
    /// unread, delivering one interrupt per packet byte.
    fn reset_highest_in_service(&mut self, channel: SccChannel) {
        match channel {
            SccChannel::B => {
                if self.b_receive_pending {
                    self.b_receive_pending = false;
                    if self.mouse_read_count < self.mouse_released_count
                        && self.b_receive_interrupt_enabled
                    {
                        self.b_receive_pending = true;
                        self.b_receive_request = true;
                    }
                }
            }
            SccChannel::A => {
                if self.a_receive_pending {
                    self.a_receive_pending = false;
                } else if self.a_send_pending {
                    self.a_send_pending = false;
                }
            }
        }
    }

    /// Returns the RR3 interrupt pending register visible on channel A.
    fn pending_register(&self) -> u8 {
        let mut value = 0;
        if self.a_receive_pending {
            value |= RR3_A_RECEIVE_PENDING;
        }
        if self.a_send_pending {
            value |= RR3_A_SEND_PENDING;
        }
        if self.b_receive_pending {
            value |= RR3_B_RECEIVE_PENDING;
        }
        value
    }

    /// Returns RR2 on channel B: the vector of the highest pending request.
    fn pending_modified_vector(&self) -> u8 {
        if self.a_receive_request {
            self.modified_vector(A_RECEIVE_STATUS_CODE)
        } else if self.a_send_request {
            self.modified_vector(A_SEND_STATUS_CODE)
        } else if self.b_receive_request {
            self.modified_vector(B_RECEIVE_STATUS_CODE)
        } else {
            self.interrupt_vector
        }
    }

    /// Applies the WR9 vector modification mode to a status code.
    fn modified_vector(&self, status_code: u8) -> u8 {
        match self.vector_mode {
            0x00 | 0x10 => self.interrupt_vector,
            0x01 => (self.interrupt_vector & 0b1111_0001) | (status_code << 1),
            _ => (self.interrupt_vector & 0b1000_1111) | (status_code << 4),
        }
    }
}

impl Default for Z8530 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a register through the WR0 pointer protocol.
    fn write_register(scc: &mut Z8530, channel: SccChannel, register: u8, value: u8) {
        assert_eq!(scc.write_control(channel, register), SccWriteEffect::None);
        scc.write_control(channel, value);
    }

    /// Reads a register through the WR0 pointer protocol.
    fn read_register(scc: &mut Z8530, channel: SccChannel, register: u8) -> u8 {
        assert_eq!(scc.write_control(channel, register), SccWriteEffect::None);
        scc.read_control(channel)
    }

    /// Programs the Human68k IOCS mouse initialization sequence.
    fn program_mouse(scc: &mut Z8530) {
        write_register(scc, SccChannel::B, 9, 0x09);
        write_register(scc, SccChannel::B, 2, 0x40);
        write_register(scc, SccChannel::B, 1, 0x10);
    }

    #[test]
    fn register_pointer_resets_after_every_access() {
        let mut scc = Z8530::new();
        scc.write_control(SccChannel::B, 12);
        assert_eq!(scc.read_control(SccChannel::B), 31);
        // The pointer dropped back to register 0.
        assert_eq!(
            scc.read_control(SccChannel::B) & RR0_TX_BUFFER_EMPTY,
            RR0_TX_BUFFER_EMPTY
        );

        scc.write_control(SccChannel::A, 13);
        scc.write_control(SccChannel::A, 0x12);
        assert_eq!(read_register(&mut scc, SccChannel::A, 13), 0x12);
        assert_eq!(read_register(&mut scc, SccChannel::A, 12), 14);
    }

    /// One mouse byte time at the reset baud constant: 11 * 32 * (31 + 2).
    const BYTE_TICKS: u64 = 11_616;

    #[test]
    fn rts_edge_requests_a_packet_without_an_immediate_interrupt() {
        let mut scc = Z8530::new();
        program_mouse(&mut scc);
        assert!(!scc.irq_asserted());

        scc.write_control(SccChannel::B, 5);
        assert_eq!(
            scc.write_control(SccChannel::B, 0x62),
            SccWriteEffect::MouseRequestEdge
        );
        // The interrupt arrives with the first serialized byte.
        assert!(!scc.irq_asserted());
        scc.load_mouse_packet([0x01, 0x05, 0xFB], 0);
        scc.advance_to(BYTE_TICKS - 1);
        assert!(!scc.irq_asserted());
        scc.advance_to(BYTE_TICKS);
        assert!(scc.irq_asserted());

        // Holding RTS high produces no further edge.
        scc.write_control(SccChannel::B, 5);
        assert_eq!(scc.write_control(SccChannel::B, 0x62), SccWriteEffect::None);

        // Dropping and raising RTS again produces the next edge.
        scc.write_control(SccChannel::B, 5);
        assert_eq!(scc.write_control(SccChannel::B, 0x60), SccWriteEffect::None);
        scc.write_control(SccChannel::B, 5);
        assert_eq!(
            scc.write_control(SccChannel::B, 0x62),
            SccWriteEffect::MouseRequestEdge
        );
    }

    #[test]
    fn packet_bytes_deliver_one_paced_interrupt_each_over_ius_resets() {
        let mut scc = Z8530::new();
        program_mouse(&mut scc);
        scc.write_control(SccChannel::B, 5);
        scc.write_control(SccChannel::B, 0x62);
        scc.load_mouse_packet([0x01, 0x05, 0xFB], 0);

        assert_eq!(scc.next_event_tick(), Some(BYTE_TICKS));
        scc.advance_to(BYTE_TICKS);
        // Vector for B receive with VIS status-low: 0x40 | (2 << 1).
        assert_eq!(scc.acknowledge_interrupt(), Some(0x44));
        assert!(!scc.irq_asserted());
        assert_eq!(scc.read_data(SccChannel::B), 0x01);

        // The IUS reset re-raises nothing until the next byte lands.
        scc.write_control(SccChannel::B, WR0_RESET_HIGHEST_IUS);
        assert!(!scc.irq_asserted());
        scc.advance_to(2 * BYTE_TICKS);
        assert!(scc.irq_asserted());
        assert_eq!(scc.acknowledge_interrupt(), Some(0x44));
        assert_eq!(scc.read_data(SccChannel::B), 0x05);
        scc.write_control(SccChannel::B, WR0_RESET_HIGHEST_IUS);
        scc.advance_to(3 * BYTE_TICKS);
        assert_eq!(scc.acknowledge_interrupt(), Some(0x44));
        assert_eq!(scc.read_data(SccChannel::B), 0xFB);

        // All three bytes were read: the chain ends.
        scc.write_control(SccChannel::B, WR0_RESET_HIGHEST_IUS);
        assert!(!scc.irq_asserted());
        assert_eq!(scc.next_event_tick(), None);
        assert_eq!(scc.read_data(SccChannel::B), 0);
    }

    #[test]
    fn ius_reset_re_raises_while_released_bytes_remain_unread() {
        let mut scc = Z8530::new();
        program_mouse(&mut scc);
        scc.write_control(SccChannel::B, 5);
        scc.write_control(SccChannel::B, 0x62);
        scc.load_mouse_packet([0x01, 0x05, 0xFB], 0);

        // A slow reader lets all three bytes accumulate.
        scc.advance_to(3 * BYTE_TICKS);
        assert_eq!(scc.acknowledge_interrupt(), Some(0x44));
        assert_eq!(scc.read_data(SccChannel::B), 0x01);
        scc.write_control(SccChannel::B, WR0_RESET_HIGHEST_IUS);
        assert!(scc.irq_asserted());
        assert_eq!(scc.acknowledge_interrupt(), Some(0x44));
        assert_eq!(scc.read_data(SccChannel::B), 0x05);
        scc.write_control(SccChannel::B, WR0_RESET_HIGHEST_IUS);
        assert!(scc.irq_asserted());
    }

    #[test]
    fn rr0_reports_only_released_packet_bytes() {
        let mut scc = Z8530::new();
        program_mouse(&mut scc);
        scc.write_control(SccChannel::B, 5);
        scc.write_control(SccChannel::B, 0x62);
        scc.load_mouse_packet([0x00, 0x01, 0x02], 0);

        assert_eq!(scc.read_control(SccChannel::B), RR0_TX_BUFFER_EMPTY);
        scc.advance_to(BYTE_TICKS);
        assert_eq!(
            scc.read_control(SccChannel::B),
            RR0_TX_BUFFER_EMPTY | RR0_RX_CHARACTER_AVAILABLE
        );
        scc.read_data(SccChannel::B);
        assert_eq!(scc.read_control(SccChannel::B), RR0_TX_BUFFER_EMPTY);
        scc.advance_to(3 * BYTE_TICKS);
        scc.read_data(SccChannel::B);
        scc.read_data(SccChannel::B);
        assert_eq!(scc.read_control(SccChannel::B), RR0_TX_BUFFER_EMPTY);
    }

    #[test]
    fn byte_duration_follows_the_programmed_baud_constant() {
        let mut scc = Z8530::new();
        assert_eq!(scc.mouse_byte_duration_ticks(), BYTE_TICKS);
        // Doubling the time constant plus two roughly halves the baud rate.
        write_register(&mut scc, SccChannel::B, 12, 64);
        assert_eq!(scc.mouse_byte_duration_ticks(), 11 * 32 * 66);
    }

    #[test]
    fn vector_modification_covers_all_wr9_modes() {
        let mut scc = Z8530::new();
        write_register(&mut scc, SccChannel::B, 2, 0x5E);

        // No VIS: the raw vector.
        write_register(&mut scc, SccChannel::B, 9, 0x08);
        assert_eq!(scc.modified_vector(B_RECEIVE_STATUS_CODE), 0x5E);
        // Status high without VIS still returns the raw vector.
        write_register(&mut scc, SccChannel::B, 9, 0x18);
        assert_eq!(scc.modified_vector(B_RECEIVE_STATUS_CODE), 0x5E);
        // VIS with status low modifies bits 3:1.
        write_register(&mut scc, SccChannel::B, 9, 0x09);
        assert_eq!(scc.modified_vector(B_RECEIVE_STATUS_CODE), 0x54);
        assert_eq!(scc.modified_vector(A_SEND_STATUS_CODE), 0x58);
        assert_eq!(scc.modified_vector(A_RECEIVE_STATUS_CODE), 0x5C);
        // VIS with status high modifies bits 6:4.
        write_register(&mut scc, SccChannel::B, 9, 0x19);
        assert_eq!(scc.modified_vector(B_RECEIVE_STATUS_CODE), 0x2E);
        assert_eq!(scc.modified_vector(A_SEND_STATUS_CODE), 0x4E);
        assert_eq!(scc.modified_vector(A_RECEIVE_STATUS_CODE), 0x6E);
    }

    #[test]
    fn rr2_on_channel_b_reflects_the_pending_request() {
        let mut scc = Z8530::new();
        program_mouse(&mut scc);
        assert_eq!(read_register(&mut scc, SccChannel::B, 2), 0x40);
        assert_eq!(read_register(&mut scc, SccChannel::A, 2), 0x40);

        scc.write_control(SccChannel::B, 5);
        scc.write_control(SccChannel::B, 0x62);
        scc.load_mouse_packet([0x01, 0x00, 0x00], 0);
        scc.advance_to(BYTE_TICKS);
        assert_eq!(read_register(&mut scc, SccChannel::B, 2), 0x44);
        // RR2 on channel A stays unmodified.
        assert_eq!(read_register(&mut scc, SccChannel::A, 2), 0x40);
    }

    #[test]
    fn channel_a_transmit_interrupt_outranks_the_mouse() {
        let mut scc = Z8530::new();
        program_mouse(&mut scc);
        write_register(&mut scc, SccChannel::A, 1, 0x02);

        scc.write_control(SccChannel::B, 5);
        scc.write_control(SccChannel::B, 0x62);
        scc.load_mouse_packet([0x01, 0x00, 0x00], 0);
        scc.advance_to(BYTE_TICKS);
        scc.write_data(SccChannel::A, 0x41);
        assert!(scc.irq_asserted());

        assert_eq!(read_register(&mut scc, SccChannel::A, 3), 0x14);
        assert_eq!(scc.acknowledge_interrupt(), Some(0x48));
        assert_eq!(scc.acknowledge_interrupt(), Some(0x44));
        assert_eq!(scc.acknowledge_interrupt(), None);
    }

    #[test]
    fn master_interrupt_enable_gates_new_requests() {
        let mut scc = Z8530::new();
        write_register(&mut scc, SccChannel::B, 2, 0x40);
        write_register(&mut scc, SccChannel::B, 1, 0x10);
        // MIE stays clear: the packet bytes arrive without an interrupt.
        scc.write_control(SccChannel::B, 5);
        assert_eq!(
            scc.write_control(SccChannel::B, 0x62),
            SccWriteEffect::MouseRequestEdge
        );
        scc.load_mouse_packet([0x01, 0x00, 0x00], 0);
        scc.advance_to(3 * BYTE_TICKS);
        assert!(!scc.irq_asserted());
        assert_eq!(scc.read_data(SccChannel::B), 0x01);
    }

    #[test]
    fn channel_reset_drops_rts_so_the_next_raise_is_an_edge() {
        let mut scc = Z8530::new();
        program_mouse(&mut scc);
        scc.write_control(SccChannel::B, 5);
        scc.write_control(SccChannel::B, 0x62);

        write_register(&mut scc, SccChannel::B, 9, 0x49);
        scc.write_control(SccChannel::B, 5);
        assert_eq!(
            scc.write_control(SccChannel::B, 0x62),
            SccWriteEffect::MouseRequestEdge
        );
    }

    #[test]
    fn reset_restores_the_power_on_state() {
        let mut scc = Z8530::new();
        program_mouse(&mut scc);
        scc.write_control(SccChannel::B, 5);
        scc.write_control(SccChannel::B, 0x62);
        scc.load_mouse_packet([1, 2, 3], 0);
        scc.reset();
        assert!(!scc.irq_asserted());
        assert_eq!(read_register(&mut scc, SccChannel::B, 12), 31);
        assert_eq!(read_register(&mut scc, SccChannel::A, 12), 14);
        assert_eq!(read_register(&mut scc, SccChannel::B, 2), 0);
    }

    #[test]
    fn channel_a_data_reads_return_zero_and_writes_need_enables() {
        let mut scc = Z8530::new();
        assert_eq!(scc.read_data(SccChannel::A), 0);
        scc.write_data(SccChannel::A, 0x55);
        assert!(!scc.irq_asserted());
    }
}
