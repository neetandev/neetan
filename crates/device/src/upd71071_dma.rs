//! uPD71071 DMA controller.
//!
//! A four-channel controller with a full 32-bit memory address, base/current
//! double-buffered address and count registers, per-channel terminal-count
//! status, and 8- or 16-bit transfer width. The register file is decoded from a
//! 16-byte port window (`port & 0x0F`); a machine wires its own base port range
//! to that window.
//!
//! Transfers are exposed as address/byte lists so the owning bus performs the
//! actual memory accesses; a block-transfer helper (`address` / `transfer_length`
//! / `advance`) supports consumers that stream a whole device operation at once.

/// Register offsets within the 16-byte port window (`port & REGISTER_OFFSET_MASK`).
const REGISTER_OFFSET_MASK: u16 = 0x0F;
const REGISTER_INITIALIZE: u16 = 0x00;
const REGISTER_CHANNEL: u16 = 0x01;
const REGISTER_COUNT_LOW: u16 = 0x02;
const REGISTER_COUNT_HIGH: u16 = 0x03;
const REGISTER_ADDRESS_BYTE0: u16 = 0x04;
const REGISTER_ADDRESS_BYTE1: u16 = 0x05;
const REGISTER_ADDRESS_BYTE2: u16 = 0x06;
const REGISTER_ADDRESS_BYTE3: u16 = 0x07;
const REGISTER_DEVICE_CONTROL_LOW: u16 = 0x08;
const REGISTER_DEVICE_CONTROL_HIGH: u16 = 0x09;
const REGISTER_MODE_CONTROL: u16 = 0x0A;
const REGISTER_STATUS: u16 = 0x0B;
const REGISTER_REQUEST: u16 = 0x0E;
const REGISTER_MASK: u16 = 0x0F;

/// Initialize register (0x00): bit 0 resets the controller, bit 1 selects 16-bit
/// transfer width (clear = 8-bit).
const INITIALIZE_RESET_BIT: u8 = 0x01;
const INITIALIZE_16BIT_BIT: u8 = 0x02;

/// Channel register (0x01): bits 1:0 select the channel, bit 2 selects the base
/// register bank (set = base, clear = current). Reads return a one-hot channel
/// select with the base flag in bit 4.
const CHANNEL_SELECT_MASK: u8 = 0x03;
const CHANNEL_BASE_BIT: u8 = 0x04;
const CHANNEL_READ_BASE_FLAG: u8 = 0x10;

/// Mode-control register (0x0A): bit 0 selects word transfers, bit 4 enables
/// auto-initialize (reload address and count from the base registers at
/// terminal count). Bit 1 reads back as zero.
const MODE_WORD_TRANSFER_BIT: u8 = 0x01;
const MODE_AUTO_INITIALIZE_BIT: u8 = 0x10;
const MODE_READ_MASK: u8 = 0xFD;

/// Device-control low (0x08) keeps only bit 3; device-control high (0x09) reads
/// back only its low two bits.
const DEVICE_CONTROL_LOW_MASK: u8 = 0x08;
const DEVICE_CONTROL_HIGH_READ_MASK: u8 = 0x03;

/// Power-on and master-clear mask value (all four channels masked).
const MASK_ALL_CHANNELS: u8 = 0x0F;

/// State of a single DMA channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upd71071ChannelState {
    /// Base count register, reloaded into the current count on auto-initialize.
    pub base_count: u16,
    /// Current count register (transfer length minus one), decremented per unit.
    pub current_count: u16,
    /// Base address register, reloaded into the current address on auto-initialize.
    pub base_address: u32,
    /// Current 32-bit memory address, advanced during transfer.
    pub current_address: u32,
    /// Mode-control register for this channel.
    pub mode: u8,
    /// Terminal-count latch, set when a transfer completes or a ~END is signalled.
    pub terminal_count: bool,
}

impl Upd71071ChannelState {
    const fn new() -> Self {
        Self {
            base_count: 0,
            current_count: 0,
            base_address: 0,
            current_address: 0,
            mode: 0,
            terminal_count: false,
        }
    }

    /// Transfer width in bytes per count decrement (1 for byte, 2 for word).
    fn bytes_per_count(&self) -> usize {
        if self.mode & MODE_WORD_TRANSFER_BIT != 0 {
            2
        } else {
            1
        }
    }

    fn auto_initialize(&self) -> bool {
        self.mode & MODE_AUTO_INITIALIZE_BIT != 0
    }

    fn reload_from_base(&mut self) {
        self.current_address = self.base_address;
        self.current_count = self.base_count;
    }
}

/// Snapshot of the DMA controller for save/restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upd71071State {
    /// The four DMA channels.
    pub channels: [Upd71071ChannelState; 4],
    /// 16-bit transfer width selected by the initialize register (else 8-bit).
    pub transfer_16bit: bool,
    /// Base-register bank flag (true selects the base registers for access).
    pub base_bank: bool,
    /// Currently selected channel for register access.
    pub selected_channel: usize,
    /// Device-control register (low, high).
    pub device_control: [u8; 2],
    /// Software request register.
    pub request: u8,
    /// Channel mask register (bit per channel; 1 = masked).
    pub mask: u8,
}

/// uPD71071 DMA controller.
pub struct Upd71071Dma {
    /// Embedded state for save/restore.
    pub state: Upd71071State,
}

impl Default for Upd71071Dma {
    fn default() -> Self {
        Self::new()
    }
}

impl Upd71071Dma {
    /// Creates a controller in its reset state (16-bit width, all channels masked).
    pub fn new() -> Self {
        Self {
            state: Upd71071State {
                channels: [const { Upd71071ChannelState::new() }; 4],
                transfer_16bit: true,
                base_bank: true,
                selected_channel: 0,
                device_control: [0; 2],
                request: 0,
                mask: MASK_ALL_CHANNELS,
            },
        }
    }

    /// Master reset: clears every register and masks all channels.
    pub fn reset(&mut self) {
        self.state = Self::new().state;
    }

    /// Writes a register selected by `port & 0x0F`.
    pub fn write(&mut self, port: u16, value: u8) {
        let channel = self.state.selected_channel;
        match port & REGISTER_OFFSET_MASK {
            REGISTER_INITIALIZE => {
                if value & INITIALIZE_RESET_BIT != 0 {
                    self.reset();
                    return;
                }
                self.state.transfer_16bit = value & INITIALIZE_16BIT_BIT != 0;
            }
            REGISTER_CHANNEL => {
                self.state.base_bank = value & CHANNEL_BASE_BIT != 0;
                self.state.selected_channel = (value & CHANNEL_SELECT_MASK) as usize;
            }
            REGISTER_COUNT_LOW => self.write_count_byte(channel, 0, value),
            REGISTER_COUNT_HIGH => self.write_count_byte(channel, 8, value),
            REGISTER_ADDRESS_BYTE0 => self.write_address_byte(channel, 0, value),
            REGISTER_ADDRESS_BYTE1 => self.write_address_byte(channel, 8, value),
            REGISTER_ADDRESS_BYTE2 => self.write_address_byte(channel, 16, value),
            REGISTER_ADDRESS_BYTE3 => self.write_address_byte(channel, 24, value),
            REGISTER_DEVICE_CONTROL_LOW => {
                self.state.device_control[0] = value & DEVICE_CONTROL_LOW_MASK
            }
            REGISTER_DEVICE_CONTROL_HIGH => self.state.device_control[1] = value,
            REGISTER_MODE_CONTROL => self.state.channels[channel].mode = value,
            REGISTER_REQUEST => self.state.request = value,
            REGISTER_MASK => self.state.mask = value,
            _ => {}
        }
    }

    /// Reads a register selected by `port & 0x0F`.
    pub fn read(&mut self, port: u16) -> u8 {
        let channel = self.state.selected_channel;
        match port & REGISTER_OFFSET_MASK {
            REGISTER_CHANNEL => {
                (1 << self.state.selected_channel)
                    | if self.state.base_bank {
                        CHANNEL_READ_BASE_FLAG
                    } else {
                        0
                    }
            }
            REGISTER_COUNT_LOW => self.selected_count(channel) as u8,
            REGISTER_COUNT_HIGH => (self.selected_count(channel) >> 8) as u8,
            REGISTER_ADDRESS_BYTE0 => self.selected_address(channel) as u8,
            REGISTER_ADDRESS_BYTE1 => (self.selected_address(channel) >> 8) as u8,
            REGISTER_ADDRESS_BYTE2 => (self.selected_address(channel) >> 16) as u8,
            REGISTER_ADDRESS_BYTE3 => (self.selected_address(channel) >> 24) as u8,
            REGISTER_DEVICE_CONTROL_LOW => self.state.device_control[0],
            REGISTER_DEVICE_CONTROL_HIGH => {
                self.state.device_control[1] & DEVICE_CONTROL_HIGH_READ_MASK
            }
            REGISTER_MODE_CONTROL => self.state.channels[channel].mode & MODE_READ_MASK,
            REGISTER_STATUS => self.read_status(),
            REGISTER_MASK => self.state.mask,
            _ => 0xFF,
        }
    }

    /// Reads the status register and clears every channel's terminal-count latch.
    fn read_status(&mut self) -> u8 {
        let mut status = 0;
        for (index, channel) in self.state.channels.iter_mut().enumerate() {
            if channel.terminal_count {
                status |= 1 << index;
            }
            channel.terminal_count = false;
        }
        status
    }

    fn write_count_byte(&mut self, channel: usize, shift: u32, value: u8) {
        let channel = &mut self.state.channels[channel];
        if self.state.base_bank {
            channel.base_count = replace_byte(channel.base_count, shift, value);
        } else {
            channel.current_count = replace_byte(channel.current_count, shift, value);
            channel.base_count = channel.current_count;
            channel.terminal_count = false;
        }
    }

    fn write_address_byte(&mut self, channel: usize, shift: u32, value: u8) {
        let channel = &mut self.state.channels[channel];
        if self.state.base_bank {
            channel.base_address = replace_byte_u32(channel.base_address, shift, value);
        } else {
            channel.current_address = replace_byte_u32(channel.current_address, shift, value);
            channel.base_address = channel.current_address;
        }
    }

    fn selected_count(&self, channel: usize) -> u16 {
        if self.state.base_bank {
            self.state.channels[channel].base_count
        } else {
            self.state.channels[channel].current_count
        }
    }

    fn selected_address(&self, channel: usize) -> u32 {
        if self.state.base_bank {
            self.state.channels[channel].base_address
        } else {
            self.state.channels[channel].current_address
        }
    }

    /// The current 32-bit memory address for a channel.
    pub fn address(&self, channel: usize) -> u32 {
        self.state.channels[channel].current_address
    }

    /// The number of units a channel will transfer (current count plus one).
    pub fn transfer_length(&self, channel: usize) -> usize {
        usize::from(self.state.channels[channel].current_count) + 1
    }

    /// True when the channel is unmasked (enabled for transfer).
    pub fn channel_unmasked(&self, channel: usize) -> bool {
        self.state.mask & (1 << channel) == 0
    }

    /// The channel's latched terminal-count status.
    pub fn terminal_count(&self, channel: usize) -> bool {
        self.state.channels[channel].terminal_count
    }

    /// Signals ~END for a channel, latching its terminal-count status.
    pub fn set_terminal_count(&mut self, channel: usize) {
        self.state.channels[channel].terminal_count = true;
    }

    /// Records a completed block transfer of `bytes` on a channel: advances the
    /// current address, exhausts the count to its terminal value, and latches
    /// terminal count. Consumers that stream a whole device operation in one go
    /// (rather than driving [`transfer_write_to_memory`]) use this to resync the
    /// register state.
    pub fn advance(&mut self, channel: usize, bytes: usize) {
        let channel = &mut self.state.channels[channel];
        channel.current_address = channel.current_address.wrapping_add(bytes as u32);
        channel.current_count = 0xFFFF;
        channel.terminal_count = true;
    }

    /// Performs a device-to-memory transfer: consumes `data` and returns the
    /// (address, byte) writes the bus must apply, plus whether terminal count was
    /// reached. Word transfers advance the address by two per count decrement.
    pub fn transfer_write_to_memory(&mut self, channel: usize, data: &[u8]) -> Upd71071WriteResult {
        let mut writes = Vec::with_capacity(data.len());
        let terminal_count = self.step_transfer(channel, data.len(), |channel, index| {
            writes.push((channel.current_address, data[index]));
            channel.current_address = channel.current_address.wrapping_add(1);
        });
        Upd71071WriteResult {
            writes,
            terminal_count,
        }
    }

    /// Performs a memory-to-device transfer: returns the addresses the bus must
    /// read (in transfer order) to assemble `byte_count` bytes, plus whether
    /// terminal count was reached.
    pub fn transfer_read_from_memory(
        &mut self,
        channel: usize,
        byte_count: usize,
    ) -> Upd71071ReadResult {
        let mut addresses = Vec::with_capacity(byte_count);
        let terminal_count = self.step_transfer(channel, byte_count, |channel, _index| {
            addresses.push(channel.current_address);
            channel.current_address = channel.current_address.wrapping_add(1);
        });
        Upd71071ReadResult {
            addresses,
            terminal_count,
        }
    }

    /// Drives `byte_count` bytes through a channel, invoking `handle_byte` for
    /// each, decrementing the count once per transfer unit and reloading from the
    /// base registers on terminal count when auto-initialize is enabled. Returns
    /// whether terminal count was reached.
    fn step_transfer(
        &mut self,
        channel: usize,
        byte_count: usize,
        mut handle_byte: impl FnMut(&mut Upd71071ChannelState, usize),
    ) -> bool {
        let unit = self.state.channels[channel].bytes_per_count();
        let channel = &mut self.state.channels[channel];
        let mut terminal_count = false;
        let mut index = 0;
        while index + unit <= byte_count && !terminal_count {
            for lane in 0..unit {
                handle_byte(channel, index + lane);
            }
            if channel.current_count == 0 {
                terminal_count = true;
            }
            channel.current_count = channel.current_count.wrapping_sub(1);
            index += unit;
        }
        if index > 0 {
            if terminal_count {
                channel.terminal_count = true;
            }
            if terminal_count && channel.auto_initialize() {
                channel.reload_from_base();
            }
        }
        terminal_count
    }
}

fn replace_byte(value: u16, shift: u32, byte: u8) -> u16 {
    let mask = !(0xFFu16 << shift);
    (value & mask) | ((u16::from(byte)) << shift)
}

fn replace_byte_u32(value: u32, shift: u32, byte: u8) -> u32 {
    let mask = !(0xFFu32 << shift);
    (value & mask) | ((u32::from(byte)) << shift)
}

/// Result of a device-to-memory transfer.
pub struct Upd71071WriteResult {
    /// (physical_address, byte) pairs the bus must write to memory.
    pub writes: Vec<(u32, u8)>,
    /// Whether the transfer reached terminal count.
    pub terminal_count: bool,
}

/// Result of a memory-to-device transfer.
pub struct Upd71071ReadResult {
    /// Physical addresses the bus must read, in transfer order.
    pub addresses: Vec<u32>,
    /// Whether the transfer reached terminal count.
    pub terminal_count: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Programs channel 2 with a 32-bit address and a 16-bit count through the
    /// current-register bank, then reads every register back.
    #[test]
    fn channel_address_and_count_round_trip() {
        let mut dma = Upd71071Dma::new();
        dma.write(0x01, 0x02); // select channel 2, current bank
        dma.write(0x02, 0xFF); // count low
        dma.write(0x03, 0x03); // count high -> 0x03FF
        dma.write(0x04, 0x00); // address byte 0
        dma.write(0x05, 0x04); // address byte 1
        dma.write(0x06, 0x03); // address byte 2
        dma.write(0x07, 0x12); // address byte 3 -> 0x12030400

        // The channel register reads back one-hot with the base flag clear.
        assert_eq!(dma.read(0x01), 1 << 2);
        assert_eq!(dma.read(0x02), 0xFF);
        assert_eq!(dma.read(0x03), 0x03);
        assert_eq!(dma.read(0x04), 0x00);
        assert_eq!(dma.read(0x05), 0x04);
        assert_eq!(dma.read(0x06), 0x03);
        assert_eq!(dma.read(0x07), 0x12);
        assert_eq!(dma.address(2), 0x1203_0400);
        assert_eq!(dma.transfer_length(2), 0x0400);
    }

    /// Writing the current registers copies the value into the base bank so
    /// auto-initialize can reload it.
    #[test]
    fn current_write_latches_base() {
        let mut dma = Upd71071Dma::new();
        dma.write(0x01, 0x00); // channel 0, current bank
        dma.write(0x02, 0x10);
        dma.write(0x03, 0x00);
        dma.write(0x04, 0x20);

        dma.write(0x01, 0x04); // channel 0, base bank
        assert_eq!(dma.read(0x02), 0x10);
        assert_eq!(dma.read(0x04), 0x20);
    }

    /// Register writes target only the selected channel.
    #[test]
    fn register_writes_are_per_selected_channel() {
        let mut dma = Upd71071Dma::new();
        dma.write(0x01, 0x00);
        dma.write(0x02, 0x10);
        dma.write(0x04, 0x20);

        dma.write(0x01, 0x02);
        dma.write(0x02, 0x99);
        dma.write(0x04, 0x88);

        assert_eq!(dma.address(0), 0x20);
        assert_eq!(dma.transfer_length(0), 0x11);
        assert_eq!(dma.address(2), 0x88);
        assert_eq!(dma.transfer_length(2), 0x9A);
    }

    /// `advance` moves the address forward by the transferred byte count (32-bit
    /// wrap), exhausts the count, and latches terminal count.
    #[test]
    fn advance_moves_address_and_exhausts_count() {
        let mut dma = Upd71071Dma::new();
        dma.write(0x01, 0x02);
        dma.write(0x04, 0xFE);
        dma.write(0x05, 0xFF);
        dma.write(0x06, 0xFF);
        dma.write(0x07, 0xFF); // address 0xFFFFFFFE
        dma.write(0x02, 0xFF);
        dma.write(0x03, 0x03); // count 0x03FF

        dma.advance(2, 4);
        assert_eq!(dma.address(2), 0x0000_0002); // wrapped within 32 bits
        assert!(dma.terminal_count(2));
    }

    /// The mask register round-trips its full byte; the initialize reset bit
    /// masks all channels again.
    #[test]
    fn mask_register_and_initialize_reset() {
        let mut dma = Upd71071Dma::new();
        assert_eq!(dma.read(0x0F), MASK_ALL_CHANNELS);

        dma.write(0x0F, 0x0B); // unmask channel 2
        assert_eq!(dma.read(0x0F), 0x0B);
        assert!(dma.channel_unmasked(2));
        assert!(!dma.channel_unmasked(0));

        dma.write(0x01, 0x01);
        dma.write(0x00, INITIALIZE_RESET_BIT);
        assert_eq!(dma.read(0x0F), MASK_ALL_CHANNELS);
        // Reset selects channel 0 with the base register bank active.
        assert_eq!(dma.read(0x01), (1 << 0) | CHANNEL_READ_BASE_FLAG);
    }

    /// A byte transfer streams count+1 bytes into memory and latches terminal
    /// count in the status register.
    #[test]
    fn byte_transfer_reaches_terminal_count() {
        let mut dma = Upd71071Dma::new();
        dma.write(0x01, 0x02); // channel 2, current bank
        dma.write(0x0A, 0x00); // byte transfers
        dma.state.channels[2].current_address = 0x0010_0000;
        dma.state.channels[2].current_count = 3; // 4 bytes

        let result = dma.transfer_write_to_memory(2, &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
        assert_eq!(result.writes.len(), 4);
        assert_eq!(result.writes[0], (0x0010_0000, 0xAA));
        assert_eq!(result.writes[3], (0x0010_0003, 0xDD));
        assert!(result.terminal_count);

        // Terminal count is visible in the status register and clears on read.
        assert_eq!(dma.read(0x0B), 1 << 2);
        assert_eq!(dma.read(0x0B), 0);
    }

    /// A word transfer advances the address two bytes per count decrement.
    #[test]
    fn word_transfer_steps_two_bytes_per_count() {
        let mut dma = Upd71071Dma::new();
        dma.write(0x01, 0x00); // channel 0, current bank
        dma.write(0x0A, MODE_WORD_TRANSFER_BIT); // word transfers
        dma.state.channels[0].current_address = 0x2000;
        dma.state.channels[0].current_count = 1; // 2 words = 4 bytes

        let result = dma.transfer_write_to_memory(0, &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(result.writes.len(), 4);
        assert_eq!(result.writes[0], (0x2000, 0x11));
        assert_eq!(result.writes[3], (0x2003, 0x44));
        assert!(result.terminal_count);
    }

    /// A partial transfer that does not reach terminal count leaves the address
    /// and count advanced.
    #[test]
    fn partial_transfer_no_terminal_count() {
        let mut dma = Upd71071Dma::new();
        dma.write(0x01, 0x01); // channel 1, current bank
        dma.state.channels[1].current_address = 0x3000;
        dma.state.channels[1].current_count = 0x03FF;

        let result = dma.transfer_write_to_memory(1, &[0x01, 0x02, 0x03]);
        assert_eq!(result.writes.len(), 3);
        assert!(!result.terminal_count);
        assert_eq!(dma.state.channels[1].current_address, 0x3003);
        assert_eq!(dma.state.channels[1].current_count, 0x03FC);
        assert!(!dma.terminal_count(1));
    }

    /// Auto-initialize reloads both address and count from the base bank at
    /// terminal count.
    #[test]
    fn auto_initialize_reloads_address_and_count() {
        let mut dma = Upd71071Dma::new();
        dma.write(0x01, 0x02); // channel 2, current bank
        dma.write(0x0A, MODE_AUTO_INITIALIZE_BIT);
        dma.state.channels[2].current_address = 0x1000;
        dma.state.channels[2].base_address = 0x1000;
        dma.state.channels[2].current_count = 3;
        dma.state.channels[2].base_count = 3;

        let result = dma.transfer_write_to_memory(2, &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
        assert_eq!(result.writes.len(), 4);
        assert!(result.terminal_count);
        assert_eq!(dma.state.channels[2].current_address, 0x1000);
        assert_eq!(dma.state.channels[2].current_count, 3);
    }

    /// Without auto-initialize the count wraps to its terminal value and the
    /// address stays advanced.
    #[test]
    fn no_auto_initialize_wraps_count() {
        let mut dma = Upd71071Dma::new();
        dma.write(0x01, 0x02); // channel 2, current bank
        dma.state.channels[2].current_address = 0x1000;
        dma.state.channels[2].current_count = 3;

        let result = dma.transfer_write_to_memory(2, &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
        assert!(result.terminal_count);
        assert_eq!(dma.state.channels[2].current_count, 0xFFFF);
        assert_eq!(dma.state.channels[2].current_address, 0x1004);
    }

    /// Memory-to-device transfers return the read addresses in order.
    #[test]
    fn read_transfer_returns_addresses() {
        let mut dma = Upd71071Dma::new();
        dma.write(0x01, 0x03); // channel 3, current bank
        dma.state.channels[3].current_address = 0x4000;
        dma.state.channels[3].current_count = 2; // 3 bytes

        let result = dma.transfer_read_from_memory(3, 5);
        assert_eq!(result.addresses, vec![0x4000, 0x4001, 0x4002]);
        assert!(result.terminal_count);
    }

    /// A ~END signal latches terminal count without any byte transfer.
    #[test]
    fn set_terminal_count_latches_status() {
        let mut dma = Upd71071Dma::new();
        dma.set_terminal_count(1);
        assert!(dma.terminal_count(1));
        assert_eq!(dma.read(0x0B), 1 << 1);
    }
}
