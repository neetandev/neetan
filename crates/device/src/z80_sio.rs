//! Zilog Z80 SIO (dual-channel serial).
//!
//! Two independent channels, each programmed through a write-register pointer
//! state machine: a WR0 write selects the next register (low 3 bits) and may
//! carry a command in bits 5:3; the following control write lands in the
//! selected register and the pointer returns to WR0. The shared mode-2
//! interrupt vector lives in channel B's WR2 and, when WR1 bit 2 (status
//! affects vector) is set, its low bits encode the highest-priority pending
//! condition.
//!
//! On the Sharp X1 turbo channel 0 is the RS-232C port and channel 1 reads the
//! mouse. There is no internal baud generator: the CTC clocks the channels
//! externally. Received bytes are injected by the machine ([`Z80Sio::receive`])
//! rather than assembled bit by bit; reading the data port pops the receive
//! buffer and clears the receive interrupt. Channel 1's RTS output (WR5 bit 1)
//! drives the mouse: a high-to-low transition latches a mouse report, which the
//! machine detects through [`Z80Sio::take_rts_falling_edge`].

use std::collections::VecDeque;

/// Number of channels.
pub const CHANNEL_COUNT: usize = 2;

/// WR0 command field (bits 5:3).
const WR0_COMMAND_MASK: u8 = 0x38;
const WR0_RESET_EXT_STATUS: u8 = 0x10;
const WR0_CHANNEL_RESET: u8 = 0x18;
const WR0_ENABLE_INT_NEXT_RX: u8 = 0x20;
const WR0_RESET_TX_INT: u8 = 0x28;
const WR0_ERROR_RESET: u8 = 0x30;
const WR0_RETURN_FROM_INT: u8 = 0x38;

/// WR1: external/status interrupt enable.
const WR1_EXT_INT_ENABLE: u8 = 0x01;
/// WR1: transmit interrupt enable.
const WR1_TX_INT_ENABLE: u8 = 0x02;
/// WR1: status affects the low vector bits (channel B only).
const WR1_STATUS_AFFECTS_VECTOR: u8 = 0x04;
/// WR1: receive interrupt mode (bits 4:3); any nonzero value enables rx ints.
const WR1_RX_INT_MODE_MASK: u8 = 0x18;

/// WR5: request-to-send output.
const WR5_RTS: u8 = 0x02;

/// One SIO channel.
#[derive(Debug, Clone)]
struct SioChannel {
    write_registers: [u8; 8],
    pointer: usize,
    receive_fifo: VecDeque<u8>,
    receive_interrupt: bool,
    transmit_interrupt: bool,
    status_interrupt: bool,
    error_interrupt: bool,
    rts_low: bool,
    rts_falling_edge: bool,
}

impl SioChannel {
    fn new() -> Self {
        Self {
            write_registers: [0; 8],
            pointer: 0,
            receive_fifo: VecDeque::new(),
            receive_interrupt: false,
            transmit_interrupt: false,
            status_interrupt: false,
            error_interrupt: false,
            rts_low: false,
            rts_falling_edge: false,
        }
    }

    fn rx_interrupt_enabled(&self) -> bool {
        self.write_registers[1] & WR1_RX_INT_MODE_MASK != 0
    }

    fn tx_interrupt_enabled(&self) -> bool {
        self.write_registers[1] & WR1_TX_INT_ENABLE != 0
    }

    fn ext_interrupt_enabled(&self) -> bool {
        self.write_registers[1] & WR1_EXT_INT_ENABLE != 0
    }
}

/// Zilog Z80 SIO device.
#[derive(Debug, Clone)]
pub struct Z80Sio {
    channels: [SioChannel; CHANNEL_COUNT],
}

impl Default for Z80Sio {
    fn default() -> Self {
        Self::new()
    }
}

impl Z80Sio {
    /// Creates an SIO with both channels reset.
    pub fn new() -> Self {
        Self {
            channels: [SioChannel::new(), SioChannel::new()],
        }
    }

    /// Resets both channels and clears pending interrupts.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Writes a control byte to `channel`.
    pub fn write_control(&mut self, channel: usize, value: u8) {
        let index = channel & 0x01;
        let pointer = self.channels[index].pointer;
        if pointer == 0 {
            self.write_wr0(index, value);
            self.channels[index].pointer = (value & 0x07) as usize;
        } else {
            self.write_register(index, pointer, value);
            self.channels[index].pointer = 0;
        }
    }

    fn write_wr0(&mut self, channel: usize, value: u8) {
        self.channels[channel].write_registers[0] = value;
        match value & WR0_COMMAND_MASK {
            WR0_RESET_EXT_STATUS => self.channels[channel].status_interrupt = false,
            WR0_CHANNEL_RESET => self.channels[channel] = SioChannel::new(),
            WR0_ENABLE_INT_NEXT_RX => {}
            WR0_RESET_TX_INT => self.channels[channel].transmit_interrupt = false,
            WR0_ERROR_RESET => self.channels[channel].error_interrupt = false,
            // The return-from-interrupt (EOI) command resets the daisy-chain
            // in-service latch on real hardware; the simplified controller
            // resolves priority at acknowledge time and needs no latch.
            WR0_RETURN_FROM_INT => {}
            _ => {}
        }
    }

    fn write_register(&mut self, channel: usize, pointer: usize, value: u8) {
        if pointer == 5 {
            self.update_rts(channel, value);
        }
        self.channels[channel].write_registers[pointer] = value;
    }

    /// Tracks the RTS output level (WR5 bit 1) and latches a high-to-low edge.
    /// The output is driven low when the bit is set, so setting the bit is the
    /// falling edge the mouse triggers on.
    fn update_rts(&mut self, channel: usize, wr5: u8) {
        let new_low = wr5 & WR5_RTS != 0;
        let old_low = self.channels[channel].rts_low;
        if !old_low && new_low {
            self.channels[channel].rts_falling_edge = true;
        }
        self.channels[channel].rts_low = new_low;
    }

    /// Consumes the latched RTS high-to-low edge for `channel`.
    pub fn take_rts_falling_edge(&mut self, channel: usize) -> bool {
        let index = channel & 0x01;
        let edge = self.channels[index].rts_falling_edge;
        self.channels[index].rts_falling_edge = false;
        edge
    }

    /// Writes a byte to `channel`'s transmit register. The byte is sent
    /// immediately; the transmit-buffer-empty interrupt is raised if enabled.
    pub fn write_data(&mut self, channel: usize, _value: u8) {
        let index = channel & 0x01;
        if self.channels[index].tx_interrupt_enabled() {
            self.channels[index].transmit_interrupt = true;
        }
    }

    /// Reads a byte from `channel`'s receive register, popping the receive FIFO
    /// and clearing the receive interrupt once the buffer drains.
    pub fn read_data(&mut self, channel: usize) -> u8 {
        let index = channel & 0x01;
        let byte = self.channels[index].receive_fifo.pop_front().unwrap_or(0);
        if self.channels[index].receive_fifo.is_empty() {
            self.channels[index].receive_interrupt = false;
        }
        byte
    }

    /// Reads a control/status register from `channel` per the current pointer.
    pub fn read_control(&mut self, channel: usize) -> u8 {
        let index = channel & 0x01;
        let pointer = self.channels[index].pointer;
        self.channels[index].pointer = 0;
        match pointer {
            1 => self.read_rr1(index),
            2 => self.vector(),
            _ => self.read_rr0(index),
        }
    }

    fn read_rr0(&self, channel: usize) -> u8 {
        let mut value = 0x04; // transmit buffer always empty in this model
        if !self.channels[channel].receive_fifo.is_empty() {
            value |= 0x01;
        }
        if channel == 0 && self.has_pending() {
            value |= 0x02;
        }
        value
    }

    fn read_rr1(&self, channel: usize) -> u8 {
        let mut value = 0x01; // all sent
        if self.channels[channel].error_interrupt {
            value |= 0x20;
        }
        value
    }

    /// Injects a received byte into `channel`, raising the receive interrupt if
    /// enabled.
    pub fn receive(&mut self, channel: usize, byte: u8) {
        let index = channel & 0x01;
        self.channels[index].receive_fifo.push_back(byte);
        if self.channels[index].rx_interrupt_enabled() {
            self.channels[index].receive_interrupt = true;
        }
    }

    /// Flushes `channel`'s receive buffer and clears its receive interrupt.
    pub fn clear_receive(&mut self, channel: usize) {
        let index = channel & 0x01;
        self.channels[index].receive_fifo.clear();
        self.channels[index].receive_interrupt = false;
    }

    fn status_affects_vector(&self) -> bool {
        self.channels[1].write_registers[1] & WR1_STATUS_AFFECTS_VECTOR != 0
    }

    /// The highest-priority pending condition as (channel, affect code), where
    /// the affect code is channel A base 4 plus the condition rank
    /// (special-rx 3, rx-available 2, ext/status 1, tx-empty 0).
    fn pending_condition(&self) -> Option<(usize, u8)> {
        for channel in 0..CHANNEL_COUNT {
            let base = if channel == 0 { 4 } else { 0 };
            let state = &self.channels[channel];
            if state.error_interrupt {
                return Some((channel, base | 3));
            }
            if state.receive_interrupt && state.rx_interrupt_enabled() {
                return Some((channel, base | 2));
            }
            if state.status_interrupt && state.ext_interrupt_enabled() {
                return Some((channel, base | 1));
            }
            if state.transmit_interrupt && state.tx_interrupt_enabled() {
                return Some((channel, base));
            }
        }
        None
    }

    /// The mode-2 interrupt vector, with the status field folded in when WR1
    /// bit 2 is set on channel B.
    fn vector(&self) -> u8 {
        let base = self.channels[1].write_registers[2];
        match self.pending_condition() {
            Some((_, affect)) if self.status_affects_vector() => (base & 0xF1) | (affect << 1),
            _ => base,
        }
    }

    /// Whether any channel has a pending interrupt.
    pub fn has_pending(&self) -> bool {
        self.pending_condition().is_some()
    }

    /// Acknowledges the highest-priority pending interrupt, returning its
    /// mode-2 vector. Receive interrupts stay latched until the data register is
    /// read; other conditions are cleared here.
    pub fn acknowledge(&mut self) -> u8 {
        let vector = self.vector();
        if let Some((channel, affect)) = self.pending_condition() {
            match affect & 0x03 {
                3 => self.channels[channel].error_interrupt = false,
                1 => self.channels[channel].status_interrupt = false,
                0 => self.channels[channel].transmit_interrupt = false,
                _ => {} // receive interrupt cleared by reading the data register
            }
        }
        vector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Programs `channel` for interrupt-on-all-rx with the shared vector `base`
    /// (the vector lives in channel B / channel 1).
    fn enable_rx_interrupts(sio: &mut Z80Sio, channel: usize, base: u8, affects_vector: bool) {
        // Channel B holds the vector: WR2 = base.
        sio.write_control(1, 0x02); // point at WR2
        sio.write_control(1, base);
        // WR1: rx int on all chars, plus status-affects-vector on channel B.
        let wr1 = WR1_RX_INT_MODE_MASK
            | if affects_vector {
                WR1_STATUS_AFFECTS_VECTOR
            } else {
                0
            };
        sio.write_control(1, 0x01); // point at WR1
        sio.write_control(1, wr1);
        if channel == 0 {
            sio.write_control(0, 0x01);
            sio.write_control(0, WR1_RX_INT_MODE_MASK);
        }
    }

    #[test]
    fn received_byte_reads_back_and_clears_the_interrupt() {
        let mut sio = Z80Sio::new();
        enable_rx_interrupts(&mut sio, 0, 0x00, false);
        assert!(!sio.has_pending());

        sio.receive(0, 0xAB);
        assert!(sio.has_pending());
        assert_eq!(sio.read_data(0), 0xAB);
        assert!(!sio.has_pending());
    }

    #[test]
    fn channel_b_receive_vector_folds_in_the_status() {
        let mut sio = Z80Sio::new();
        enable_rx_interrupts(&mut sio, 1, 0x00, true);

        sio.receive(1, 0x55);
        assert!(sio.has_pending());
        // Channel B receive-available: affect = 2, so bits 3:1 become 0b010 = 4.
        assert_eq!(sio.acknowledge(), 0x04);
    }

    #[test]
    fn channel_a_outranks_channel_b() {
        let mut sio = Z80Sio::new();
        enable_rx_interrupts(&mut sio, 0, 0x00, true);

        sio.receive(1, 0x11);
        sio.receive(0, 0x22);
        // Channel A receive: affect = 4 | 2 = 6, bits 3:1 = 0b110 = 0x0C.
        assert_eq!(sio.acknowledge(), 0x0C);
    }

    #[test]
    fn setting_rts_latches_a_falling_edge() {
        let mut sio = Z80Sio::new();
        assert!(!sio.take_rts_falling_edge(1));

        // Point at WR5 and set RTS (drives the output low).
        sio.write_control(1, 0x05);
        sio.write_control(1, WR5_RTS);
        assert!(sio.take_rts_falling_edge(1));
        // The edge is consumed once.
        assert!(!sio.take_rts_falling_edge(1));

        // Clearing then setting again latches a fresh edge.
        sio.write_control(1, 0x05);
        sio.write_control(1, 0x00);
        sio.write_control(1, 0x05);
        sio.write_control(1, WR5_RTS);
        assert!(sio.take_rts_falling_edge(1));
    }

    #[test]
    fn clear_receive_flushes_the_buffer() {
        let mut sio = Z80Sio::new();
        enable_rx_interrupts(&mut sio, 1, 0x00, false);
        sio.receive(1, 0x01);
        sio.receive(1, 0x02);
        sio.clear_receive(1);
        assert!(!sio.has_pending());
        assert_eq!(sio.read_data(1), 0);
    }
}
