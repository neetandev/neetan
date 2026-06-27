//! µPD71071 DMA controller (I/O ports 0x160-0x16F).
//!
//! The PC-88VA2 wires DMA channel 2 to the uPD765A FDC for native-mode (DMA)
//! floppy transfers. Only the register file needed to drive that path is
//! modelled: the controller does not free-run, the bus performs the channel-2
//! transfer in a block when the FDC starts a data command.

const CHANNEL_COUNT: usize = 4;

/// Minimal µPD71071 register file.
pub(crate) struct Dmac71071 {
    /// Currently selected channel for count/address/mode register access.
    selected_channel: usize,
    /// Per-channel current 24-bit address.
    address: [u32; CHANNEL_COUNT],
    /// Per-channel current 16-bit count (transfer length minus one).
    count: [u16; CHANNEL_COUNT],
    /// Per-channel mode register.
    mode: [u8; CHANNEL_COUNT],
    /// Per-channel mask bit (1 = masked / disabled).
    mask: u8,
    /// 16-bit device-control register.
    device_control: u16,
}

impl Dmac71071 {
    pub(crate) fn new() -> Self {
        Self {
            selected_channel: 0,
            address: [0; CHANNEL_COUNT],
            count: [0; CHANNEL_COUNT],
            mode: [0; CHANNEL_COUNT],
            mask: 0x0F,
            device_control: 0,
        }
    }

    pub(crate) fn write(&mut self, port: u16, value: u8) {
        let channel = self.selected_channel;
        match port & 0x0F {
            0x00 => {
                // Initialize: bit 0 resets the controller.
                if value & 0x01 != 0 {
                    *self = Self::new();
                }
            }
            0x01 => self.selected_channel = (value & 0x03) as usize,
            0x02 => self.count[channel] = (self.count[channel] & 0xFF00) | u16::from(value),
            0x03 => self.count[channel] = (self.count[channel] & 0x00FF) | (u16::from(value) << 8),
            0x04 => self.address[channel] = (self.address[channel] & 0xFF_FF00) | u32::from(value),
            0x05 => {
                self.address[channel] =
                    (self.address[channel] & 0xFF_00FF) | (u32::from(value) << 8)
            }
            0x06 => {
                self.address[channel] =
                    (self.address[channel] & 0x00_FFFF) | (u32::from(value) << 16)
            }
            0x07 => {}
            0x08 => self.device_control = (self.device_control & 0xFF00) | u16::from(value),
            0x09 => self.device_control = (self.device_control & 0x00FF) | (u16::from(value) << 8),
            0x0A => self.mode[channel] = value,
            0x0E => {} // request register: software requests are unused
            0x0F => self.mask = value & 0x0F,
            _ => {}
        }
    }

    pub(crate) fn read(&self, port: u16) -> u8 {
        let channel = self.selected_channel;
        match port & 0x0F {
            0x01 => self.selected_channel as u8,
            0x02 => self.count[channel] as u8,
            0x03 => (self.count[channel] >> 8) as u8,
            0x04 => self.address[channel] as u8,
            0x05 => (self.address[channel] >> 8) as u8,
            0x06 => (self.address[channel] >> 16) as u8,
            0x08 => self.device_control as u8,
            0x09 => (self.device_control >> 8) as u8,
            0x0A => self.mode[channel],
            0x0F => self.mask | 0xF0,
            _ => 0xFF,
        }
    }

    /// The current 24-bit memory address for a channel.
    pub(crate) fn address(&self, channel: usize) -> u32 {
        self.address[channel]
    }

    /// The number of bytes a channel will transfer (count register plus one).
    pub(crate) fn transfer_length(&self, channel: usize) -> usize {
        usize::from(self.count[channel]) + 1
    }

    /// Records a completed transfer of `bytes` on a channel: advances the
    /// address and exhausts the count, mirroring the terminal-count state the
    /// driver reads back.
    pub(crate) fn advance(&mut self, channel: usize, bytes: usize) {
        self.address[channel] = self.address[channel].wrapping_add(bytes as u32) & 0xFF_FFFF;
        self.count[channel] = 0xFFFF;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Programs channel 2 with a 24-bit address and a 16-bit count (the disk
    /// BIOS sequence), then reads every register back for the selected channel.
    #[test]
    fn channel_address_and_count_round_trip() {
        let mut dmac = Dmac71071::new();
        dmac.write(0x161, 0x02); // select channel 2
        dmac.write(0x162, 0xFF); // count low
        dmac.write(0x163, 0x03); // count high -> 0x03FF
        dmac.write(0x164, 0x00); // address bits 0-7
        dmac.write(0x165, 0x04); // address bits 8-15
        dmac.write(0x166, 0x03); // address bits 16-23 -> 0x030400

        assert_eq!(dmac.read(0x161), 0x02);
        assert_eq!(dmac.read(0x162), 0xFF);
        assert_eq!(dmac.read(0x163), 0x03);
        assert_eq!(dmac.read(0x164), 0x00);
        assert_eq!(dmac.read(0x165), 0x04);
        assert_eq!(dmac.read(0x166), 0x03);
        assert_eq!(dmac.address(2), 0x03_0400);
        assert_eq!(dmac.transfer_length(2), 0x0400);
    }

    /// Register writes target only the selected channel; selecting another
    /// channel leaves the first one's address and count intact.
    #[test]
    fn register_writes_are_per_selected_channel() {
        let mut dmac = Dmac71071::new();
        dmac.write(0x161, 0x00);
        dmac.write(0x162, 0x10);
        dmac.write(0x164, 0x20);

        dmac.write(0x161, 0x02);
        dmac.write(0x162, 0x99);
        dmac.write(0x164, 0x88);

        assert_eq!(dmac.address(0), 0x20);
        assert_eq!(dmac.transfer_length(0), 0x11);
        assert_eq!(dmac.address(2), 0x88);
        assert_eq!(dmac.transfer_length(2), 0x9A);
    }

    /// `advance` moves the address forward by the transferred byte count
    /// (24-bit wrap) and exhausts the count to its terminal value.
    #[test]
    fn advance_moves_address_and_exhausts_count() {
        let mut dmac = Dmac71071::new();
        dmac.write(0x161, 0x02);
        dmac.write(0x164, 0xFE);
        dmac.write(0x165, 0xFF);
        dmac.write(0x166, 0xFF); // address 0xFFFFFE
        dmac.write(0x162, 0xFF);
        dmac.write(0x163, 0x03); // count 0x03FF

        dmac.advance(2, 4);
        assert_eq!(dmac.address(2), 0x00_0002); // wrapped within 24 bits
        assert_eq!(dmac.read(0x162), 0xFF);
        assert_eq!(dmac.read(0x163), 0xFF);
    }

    /// The mask register reads back with its upper nibble set; writing the
    /// initialize port resets the controller to all channels masked.
    #[test]
    fn mask_register_and_initialize_reset() {
        let mut dmac = Dmac71071::new();
        assert_eq!(dmac.read(0x16F), 0xFF); // reset default: all masked

        dmac.write(0x16F, 0xFB); // unmask channel 2
        assert_eq!(dmac.read(0x16F), 0xFB | 0xF0);

        dmac.write(0x161, 0x01);
        dmac.write(0x160, 0x01); // initialize: reset everything
        assert_eq!(dmac.read(0x16F), 0xFF);
        assert_eq!(dmac.read(0x161), 0x00);
    }
}
