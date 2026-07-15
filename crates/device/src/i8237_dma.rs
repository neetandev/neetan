//! i8237A DMA controller for the PC-98.
//!
//! Supports register programming and burst-mode transfers used by the
//! FDC: write transfers (device->memory) for reads, read transfers
//! (memory->device) for writes.

/// All four channels masked (bits 0-3 set). Applied at power-on and master clear.
/// Ref: undoc98 `io_dma.txt` (I/O 001Bh master clear)
const MASK_ALL_CHANNELS: u8 = 0x0F;

/// Channel select mask: bits 1:0 of mode/single-mask register select the channel number (0-3).
/// Ref: undoc98 `io_dma.txt` (I/O 0017h bits CS1,CS0)
const CHANNEL_SELECT_MASK: u8 = 0x03;

/// Single mask register: bit 2 (MK) controls mask set (1) / clear (0).
/// Ref: undoc98 `io_dma.txt` (I/O 0015h bit 2)
const SINGLE_MASK_SET_BIT: u8 = 0x04;

/// Mode register: bit 5 (ID) selects address direction.
/// 0 = increment, 1 = decrement.
/// Ref: undoc98 `io_dma.txt` (I/O 0017h bit 5)
const MODE_DECREMENT_BIT: u8 = 0x20;

/// Status register: lower nibble holds TC (terminal count) bits for channels 0-3.
/// Upper nibble holds request bits. TC bits are cleared on status read.
/// Ref: undoc98 `io_dma.txt` (I/O 0011h)
const STATUS_TC_MASK: u8 = 0x0F;

/// Mask for the lower nibble of the all-mask register (bits 0-3, one per channel).
/// Ref: undoc98 `io_dma.txt` (I/O 001Fh bits MB3-MB0)
const ALL_MASK_NIBBLE: u8 = 0x0F;

/// Auto-increment boundary register: bits 1:0 select the channel, bits 3:2 select the mode.
/// Ref: undoc98 `io_dma.txt` (I/O 0029h)
const BOUND_CHANNEL_MASK: u8 = 0x03;

/// Auto-increment boundary mode mask after shifting (2 bits).
/// Ref: undoc98 `io_dma.txt` (I/O 0029h bits M1,M0)
const BOUND_MODE_MASK: u8 = 0x03;

/// Boundary mode 1: 1 MB boundary. Page low nibble (A19-A16) increments on address wrap.
/// Ref: undoc98 `io_dma.txt` (I/O 0029h M1,M0 = 01b)
const BOUND_MODE_1MB: u8 = 1;

/// Boundary mode 3: 16 MB boundary. Entire page register (A23-A16) increments on address wrap.
/// Ref: undoc98 `io_dma.txt` (I/O 0029h M1,M0 = 11b)
const BOUND_MODE_16MB: u8 = 3;

/// Extended bank register mask: bits 6:0 are valid (A24-A30), bit 7 unused.
/// Ref: undoc98 `io_dma.txt` (I/O 0E05h-0E0Bh)
const EXTENDED_PAGE_MASK: u8 = 0x7F;

/// Page register low nibble mask, used for 1 MB boundary auto-increment.
/// Only A19-A16 (bits 3:0) wrap; A23-A20 (bits 7:4) are preserved.
const PAGE_LOW_NIBBLE_MASK: u8 = 0x0F;

/// Page register high nibble mask, preserved during 1 MB boundary auto-increment.
const PAGE_HIGH_NIBBLE_MASK: u8 = 0xF0;

/// Mode register: bit 4 (AI) selects auto-init mode.
/// 0 = single transfer (stops at TC), 1 = auto-init (reloads base on TC).
const MODE_AUTO_INIT_BIT: u8 = 0x10;

save_state::runtime_state! {
/// Authoritative state of a single DMA channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8237DmaChannelState {
    /// Current address register (modified during transfer).
    pub address: u16,
    /// Current word count register (modified during transfer).
    pub count: u16,
    /// Start address, latched when address is programmed. Reloaded on auto-init TC.
    pub start_address: u16,
    /// Start count, latched when count is programmed. Reloaded on auto-init TC.
    pub start_count: u16,
    /// Page register (A16-A23).
    pub page: u8,
    /// Extended page register (A24-A31, 386+ extended bank).
    pub extended_page: u8,
    /// Mode register for this channel.
    pub mode: u8,
    /// Auto-increment boundary mode (0-3).
    pub bound: u8,
}}

save_state::runtime_state! {
/// Authoritative state of the DMA controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I8237DmaState {
    /// The four DMA channels.
    pub channels: [I8237DmaChannelState; 4],
    /// Byte flip-flop (false = low byte next, true = high byte next).
    pub flip_flop: bool,
    /// Command register.
    pub command: u8,
    /// Mask register (bits 0-3 for channels 0-3).
    pub mask: u8,
    /// Status register.
    pub status: u8,
}}

/// i8237A DMA controller.
pub struct I8237Dma {
    /// Embedded state for save/restore.
    pub state: I8237DmaState,
}

impl_simple_state_accessors!(I8237Dma, I8237DmaState);

impl Default for I8237Dma {
    fn default() -> Self {
        Self::new()
    }
}

impl I8237Dma {
    /// Creates a new DMA controller in reset state (all channels masked).
    pub fn new() -> Self {
        Self {
            state: I8237DmaState {
                channels: [
                    I8237DmaChannelState {
                        address: 0,
                        count: 0,
                        start_address: 0,
                        start_count: 0,
                        page: 0,
                        extended_page: 0,
                        mode: 0,
                        bound: 0,
                    },
                    I8237DmaChannelState {
                        address: 0,
                        count: 0,
                        start_address: 0,
                        start_count: 0,
                        page: 0,
                        extended_page: 0,
                        mode: 0,
                        bound: 0,
                    },
                    I8237DmaChannelState {
                        address: 0,
                        count: 0,
                        start_address: 0,
                        start_count: 0,
                        page: 0,
                        extended_page: 0,
                        mode: 0,
                        bound: 0,
                    },
                    I8237DmaChannelState {
                        address: 0,
                        count: 0,
                        start_address: 0,
                        start_count: 0,
                        page: 0,
                        extended_page: 0,
                        mode: 0,
                        bound: 0,
                    },
                ],
                flip_flop: false,
                command: 0,
                mask: MASK_ALL_CHANNELS,
                status: 0,
            },
        }
    }

    /// Master reset: clears all registers and masks all channels.
    /// Ref: undoc98 `io_dma.txt` (I/O 001Bh)
    pub fn master_clear(&mut self) {
        self.state.flip_flop = false;
        self.state.command = 0;
        self.state.status = 0;
        self.state.mask = MASK_ALL_CHANNELS;
    }

    /// Reads a channel address register (alternates low/high byte).
    pub fn read_address(&mut self, channel: usize) -> u8 {
        let val = self.state.channels[channel].address;
        let byte = if self.state.flip_flop {
            (val >> 8) as u8
        } else {
            val as u8
        };
        self.state.flip_flop = !self.state.flip_flop;
        byte
    }

    /// Reads a channel count register (alternates low/high byte).
    pub fn read_count(&mut self, channel: usize) -> u8 {
        let val = self.state.channels[channel].count;
        let byte = if self.state.flip_flop {
            (val >> 8) as u8
        } else {
            val as u8
        };
        self.state.flip_flop = !self.state.flip_flop;
        byte
    }

    /// Writes a channel address register (alternates low/high byte).
    /// Also latches the start address for auto-init reload.
    pub fn write_address(&mut self, channel: usize, value: u8) {
        let ch = &mut self.state.channels[channel];
        if self.state.flip_flop {
            ch.address = (ch.address & 0x00FF) | ((value as u16) << 8);
        } else {
            ch.address = (ch.address & 0xFF00) | value as u16;
        }
        ch.start_address = ch.address;
        self.state.flip_flop = !self.state.flip_flop;
    }

    /// Writes a channel count register (alternates low/high byte).
    /// Also latches the start count for auto-init reload.
    pub fn write_count(&mut self, channel: usize, value: u8) {
        let ch = &mut self.state.channels[channel];
        if self.state.flip_flop {
            ch.count = (ch.count & 0x00FF) | ((value as u16) << 8);
        } else {
            ch.count = (ch.count & 0xFF00) | value as u16;
        }
        ch.start_count = ch.count;
        self.state.flip_flop = !self.state.flip_flop;
    }

    /// Reads the status register (clears TC bits on read).
    /// Ref: undoc98 `io_dma.txt` (I/O 0011h)
    pub fn read_status(&mut self) -> u8 {
        let s = self.state.status;
        self.state.status &= !STATUS_TC_MASK;
        s
    }

    /// Writes the command register.
    pub fn write_command(&mut self, value: u8) {
        self.state.command = value;
    }

    /// Writes the mode register for a channel (bits 1:0 = channel).
    /// Ref: undoc98 `io_dma.txt` (I/O 0017h)
    pub fn write_mode(&mut self, value: u8) {
        let channel = (value & CHANNEL_SELECT_MASK) as usize;
        self.state.channels[channel].mode = value;
    }

    /// Writes the single mask bit for a channel.
    /// Ref: undoc98 `io_dma.txt` (I/O 0015h)
    pub fn write_single_mask(&mut self, value: u8) {
        let channel = value & CHANNEL_SELECT_MASK;
        if value & SINGLE_MASK_SET_BIT != 0 {
            self.state.mask |= 1 << channel;
        } else {
            self.state.mask &= !(1 << channel);
        }
    }

    /// Clears the byte flip-flop.
    pub fn clear_flip_flop(&mut self) {
        self.state.flip_flop = false;
    }

    /// Reads the mask register.
    pub fn read_mask(&self) -> u8 {
        self.state.mask
    }

    /// Writes the all-mask register.
    /// Ref: undoc98 `io_dma.txt` (I/O 001Fh)
    pub fn write_all_mask(&mut self, value: u8) {
        self.state.mask = value & ALL_MASK_NIBBLE;
    }

    /// Reads a page register.
    pub fn read_page(&self, channel: usize) -> u8 {
        self.state.channels[channel].page
    }

    /// Writes a page register.
    pub fn write_page(&mut self, channel: usize, value: u8) {
        self.state.channels[channel].page = value;
    }

    /// Writes the auto-increment boundary mode register (port 0x29).
    /// Bits 1:0 select the channel, bits 3:2 select the boundary mode.
    /// Ref: undoc98 `io_dma.txt` (I/O 0029h)
    pub fn write_bound(&mut self, value: u8) {
        let channel = (value & BOUND_CHANNEL_MASK) as usize;
        self.state.channels[channel].bound = (value >> 2) & BOUND_MODE_MASK;
    }

    /// Writes the extended bank register (A24-A31) for a channel.
    /// Ref: undoc98 `io_dma.txt` (I/O 0E05h-0E0Bh)
    pub fn write_extended_page(&mut self, channel: usize, value: u8) {
        self.state.channels[channel].extended_page = value & EXTENDED_PAGE_MASK;
    }

    /// Returns true if the given channel is unmasked (enabled for transfer).
    pub fn channel_unmasked(&self, channel: usize) -> bool {
        self.state.mask & (1 << channel) == 0
    }

    /// Performs a write transfer (device->memory) on the given channel.
    ///
    /// Iterates through `data`, computing physical addresses from
    /// extended_page:page:address registers, incrementing or decrementing
    /// the address per mode bit 5, and decrementing the count register.
    /// When the 16-bit address wraps at 64KB boundaries, the page register
    /// is auto-incremented according to the channel's bound mode.
    ///
    /// Returns a list of (address, byte) pairs for the bus to write to
    /// memory, plus whether terminal count was reached.
    pub fn transfer_write_to_memory(&mut self, channel: usize, data: &[u8]) -> DmaTransferResult {
        let ch = &mut self.state.channels[channel];
        let increment = ch.mode & MODE_DECREMENT_BIT == 0;
        let mut writes = Vec::with_capacity(data.len());
        let mut terminal_count = false;

        for &byte in data {
            // Compute physical address: extended_page[6:0]:page[7:0]:address[15:0].
            let physical =
                ((ch.extended_page as u32) << 24) | ((ch.page as u32) << 16) | (ch.address as u32);
            writes.push((physical, byte));

            // Auto-increment bank when 8237 address is at boundary wrap point.
            if increment && ch.address == 0xFFFF {
                match ch.bound {
                    BOUND_MODE_1MB => {
                        ch.page = (ch.page.wrapping_add(1) & PAGE_LOW_NIBBLE_MASK)
                            | (ch.page & PAGE_HIGH_NIBBLE_MASK)
                    }
                    BOUND_MODE_16MB => ch.page = ch.page.wrapping_add(1),
                    _ => {}
                }
            }

            // Update address register.
            if increment {
                ch.address = ch.address.wrapping_add(1);
            } else {
                ch.address = ch.address.wrapping_sub(1);
            }

            // Decrement count (16-bit wrapping). TC when count wraps from 0 to 0xFFFF.
            if ch.count == 0 {
                terminal_count = true;
                self.state.status |= 1 << channel;
                if ch.mode & MODE_AUTO_INIT_BIT != 0 {
                    ch.address = ch.start_address;
                    ch.count = ch.start_count;
                } else {
                    ch.count = 0xFFFF;
                }
                break;
            } else {
                ch.count -= 1;
            }
        }

        DmaTransferResult {
            writes,
            terminal_count,
        }
    }

    /// Performs a read transfer (memory->device) on the given channel.
    ///
    /// Computes `byte_count` physical addresses from
    /// extended_page:page:address registers, advancing registers exactly
    /// like [`transfer_write_to_memory`](Self::transfer_write_to_memory).
    /// The bus reads memory at each returned address to assemble the data.
    pub fn transfer_read_from_memory(
        &mut self,
        channel: usize,
        byte_count: usize,
    ) -> DmaReadTransferResult {
        let ch = &mut self.state.channels[channel];
        let increment = ch.mode & MODE_DECREMENT_BIT == 0;
        let mut addresses = Vec::with_capacity(byte_count);
        let mut terminal_count = false;

        for _ in 0..byte_count {
            let physical =
                ((ch.extended_page as u32) << 24) | ((ch.page as u32) << 16) | (ch.address as u32);
            addresses.push(physical);

            if increment && ch.address == 0xFFFF {
                match ch.bound {
                    BOUND_MODE_1MB => {
                        ch.page = (ch.page.wrapping_add(1) & PAGE_LOW_NIBBLE_MASK)
                            | (ch.page & PAGE_HIGH_NIBBLE_MASK)
                    }
                    BOUND_MODE_16MB => ch.page = ch.page.wrapping_add(1),
                    _ => {}
                }
            }

            if increment {
                ch.address = ch.address.wrapping_add(1);
            } else {
                ch.address = ch.address.wrapping_sub(1);
            }

            if ch.count == 0 {
                terminal_count = true;
                self.state.status |= 1 << channel;
                if ch.mode & MODE_AUTO_INIT_BIT != 0 {
                    ch.address = ch.start_address;
                    ch.count = ch.start_count;
                } else {
                    ch.count = 0xFFFF;
                }
                break;
            } else {
                ch.count -= 1;
            }
        }

        DmaReadTransferResult {
            addresses,
            terminal_count,
        }
    }
}

/// Result of a DMA write transfer (device->memory).
pub struct DmaTransferResult {
    /// (physical_address, byte) pairs to write to memory.
    pub writes: Vec<(u32, u8)>,
    /// Whether the transfer reached terminal count (count wrapped to 0xFFFF).
    pub terminal_count: bool,
}

/// Result of a DMA read transfer (memory->device).
pub struct DmaReadTransferResult {
    /// Physical addresses to read from memory, in transfer order.
    pub addresses: Vec<u32>,
    /// Whether the transfer reached terminal count (count wrapped to 0xFFFF).
    pub terminal_count: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_increment_mode() {
        let mut dma = I8237Dma::new();
        dma.state.channels[2].page = 0x01;
        dma.state.channels[2].address = 0xFC00;
        dma.state.channels[2].count = 3; // Transfer 4 bytes (count+1)
        dma.state.channels[2].mode = 0x44; // Single mode, write, increment, channel 0

        let data = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        let result = dma.transfer_write_to_memory(2, &data);

        assert_eq!(result.writes.len(), 4);
        assert_eq!(result.writes[0], (0x01FC00, 0xAA));
        assert_eq!(result.writes[1], (0x01FC01, 0xBB));
        assert_eq!(result.writes[2], (0x01FC02, 0xCC));
        assert_eq!(result.writes[3], (0x01FC03, 0xDD));
        assert!(result.terminal_count);
    }

    #[test]
    fn transfer_decrement_mode() {
        let mut dma = I8237Dma::new();
        dma.state.channels[0].page = 0x00;
        dma.state.channels[0].address = 0x0003;
        dma.state.channels[0].count = 3;
        dma.state.channels[0].mode = 0x20; // Decrement mode

        let data = [0x11, 0x22, 0x33, 0x44];
        let result = dma.transfer_write_to_memory(0, &data);

        assert_eq!(result.writes.len(), 4);
        assert_eq!(result.writes[0], (0x000003, 0x11));
        assert_eq!(result.writes[1], (0x000002, 0x22));
        assert_eq!(result.writes[2], (0x000001, 0x33));
        assert_eq!(result.writes[3], (0x000000, 0x44));
        assert!(result.terminal_count);
    }

    #[test]
    fn transfer_partial_no_tc() {
        let mut dma = I8237Dma::new();
        dma.state.channels[1].page = 0x02;
        dma.state.channels[1].address = 0x0000;
        dma.state.channels[1].count = 0x03FF; // 1024 bytes
        dma.state.channels[1].mode = 0x45; // Increment

        let data = [0x01, 0x02, 0x03];
        let result = dma.transfer_write_to_memory(1, &data);

        assert_eq!(result.writes.len(), 3);
        assert!(!result.terminal_count);
        assert_eq!(dma.state.channels[1].address, 0x0003);
        assert_eq!(dma.state.channels[1].count, 0x03FC);
    }

    #[test]
    fn channel_mask() {
        let mut dma = I8237Dma::new();
        // All channels masked by default.
        assert!(!dma.channel_unmasked(0));
        assert!(!dma.channel_unmasked(2));

        // Unmask channel 2.
        dma.write_single_mask(0x02); // bit 2 = 0 (unmask), channel = 2
        assert!(dma.channel_unmasked(2));
        assert!(!dma.channel_unmasked(0));
    }

    #[test]
    fn transfer_auto_increment_1mb_boundary() {
        let mut dma = I8237Dma::new();
        dma.state.channels[0].page = 0x01;
        dma.state.channels[0].address = 0xFFFE;
        dma.state.channels[0].count = 3;
        dma.state.channels[0].mode = 0x40; // Increment
        dma.state.channels[0].bound = 1; // 1MB boundary

        let data = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        let result = dma.transfer_write_to_memory(0, &data);

        assert_eq!(result.writes.len(), 4);
        assert_eq!(result.writes[0], (0x01FFFE, 0xAA));
        assert_eq!(result.writes[1], (0x01FFFF, 0xBB)); // Page still 0x01
        // After 0xFFFF, page low nibble increments: 0x01 -> 0x02
        assert_eq!(result.writes[2], (0x020000, 0xCC)); // Page now 0x02
        assert_eq!(result.writes[3], (0x020001, 0xDD));
        assert!(result.terminal_count);
    }

    #[test]
    fn tc_sets_status_bit() {
        let mut dma = I8237Dma::new();
        dma.state.channels[3].count = 1; // 2 bytes
        dma.state.channels[3].mode = 0x47;

        let data = [0xAA, 0xBB, 0xCC];
        let result = dma.transfer_write_to_memory(3, &data);

        assert!(result.terminal_count);
        assert_eq!(dma.state.status & 0x08, 0x08); // TC bit for channel 3
    }

    #[test]
    fn read_transfer_increment_mode() {
        let mut dma = I8237Dma::new();
        dma.state.channels[2].page = 0x01;
        dma.state.channels[2].address = 0xFC00;
        dma.state.channels[2].count = 3; // Transfer 4 bytes (count+1)
        dma.state.channels[2].mode = 0x48; // Single mode, read, increment

        let result = dma.transfer_read_from_memory(2, 5);

        assert_eq!(result.addresses.len(), 4);
        assert_eq!(result.addresses[0], 0x01FC00);
        assert_eq!(result.addresses[1], 0x01FC01);
        assert_eq!(result.addresses[2], 0x01FC02);
        assert_eq!(result.addresses[3], 0x01FC03);
        assert!(result.terminal_count);
    }

    #[test]
    fn read_transfer_decrement_mode() {
        let mut dma = I8237Dma::new();
        dma.state.channels[0].page = 0x00;
        dma.state.channels[0].address = 0x0003;
        dma.state.channels[0].count = 3;
        dma.state.channels[0].mode = 0x20; // Decrement mode

        let result = dma.transfer_read_from_memory(0, 4);

        assert_eq!(result.addresses.len(), 4);
        assert_eq!(result.addresses[0], 0x000003);
        assert_eq!(result.addresses[1], 0x000002);
        assert_eq!(result.addresses[2], 0x000001);
        assert_eq!(result.addresses[3], 0x000000);
        assert!(result.terminal_count);
    }

    #[test]
    fn read_transfer_partial_no_tc() {
        let mut dma = I8237Dma::new();
        dma.state.channels[1].page = 0x02;
        dma.state.channels[1].address = 0x0000;
        dma.state.channels[1].count = 0x03FF; // 1024 bytes
        dma.state.channels[1].mode = 0x48; // Increment

        let result = dma.transfer_read_from_memory(1, 3);

        assert_eq!(result.addresses.len(), 3);
        assert!(!result.terminal_count);
        assert_eq!(dma.state.channels[1].address, 0x0003);
        assert_eq!(dma.state.channels[1].count, 0x03FC);
    }

    #[test]
    fn write_address_latches_start_address() {
        let mut dma = I8237Dma::new();
        dma.clear_flip_flop();
        dma.write_address(0, 0x00); // Low byte
        dma.write_address(0, 0x80); // High byte

        assert_eq!(dma.state.channels[0].address, 0x8000);
        assert_eq!(dma.state.channels[0].start_address, 0x8000);
    }

    #[test]
    fn write_count_latches_start_count() {
        let mut dma = I8237Dma::new();
        dma.clear_flip_flop();
        dma.write_count(0, 0xFF); // Low byte
        dma.write_count(0, 0x03); // High byte

        assert_eq!(dma.state.channels[0].count, 0x03FF);
        assert_eq!(dma.state.channels[0].start_count, 0x03FF);
    }

    #[test]
    fn write_transfer_auto_init_reloads_on_tc() {
        let mut dma = I8237Dma::new();
        let ch = 2;
        dma.state.channels[ch].page = 0x01;
        dma.state.channels[ch].address = 0x1000;
        dma.state.channels[ch].count = 3;
        dma.state.channels[ch].start_address = 0x1000;
        dma.state.channels[ch].start_count = 3;
        dma.state.channels[ch].mode = 0x54; // Write, increment, auto-init (bit 4 set)

        let data = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        let result = dma.transfer_write_to_memory(ch, &data);

        assert_eq!(result.writes.len(), 4);
        assert!(result.terminal_count);
        // After auto-init TC, address and count are reloaded from start values.
        assert_eq!(dma.state.channels[ch].address, 0x1000);
        assert_eq!(dma.state.channels[ch].count, 3);
    }

    #[test]
    fn write_transfer_no_auto_init_wraps_count() {
        let mut dma = I8237Dma::new();
        let ch = 2;
        dma.state.channels[ch].address = 0x1000;
        dma.state.channels[ch].count = 3;
        dma.state.channels[ch].start_address = 0x1000;
        dma.state.channels[ch].start_count = 3;
        dma.state.channels[ch].mode = 0x44; // Write, increment, NO auto-init

        let data = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        let result = dma.transfer_write_to_memory(ch, &data);

        assert!(result.terminal_count);
        // Without auto-init, count wraps to 0xFFFF and address stays advanced.
        assert_eq!(dma.state.channels[ch].count, 0xFFFF);
        assert_eq!(dma.state.channels[ch].address, 0x1004);
    }

    #[test]
    fn read_transfer_auto_init_reloads_on_tc() {
        let mut dma = I8237Dma::new();
        let ch = 1;
        dma.state.channels[ch].page = 0x02;
        dma.state.channels[ch].address = 0x5000;
        dma.state.channels[ch].count = 2;
        dma.state.channels[ch].start_address = 0x5000;
        dma.state.channels[ch].start_count = 2;
        dma.state.channels[ch].mode = 0x58; // Read, increment, auto-init (bit 4 set)

        let result = dma.transfer_read_from_memory(ch, 5);

        assert_eq!(result.addresses.len(), 3);
        assert!(result.terminal_count);
        assert_eq!(dma.state.channels[ch].address, 0x5000);
        assert_eq!(dma.state.channels[ch].count, 2);
    }

    #[test]
    fn read_transfer_no_auto_init_wraps_count() {
        let mut dma = I8237Dma::new();
        let ch = 1;
        dma.state.channels[ch].address = 0x5000;
        dma.state.channels[ch].count = 2;
        dma.state.channels[ch].start_address = 0x5000;
        dma.state.channels[ch].start_count = 2;
        dma.state.channels[ch].mode = 0x48; // Read, increment, NO auto-init

        let result = dma.transfer_read_from_memory(ch, 5);

        assert!(result.terminal_count);
        assert_eq!(dma.state.channels[ch].count, 0xFFFF);
        assert_eq!(dma.state.channels[ch].address, 0x5003);
    }

    #[test]
    fn auto_init_allows_repeated_transfers() {
        let mut dma = I8237Dma::new();
        let ch = 0;
        dma.state.channels[ch].address = 0x2000;
        dma.state.channels[ch].count = 1; // 2 bytes per cycle
        dma.state.channels[ch].start_address = 0x2000;
        dma.state.channels[ch].start_count = 1;
        dma.state.channels[ch].mode = 0x50; // Increment, auto-init

        // First transfer: 2 bytes then TC.
        let result1 = dma.transfer_write_to_memory(ch, &[0x11, 0x22, 0x33]);
        assert_eq!(result1.writes.len(), 2);
        assert!(result1.terminal_count);
        assert_eq!(dma.state.channels[ch].address, 0x2000);
        assert_eq!(dma.state.channels[ch].count, 1);

        // Clear TC status so we can observe it set again.
        dma.read_status();

        // Second transfer from the same reloaded state.
        let result2 = dma.transfer_write_to_memory(ch, &[0xAA, 0xBB, 0xCC]);
        assert_eq!(result2.writes.len(), 2);
        assert!(result2.terminal_count);
        assert_eq!(result2.writes[0], (0x002000, 0xAA));
        assert_eq!(result2.writes[1], (0x002001, 0xBB));
        assert_eq!(dma.state.channels[ch].address, 0x2000);
        assert_eq!(dma.state.channels[ch].count, 1);
        // TC status bit for channel 0 is set again.
        assert_eq!(dma.state.status & 0x01, 0x01);
    }

    #[test]
    fn auto_init_tc_sets_status_bit() {
        let mut dma = I8237Dma::new();
        let ch = 3;
        dma.state.channels[ch].address = 0x0000;
        dma.state.channels[ch].count = 0;
        dma.state.channels[ch].start_address = 0x0000;
        dma.state.channels[ch].start_count = 0;
        dma.state.channels[ch].mode = 0x57; // Auto-init, increment, channel 3

        let result = dma.transfer_write_to_memory(ch, &[0xFF]);
        assert!(result.terminal_count);
        assert_eq!(dma.state.status & 0x08, 0x08);
    }

    #[test]
    fn start_values_latch_after_each_write() {
        let mut dma = I8237Dma::new();
        // Program address as 0x1234.
        dma.clear_flip_flop();
        dma.write_address(0, 0x34); // Low byte -> address = 0x0034, start = 0x0034
        assert_eq!(dma.state.channels[0].start_address, 0x0034);
        dma.write_address(0, 0x12); // High byte -> address = 0x1234, start = 0x1234
        assert_eq!(dma.state.channels[0].start_address, 0x1234);

        // Program count as 0x00FF.
        dma.clear_flip_flop();
        dma.write_count(0, 0xFF); // Low byte -> count = 0x00FF, start = 0x00FF
        assert_eq!(dma.state.channels[0].start_count, 0x00FF);
        dma.write_count(0, 0x00); // High byte -> count = 0x00FF, start = 0x00FF
        assert_eq!(dma.state.channels[0].start_count, 0x00FF);
    }
}
