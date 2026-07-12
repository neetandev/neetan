//! PC/AT dual-8237 DMA front-end.
//!
//! The AT wires two i8237A controllers in cascade: controller 0 drives the
//! 8-bit channels 0-3 at ports 0x00-0x0F, controller 1 drives the 16-bit word
//! channels 4-7 at ports 0xC0-0xDF, and channel 4 is the cascade that couples
//! them. A separate page-register file at ports 0x80-0x8F supplies the upper
//! address bits.

use crate::i8237_dma::{DmaReadTransferResult, DmaTransferResult, I8237Dma, I8237DmaChannelState};

/// Mode register bit 5 (0 = increment, 1 = decrement).
const MODE_DECREMENT_BIT: u8 = 0x20;
/// Mode register bit 4 (auto-init reload on terminal count).
const MODE_AUTO_INIT_BIT: u8 = 0x10;
/// Mode register bits 3-2 select the transfer type.
const MODE_TRANSFER_TYPE_MASK: u8 = 0x0C;
/// Transfer type 00 advances the channel without accessing memory.
const MODE_VERIFY_TRANSFER: u8 = 0x00;

/// Number of page-register bytes (ports 0x80-0x8F).
pub const AT_DMA_PAGE_COUNT: usize = 16;

/// Controller index for the 8-bit channels (0-3).
const BYTE_CONTROLLER: usize = 0;
/// Controller index for the 16-bit word channels (4-7).
const WORD_CONTROLLER: usize = 1;
/// Cascade channel (controller 1 channel 0) that couples the controllers.
const CASCADE_LOCAL_CHANNEL: usize = 0;

/// PC/AT dual-8237 DMA front-end with the AT page-register file.
pub struct AtDma {
    /// The two cascaded controllers: `[0]` = 8-bit ch0-3, `[1]` = 16-bit ch4-7.
    pub controllers: [I8237Dma; 2],
    /// AT page-register file at ports 0x80-0x8F (all 16 bytes are POST scratch).
    pub pages: [u8; AT_DMA_PAGE_COUNT],
}

impl Default for AtDma {
    fn default() -> Self {
        Self::new()
    }
}

impl AtDma {
    /// Creates the DMA front-end in its power-on state.
    pub fn new() -> Self {
        Self {
            controllers: [I8237Dma::new(), I8237Dma::new()],
            pages: [0; AT_DMA_PAGE_COUNT],
        }
    }

    /// Reads a DMA-related I/O port (0x00-0x0F, 0x80-0x8F, 0xC0-0xDF).
    ///
    /// Returns `None` for ports outside the DMA decode so the bus can fall
    /// through to its unknown-port handling.
    pub fn io_read(&mut self, port: u16) -> Option<u8> {
        match port {
            0x00..=0x0F => {
                Some(self.read_controller_register(BYTE_CONTROLLER, (port & 0x0F) as u8))
            }
            0x80..=0x8F => Some(self.pages[(port & 0x0F) as usize]),
            0xC0..=0xDF => {
                if port & 0x01 == 0 {
                    Some(self.read_controller_register(WORD_CONTROLLER, ((port - 0xC0) / 2) as u8))
                } else {
                    Some(0xFF)
                }
            }
            _ => None,
        }
    }

    /// Writes a DMA-related I/O port. Returns `true` when the port was decoded.
    pub fn io_write(&mut self, port: u16, value: u8) -> bool {
        match port {
            0x00..=0x0F => {
                self.write_controller_register(BYTE_CONTROLLER, (port & 0x0F) as u8, value);
                true
            }
            0x80..=0x8F => {
                self.pages[(port & 0x0F) as usize] = value;
                true
            }
            0xC0..=0xDF => {
                if port & 0x01 == 0 {
                    self.write_controller_register(
                        WORD_CONTROLLER,
                        ((port - 0xC0) / 2) as u8,
                        value,
                    );
                }
                true
            }
            _ => false,
        }
    }

    /// Reads a controller's register by its 0x00-0x0F offset.
    fn read_controller_register(&mut self, controller: usize, register: u8) -> u8 {
        let dma = &mut self.controllers[controller];
        match register {
            0x00..=0x07 => {
                let channel = (register / 2) as usize;
                if register & 1 == 0 {
                    dma.read_address(channel)
                } else {
                    dma.read_count(channel)
                }
            }
            0x08 => dma.read_status(),
            0x0F => dma.read_mask(),
            _ => 0xFF,
        }
    }

    /// Writes a controller's register by its 0x00-0x0F offset.
    fn write_controller_register(&mut self, controller: usize, register: u8, value: u8) {
        let dma = &mut self.controllers[controller];
        match register {
            0x00..=0x07 => {
                let channel = (register / 2) as usize;
                if register & 1 == 0 {
                    dma.write_address(channel, value);
                } else {
                    dma.write_count(channel, value);
                }
            }
            0x08 => dma.write_command(value),
            0x09 => {} // Request register: software requests are not modeled.
            0x0A => dma.write_single_mask(value),
            0x0B => dma.write_mode(value),
            0x0C => dma.clear_flip_flop(),
            0x0D => dma.master_clear(),
            0x0E => dma.write_all_mask(0x00), // Clear-mask: unmask all channels.
            0x0F => dma.write_all_mask(value),
            _ => {}
        }
    }

    /// Returns the page-register index that supplies the upper address bits
    /// for a channel, per the AT page-register wiring.
    fn page_index_for_channel(channel: usize) -> usize {
        match channel {
            0 => 7,
            1 => 3,
            2 => 1,
            3 => 2,
            5 => 0xB,
            6 => 9,
            7 => 0xA,
            _ => 0,
        }
    }

    /// Returns whether a channel is unmasked and able to transfer.
    ///
    /// Byte channels additionally require the cascade channel (controller 1
    /// channel 0) to be unmasked.
    pub fn channel_unmasked(&self, channel: usize) -> bool {
        if channel < 4 {
            self.controllers[BYTE_CONTROLLER].channel_unmasked(channel)
                && self.controllers[WORD_CONTROLLER].channel_unmasked(CASCADE_LOCAL_CHANNEL)
        } else {
            self.controllers[WORD_CONTROLLER].channel_unmasked(channel - 4)
        }
    }

    /// Performs a device-to-memory transfer (a peripheral read filling RAM).
    ///
    /// For byte channels the physical address is `page << 16 | address`; for
    /// word channels it is `(page & 0xFE) << 16 | address << 1`, with the
    /// address register counting words and `data` consumed as little-endian
    /// 16-bit words.
    pub fn transfer_write_to_memory(&mut self, channel: usize, data: &[u8]) -> DmaTransferResult {
        let page = self.pages[Self::page_index_for_channel(channel)];
        let mut writes = Vec::with_capacity(data.len());
        let mut terminal_count = false;

        if channel < 4 {
            let ch = &mut self.controllers[BYTE_CONTROLLER].state.channels[channel];
            let increment = ch.mode & MODE_DECREMENT_BIT == 0;
            let verify = ch.mode & MODE_TRANSFER_TYPE_MASK == MODE_VERIFY_TRANSFER;
            for &byte in data {
                let physical = ((page as u32) << 16) | (ch.address as u32);
                if !verify {
                    writes.push((physical, byte));
                }
                advance_address(&mut ch.address, increment);
                if step_count(ch) {
                    terminal_count = true;
                    self.controllers[BYTE_CONTROLLER].state.status |= 1 << channel;
                    break;
                }
            }
        } else {
            let local = channel - 4;
            let ch = &mut self.controllers[WORD_CONTROLLER].state.channels[local];
            let increment = ch.mode & MODE_DECREMENT_BIT == 0;
            let verify = ch.mode & MODE_TRANSFER_TYPE_MASK == MODE_VERIFY_TRANSFER;
            for word in data.chunks(2) {
                let low = word[0];
                let high = *word.get(1).unwrap_or(&0);
                let physical = (((page & 0xFE) as u32) << 16) | ((ch.address as u32) << 1);
                if !verify {
                    writes.push((physical, low));
                    writes.push((physical + 1, high));
                }
                advance_address(&mut ch.address, increment);
                if step_count(ch) {
                    terminal_count = true;
                    self.controllers[WORD_CONTROLLER].state.status |= 1 << local;
                    break;
                }
            }
        }

        DmaTransferResult {
            writes,
            terminal_count,
        }
    }

    /// Performs a memory-to-device transfer (a peripheral write draining RAM).
    ///
    /// Returns the physical byte addresses to read, in order. Word channels
    /// return two consecutive byte addresses per word.
    pub fn transfer_read_from_memory(
        &mut self,
        channel: usize,
        byte_count: usize,
    ) -> DmaReadTransferResult {
        let page = self.pages[Self::page_index_for_channel(channel)];
        let mut addresses = Vec::with_capacity(byte_count);
        let mut terminal_count = false;

        if channel < 4 {
            let ch = &mut self.controllers[BYTE_CONTROLLER].state.channels[channel];
            let increment = ch.mode & MODE_DECREMENT_BIT == 0;
            let verify = ch.mode & MODE_TRANSFER_TYPE_MASK == MODE_VERIFY_TRANSFER;
            for _ in 0..byte_count {
                let physical = ((page as u32) << 16) | (ch.address as u32);
                if !verify {
                    addresses.push(physical);
                }
                advance_address(&mut ch.address, increment);
                if step_count(ch) {
                    terminal_count = true;
                    self.controllers[BYTE_CONTROLLER].state.status |= 1 << channel;
                    break;
                }
            }
        } else {
            let local = channel - 4;
            let ch = &mut self.controllers[WORD_CONTROLLER].state.channels[local];
            let increment = ch.mode & MODE_DECREMENT_BIT == 0;
            let verify = ch.mode & MODE_TRANSFER_TYPE_MASK == MODE_VERIFY_TRANSFER;
            let words = byte_count.div_ceil(2);
            for _ in 0..words {
                let physical = (((page & 0xFE) as u32) << 16) | ((ch.address as u32) << 1);
                if !verify {
                    addresses.push(physical);
                    addresses.push(physical + 1);
                }
                advance_address(&mut ch.address, increment);
                if step_count(ch) {
                    terminal_count = true;
                    self.controllers[WORD_CONTROLLER].state.status |= 1 << local;
                    break;
                }
            }
            if !verify {
                addresses.truncate(byte_count);
            }
        }

        DmaReadTransferResult {
            addresses,
            terminal_count,
        }
    }
}

/// Advances a channel address register by one unit in the given direction.
fn advance_address(address: &mut u16, increment: bool) {
    *address = if increment {
        address.wrapping_add(1)
    } else {
        address.wrapping_sub(1)
    };
}

/// Decrements a channel count register, applying auto-init reload at terminal
/// count. Returns whether terminal count was reached this step.
fn step_count(ch: &mut I8237DmaChannelState) -> bool {
    if ch.count == 0 {
        if ch.mode & MODE_AUTO_INIT_BIT != 0 {
            ch.address = ch.start_address;
            ch.count = ch.start_count;
        } else {
            ch.count = 0xFFFF;
        }
        true
    } else {
        ch.count -= 1;
        false
    }
}
