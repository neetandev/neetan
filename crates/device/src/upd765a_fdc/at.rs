use std::path::PathBuf;

use super::{CTRL_RESET, ST0_READY_LINE_CHANGED, UPD765_PLATFORM_ISA_AT, Upd765aFdc};
use crate::floppy::{
    FloppyImage, MountedFloppy,
    d88::{D88MediaType, D88Sector},
};

/// AT DOR bits 0-1 select the drive.
const DOR_DRIVE_SELECT_MASK: u8 = 0x03;
/// AT DOR bit 2 releases controller reset when set.
const DOR_NOT_RESET: u8 = 0x04;
/// AT DOR bit 3 gates IRQ and DRQ onto the bus.
const DOR_IRQ_DMA_ENABLE: u8 = 0x08;
/// AT DOR bit 4 enables the drive 0 motor.
const DOR_MOTOR_0: u8 = 0x10;
/// AT DSR bit 7 triggers a self-clearing soft reset.
const DSR_SOFT_RESET: u8 = 0x80;
/// AT CCR and DSR bits 0-1 select the data rate.
const RATE_SELECT_MASK: u8 = 0x03;
/// Number of drive slots on the AT controller.
pub(super) const AT_DRIVE_COUNT: usize = 2;
/// Bitmask of the two equipped, permanently ready AT drives.
pub(super) const AT_DRIVES_READY_MASK: u8 = 0x03;

/// FDC data rate selected through the AT CCR or DSR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdcDataRate {
    /// 500 kbps for 2HD media.
    Rate500Kbps,
    /// 300 kbps for 360 KB media in a 1.2 MB drive.
    Rate300Kbps,
    /// 250 kbps for 2DD and 2D media.
    Rate250Kbps,
    /// 1 Mbps for unsupported 2.88 MB ED media.
    Rate1Mbps,
}

impl FdcDataRate {
    fn from_select_bits(value: u8) -> Self {
        match value & RATE_SELECT_MASK {
            0 => FdcDataRate::Rate500Kbps,
            1 => FdcDataRate::Rate300Kbps,
            2 => FdcDataRate::Rate250Kbps,
            _ => FdcDataRate::Rate1Mbps,
        }
    }
}

/// Bus-visible effects of an AT Digital Output Register write.
#[derive(Debug, Clone, Copy, Default)]
pub struct DorEffect {
    /// Reset was asserted and the controller is held in reset.
    pub reset_started: bool,
    /// Reset was released and reset polling should be scheduled.
    pub reset_released: bool,
    /// The IRQ and DRQ gate was dropped.
    pub irq_gate_dropped: bool,
    /// The IRQ and DRQ gate was raised.
    pub irq_gate_raised: bool,
}

impl Upd765aFdc<UPD765_PLATFORM_ISA_AT> {
    /// Reads the AT Digital Output Register.
    pub fn read_dor(&self) -> u8 {
        self.dor
    }

    /// Writes the AT Digital Output Register.
    pub fn write_dor(&mut self, value: u8) -> DorEffect {
        let previous = self.dor;
        self.dor = value;
        let mut effect = DorEffect::default();

        if previous & DOR_NOT_RESET != 0 && value & DOR_NOT_RESET == 0 {
            self.perform_at_reset();
            self.reset_held = true;
            effect.reset_started = true;
        }
        if previous & DOR_NOT_RESET == 0 && value & DOR_NOT_RESET != 0 {
            self.reset_held = false;
            effect.reset_released = true;
        }
        if previous & DOR_IRQ_DMA_ENABLE != 0 && value & DOR_IRQ_DMA_ENABLE == 0 {
            self.state.tc = true;
            self.state.interrupt_pending = false;
            effect.irq_gate_dropped = true;
        }
        if previous & DOR_IRQ_DMA_ENABLE == 0 && value & DOR_IRQ_DMA_ENABLE != 0 {
            effect.irq_gate_raised = true;
        }

        effect
    }

    /// Reads the AT main status register.
    pub fn read_main_status(&self) -> u8 {
        if self.reset_held {
            0
        } else {
            self.read_status()
        }
    }

    /// Reads the AT Digital Input Register.
    pub fn read_dir(&self) -> u8 {
        let drive = (self.dor & DOR_DRIVE_SELECT_MASK) as usize;
        let changed = match self.drives.get(drive) {
            Some(slot) => self.disk_change[drive] || slot.is_none(),
            None => true,
        };
        if changed && self.motor_on(drive) {
            0xFF
        } else {
            0x7F
        }
    }

    /// Writes the AT Configuration Control Register.
    pub fn write_ccr(&mut self, value: u8) {
        self.rate = FdcDataRate::from_select_bits(value);
    }

    /// Writes the AT Data Rate Select Register.
    pub fn write_dsr(&mut self, value: u8) -> bool {
        self.rate = FdcDataRate::from_select_bits(value);
        if value & DSR_SOFT_RESET != 0 && !self.reset_held {
            self.perform_at_reset();
            return true;
        }
        false
    }

    /// Returns the AT data rate.
    pub fn data_rate(&self) -> FdcDataRate {
        self.rate
    }

    /// Returns whether the AT data rate can read the mounted media.
    pub fn data_rate_matches(&self, drive: usize) -> bool {
        let Some(mounted) = self.drives.get(drive).and_then(Option::as_ref) else {
            return true;
        };
        match mounted.image().media_type {
            D88MediaType::Disk2HD => self.rate == FdcDataRate::Rate500Kbps,
            D88MediaType::Disk2DD => self.rate == FdcDataRate::Rate250Kbps,
            D88MediaType::Disk2D => matches!(
                self.rate,
                FdcDataRate::Rate250Kbps | FdcDataRate::Rate300Kbps
            ),
        }
    }

    /// Loads the four AT ready-change statuses raised after reset.
    pub fn raise_reset_polling_status(&mut self) {
        for drive in 0..self.state.drive_st0.len() {
            self.state.drive_st0[drive] = ST0_READY_LINE_CHANGED | drive as u8;
        }
        self.state.interrupt_pending = true;
    }

    /// Returns whether the AT IRQ and DRQ outputs are enabled.
    pub fn irq_enabled(&self) -> bool {
        self.dor & DOR_NOT_RESET != 0 && self.dor & DOR_IRQ_DMA_ENABLE != 0
    }

    /// Returns whether an AT drive motor is enabled.
    pub fn motor_on(&self, drive: usize) -> bool {
        self.dor & (DOR_MOTOR_0 << drive) != 0
    }

    /// Returns the AT drive selected in the DOR.
    pub fn selected_drive(&self) -> usize {
        (self.dor & DOR_DRIVE_SELECT_MASK) as usize
    }

    /// Clears an AT disk-change latch after a head step.
    pub fn clear_disk_change_on_step(&mut self, drive: usize) {
        if drive < AT_DRIVE_COUNT && self.drives[drive].is_some() {
            self.disk_change[drive] = false;
        }
    }

    /// Inserts an AT floppy disk image.
    pub fn insert_drive(&mut self, drive: usize, image: FloppyImage, path: Option<PathBuf>) {
        if drive >= AT_DRIVE_COUNT {
            return;
        }
        if let Some(mounted) = self.drives[drive].take() {
            mounted.eject();
        }
        let mask = 1u8 << drive;
        if image.write_protected {
            self.state.drive_write_protected |= mask;
        } else {
            self.state.drive_write_protected &= !mask;
        }
        self.drives[drive] = Some(MountedFloppy::new(image, path));
        self.disk_change[drive] = true;
    }

    /// Ejects an AT floppy disk image.
    pub fn eject_drive(&mut self, drive: usize) {
        if drive >= AT_DRIVE_COUNT {
            return;
        }
        if let Some(mounted) = self.drives[drive].take() {
            mounted.eject();
        }
        self.state.drive_write_protected &= !(1u8 << drive);
        self.disk_change[drive] = true;
    }

    /// Flushes all dirty AT floppy images.
    pub fn flush_all_drives(&mut self) {
        for drive in self.drives.iter_mut().flatten() {
            drive.flush();
        }
    }

    /// Returns an AT floppy image.
    pub fn drive(&self, drive: usize) -> Option<&FloppyImage> {
        self.drives.get(drive)?.as_ref().map(MountedFloppy::image)
    }

    /// Returns whether an AT drive has media.
    pub fn has_drive(&self, drive: usize) -> bool {
        self.drives.get(drive).is_some_and(Option::is_some)
    }

    /// Returns whether an AT floppy is write-protected.
    pub fn is_write_protected(&self, drive: usize) -> bool {
        self.drives
            .get(drive)
            .and_then(Option::as_ref)
            .is_some_and(|mounted| mounted.image().write_protected)
    }

    /// Reads AT sector data by C/H/R/N near a track index.
    pub fn read_sector_data(
        &self,
        drive: usize,
        track_index: usize,
        cylinder: u8,
        head: u8,
        record: u8,
        size_code: u8,
    ) -> Option<&[u8]> {
        self.drives
            .get(drive)?
            .as_ref()
            .and_then(|mounted| {
                mounted.image().find_sector_near_track_index(
                    track_index,
                    cylinder,
                    head,
                    record,
                    size_code,
                )
            })
            .map(|sector| sector.data.as_slice())
    }

    /// Returns an AT sector at a rotational track index.
    pub fn sector_at_index(
        &self,
        drive: usize,
        track_index: usize,
        sector_index: usize,
    ) -> Option<&D88Sector> {
        self.drives
            .get(drive)?
            .as_ref()
            .and_then(|mounted| mounted.image().sector_at_index(track_index, sector_index))
    }

    /// Writes AT sector data by C/H/R/N near a track index.
    #[allow(clippy::too_many_arguments)]
    pub fn write_sector_data(
        &mut self,
        drive: usize,
        track_index: usize,
        cylinder: u8,
        head: u8,
        record: u8,
        size_code: u8,
        data: &[u8],
    ) -> bool {
        match self.drives.get_mut(drive).and_then(Option::as_mut) {
            Some(mounted) => {
                mounted.write_sector_data(track_index, cylinder, head, record, size_code, data)
            }
            None => false,
        }
    }

    /// Formats an AT floppy track.
    pub fn format_track(
        &mut self,
        drive: usize,
        track_index: usize,
        identifiers: &[(u8, u8, u8, u8)],
        data_size_code: u8,
        fill_byte: u8,
    ) {
        if let Some(mounted) = self.drives.get_mut(drive).and_then(Option::as_mut) {
            mounted.format_track(track_index, identifiers, data_size_code, fill_byte);
        }
    }

    /// Returns an AT sector identifier at a rotational track index.
    pub fn read_id_at_index(
        &self,
        drive: usize,
        track_index: usize,
        sector_index: usize,
    ) -> Option<(u8, u8, u8, u8)> {
        self.sector_at_index(drive, track_index, sector_index)
            .map(|sector| {
                (
                    sector.cylinder,
                    sector.head,
                    sector.record,
                    sector.size_code,
                )
            })
    }

    /// Returns the number of sectors on an AT floppy track.
    pub fn sector_count(&self, drive: usize, track_index: usize) -> usize {
        self.drives
            .get(drive)
            .and_then(Option::as_ref)
            .map(|mounted| mounted.image().sector_count(track_index))
            .unwrap_or(0)
    }

    fn perform_at_reset(&mut self) {
        self.write_control(CTRL_RESET);
        self.write_control(0);
        self.state.drive_has_disk = AT_DRIVES_READY_MASK;
    }
}
