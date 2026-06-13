//! uPD8257 (i8257-compatible) four-channel DMA controller for the PC-8801.
//!
//! Decoded at ports 0x60-0x68: even ports 0x60/0x62/0x64/0x66 are the per-channel
//! address registers and odd ports 0x61/0x63/0x65/0x67 the count/mode registers,
//! each accessed low byte then high byte through a shared flip-flop. Port 0x68 is
//! the mode register on write and the terminal-count status on read.
//!
//! On the PC-88 channel 2 feeds the uPD3301 with text characters and attributes
//! each frame. The autoload feature (mode bit 7) mirrors channel-2 register writes
//! into channel 3 and reloads channel 2 from channel 3 at terminal count, so the
//! text screen refreshes without per-frame CPU programming. Only the channel-2
//! path is exercised on the bare MA; the transfer itself is driven by the bus,
//! which reads memory and feeds the CRTC while stepping this controller.

use std::ops::{Deref, DerefMut};

/// Number of DMA channels.
pub const CHANNEL_COUNT: usize = 4;
/// The channel that feeds the uPD3301 text display.
pub const TEXT_CHANNEL: usize = 2;

/// Mode register bit: autoload channel 2 from channel 3 at terminal count.
pub const MODE_AUTOLOAD: u8 = 0x80;
/// Mode register bit: stop the channel (clear its enable) at terminal count.
pub const MODE_TC_STOP: u8 = 0x40;

/// Per-channel DMA register state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct I8257ChannelState {
    /// Current transfer address (16-bit).
    pub address: u16,
    /// Remaining transfer count; transfers `count + 1` bytes and stops below 0.
    pub count: i32,
    /// Per-channel mode bits latched from the count high-byte write (0x80/0x40).
    pub mode_bits: u8,
    /// Whether the channel is currently transferring.
    pub running: bool,
}

/// Snapshot of the uPD8257 state for save/restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8257DmaState {
    /// The four DMA channels.
    pub channels: [I8257ChannelState; CHANNEL_COUNT],
    /// Mode register (port 0x68): bits 0-3 per-channel enable, 0x40 TC-stop,
    /// 0x80 autoload.
    pub mode: u8,
    /// Terminal-count status: bit `c` set when channel `c` reached TC.
    pub status: u8,
    /// Address/count byte flip-flop (false = low byte next).
    pub high_low: bool,
}

/// uPD8257 four-channel DMA controller.
pub struct I8257Dma {
    /// Embedded state for save/restore.
    pub state: I8257DmaState,
}

impl Deref for I8257Dma {
    type Target = I8257DmaState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for I8257Dma {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Default for I8257Dma {
    fn default() -> Self {
        Self::new()
    }
}

impl I8257Dma {
    /// Creates a controller in its power-on reset state.
    pub fn new() -> Self {
        Self {
            state: I8257DmaState {
                channels: [I8257ChannelState::default(); CHANNEL_COUNT],
                mode: 0,
                status: 0,
                high_low: false,
            },
        }
    }

    /// Handles an I/O write at `port & 0x0F` (0x60-0x68 decoded to 0x00-0x08).
    pub fn write_io(&mut self, port: u16, data: u8) {
        let channel = ((port >> 1) & 3) as usize;
        match port & 0x0F {
            0 | 2 | 4 | 6 => {
                if !self.state.high_low {
                    self.write_address_low(channel, data);
                } else {
                    self.write_address_high(channel, data);
                }
                self.state.high_low = !self.state.high_low;
            }
            1 | 3 | 5 | 7 => {
                if !self.state.high_low {
                    self.write_count_low(channel, data);
                } else {
                    self.write_count_high(channel, data);
                }
                self.state.high_low = !self.state.high_low;
            }
            8 => {
                self.state.mode = data;
                self.state.high_low = false;
            }
            _ => {}
        }
    }

    /// Handles an I/O read at `port & 0x0F`.
    pub fn read_io(&mut self, port: u16) -> u8 {
        let channel = ((port >> 1) & 3) as usize;
        match port & 0x0F {
            0 | 2 | 4 | 6 => {
                let value = if !self.state.high_low {
                    (self.state.channels[channel].address & 0xFF) as u8
                } else {
                    (self.state.channels[channel].address >> 8) as u8
                };
                self.state.high_low = !self.state.high_low;
                value
            }
            1 | 3 | 5 | 7 => {
                let value = if !self.state.high_low {
                    (self.state.channels[channel].count & 0xFF) as u8
                } else {
                    (((self.state.channels[channel].count >> 8) & 0x3F) as u8)
                        | self.state.channels[channel].mode_bits
                };
                self.state.high_low = !self.state.high_low;
                value
            }
            8 => {
                let value = self.state.status;
                self.state.status &= 0xF0;
                value
            }
            _ => 0xFF,
        }
    }

    fn autoload_channel_3(&self, channel: usize) -> bool {
        self.state.mode & MODE_AUTOLOAD != 0 && channel == TEXT_CHANNEL
    }

    fn write_address_low(&mut self, channel: usize, data: u8) {
        if self.autoload_channel_3(channel) {
            let address = &mut self.state.channels[3].address;
            *address = (*address & 0xFF00) | u16::from(data);
        }
        let address = &mut self.state.channels[channel].address;
        *address = (*address & 0xFF00) | u16::from(data);
    }

    fn write_address_high(&mut self, channel: usize, data: u8) {
        if self.autoload_channel_3(channel) {
            let address = &mut self.state.channels[3].address;
            *address = (*address & 0x00FF) | (u16::from(data) << 8);
        }
        let address = &mut self.state.channels[channel].address;
        *address = (*address & 0x00FF) | (u16::from(data) << 8);
    }

    fn write_count_low(&mut self, channel: usize, data: u8) {
        if self.autoload_channel_3(channel) {
            let count = &mut self.state.channels[3].count;
            *count = (*count & !0xFF) | i32::from(data);
        }
        let count = &mut self.state.channels[channel].count;
        *count = (*count & !0xFF) | i32::from(data);
    }

    fn write_count_high(&mut self, channel: usize, data: u8) {
        if self.autoload_channel_3(channel) {
            self.state.channels[3].count =
                (self.state.channels[3].count & 0xFF) | (i32::from(data & 0x3F) << 8);
            self.state.channels[3].mode_bits = data & 0xC0;
        }
        self.state.channels[channel].count =
            (self.state.channels[channel].count & 0xFF) | (i32::from(data & 0x3F) << 8);
        self.state.channels[channel].mode_bits = data & 0xC0;
    }

    /// Starts a channel: if enabled in the mode register it clears its TC status
    /// and begins running, otherwise it stays idle.
    pub fn start(&mut self, channel: usize) {
        if self.state.mode & (1 << channel) != 0 {
            self.state.status &= !(1 << channel);
            self.state.channels[channel].running = true;
        } else {
            self.state.channels[channel].running = false;
        }
    }

    /// Returns whether `channel` is running and still has bytes to transfer.
    pub fn channel_active(&self, channel: usize) -> bool {
        self.state.channels[channel].running && self.state.channels[channel].count >= 0
    }

    /// Returns the current transfer address of `channel`.
    pub fn channel_address(&self, channel: usize) -> u16 {
        self.state.channels[channel].address
    }

    /// Advances `channel` by one byte: increments the address and decrements the
    /// count.
    pub fn channel_advance(&mut self, channel: usize) {
        self.state.channels[channel].address = self.state.channels[channel].address.wrapping_add(1);
        self.state.channels[channel].count -= 1;
    }

    /// Completes a channel after its row/burst drain: reloads channel 2 from
    /// channel 3 when autoload is set, otherwise clears the enable when TC-stop is
    /// set, then latches the TC status and stops the channel.
    pub fn finish(&mut self, channel: usize) {
        if !self.state.channels[channel].running {
            return;
        }
        if self.state.mode & MODE_AUTOLOAD != 0 && channel == TEXT_CHANNEL {
            self.state.channels[2].address = self.state.channels[3].address;
            self.state.channels[2].count = self.state.channels[3].count;
            self.state.channels[2].mode_bits = self.state.channels[3].mode_bits;
        } else if self.state.mode & MODE_TC_STOP != 0 {
            self.state.mode &= !(1 << channel);
        }
        self.state.status |= 1 << channel;
        self.state.channels[channel].running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program_channel(dma: &mut I8257Dma, channel: usize, address: u16, count: u16) {
        let address_port = (channel as u16) << 1;
        let count_port = address_port | 1;
        dma.write_io(address_port, (address & 0xFF) as u8);
        dma.write_io(address_port, (address >> 8) as u8);
        dma.write_io(count_port, (count & 0xFF) as u8);
        dma.write_io(count_port, ((count >> 8) & 0x3F) as u8);
    }

    fn transfer_channel(dma: &mut I8257Dma, channel: usize, limit: u32) -> Vec<u16> {
        let mut addresses = Vec::new();
        let mut remaining = limit;
        while remaining > 0 && dma.channel_active(channel) {
            addresses.push(dma.channel_address(channel));
            dma.channel_advance(channel);
            remaining -= 1;
        }
        addresses
    }

    #[test]
    fn programming_with_flip_flop() {
        let mut dma = I8257Dma::new();
        program_channel(&mut dma, 2, 0xF000, 7);
        assert_eq!(dma.channels[2].address, 0xF000);
        assert_eq!(dma.channels[2].count, 7);
    }

    #[test]
    fn transfers_count_plus_one_bytes() {
        let mut dma = I8257Dma::new();
        dma.write_io(0x08, 1 << 2); // enable channel 2
        program_channel(&mut dma, 2, 0xF000, 3);
        dma.start(2);
        let addresses = transfer_channel(&mut dma, 2, 100);
        // count = 3 transfers 4 bytes.
        assert_eq!(addresses, vec![0xF000, 0xF001, 0xF002, 0xF003]);
        assert!(!dma.channel_active(2));
    }

    #[test]
    fn autoload_reloads_channel_2_from_channel_3() {
        let mut dma = I8257Dma::new();
        dma.write_io(0x08, MODE_AUTOLOAD | (1 << 2)); // autoload + enable ch2
        program_channel(&mut dma, 2, 0xF000, 2);
        // Channel 3 mirrors the channel-2 programming.
        assert_eq!(dma.channels[3].address, 0xF000);
        assert_eq!(dma.channels[3].count, 2);

        dma.start(2);
        let _ = transfer_channel(&mut dma, 2, 100);
        dma.finish(2);
        // Reloaded for the next frame.
        assert_eq!(dma.channels[2].address, 0xF000);
        assert_eq!(dma.channels[2].count, 2);
        // TC status latched.
        assert_ne!(dma.status & (1 << 2), 0);
    }

    #[test]
    fn status_read_clears_terminal_count_bits() {
        let mut dma = I8257Dma::new();
        dma.write_io(0x08, 1 << 2);
        program_channel(&mut dma, 2, 0xF000, 0);
        dma.start(2);
        let _ = transfer_channel(&mut dma, 2, 100);
        dma.finish(2);
        assert_ne!(dma.read_io(0x08) & (1 << 2), 0);
        // Cleared after the read.
        assert_eq!(dma.read_io(0x08) & 0x0F, 0);
    }
}
