//! IDE Low-Level Emulation (LLE).
//!
//! Emulates the PC-98 IDE (ATA) interface at the hardware register level.
//! The ATA registers are mapped at I/O ports 0x0640-0x064E (CS0 space) and
//! 0x074C-0x074E (CS1 space), with bank selection at 0x0430/0x0432.
//!
//! The PC-98 uses a bank-switched dual-channel IDE architecture: a single
//! set of I/O ports is shared by two independent channels, with port 0x0432
//! selecting which channel (0 or 1) the CPU addresses.
//!
//! PC-98 IDE uses PIO exclusively (no DMA). The 16-bit data register at
//! port 0x0640 transfers words. IRQ 9 (slave PIC IRQ 1, INT 0x11) is used
//! for completion interrupts.

use super::atapi::{self, AtapiState};
use crate::disk::{HddGeometry, MountedHdd};

/// IDE controller phase (data transfer state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdePhase {
    /// No data transfer in progress.
    Idle,
    /// Host reading data from drive (Read Sector, Identify Device).
    DataIn,
    /// Host writing data to drive (Write Sector).
    DataOut,
    /// Host writing ATAPI 12-byte command packet.
    PacketCommand,
    /// Device sending ATAPI response data to host.
    PacketDataIn,
}

/// Actions the bus must perform after an IDE controller method call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdeAction {
    /// No action needed.
    None,
    /// Schedule a completion event after a delay.
    /// The bus should schedule `Event98::IdeExecution`.
    ScheduleCompletion,
}

// ATA status register bit masks.
#[cfg(test)]
const STATUS_BSY: u8 = 0x80;
const STATUS_DRDY: u8 = 0x40;
const STATUS_DSC: u8 = 0x10;
const STATUS_DRQ: u8 = 0x08;
const STATUS_ERR: u8 = 0x01;

// ATA error register bit masks.
const ERROR_ABRT: u8 = 0x04;

// Device control register bit masks.
const CONTROL_SRST: u8 = 0x04;
const CONTROL_NIEN: u8 = 0x02;

// Device/Head register bit masks.
const DEVHEAD_LBA: u8 = 0x40;
const DEVHEAD_DEV: u8 = 0x10;
const DEVHEAD_HEAD_MASK: u8 = 0x0F;

/// Sector size for IDE drives.
const IDE_SECTOR_SIZE: usize = 512;

/// Per-drive ATA state.
#[derive(Debug)]
struct IdeDrive {
    status: u8,
    error: u8,
    features: u8,
    sector_count: u8,
    sector_number: u8,
    cylinder_low: u8,
    cylinder_high: u8,
    device_head: u8,
    control: u8,
    multiple_count: u8,
    buffer: Vec<u8>,
    buffer_position: usize,
    buffer_size: usize,
    sector_size: usize,
    sectors_pending: u16,
    data_in_updates_sector_registers: bool,
    interrupt_pending: bool,
    block_size: u16,
    sectors_in_block: u16,
    logical_heads: u8,
    logical_sectors_per_track: u8,
}

impl IdeDrive {
    fn new() -> Self {
        Self {
            status: STATUS_DRDY | STATUS_DSC,
            error: 0x01,
            features: 0,
            sector_count: 1,
            sector_number: 1,
            cylinder_low: 0,
            cylinder_high: 0,
            device_head: 0xA0,
            control: 0,
            multiple_count: 0,
            buffer: vec![0u8; IDE_SECTOR_SIZE],
            buffer_position: 0,
            buffer_size: 0,
            sector_size: IDE_SECTOR_SIZE,
            sectors_pending: 0,
            data_in_updates_sector_registers: false,
            interrupt_pending: false,
            block_size: 1,
            sectors_in_block: 0,
            logical_heads: 0,
            logical_sectors_per_track: 0,
        }
    }

    fn reset(&mut self) {
        self.status = STATUS_DRDY | STATUS_DSC;
        self.error = 0x01;
        self.sector_count = 1;
        self.sector_number = 1;
        self.cylinder_low = 0;
        self.cylinder_high = 0;
        self.buffer_position = 0;
        self.buffer_size = 0;
        self.sectors_pending = 0;
        self.data_in_updates_sector_registers = false;
        self.interrupt_pending = false;
        self.block_size = 1;
        self.sectors_in_block = 0;
        self.logical_heads = 0;
        self.logical_sectors_per_track = 0;
    }

    fn interrupts_enabled(&self) -> bool {
        self.control & CONTROL_NIEN == 0
    }

    fn decrement_sector_count_register(&mut self) {
        self.sector_count = if self.sector_count == 0 {
            0xFF
        } else {
            self.sector_count - 1
        };
    }
}

/// Per-channel IDE state.
#[derive(Debug)]
struct IdeChannel {
    drives: [IdeDrive; 2],
    selected_drive: usize,
    phase: IdePhase,
}

impl IdeChannel {
    fn new() -> Self {
        Self {
            drives: [IdeDrive::new(), IdeDrive::new()],
            selected_drive: 0,
            phase: IdePhase::Idle,
        }
    }

    fn drive(&self) -> &IdeDrive {
        &self.drives[self.selected_drive]
    }

    fn drive_mut(&mut self) -> &mut IdeDrive {
        &mut self.drives[self.selected_drive]
    }
}

/// IDE (ATA) controller state with dual-channel bank switching.
#[derive(Debug)]
pub(super) struct Controller {
    channels: [IdeChannel; 2],
    active_channel: usize,
    bank: [u8; 2],
    srst_active: bool,
    has_atapi_device: bool,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller {
    pub(super) fn new() -> Self {
        Self {
            channels: [IdeChannel::new(), IdeChannel::new()],
            active_channel: 0,
            bank: [0; 2],
            srst_active: false,
            has_atapi_device: false,
        }
    }

    /// Returns the current phase of the active channel.
    #[cfg(test)]
    pub(super) fn phase(&self) -> IdePhase {
        self.channels[self.active_channel].phase
    }

    /// Returns the currently active channel index (0 or 1).
    #[cfg(test)]
    pub(super) fn active_channel(&self) -> usize {
        self.active_channel
    }

    /// Selects the active IDE channel (0 or 1).
    ///
    /// The PC-98 switches channels through the bank register at port 0x0432.
    /// A PC/AT decodes two separate port ranges, so its ATAPI controller pins
    /// the active channel to 1 once and drives every access through it.
    pub(super) fn select_channel(&mut self, channel: usize) {
        self.active_channel = channel;
    }

    /// Returns whether the active channel is channel 1 (ATAPI).
    pub(super) fn is_atapi_channel_active(&self) -> bool {
        self.active_channel == 1
    }

    /// Initializes channel 1 master drive as an ATAPI device.
    /// Sets the ATAPI signature registers (CYL=0xEB14) and power-on status.
    pub(super) fn initialize_atapi_drive(&mut self) {
        self.has_atapi_device = true;
        let drive = &mut self.channels[1].drives[0];
        drive.status = 0x00;
        drive.error = 0x00;
        drive.buffer_position = 0;
        drive.buffer_size = 0;
        atapi::AtapiState::set_signature(
            &mut drive.sector_count,
            &mut drive.sector_number,
            &mut drive.cylinder_low,
            &mut drive.cylinder_high,
        );
        self.channels[1].phase = IdePhase::Idle;
    }

    /// Sets the sector size for a specific drive on a channel.
    /// Called when inserting a drive image with a non-standard sector size.
    pub(super) fn set_drive_sector_size(
        &mut self,
        channel: usize,
        drive: usize,
        sector_size: usize,
    ) {
        self.channels[channel].drives[drive].sector_size = sector_size;
    }

    fn channel(&self) -> &IdeChannel {
        &self.channels[self.active_channel]
    }

    fn channel_mut(&mut self) -> &mut IdeChannel {
        &mut self.channels[self.active_channel]
    }

    /// Reads the 16-bit data register (port 0x0640).
    pub(super) fn read_data_word(&mut self, drives: &[Option<MountedHdd>; 2]) -> (u16, IdeAction) {
        let ch = self.active_channel;
        if self.channels[ch].phase != IdePhase::DataIn {
            return (0xFFFF, IdeAction::None);
        }

        let sel = self.channels[ch].selected_drive;
        let drive = &mut self.channels[ch].drives[sel];
        if drive.buffer_position + 1 >= drive.buffer_size {
            let low = drive.buffer[drive.buffer_position] as u16;
            let high = if drive.buffer_position + 1 < drive.buffer_size {
                drive.buffer[drive.buffer_position + 1] as u16
            } else {
                0
            };
            drive.buffer_position = drive.buffer_size;
            let word = low | (high << 8);
            let action = self.check_data_in_complete(drives);
            return (word, action);
        }

        let pos = drive.buffer_position;
        let word = u16::from(drive.buffer[pos]) | (u16::from(drive.buffer[pos + 1]) << 8);
        drive.buffer_position += 2;

        let action = if drive.buffer_position >= drive.buffer_size {
            self.check_data_in_complete(drives)
        } else {
            IdeAction::None
        };

        (word, action)
    }

    /// Writes the 16-bit data register (port 0x0640).
    /// Returns an action if a sector buffer is now full.
    pub(super) fn write_data_word(
        &mut self,
        value: u16,
        drives: &mut [Option<MountedHdd>; 2],
    ) -> IdeAction {
        let ch = self.active_channel;
        if self.channels[ch].phase != IdePhase::DataOut {
            return IdeAction::None;
        }

        let sel = self.channels[ch].selected_drive;
        let drive = &mut self.channels[ch].drives[sel];
        if drive.buffer_position + 1 < drive.buffer_size {
            drive.buffer[drive.buffer_position] = value as u8;
            drive.buffer[drive.buffer_position + 1] = (value >> 8) as u8;
            drive.buffer_position += 2;
        }

        if drive.buffer_position >= drive.buffer_size {
            self.handle_write_complete(drives)
        } else {
            IdeAction::None
        }
    }

    /// Reads the error register (port 0x0642).
    /// Clears the ERR bit in the status register.
    pub(super) fn read_error(&mut self) -> u8 {
        let ch = self.channel_mut();
        let drive = ch.drive_mut();
        drive.status &= !STATUS_ERR;
        drive.error
    }

    /// Reads the sector count register (port 0x0644).
    pub(super) fn read_sector_count(&self) -> u8 {
        self.channel().drive().sector_count
    }

    /// Reads the sector number register (port 0x0646).
    pub(super) fn read_sector_number(&self) -> u8 {
        self.channel().drive().sector_number
    }

    /// Reads the cylinder low register (port 0x0648).
    pub(super) fn read_cylinder_low(&self) -> u8 {
        self.channel().drive().cylinder_low
    }

    /// Reads the cylinder high register (port 0x064A).
    pub(super) fn read_cylinder_high(&self) -> u8 {
        self.channel().drive().cylinder_high
    }

    /// Reads the device/head register (port 0x064C).
    pub(super) fn read_device_head(&self) -> u8 {
        self.channel().drive().device_head
    }

    /// Reads the status register (port 0x064E).
    /// Clears the pending interrupt and returns whether the PIC IRQ
    /// should be deasserted (true when NIEN=0, matching NP21W behavior).
    pub(super) fn read_status(&mut self) -> (u8, bool) {
        let ch = self.channel_mut();
        let drive = ch.drive_mut();
        let status = drive.status;
        let clear_irq = drive.interrupts_enabled();
        drive.interrupt_pending = false;
        (status, clear_irq)
    }

    /// Reads the alternate status register (port 0x074C).
    /// Does NOT clear the pending interrupt.
    pub(super) fn read_alt_status(&self) -> u8 {
        self.channel().drive().status
    }

    /// Writes the features register (port 0x0642).
    pub(super) fn write_features(&mut self, value: u8) {
        self.channel_mut().drive_mut().features = value;
    }

    /// Writes the sector count register (port 0x0644).
    pub(super) fn write_sector_count(&mut self, value: u8) {
        self.channel_mut().drive_mut().sector_count = value;
    }

    /// Writes the sector number register (port 0x0646).
    pub(super) fn write_sector_number(&mut self, value: u8) {
        self.channel_mut().drive_mut().sector_number = value;
    }

    /// Writes the cylinder low register (port 0x0648).
    pub(super) fn write_cylinder_low(&mut self, value: u8) {
        self.channel_mut().drive_mut().cylinder_low = value;
    }

    /// Writes the cylinder high register (port 0x064A).
    pub(super) fn write_cylinder_high(&mut self, value: u8) {
        self.channel_mut().drive_mut().cylinder_high = value;
    }

    /// Writes the device/head register (port 0x064C).
    /// Updates the selected drive from bit 4.
    pub(super) fn write_device_head(&mut self, value: u8) {
        let ch = self.channel_mut();
        ch.selected_drive = if value & DEVHEAD_DEV != 0 { 1 } else { 0 };
        ch.drives[ch.selected_drive].device_head = value;
    }

    /// Writes the device control register (port 0x074C).
    pub(super) fn write_device_control(&mut self, value: u8) {
        let old_srst = self.srst_active;
        self.srst_active = value & CONTROL_SRST != 0;

        let ch = &mut self.channels[self.active_channel];
        ch.drive_mut().control = value;

        // SRST falling edge (1->0) resets drives on the active channel.
        if old_srst && !self.srst_active {
            if self.active_channel == 1 && self.has_atapi_device {
                // ATAPI device: set signature, then DRDY|DSC|ERR with AMNF error.
                let drive = &mut ch.drives[0];
                drive.reset();
                atapi::AtapiState::set_signature(
                    &mut drive.sector_count,
                    &mut drive.sector_number,
                    &mut drive.cylinder_low,
                    &mut drive.cylinder_high,
                );
                drive.status = STATUS_DRDY | STATUS_DSC | STATUS_ERR;
                drive.error = 0x01;
                ch.drives[1].reset();
            } else {
                ch.drives[0].reset();
                ch.drives[1].reset();
            }
            ch.phase = IdePhase::Idle;
        }
    }

    /// Writes the command register (port 0x064E).
    pub(super) fn write_command(
        &mut self,
        command: u8,
        drives: &[Option<MountedHdd>; 2],
    ) -> IdeAction {
        // Channel 1 has no HDD drives attached - commands abort until ATAPI
        // support is wired in.
        if self.active_channel == 1 {
            return self.abort_command();
        }

        let ch = &self.channels[self.active_channel];
        let drive_present = drives[ch.selected_drive].is_some();

        match command {
            // NOP
            0x00 => self.abort_command(),

            // Device Reset
            0x08 => {
                let sel = self.channels[self.active_channel].selected_drive;
                self.channels[self.active_channel].drives[sel].reset();
                let drive = &mut self.channels[self.active_channel].drives[sel];
                drive.error = if drives[sel].is_some() { 0x01 } else { 0x00 };
                if sel == 0 && drives[1].is_none() {
                    drive.error |= 0x80;
                }
                drive.interrupt_pending = true;
                self.channels[self.active_channel].phase = IdePhase::Idle;
                IdeAction::ScheduleCompletion
            }

            // Recalibrate (0x10-0x1F)
            0x10..=0x1F => {
                if drive_present {
                    let ch = self.channel_mut();
                    let drive = ch.drive_mut();
                    drive.cylinder_low = 0;
                    drive.cylinder_high = 0;
                    self.set_ready();
                    IdeAction::ScheduleCompletion
                } else {
                    self.abort_command()
                }
            }

            // Read Sector(s) (0x20/0x21)
            0x20 | 0x21 => {
                if !drive_present {
                    return self.abort_command();
                }
                self.start_read(drives, false)
            }

            // Write Sector(s) (0x30/0x31)
            0x30 | 0x31 => {
                if !drive_present {
                    return self.abort_command();
                }
                self.start_write(false)
            }

            // Read Verify (0x40/0x41)
            0x40 | 0x41 => {
                if drive_present {
                    self.set_ready();
                    IdeAction::ScheduleCompletion
                } else {
                    self.abort_command()
                }
            }

            // Seek (0x70-0x7F)
            0x70..=0x7F => {
                if drive_present {
                    self.set_ready();
                    IdeAction::ScheduleCompletion
                } else {
                    self.abort_command()
                }
            }

            // Execute Drive Diagnostic (0x90)
            0x90 => self.execute_diagnostic(drives),

            // Initialize Device Parameters (0x91)
            0x91 => {
                if drive_present {
                    let ch = self.channel_mut();
                    let drive = ch.drive_mut();
                    drive.logical_heads = (drive.device_head & DEVHEAD_HEAD_MASK) + 1;
                    drive.logical_sectors_per_track = drive.sector_count;
                    self.set_ready();
                    IdeAction::ScheduleCompletion
                } else {
                    self.abort_command()
                }
            }

            // Read Multiple (0xC4)
            0xC4 => {
                if !drive_present {
                    return self.abort_command();
                }
                let ch = &self.channels[self.active_channel];
                if ch.drive().multiple_count == 0 {
                    return self.abort_command();
                }
                self.start_read(drives, true)
            }

            // Write Multiple (0xC5)
            0xC5 => {
                if !drive_present {
                    return self.abort_command();
                }
                let ch = &self.channels[self.active_channel];
                if ch.drive().multiple_count == 0 {
                    return self.abort_command();
                }
                self.start_write(true)
            }

            // Set Multiple Mode (0xC6)
            0xC6 => {
                if !drive_present {
                    return self.abort_command();
                }
                let count = self.channel().drive().sector_count;
                if count == 0 || !count.is_power_of_two() || count > 128 {
                    return self.abort_command();
                }
                self.channel_mut().drive_mut().multiple_count = count;
                self.set_ready();
                IdeAction::ScheduleCompletion
            }

            // Standby Immediate (0xE0)
            0xE0 => {
                self.set_ready();
                IdeAction::ScheduleCompletion
            }

            // Idle Immediate (0xE1)
            0xE1 => {
                self.set_ready();
                IdeAction::ScheduleCompletion
            }

            // Check Power Mode (0xE5)
            0xE5 => {
                self.channel_mut().drive_mut().sector_count = 0xFF;
                self.set_ready();
                IdeAction::ScheduleCompletion
            }

            // Flush Cache (0xE7)
            0xE7 => {
                self.set_ready();
                IdeAction::ScheduleCompletion
            }

            // Identify Device (0xEC)
            0xEC => {
                if !drive_present {
                    return self.abort_command();
                }
                let sel = self.channels[self.active_channel].selected_drive;
                let geometry = drives[sel].as_ref().unwrap().geometry();
                self.build_identify_data(geometry);
                let ch = self.channel_mut();
                ch.phase = IdePhase::DataIn;
                let drive = ch.drive_mut();
                drive.status = STATUS_DRDY | STATUS_DSC | STATUS_DRQ;
                drive.data_in_updates_sector_registers = false;
                drive.interrupt_pending = true;
                IdeAction::ScheduleCompletion
            }

            // Set Features (0xEF)
            0xEF => match self.channel().drive().features {
                0x02 | 0x82 => {
                    self.set_ready();
                    IdeAction::ScheduleCompletion
                }
                0x03 => {
                    self.set_ready();
                    IdeAction::ScheduleCompletion
                }
                _ => self.abort_command(),
            },

            // Unknown command
            _ => self.abort_command(),
        }
    }

    /// Reads the bank select register.
    /// Clears the interrupt pending flag (bit 7) after reading.
    pub(super) fn read_bank(&mut self, index: usize) -> u8 {
        let value = self.bank[index];
        self.bank[index] &= !0x80;
        value
    }

    /// Writes the bank select register.
    /// For index 1 (port 0x0432): bits 0-6 are stored, bit 0 selects the
    /// active IDE channel. Writes with bit 7 set are ignored (status read
    /// dummy writes).
    pub(super) fn write_bank(&mut self, index: usize, value: u8) {
        if index == 1 && value & 0x80 != 0 {
            return;
        }
        self.bank[index] = value & 0x71;
        if index == 1 {
            self.active_channel = (value & 0x01) as usize;
        }
    }

    /// Reads the bank 0 status register (port 0x0430).
    /// Unlike port 0x0432 which returns the raw bank register, port 0x0430
    /// returns a computed status: 0 in compatibility mode (CD-ROM only on
    /// channel 1), 1 otherwise. Bit 6 is set when the current channel has
    /// a slave device. The computed value is stored back into bank[0].
    pub(super) fn read_bank0_status(
        &mut self,
        drives: &[Option<MountedHdd>; 2],
        has_cdrom: bool,
    ) -> u8 {
        // Compatibility mode: CD-ROM present only on channel 1 master.
        // In neetan, CD-ROM is always on channel 1 if present.
        let compmode = has_cdrom;

        let device_exists = if self.active_channel == 0 {
            let selected = self.channels[0].selected_drive;
            drives[selected].is_some()
        } else {
            has_cdrom
        };

        let ret = if device_exists {
            let mut value = if compmode { 0x00 } else { 0x01 };
            let slave_exists = if self.active_channel == 0 {
                drives[1].is_some()
            } else {
                false
            };
            if slave_exists {
                value |= 0x40;
            }
            value
        } else {
            self.bank[0]
        };

        self.bank[0] = ret & !0x80;
        ret & 0x7F
    }

    /// Reads the IDE presence detection register (port 0x0433).
    /// Returns 0x02 when channel 1 is selected (bank[1] bit 0 set) and
    /// has a device (CD-ROM). Returns 0x00 otherwise.
    pub(super) fn read_presence(&self, has_cdrom: bool) -> u8 {
        if self.bank[1] & 0x01 != 0 && has_cdrom {
            0x02
        } else {
            0x00
        }
    }

    /// Reads the additional status register (port 0x0435).
    /// Bit 1: 0 = IDE HDD present, 1 = no IDE HDD.
    pub(super) fn read_additional_status(&self, drives: &[Option<MountedHdd>; 2]) -> u8 {
        let has_hdd = drives[0].is_some() || drives[1].is_some();
        if has_hdd { 0x00 } else { 0x02 }
    }

    /// Reads the digital input register (port 0x074E).
    /// Returns drive status with inverted head bits.
    pub(super) fn read_digital_input(&self) -> u8 {
        let ch = self.channel();
        let drive = ch.drive();
        let head = drive.device_head & DEVHEAD_HEAD_MASK;
        let drive_select = if ch.selected_drive == 0 { 0x02 } else { 0x01 };
        0xC0 | ((!head & DEVHEAD_HEAD_MASK) << 2) | drive_select
    }

    /// Called when the scheduled completion event fires.
    /// Returns true if an interrupt should be raised.
    pub(super) fn complete_operation(&mut self) -> bool {
        let ch = &mut self.channels[self.active_channel];
        let drive = ch.drive();
        let should_interrupt = drive.interrupt_pending && drive.interrupts_enabled();
        if should_interrupt {
            self.bank[0] = self.bank[1] | 0x80;
        }
        should_interrupt
    }

    /// EXECUTE DEVICE DIAGNOSTIC: both devices post their results and
    /// device 0 becomes selected and raises the completion interrupt.
    /// Diagnostic code 0x01 means the device passed. Bit 7 is only set
    /// when a present device 1 failed, never for an absent device 1.
    fn execute_diagnostic(&mut self, _drives: &[Option<MountedHdd>; 2]) -> IdeAction {
        let ch = &mut self.channels[self.active_channel];
        for drive in &mut ch.drives {
            drive.reset();
            drive.error = 0x01;
        }
        ch.selected_drive = 0;
        ch.drives[0].interrupt_pending = true;
        ch.phase = IdePhase::Idle;
        IdeAction::ScheduleCompletion
    }

    fn set_ready(&mut self) {
        let ch = self.channel_mut();
        let drive = ch.drive_mut();
        drive.status = STATUS_DRDY | STATUS_DSC;
        drive.error = 0;
        drive.interrupt_pending = true;
    }

    fn abort_command(&mut self) -> IdeAction {
        let ch = self.channel_mut();
        let drive = ch.drive_mut();
        drive.status = STATUS_DRDY | STATUS_DSC | STATUS_ERR;
        drive.error = ERROR_ABRT;
        drive.interrupt_pending = true;
        ch.phase = IdePhase::Idle;
        IdeAction::ScheduleCompletion
    }

    fn get_current_sector(&self, geometry: &HddGeometry) -> u32 {
        let drive = self.channel().drive();
        if drive.device_head & DEVHEAD_LBA != 0 {
            (drive.sector_number as u32)
                | ((drive.cylinder_low as u32) << 8)
                | ((drive.cylinder_high as u32) << 16)
                | (((drive.device_head & DEVHEAD_HEAD_MASK) as u32) << 24)
        } else {
            let cylinder = u16::from(drive.cylinder_low) | (u16::from(drive.cylinder_high) << 8);
            let head = drive.device_head & DEVHEAD_HEAD_MASK;
            let sector = drive.sector_number;
            let heads = if drive.logical_heads > 0 {
                drive.logical_heads
            } else {
                geometry.heads
            };
            let sectors_per_track = if drive.logical_sectors_per_track > 0 {
                drive.logical_sectors_per_track
            } else {
                geometry.sectors_per_track
            };
            (cylinder as u32 * heads as u32 + head as u32) * sectors_per_track as u32
                + (sector as u32).saturating_sub(1)
        }
    }

    fn advance_sector_address(&mut self, geometry: &HddGeometry) {
        let ch = self.channel_mut();
        let drive = ch.drive_mut();
        if drive.device_head & DEVHEAD_LBA != 0 {
            let mut lba = (drive.sector_number as u32)
                | ((drive.cylinder_low as u32) << 8)
                | ((drive.cylinder_high as u32) << 16)
                | (((drive.device_head & DEVHEAD_HEAD_MASK) as u32) << 24);
            lba += 1;
            drive.sector_number = lba as u8;
            drive.cylinder_low = (lba >> 8) as u8;
            drive.cylinder_high = (lba >> 16) as u8;
            drive.device_head =
                (drive.device_head & !DEVHEAD_HEAD_MASK) | ((lba >> 24) as u8 & DEVHEAD_HEAD_MASK);
        } else {
            let heads = if drive.logical_heads > 0 {
                drive.logical_heads
            } else {
                geometry.heads
            };
            let sectors_per_track = if drive.logical_sectors_per_track > 0 {
                drive.logical_sectors_per_track
            } else {
                geometry.sectors_per_track
            };
            let mut sector = drive.sector_number;
            let mut head = drive.device_head & DEVHEAD_HEAD_MASK;
            let mut cylinder =
                u16::from(drive.cylinder_low) | (u16::from(drive.cylinder_high) << 8);

            sector += 1;
            if sector > sectors_per_track {
                sector = 1;
                head += 1;
                if head >= heads {
                    head = 0;
                    cylinder += 1;
                }
            }

            drive.sector_number = sector;
            drive.device_head = (drive.device_head & !DEVHEAD_HEAD_MASK) | head;
            drive.cylinder_low = cylinder as u8;
            drive.cylinder_high = (cylinder >> 8) as u8;
        }
    }

    fn start_read(&mut self, drives: &[Option<MountedHdd>; 2], multiple: bool) -> IdeAction {
        let sel = self.channels[self.active_channel].selected_drive;
        let geometry = drives[sel].as_ref().unwrap().geometry();
        let lba = self.get_current_sector(&geometry);
        let sector_count = self.channel().drive().sector_count;
        let count = if sector_count == 0 {
            256
        } else {
            sector_count as u16
        };

        let Some(sector_data) = drives[sel].as_ref().unwrap().read_sector(lba) else {
            return self.abort_command();
        };

        {
            let ch = self.channel_mut();
            let drive = ch.drive_mut();
            let sector_size = drive.sector_size;
            drive.buffer[..sector_size].copy_from_slice(sector_data);
            drive.buffer_position = 0;
            drive.buffer_size = sector_size;
            drive.sectors_pending = count - 1;
            drive.data_in_updates_sector_registers = true;
            drive.block_size = if multiple {
                drive.multiple_count as u16
            } else {
                1
            };
            drive.sectors_in_block = 1;
            drive.status = STATUS_DRDY | STATUS_DSC | STATUS_DRQ;
            drive.error = 0;
            drive.interrupt_pending = true;
            ch.phase = IdePhase::DataIn;
        }
        IdeAction::ScheduleCompletion
    }

    fn check_data_in_complete(&mut self, drives: &[Option<MountedHdd>; 2]) -> IdeAction {
        let ch_idx = self.active_channel;
        let sel = self.channels[ch_idx].selected_drive;
        let drive = &self.channels[ch_idx].drives[sel];
        if drive.buffer_position < drive.buffer_size {
            return IdeAction::None;
        }

        if drive.sectors_pending == 0 {
            let data_in_updates_sector_registers = drive.data_in_updates_sector_registers;
            if data_in_updates_sector_registers && let Some(drive) = drives[sel].as_ref() {
                let geometry = drive.geometry();
                self.advance_sector_address(&geometry);
            }
            let ch = &mut self.channels[ch_idx];
            let drive = &mut ch.drives[sel];
            if data_in_updates_sector_registers {
                drive.decrement_sector_count_register();
            }
            drive.data_in_updates_sector_registers = false;
            drive.status = STATUS_DRDY | STATUS_DSC;
            drive.interrupt_pending = true;
            ch.phase = IdePhase::Idle;
            return IdeAction::ScheduleCompletion;
        }

        let geometry = drives[sel].as_ref().unwrap().geometry();
        self.advance_sector_address(&geometry);
        let lba = self.get_current_sector(&geometry);
        let Some(sector_data) = drives[sel].as_ref().unwrap().read_sector(lba) else {
            let ch = &mut self.channels[ch_idx];
            let drive = &mut ch.drives[sel];
            drive.status = STATUS_DRDY | STATUS_DSC | STATUS_ERR;
            drive.error = ERROR_ABRT;
            drive.interrupt_pending = true;
            ch.phase = IdePhase::Idle;
            return IdeAction::ScheduleCompletion;
        };

        let ch = &mut self.channels[ch_idx];
        let drive = &mut ch.drives[sel];
        let sector_size = drive.sector_size;
        drive.buffer[..sector_size].copy_from_slice(sector_data);
        drive.buffer_position = 0;
        drive.sectors_pending -= 1;
        drive.decrement_sector_count_register();
        let at_block_boundary = drive.sectors_in_block >= drive.block_size;
        drive.status = STATUS_DRDY | STATUS_DSC | STATUS_DRQ;
        ch.phase = IdePhase::DataIn;
        if at_block_boundary {
            drive.sectors_in_block = 1;
            drive.interrupt_pending = true;
            IdeAction::ScheduleCompletion
        } else {
            drive.sectors_in_block += 1;
            IdeAction::None
        }
    }

    fn start_write(&mut self, multiple: bool) -> IdeAction {
        let ch = self.channel_mut();
        let sector_count = ch.drive().sector_count;
        let count = if sector_count == 0 {
            256
        } else {
            sector_count as u16
        };

        let drive = ch.drive_mut();
        drive.buffer_position = 0;
        drive.buffer_size = drive.sector_size;
        drive.sectors_pending = count - 1;
        drive.block_size = if multiple {
            drive.multiple_count as u16
        } else {
            1
        };
        drive.sectors_in_block = 1;
        drive.status = STATUS_DRDY | STATUS_DSC | STATUS_DRQ;
        drive.error = 0;
        ch.phase = IdePhase::DataOut;
        // No interrupt on initial DRQ for write commands.
        IdeAction::None
    }

    fn handle_write_complete(&mut self, drives: &mut [Option<MountedHdd>; 2]) -> IdeAction {
        let ch_idx = self.active_channel;
        let sel = self.channels[ch_idx].selected_drive;
        let geometry = drives[sel].as_ref().unwrap().geometry();
        let lba = self.get_current_sector(&geometry);

        let drive = &self.channels[ch_idx].drives[sel];
        let data = &drive.buffer[..drive.buffer_size];

        let Some(disk) = &mut drives[sel] else {
            return self.abort_command();
        };

        if !disk.write_sector(lba, data) {
            return self.abort_command();
        }

        self.advance_sector_address(&geometry);

        let ch = &mut self.channels[ch_idx];
        let drive = &mut ch.drives[sel];
        if drive.sectors_pending == 0 {
            drive.status = STATUS_DRDY | STATUS_DSC;
            drive.interrupt_pending = true;
            ch.phase = IdePhase::Idle;
            IdeAction::ScheduleCompletion
        } else {
            drive.buffer_position = 0;
            drive.sectors_pending -= 1;
            drive.status = STATUS_DRDY | STATUS_DSC | STATUS_DRQ;
            let at_block_boundary = drive.sectors_in_block >= drive.block_size;
            if at_block_boundary {
                drive.sectors_in_block = 1;
                drive.interrupt_pending = true;
            } else {
                drive.sectors_in_block += 1;
            }
            IdeAction::ScheduleCompletion
        }
    }

    fn build_identify_data(&mut self, geometry: HddGeometry) {
        let ch = &self.channels[self.active_channel];
        let multiple_count = ch.drive().multiple_count;
        let drive_index = ch.selected_drive;
        let ch = self.channel_mut();
        let drive = ch.drive_mut();
        drive.buffer.fill(0);
        drive.buffer_position = 0;
        drive.buffer_size = IDE_SECTOR_SIZE;

        let total_sectors = geometry.total_sectors();

        let set_word = |buf: &mut Vec<u8>, word_index: usize, value: u16| {
            let byte_index = word_index * 2;
            buf[byte_index] = value as u8;
            buf[byte_index + 1] = (value >> 8) as u8;
        };

        let buf = &mut drive.buffer;

        // Word 0: General configuration (fixed drive, non-removable).
        set_word(buf, 0, 0x0040);
        // Word 1: Number of cylinders.
        set_word(buf, 1, geometry.cylinders);
        // Word 3: Number of heads.
        set_word(buf, 3, geometry.heads as u16);
        // Word 4: Bytes per track (unformatted).
        set_word(
            buf,
            4,
            geometry.sectors_per_track as u16 * geometry.sector_size,
        );
        // Word 6: Sectors per track.
        set_word(buf, 6, geometry.sectors_per_track as u16);

        // Words 10-19: Serial number (20 ASCII chars, big-endian byte pairs).
        let serial = b"RPC98IDE00000000    ";
        for (i, chunk) in serial.chunks(2).enumerate() {
            set_word(buf, 10 + i, u16::from(chunk[0]) << 8 | u16::from(chunk[1]));
        }

        // Word 22: Vendor-specific (ECC bytes).
        set_word(buf, 22, 4);

        // Words 23-26: Firmware revision (8 ASCII chars).
        let firmware = b"1.0     ";
        for (i, chunk) in firmware.chunks(2).enumerate() {
            set_word(buf, 23 + i, u16::from(chunk[0]) << 8 | u16::from(chunk[1]));
        }

        // Words 27-46: Model number (40 ASCII chars).
        let model = b"RPC98 IDE Hard Disk             ";
        for (i, chunk) in model.chunks(2).enumerate() {
            set_word(buf, 27 + i, u16::from(chunk[0]) << 8 | u16::from(chunk[1]));
        }

        // Word 47: Max sectors per interrupt for Read/Write Multiple.
        set_word(buf, 47, 0x8080);
        // Word 49: Capabilities (LBA supported).
        set_word(buf, 49, 0x0200);
        // Word 51: PIO data transfer cycle timing.
        set_word(buf, 51, 0x0278);
        // Word 53: Words 54-58 and 64-70 are valid.
        set_word(buf, 53, 0x0003);

        // Words 54-56: Current CHS.
        set_word(buf, 54, geometry.cylinders);
        set_word(buf, 55, geometry.heads as u16);
        set_word(buf, 56, geometry.sectors_per_track as u16);

        // Words 57-58: Current capacity in sectors.
        set_word(buf, 57, total_sectors as u16);
        set_word(buf, 58, (total_sectors >> 16) as u16);

        // Word 59: Current multiple sector setting.
        set_word(buf, 59, 0x0100 | multiple_count as u16);

        // Words 60-61: Total number of user addressable sectors (LBA).
        set_word(buf, 60, total_sectors as u16);
        set_word(buf, 61, (total_sectors >> 16) as u16);

        // Word 64: PIO modes supported (modes 3 and 4).
        set_word(buf, 64, 0x0003);

        // Word 80: Major version (ATA-1 through ATA-5).
        set_word(buf, 80, 0x003E);
        // Word 82: Supported command set (NOP, Device Reset, Write Cache).
        set_word(buf, 82, 0x4200);

        // Word 93: Hardware reset result (master/slave configuration).
        let word_93 = if drive_index == 0 { 0x407B } else { 0x4B00 };
        set_word(buf, 93, word_93);
    }

    // --- ATAPI integration methods (called from IdeController) ---

    /// Returns the current phase of the ATAPI channel (channel 1).
    pub(super) fn atapi_phase(&self) -> IdePhase {
        self.channels[1].phase
    }

    /// Performs DEVICE RESET on the ATAPI channel.
    pub(super) fn atapi_device_reset(&mut self) {
        let ch = &mut self.channels[1];
        ch.drives[0].reset();
        atapi::AtapiState::set_signature(
            &mut ch.drives[0].sector_count,
            &mut ch.drives[0].sector_number,
            &mut ch.drives[0].cylinder_low,
            &mut ch.drives[0].cylinder_high,
        );
        ch.drives[0].interrupt_pending = true;
        ch.phase = IdePhase::Idle;
    }

    /// Starts the PACKET command phase on channel 1.
    pub(super) fn atapi_start_packet(&mut self) {
        let ch = &mut self.channels[1];
        // Interrupt reason: CD=1 (command from host), IO=0 (host-to-device).
        ch.drives[0].sector_count = 0x01;
        ch.drives[0].status = STATUS_DRDY | STATUS_DSC | STATUS_DRQ;
        ch.phase = IdePhase::PacketCommand;
    }

    /// Fills the identify buffer with ATAPI CD-ROM identification data.
    pub(super) fn atapi_identify_packet_device(&mut self, _atapi_state: &AtapiState) {
        let ch = &mut self.channels[1];
        let drive = &mut ch.drives[0];
        drive.buffer.resize(IDE_SECTOR_SIZE, 0);
        atapi::build_identify_packet_device(&mut drive.buffer);
        drive.buffer_position = 0;
        drive.buffer_size = IDE_SECTOR_SIZE;
        drive.data_in_updates_sector_registers = false;
        drive.status = STATUS_DRDY | STATUS_DSC | STATUS_DRQ;
        drive.interrupt_pending = true;
        ch.phase = IdePhase::DataIn;
    }

    /// Aborts IDENTIFY DEVICE with ATAPI signature (0xEC on ATAPI device).
    pub(super) fn atapi_identify_device_abort(&mut self) {
        let ch = &mut self.channels[1];
        let drive = &mut ch.drives[0];
        drive.status = STATUS_DRDY | STATUS_DSC | STATUS_ERR;
        drive.error = ERROR_ABRT;
        AtapiState::set_signature(
            &mut drive.sector_count,
            &mut drive.sector_number,
            &mut drive.cylinder_low,
            &mut drive.cylinder_high,
        );
        drive.interrupt_pending = true;
        ch.phase = IdePhase::Idle;
    }

    /// Aborts a generic ATA command on the ATAPI channel.
    pub(super) fn atapi_abort(&mut self) {
        let ch = &mut self.channels[1];
        let drive = &mut ch.drives[0];
        drive.status = STATUS_DRDY | STATUS_DSC | STATUS_ERR;
        drive.error = ERROR_ABRT;
        drive.interrupt_pending = true;
        ch.phase = IdePhase::Idle;
    }

    /// Sets the ATAPI drive to ready state (DRDY, no errors).
    pub(super) fn atapi_set_ready(&mut self) {
        let ch = &mut self.channels[1];
        let drive = &mut ch.drives[0];
        drive.status = STATUS_DRDY;
        drive.error = 0;
        drive.interrupt_pending = true;
        ch.phase = IdePhase::Idle;
    }

    /// Reads the features register from the ATAPI channel drive.
    pub(super) fn read_atapi_features(&self) -> u8 {
        self.channels[1].drives[0].features
    }

    /// Transitions to PacketDataIn phase after a successful SCSI command.
    pub(super) fn atapi_start_data_in(&mut self, transfer_size: u16) {
        let ch = &mut self.channels[1];
        let drive = &mut ch.drives[0];
        // Interrupt reason: CD=0 (data), IO=1 (device-to-host).
        drive.sector_count = 0x02;
        drive.cylinder_low = transfer_size as u8;
        drive.cylinder_high = (transfer_size >> 8) as u8;
        drive.status = STATUS_DRDY | STATUS_DSC | STATUS_DRQ;
        drive.interrupt_pending = true;
        ch.phase = IdePhase::PacketDataIn;
    }

    /// Completes an ATAPI command (no data or all data transferred).
    pub(super) fn atapi_command_done(&mut self) {
        let ch = &mut self.channels[1];
        let drive = &mut ch.drives[0];
        // Interrupt reason: CD=1 (completion), IO=1 (device-to-host).
        drive.sector_count = 0x03;
        drive.status = STATUS_DRDY | STATUS_DSC;
        drive.interrupt_pending = true;
        ch.phase = IdePhase::Idle;
    }

    /// Sets error state after a failed SCSI command (CHECK CONDITION).
    pub(super) fn atapi_command_error(&mut self, atapi_state: &AtapiState) {
        let ch = &mut self.channels[1];
        let drive = &mut ch.drives[0];
        // Interrupt reason: CD=1 (completion), IO=1 (device-to-host).
        drive.sector_count = 0x03;
        drive.status = STATUS_DRDY | STATUS_DSC | STATUS_ERR;
        drive.error = atapi_state.error_register();
        drive.interrupt_pending = true;
        ch.phase = IdePhase::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{HddFormat, HddGeometry, HddImage};

    fn make_test_drive() -> MountedHdd {
        let geometry = HddGeometry {
            cylinders: 20,
            heads: 4,
            sectors_per_track: 17,
            sector_size: 512,
        };
        let total = geometry.total_bytes() as usize;
        let mut data = vec![0u8; total];
        for lba in 0..geometry.total_sectors() {
            let offset = lba as usize * 512;
            data[offset] = (lba >> 8) as u8;
            data[offset + 1] = lba as u8;
        }
        MountedHdd::new(HddImage::from_raw(geometry, HddFormat::Hdi, data), None)
    }

    fn make_drives(drive0: Option<MountedHdd>) -> [Option<MountedHdd>; 2] {
        [drive0, None]
    }

    #[test]
    fn initial_state_is_idle() {
        let controller = Controller::new();
        assert_eq!(controller.phase(), IdePhase::Idle);
        let status = controller.read_alt_status();
        assert_ne!(status & STATUS_DRDY, 0);
        assert_ne!(status & STATUS_DSC, 0);
        assert_eq!(status & STATUS_BSY, 0);
    }

    #[test]
    fn identify_device() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        let action = controller.write_command(0xEC, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert_eq!(controller.phase(), IdePhase::DataIn);

        // Read 256 words.
        let mut data = vec![0u16; 256];
        for word in data.iter_mut() {
            (*word, _) = controller.read_data_word(&drives);
        }

        // Word 0: 0x0040
        assert_eq!(data[0], 0x0040);
        // Word 1: cylinders = 20
        assert_eq!(data[1], 20);
        // Word 3: heads = 4
        assert_eq!(data[3], 4);
        // Word 4: bytes per track (17 * 512 = 8704)
        assert_eq!(data[4], 8704);
        // Word 6: sectors per track = 17
        assert_eq!(data[6], 17);
        // Word 22: vendor-specific ECC bytes
        assert_eq!(data[22], 4);
        // Word 49: LBA supported
        assert_eq!(data[49], 0x0200);
        // Word 51: PIO cycle timing
        assert_eq!(data[51], 0x0278);
        // Word 53: validity flags
        assert_eq!(data[53], 0x0003);
        // Word 59: current multiple mode (default 0)
        assert_eq!(data[59], 0x0100);
        // Word 60-61: total sectors = 20 * 4 * 17 = 1360
        let total = data[60] as u32 | ((data[61] as u32) << 16);
        assert_eq!(total, 1360);
        // Word 82: supported command set
        assert_eq!(data[82], 0x4200);
        // Word 93: master configuration
        assert_eq!(data[93], 0x407B);

        assert_eq!(controller.phase(), IdePhase::Idle);
    }

    #[test]
    fn read_sector_chs() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // Set CHS: cylinder 0, head 0, sector 6 (LBA = 5)
        controller.write_cylinder_low(0);
        controller.write_cylinder_high(0);
        controller.write_device_head(0xA0); // CHS mode, head 0
        controller.write_sector_number(6); // 1-based
        controller.write_sector_count(1);

        let action = controller.write_command(0x20, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert_eq!(controller.phase(), IdePhase::DataIn);

        // Read 256 words (512 bytes).
        let mut data = vec![0u16; 256];
        for word in data.iter_mut() {
            (*word, _) = controller.read_data_word(&drives);
        }

        // First two bytes should be LBA 5: 0x00, 0x05.
        assert_eq!(data[0] & 0xFF, 0x00);
        assert_eq!(data[0] >> 8, 0x05);

        assert_eq!(controller.phase(), IdePhase::Idle);
    }

    #[test]
    fn read_sector_lba() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // LBA mode, sector 42
        controller.write_sector_number(42);
        controller.write_cylinder_low(0);
        controller.write_cylinder_high(0);
        controller.write_device_head(0xE0); // LBA mode, head 0
        controller.write_sector_count(1);

        let action = controller.write_command(0x20, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);

        let (first_word, _) = controller.read_data_word(&drives);
        assert_eq!(first_word & 0xFF, 0x00);
        assert_eq!(first_word >> 8, 42);
    }

    #[test]
    fn write_sector() {
        let mut controller = Controller::new();
        let mut drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), None];

        // LBA mode, sector 10
        controller.write_sector_number(10);
        controller.write_cylinder_low(0);
        controller.write_cylinder_high(0);
        controller.write_device_head(0xE0);
        controller.write_sector_count(1);

        let action = controller.write_command(0x30, &drives);
        assert_eq!(action, IdeAction::None); // No IRQ for initial DRQ
        assert_eq!(controller.phase(), IdePhase::DataOut);

        // Write 256 words of 0xAABB.
        for _ in 0..256 {
            controller.write_data_word(0xAABB, &mut drives);
        }

        // Verify the write was committed.
        let sector = drives[0].as_ref().unwrap().read_sector(10).unwrap();
        assert_eq!(sector[0], 0xBB); // Low byte
        assert_eq!(sector[1], 0xAA); // High byte
    }

    #[test]
    fn read_multiple_sectors() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // LBA mode, read 2 sectors starting at LBA 0
        controller.write_sector_number(0);
        controller.write_cylinder_low(0);
        controller.write_cylinder_high(0);
        controller.write_device_head(0xE0);
        controller.write_sector_count(2);

        controller.write_command(0x20, &drives);

        // Read first sector (256 words).
        for _ in 0..256 {
            controller.read_data_word(&drives);
        }
        // Should still be in DataIn phase.
        assert_eq!(controller.phase(), IdePhase::DataIn);

        // Read second sector - first word should be LBA 1.
        let (first_word, _) = controller.read_data_word(&drives);
        assert_eq!(first_word & 0xFF, 0x00);
        assert_eq!(first_word >> 8, 0x01);

        // Read remaining words.
        for _ in 1..256 {
            controller.read_data_word(&drives);
        }

        assert_eq!(controller.phase(), IdePhase::Idle);
    }

    #[test]
    fn software_reset() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // Start a read to enter DataIn phase.
        controller.write_device_head(0xE0);
        controller.write_sector_number(0);
        controller.write_sector_count(1);
        controller.write_command(0x20, &drives);
        assert_eq!(controller.phase(), IdePhase::DataIn);

        // Software reset: set SRST bit.
        controller.write_device_control(CONTROL_SRST);
        // Clear SRST bit (falling edge triggers reset).
        controller.write_device_control(0);
        assert_eq!(controller.phase(), IdePhase::Idle);
        assert_ne!(controller.read_alt_status() & STATUS_DRDY, 0);
    }

    #[test]
    fn reading_status_clears_interrupt() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_command(0xE0, &drives); // Standby Immediate
        controller.complete_operation();

        // Interrupt should be pending.
        assert!(controller.channels[0].drives[0].interrupt_pending);

        // Reading status clears interrupt.
        controller.read_status();
        assert!(!controller.channels[0].drives[0].interrupt_pending);
    }

    #[test]
    fn reading_alt_status_does_not_clear_interrupt() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_command(0xE0, &drives); // Standby Immediate

        // Interrupt should be pending.
        assert!(controller.channels[0].drives[0].interrupt_pending);

        // Reading alt status does NOT clear interrupt.
        controller.read_alt_status();
        assert!(controller.channels[0].drives[0].interrupt_pending);
    }

    #[test]
    fn recalibrate() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        let action = controller.write_command(0x10, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        let status = controller.read_alt_status();
        assert_ne!(status & STATUS_DRDY, 0);
        assert_eq!(status & STATUS_ERR, 0);
    }

    #[test]
    fn no_drive_returns_error() {
        let mut controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [None, None];

        let action = controller.write_command(0x20, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        let status = controller.read_alt_status();
        assert_ne!(status & STATUS_ERR, 0);
    }

    #[test]
    fn set_multiple_mode() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_sector_count(2);
        let action = controller.write_command(0xC6, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert_eq!(controller.channels[0].drives[0].multiple_count, 2);
    }

    #[test]
    fn set_multiple_mode_invalid_count() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_sector_count(3); // Not a power of 2
        let action = controller.write_command(0xC6, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        let status = controller.read_alt_status();
        assert_ne!(status & STATUS_ERR, 0);
    }

    #[test]
    fn check_power_mode() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_command(0xE5, &drives);
        assert_eq!(controller.read_sector_count(), 0xFF);
    }

    #[test]
    fn bank_select() {
        let mut controller = Controller::new();
        controller.write_bank(0, 0x01);
        assert_eq!(controller.read_bank(0), 0x01);
        controller.write_bank(1, 0x71);
        assert_eq!(controller.read_bank(1), 0x71);
    }

    #[test]
    fn presence_detection_channel1_not_selected() {
        let controller = Controller::new();
        // Channel 1 not selected (bank[1] bit 0 = 0): returns 0x00 regardless.
        assert_eq!(controller.read_presence(true), 0x00);
        assert_eq!(controller.read_presence(false), 0x00);
    }

    #[test]
    fn presence_detection_channel1_selected_with_cdrom() {
        let mut controller = Controller::new();
        controller.write_bank(1, 0x01); // Select channel 1.
        assert_eq!(controller.read_presence(true), 0x02);
    }

    #[test]
    fn presence_detection_channel1_selected_without_cdrom() {
        let mut controller = Controller::new();
        controller.write_bank(1, 0x01); // Select channel 1.
        assert_eq!(controller.read_presence(false), 0x00);
    }

    #[test]
    fn bank0_status_compmode_returns_0() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));
        // Compatibility mode (has_cdrom=true): returns 0 for active device.
        assert_eq!(controller.read_bank0_status(&drives, true), 0x00);
    }

    #[test]
    fn bank0_status_no_compmode_returns_1() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));
        // Non-compatibility mode (no CD-ROM): returns 1 for active device.
        assert_eq!(controller.read_bank0_status(&drives, false), 0x01);
    }

    #[test]
    fn bank0_status_slave_sets_bit6() {
        let mut controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), Some(make_test_drive())];
        // Non-compmode with slave: returns 0x41 (bit 0 + bit 6).
        assert_eq!(controller.read_bank0_status(&drives, false), 0x41);
    }

    #[test]
    fn bank0_status_stores_back_to_bank() {
        let mut controller = Controller::new();
        controller.write_bank(0, 0x31);
        let drives = make_drives(Some(make_test_drive()));
        controller.read_bank0_status(&drives, true);
        // bank[0] should now be 0x00 (compmode result), not 0x31.
        assert_eq!(controller.bank[0], 0x00);
    }

    #[test]
    fn bank0_status_no_device_returns_raw_bank() {
        let mut controller = Controller::new();
        controller.write_bank(0, 0x31);
        let drives: [Option<MountedHdd>; 2] = [None, None];
        // No device on channel 0: returns raw bank[0].
        assert_eq!(controller.read_bank0_status(&drives, false), 0x31);
    }

    #[test]
    fn reading_error_clears_err_status_bit() {
        let mut controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [None, None];

        controller.write_command(0x20, &drives);
        assert_ne!(controller.read_alt_status() & STATUS_ERR, 0);

        controller.read_error();
        assert_eq!(controller.read_alt_status() & STATUS_ERR, 0);
    }

    #[test]
    fn reading_error_preserves_other_status_bits() {
        let mut controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [None, None];

        controller.write_command(0x20, &drives);
        let status = controller.read_alt_status();
        assert_ne!(status & STATUS_DRDY, 0);
        assert_ne!(status & STATUS_DSC, 0);
        assert_ne!(status & STATUS_ERR, 0);

        controller.read_error();
        let status = controller.read_alt_status();
        assert_ne!(status & STATUS_DRDY, 0);
        assert_ne!(status & STATUS_DSC, 0);
        assert_eq!(status & STATUS_ERR, 0);
    }

    #[test]
    fn additional_status_no_hdd() {
        let controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [None, None];
        assert_eq!(controller.read_additional_status(&drives), 0x02);
    }

    #[test]
    fn additional_status_with_hdd() {
        let controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));
        assert_eq!(controller.read_additional_status(&drives), 0x00);
    }

    #[test]
    fn interrupt_sets_bank0_bit7() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_bank(1, 0x11);
        // write_bank(1, 0x11) sets active_channel to 1 (bit 0 = 1).
        // Switch back to channel 0 for HDD commands.
        controller.write_bank(1, 0x10);
        controller.write_command(0x10, &drives);
        controller.complete_operation();

        assert_eq!(controller.read_bank(0), 0x90);
    }

    #[test]
    fn interrupt_copies_bank1_to_bank0_with_bit7() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_bank(0, 0x01);
        controller.write_bank(1, 0x40);
        controller.write_command(0x10, &drives);
        controller.complete_operation();

        assert_eq!(controller.read_bank(0), 0xC0);
    }

    #[test]
    fn no_interrupt_when_nien_preserves_bank() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_device_control(CONTROL_NIEN);
        controller.write_bank(0, 0x01);
        controller.write_bank(1, 0x40);
        controller.write_command(0x10, &drives);

        assert!(!controller.complete_operation());
        assert_eq!(controller.read_bank(0), 0x01);
    }

    #[test]
    fn digital_input_master_default() {
        let mut controller = Controller::new();
        controller.write_device_head(0xA0);

        assert_eq!(controller.read_digital_input(), 0xFE);
    }

    #[test]
    fn digital_input_slave() {
        let mut controller = Controller::new();
        controller.write_device_head(0xB0);

        assert_eq!(controller.read_digital_input() & 0x03, 0x01);
    }

    #[test]
    fn digital_input_head_bits_inverted() {
        let mut controller = Controller::new();
        controller.write_device_head(0xA5);

        assert_eq!(controller.read_digital_input(), 0xEA);
    }

    #[test]
    fn identify_device_word59_reflects_multiple_mode() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_sector_count(4);
        controller.write_command(0xC6, &drives);

        controller.write_command(0xEC, &drives);
        let mut data = vec![0u16; 256];
        for word in data.iter_mut() {
            (*word, _) = controller.read_data_word(&drives);
        }

        assert_eq!(data[59], 0x0104);
    }

    #[test]
    fn identify_device_slave_word93() {
        let mut controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), Some(make_test_drive())];

        controller.write_device_head(0xF0);
        controller.write_command(0xEC, &drives);
        let mut data = vec![0u16; 256];
        for word in data.iter_mut() {
            (*word, _) = controller.read_data_word(&drives);
        }

        assert_eq!(data[93], 0x4B00);
    }

    #[test]
    fn execute_diagnostic_with_both_drives() {
        let mut controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), Some(make_test_drive())];

        let action = controller.write_command(0x90, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert_eq!(controller.phase(), IdePhase::Idle);
        assert_eq!(controller.channels[0].drives[0].error, 0x01);
        assert_eq!(controller.channels[0].drives[1].error, 0x01);
        assert_ne!(controller.channels[0].drives[0].status & STATUS_DRDY, 0);
    }

    #[test]
    fn execute_diagnostic_master_only_passes_and_interrupts() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_command(0x90, &drives);
        assert_eq!(controller.channels[0].drives[0].error, 0x01);
        assert_eq!(controller.channels[0].drives[1].error, 0x01);
        assert!(
            controller.complete_operation(),
            "the diagnostic completion raises the interrupt"
        );
    }

    #[test]
    fn execute_diagnostic_selects_device_zero() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_device_head(0xB0);
        controller.write_command(0x90, &drives);
        assert_eq!(controller.channels[0].selected_drive, 0);
        assert_eq!(controller.channels[0].drives[0].error, 0x01);
    }

    #[test]
    fn bank_read_clears_bit7() {
        let mut controller = Controller::new();
        controller.write_bank(0, 0x11);
        controller.bank[0] |= 0x80;
        assert_eq!(controller.read_bank(0), 0x91);
        assert_eq!(controller.read_bank(0), 0x11);
    }

    #[test]
    fn seek_command_succeeds_with_drive() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        let action = controller.write_command(0x70, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        let status = controller.read_alt_status();
        assert_ne!(status & STATUS_DRDY, 0);
        assert_eq!(status & STATUS_ERR, 0);
    }

    #[test]
    fn seek_command_aborts_without_drive() {
        let mut controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [None, None];

        let action = controller.write_command(0x70, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        let status = controller.read_alt_status();
        assert_ne!(status & STATUS_ERR, 0);
        assert_eq!(controller.read_error() & ERROR_ABRT, ERROR_ABRT);
    }

    #[test]
    fn identify_device_word47_max_multiple() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_command(0xEC, &drives);
        let mut data = vec![0u16; 256];
        for word in data.iter_mut() {
            (*word, _) = controller.read_data_word(&drives);
        }
        assert_eq!(data[47], 0x8080);
    }

    #[test]
    fn read_multiple_block_grouping() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_sector_count(4);
        controller.write_command(0xC6, &drives);

        controller.write_sector_number(0);
        controller.write_cylinder_low(0);
        controller.write_cylinder_high(0);
        controller.write_device_head(0xE0);
        controller.write_sector_count(8);
        controller.write_command(0xC4, &drives);
        assert_eq!(controller.phase(), IdePhase::DataIn);

        // First sector: interrupt_pending should be true (start of block).
        assert!(controller.channels[0].drives[0].interrupt_pending);
        controller.read_status();
        assert!(!controller.channels[0].drives[0].interrupt_pending);

        // Read sector 1 (256 words).
        for _ in 0..256 {
            controller.read_data_word(&drives);
        }

        // After sector 1: within block (sectors_in_block=2), no interrupt.
        assert!(!controller.channels[0].drives[0].interrupt_pending);

        // Read sector 2.
        for _ in 0..256 {
            controller.read_data_word(&drives);
        }
        assert!(!controller.channels[0].drives[0].interrupt_pending);

        // Read sector 3.
        for _ in 0..256 {
            controller.read_data_word(&drives);
        }
        assert!(!controller.channels[0].drives[0].interrupt_pending);

        // Read sector 4 (end of first block): interrupt should fire.
        for _ in 0..256 {
            controller.read_data_word(&drives);
        }
        assert!(controller.channels[0].drives[0].interrupt_pending);
        controller.read_status();

        // Read sector 5 (start of second block).
        for _ in 0..256 {
            controller.read_data_word(&drives);
        }
        assert!(!controller.channels[0].drives[0].interrupt_pending);

        // Read sectors 6, 7.
        for _ in 0..512 {
            controller.read_data_word(&drives);
        }
        assert!(!controller.channels[0].drives[0].interrupt_pending);

        // Read sector 8 (end of second block, also last sector): interrupt should fire.
        for _ in 0..256 {
            controller.read_data_word(&drives);
        }
        assert!(controller.channels[0].drives[0].interrupt_pending);
        assert_eq!(controller.phase(), IdePhase::Idle);
    }

    #[test]
    fn recalibrate_resets_cylinder() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_cylinder_low(0x42);
        controller.write_cylinder_high(0x01);
        controller.write_command(0x10, &drives);

        assert_eq!(controller.read_cylinder_low(), 0);
        assert_eq!(controller.read_cylinder_high(), 0);
    }

    #[test]
    fn bank_write_masks_invalid_bits() {
        let mut controller = Controller::new();
        controller.write_bank(0, 0xFF);
        assert_eq!(controller.read_bank(0), 0x71);

        controller.write_bank(1, 0x80);
        // Bit 7 set: write ignored.
        assert_eq!(controller.read_bank(1), 0x00);

        controller.write_bank(0, 0x31);
        assert_eq!(controller.read_bank(0), 0x31);
    }

    #[test]
    fn device_reset_generates_interrupt() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        let action = controller.write_command(0x08, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert!(controller.complete_operation());
    }

    #[test]
    fn set_features_valid_codes_succeed() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        for code in [0x02, 0x82, 0x03] {
            controller.write_features(code);
            let action = controller.write_command(0xEF, &drives);
            assert_eq!(action, IdeAction::ScheduleCompletion);
            assert_eq!(controller.read_alt_status() & STATUS_ERR, 0);
        }
    }

    #[test]
    fn set_features_invalid_code_aborts() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_features(0xFF);
        let action = controller.write_command(0xEF, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        let status = controller.read_alt_status();
        assert_ne!(status & STATUS_ERR, 0);
        assert_eq!(controller.read_error() & ERROR_ABRT, ERROR_ABRT);
    }

    #[test]
    fn device_reset_master_present_slave_absent() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        let action = controller.write_command(0x08, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        // Drive 0 present: error = 0x01, slave absent: |= 0x80 -> 0x81.
        assert_eq!(controller.channels[0].drives[0].error, 0x81);
    }

    #[test]
    fn device_reset_both_drives_present() {
        let mut controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [Some(make_test_drive()), Some(make_test_drive())];

        let action = controller.write_command(0x08, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        // Drive 0 present, drive 1 present: error = 0x01.
        assert_eq!(controller.channels[0].drives[0].error, 0x01);
    }

    #[test]
    fn device_reset_no_drives() {
        let mut controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [None, None];

        let action = controller.write_command(0x08, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        // Drive 0 absent: error = 0x00, slave absent: |= 0x80 -> 0x80.
        assert_eq!(controller.channels[0].drives[0].error, 0x80);
    }

    #[test]
    fn device_reset_slave_selected_absent() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // Select drive 1 (slave).
        controller.write_device_head(0xB0);
        let action = controller.write_command(0x08, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        // Drive 1 absent: error = 0x00, not drive 0 so no bit 7.
        assert_eq!(controller.channels[0].drives[1].error, 0x00);
    }

    #[test]
    fn init_device_params_stores_geometry() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // Set logical geometry: 5 heads, 17 sectors per track.
        controller.write_device_head(0xA4); // CHS mode, head = 4 -> heads = 5
        controller.write_sector_count(17);
        let action = controller.write_command(0x91, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);

        assert_eq!(controller.channels[0].drives[0].logical_heads, 5);
        assert_eq!(
            controller.channels[0].drives[0].logical_sectors_per_track,
            17
        );
    }

    #[test]
    fn init_device_params_affects_chs_translation() {
        let mut controller = Controller::new();
        // Physical geometry: 20 cyl, 4 heads, 17 spt.
        let drives = make_drives(Some(make_test_drive()));

        // Override logical geometry to 2 heads, 8 spt via command 0x91.
        controller.write_device_head(0xA1); // head = 1 -> heads = 2
        controller.write_sector_count(8);
        controller.write_command(0x91, &drives);

        // Read CHS: cylinder 1, head 0, sector 1.
        // With logical geometry (2 heads, 8 spt): LBA = (1*2+0)*8+(1-1) = 16.
        // With physical geometry (4 heads, 17 spt): LBA = (1*4+0)*17+(1-1) = 68.
        controller.write_cylinder_low(1);
        controller.write_cylinder_high(0);
        controller.write_device_head(0xA0); // CHS, head 0
        controller.write_sector_number(1);
        controller.write_sector_count(1);

        let action = controller.write_command(0x20, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);

        let (first_word, _) = controller.read_data_word(&drives);
        // LBA 16: high byte = 0x00, low byte = 16.
        assert_eq!(first_word & 0xFF, 0x00);
        assert_eq!(first_word >> 8, 16);
    }

    #[test]
    fn bank_switching_selects_channel() {
        let mut controller = Controller::new();
        assert_eq!(controller.active_channel(), 0);

        // Switch to channel 1.
        controller.write_bank(1, 0x01);
        assert_eq!(controller.active_channel(), 1);

        // Switch back to channel 0.
        controller.write_bank(1, 0x00);
        assert_eq!(controller.active_channel(), 0);
    }

    #[test]
    fn bank_switching_isolates_channels() {
        let mut controller = Controller::new();

        // Write to channel 0 registers.
        controller.write_sector_count(0x42);
        controller.write_sector_number(0x13);

        // Switch to channel 1.
        controller.write_bank(1, 0x01);

        // Channel 1 should have default values.
        assert_eq!(controller.read_sector_count(), 1);
        assert_eq!(controller.read_sector_number(), 1);

        // Write different values on channel 1.
        controller.write_sector_count(0x99);

        // Switch back to channel 0.
        controller.write_bank(1, 0x00);

        // Channel 0 values should be preserved.
        assert_eq!(controller.read_sector_count(), 0x42);
        assert_eq!(controller.read_sector_number(), 0x13);
    }

    #[test]
    fn channel1_commands_abort_without_atapi() {
        let mut controller = Controller::new();
        let drives: [Option<MountedHdd>; 2] = [None, None];

        // Switch to channel 1.
        controller.write_bank(1, 0x01);

        // Any command on channel 1 should abort (no ATAPI support yet).
        let action = controller.write_command(0xEC, &drives);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert_ne!(controller.read_alt_status() & STATUS_ERR, 0);
    }

    #[test]
    fn bank_write_bit7_ignored() {
        let mut controller = Controller::new();
        controller.write_bank(1, 0x01); // Channel 1.
        assert_eq!(controller.active_channel(), 1);

        // Write with bit 7 set: should be ignored (status read dummy).
        controller.write_bank(1, 0x80);
        assert_eq!(controller.active_channel(), 1); // Unchanged.
    }

    #[test]
    fn presence_detection_with_cdrom_on_channel1() {
        let mut controller = Controller::new();
        controller.write_bank(1, 0x01); // Select channel 1.
        // Channel 1 selected with CD-ROM: returns 0x02.
        assert_eq!(controller.read_presence(true), 0x02);
    }

    #[test]
    fn initialize_atapi_drive_sets_signature() {
        let mut controller = Controller::new();
        controller.initialize_atapi_drive();

        let drive = &controller.channels[1].drives[0];
        assert_eq!(drive.cylinder_low, 0x14);
        assert_eq!(drive.cylinder_high, 0xEB);
        assert_eq!(drive.sector_count, 0x01);
        assert_eq!(drive.sector_number, 0x01);
        assert_eq!(drive.status, 0x00);
        assert_eq!(drive.error, 0x00);
        assert!(controller.has_atapi_device);
    }

    #[test]
    fn initialize_atapi_drive_signature_readable_on_channel1() {
        let mut controller = Controller::new();
        controller.initialize_atapi_drive();
        controller.write_bank(1, 0x01);

        assert_eq!(controller.read_cylinder_low(), 0x14);
        assert_eq!(controller.read_cylinder_high(), 0xEB);
        assert_eq!(controller.read_sector_count(), 0x01);
        assert_eq!(controller.read_sector_number(), 0x01);
        assert_eq!(controller.read_alt_status(), 0x00);
    }

    #[test]
    fn srst_on_atapi_channel_sets_signature_and_error() {
        let mut controller = Controller::new();
        controller.initialize_atapi_drive();
        controller.write_bank(1, 0x01);

        // Clobber registers to verify SRST restores them.
        controller.channels[1].drives[0].cylinder_low = 0x00;
        controller.channels[1].drives[0].cylinder_high = 0x00;

        // Software reset: set SRST, then clear.
        controller.write_device_control(CONTROL_SRST);
        controller.write_device_control(0);

        let drive = &controller.channels[1].drives[0];
        assert_eq!(drive.cylinder_low, 0x14);
        assert_eq!(drive.cylinder_high, 0xEB);
        assert_eq!(drive.status, STATUS_DRDY | STATUS_DSC | STATUS_ERR);
        assert_eq!(drive.error, 0x01);
        assert_eq!(controller.channels[1].phase, IdePhase::Idle);
    }

    #[test]
    fn srst_on_hdd_channel_does_not_set_atapi_signature() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // Start a read on channel 0.
        controller.write_device_head(0xE0);
        controller.write_sector_number(0);
        controller.write_sector_count(1);
        controller.write_command(0x20, &drives);

        // Software reset on channel 0.
        controller.write_device_control(CONTROL_SRST);
        controller.write_device_control(0);

        // Channel 0 should NOT have ATAPI signature.
        let drive = &controller.channels[0].drives[0];
        assert_eq!(drive.cylinder_low, 0x00);
        assert_eq!(drive.cylinder_high, 0x00);
        assert_ne!(drive.status & STATUS_DRDY, 0);
    }

    #[test]
    fn atapi_set_ready_sets_drdy() {
        let mut controller = Controller::new();
        controller.initialize_atapi_drive();

        controller.atapi_set_ready();

        let drive = &controller.channels[1].drives[0];
        assert_eq!(drive.status, STATUS_DRDY);
        assert_eq!(drive.error, 0);
        assert!(drive.interrupt_pending);
    }

    #[test]
    fn read_atapi_features_returns_channel1_features() {
        let mut controller = Controller::new();
        controller.initialize_atapi_drive();
        controller.write_bank(1, 0x01);
        controller.write_features(0x03);

        assert_eq!(controller.read_atapi_features(), 0x03);
    }

    #[test]
    fn atapi_start_data_in_sets_drdy() {
        let mut controller = Controller::new();
        controller.initialize_atapi_drive();

        controller.atapi_start_data_in(512);

        let drive = &controller.channels[1].drives[0];
        assert_ne!(drive.status & STATUS_DRDY, 0);
        assert_ne!(drive.status & STATUS_DSC, 0);
        assert_ne!(drive.status & STATUS_DRQ, 0);
        assert!(drive.interrupt_pending);
        assert_eq!(controller.channels[1].phase, IdePhase::PacketDataIn);
    }

    #[test]
    fn atapi_start_packet_does_not_set_interrupt_pending() {
        let mut controller = Controller::new();
        controller.initialize_atapi_drive();
        controller.write_bank(1, 0x01);

        controller.channels[1].drives[0].interrupt_pending = false;
        controller.atapi_start_packet();

        assert!(!controller.channels[1].drives[0].interrupt_pending);
        assert_ne!(controller.channels[1].drives[0].status & STATUS_DRQ, 0);
        assert_eq!(controller.channels[1].phase, IdePhase::PacketCommand);
    }

    #[test]
    fn read_status_signals_clear_irq_when_nien_clear() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        controller.write_command(0xE0, &drives);
        controller.complete_operation();
        assert!(controller.channels[0].drives[0].interrupt_pending);

        // NIEN=0 (default): read_status should signal IRQ deassertion.
        let (status, clear_irq) = controller.read_status();
        assert_ne!(status & STATUS_DRDY, 0);
        assert!(clear_irq);
        assert!(!controller.channels[0].drives[0].interrupt_pending);
    }

    #[test]
    fn read_status_does_not_signal_clear_irq_when_nien_set() {
        let mut controller = Controller::new();
        let drives = make_drives(Some(make_test_drive()));

        // Set NIEN (disable interrupts).
        controller.write_device_control(CONTROL_NIEN);

        controller.write_command(0xE0, &drives);
        controller.complete_operation();

        // NIEN=1: read_status should NOT signal IRQ deassertion.
        let (_status, clear_irq) = controller.read_status();
        assert!(!clear_irq);
    }
}
