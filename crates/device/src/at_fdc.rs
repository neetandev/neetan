//! uPD765-compatible Floppy Disk Controller as wired on the IBM PC/AT.
//!
//! The AT front-end adds the Digital Output Register (drive select, reset,
//! IRQ/DMA gate, motor enables), the Digital Input Register with the
//! disk-change bit, and the Configuration Control Register data-rate
//! select. The command engine is the shared [`Upd765aFdc`].

use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use common::warn;

use crate::{
    floppy::{
        FloppyImage, MountedFloppy,
        d88::{D88MediaType, D88Sector},
    },
    upd765a_fdc::{FdcAction, ST0_READY_LINE_CHANGED, Upd765aFdc},
};

/// DOR bits 0-1: drive select.
const DOR_DRIVE_SELECT_MASK: u8 = 0x03;
/// DOR bit 2: controller reset, active low (0 holds the controller in reset).
const DOR_NOT_RESET: u8 = 0x04;
/// DOR bit 3: gates the IRQ and DRQ outputs onto the bus.
const DOR_IRQ_DMA_ENABLE: u8 = 0x08;
/// DOR bit 4: drive 0 motor enable; bits 5-7 are drives 1-3.
const DOR_MOTOR_0: u8 = 0x10;

/// DSR bit 7: self-clearing soft reset.
const DSR_SOFT_RESET: u8 = 0x80;

/// CCR/DSR bits 0-1: data rate select.
const RATE_SELECT_MASK: u8 = 0x03;

/// Number of drive slots on the AT controller.
const DRIVE_COUNT: usize = 2;

/// Bitmask of the two equipped, permanently-ready drives.
const DRIVES_READY_MASK: u8 = 0x03;

/// FDC data rate selected through the CCR or DSR (bits 0-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdcDataRate {
    /// 500 kbps (2HD media).
    Rate500Kbps,
    /// 300 kbps (360 KB media spun at 360 rpm in a 1.2 MB drive).
    Rate300Kbps,
    /// 250 kbps (2DD and 2D media).
    Rate250Kbps,
    /// 1 Mbps (2.88 MB ED media, unsupported).
    Rate1Mbps,
}

impl FdcDataRate {
    /// Decodes the CCR/DSR rate-select bits.
    fn from_select_bits(value: u8) -> Self {
        match value & RATE_SELECT_MASK {
            0 => FdcDataRate::Rate500Kbps,
            1 => FdcDataRate::Rate300Kbps,
            2 => FdcDataRate::Rate250Kbps,
            _ => FdcDataRate::Rate1Mbps,
        }
    }
}

/// Bus-visible effects of a DOR write.
#[derive(Debug, Clone, Copy, Default)]
pub struct DorEffect {
    /// Reset was asserted (bit 2 fell): the controller is held in reset.
    pub reset_started: bool,
    /// Reset was released (bit 2 rose): the bus schedules the polling
    /// interrupt that yields the four-drive SENSE INTERRUPT drain.
    pub reset_released: bool,
    /// The IRQ/DRQ gate (bit 3) was dropped: the bus clears the IRQ line.
    pub irq_gate_dropped: bool,
    /// The IRQ/DRQ gate (bit 3) was raised: the bus may deliver a pending IRQ.
    pub irq_gate_raised: bool,
}

/// uPD765-compatible FDC with the AT front-end registers.
pub struct AtFdc {
    /// Embedded uPD765A command engine.
    core: Upd765aFdc,
    /// Mounted floppy disks (drives 0 and 1).
    drives: [Option<MountedFloppy>; DRIVE_COUNT],
    /// Digital Output Register.
    dor: u8,
    /// Data rate selected through the CCR or DSR.
    rate: FdcDataRate,
    /// Disk-change latch per drive, reported in DIR bit 7.
    disk_change: [bool; DRIVE_COUNT],
    /// Whether DOR bit 2 currently holds the controller in reset.
    reset_held: bool,
    /// Whether the unsupported non-DMA mode was already reported.
    warned_non_dma: bool,
}

impl Deref for AtFdc {
    type Target = Upd765aFdc;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl DerefMut for AtFdc {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl Default for AtFdc {
    fn default() -> Self {
        Self::new()
    }
}

impl AtFdc {
    /// Creates an AT FDC held in reset (power-on DOR is 0x00).
    pub fn new() -> Self {
        let mut core = Upd765aFdc::new();
        core.state.scan_enabled = true;
        core.state.drive_has_disk = DRIVES_READY_MASK;
        Self {
            core,
            drives: [None, None],
            dor: 0x00,
            rate: FdcDataRate::Rate250Kbps,
            disk_change: [false; DRIVE_COUNT],
            reset_held: true,
            warned_non_dma: false,
        }
    }

    /// Reads the Digital Output Register.
    pub fn read_dor(&self) -> u8 {
        self.dor
    }

    /// Writes the Digital Output Register.
    pub fn write_dor(&mut self, value: u8) -> DorEffect {
        let previous = self.dor;
        self.dor = value;
        let mut effect = DorEffect::default();

        if previous & DOR_NOT_RESET != 0 && value & DOR_NOT_RESET == 0 {
            self.perform_reset();
            self.reset_held = true;
            effect.reset_started = true;
        }
        if previous & DOR_NOT_RESET == 0 && value & DOR_NOT_RESET != 0 {
            self.reset_held = false;
            effect.reset_released = true;
        }
        if previous & DOR_IRQ_DMA_ENABLE != 0 && value & DOR_IRQ_DMA_ENABLE == 0 {
            self.core.state.tc = true;
            self.core.state.interrupt_pending = false;
            effect.irq_gate_dropped = true;
        }
        if previous & DOR_IRQ_DMA_ENABLE == 0 && value & DOR_IRQ_DMA_ENABLE != 0 {
            effect.irq_gate_raised = true;
        }

        effect
    }

    /// Reads the main status register; 0x00 while reset is held.
    pub fn read_main_status(&self) -> u8 {
        if self.reset_held {
            0x00
        } else {
            self.core.read_status()
        }
    }

    /// Reads the data register.
    pub fn read_data(&mut self) -> u8 {
        if self.reset_held {
            return 0xFF;
        }
        self.core.read_data()
    }

    /// Writes the data register; command bytes are ignored while reset is held.
    pub fn write_data(&mut self, value: u8) -> FdcAction {
        if self.reset_held {
            return FdcAction::None;
        }
        let action = self.core.write_data(value);
        if action != FdcAction::None && self.core.state.nd && !self.warned_non_dma {
            warn!("AT FDC: non-DMA mode requested; only the DMA path is implemented");
            self.warned_non_dma = true;
        }
        action
    }

    /// Reads the Digital Input Register: bit 7 is the disk-change latch of
    /// the selected drive, gated by its motor bit; bits 0-6 read as 1 on AT.
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

    /// Writes the Configuration Control Register (data rate select).
    pub fn write_ccr(&mut self, value: u8) {
        self.rate = FdcDataRate::from_select_bits(value);
    }

    /// Writes the Data Rate Select Register. Returns `true` when bit 7
    /// triggered the self-clearing soft reset (the bus then schedules the
    /// reset polling interrupt).
    pub fn write_dsr(&mut self, value: u8) -> bool {
        self.rate = FdcDataRate::from_select_bits(value);
        if value & DSR_SOFT_RESET != 0 && !self.reset_held {
            self.perform_reset();
            return true;
        }
        false
    }

    /// Returns the currently selected data rate.
    pub fn data_rate(&self) -> FdcDataRate {
        self.rate
    }

    /// Whether the selected data rate can read the media in `drive`.
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

    /// Loads the four-drive ready-change statuses raised after a reset;
    /// the host drains them with four SENSE INTERRUPT STATUS commands.
    pub fn raise_reset_polling_status(&mut self) {
        for drive in 0..self.core.state.drive_st0.len() {
            self.core.state.drive_st0[drive] = ST0_READY_LINE_CHANGED | drive as u8;
        }
        self.core.state.interrupt_pending = true;
    }

    /// Whether the IRQ and DRQ outputs are gated onto the bus.
    pub fn irq_enabled(&self) -> bool {
        self.dor & DOR_NOT_RESET != 0 && self.dor & DOR_IRQ_DMA_ENABLE != 0
    }

    /// Whether the selected drive's motor bit is on.
    pub fn motor_on(&self, drive: usize) -> bool {
        self.dor & (DOR_MOTOR_0 << drive) != 0
    }

    /// Returns the drive selected in the DOR.
    pub fn selected_drive(&self) -> usize {
        (self.dor & DOR_DRIVE_SELECT_MASK) as usize
    }

    /// Clears the disk-change latch when a commanded head step executes
    /// with media present.
    pub fn clear_disk_change_on_step(&mut self, drive: usize) {
        if drive < DRIVE_COUNT && self.drives[drive].is_some() {
            self.disk_change[drive] = false;
        }
    }

    /// Inserts a floppy disk image into the specified drive (0-1).
    pub fn insert_drive(&mut self, drive: usize, image: FloppyImage, path: Option<PathBuf>) {
        if drive >= DRIVE_COUNT {
            return;
        }
        if let Some(mounted) = self.drives[drive].take() {
            mounted.eject();
        }
        let mask = 1u8 << drive;
        if image.write_protected {
            self.core.state.drive_write_protected |= mask;
        } else {
            self.core.state.drive_write_protected &= !mask;
        }
        self.drives[drive] = Some(MountedFloppy::new(image, path));
        self.disk_change[drive] = true;
    }

    /// Ejects the floppy disk from the specified drive, flushing if dirty.
    pub fn eject_drive(&mut self, drive: usize) {
        if drive >= DRIVE_COUNT {
            return;
        }
        if let Some(mounted) = self.drives[drive].take() {
            mounted.eject();
        }
        self.core.state.drive_write_protected &= !(1u8 << drive);
        self.disk_change[drive] = true;
    }

    /// Flushes all dirty floppy images to disk.
    pub fn flush_all_drives(&mut self) {
        for drive in self.drives.iter_mut().flatten() {
            drive.flush();
        }
    }

    /// Returns a reference to the disk image in the given drive, if present.
    pub fn drive(&self, drive: usize) -> Option<&FloppyImage> {
        self.drives.get(drive)?.as_ref().map(MountedFloppy::image)
    }

    /// Returns whether a drive has a disk inserted.
    pub fn has_drive(&self, drive: usize) -> bool {
        self.drives.get(drive).is_some_and(Option::is_some)
    }

    /// Returns whether the disk in the specified drive is write-protected.
    pub fn is_write_protected(&self, drive: usize) -> bool {
        self.drives
            .get(drive)
            .and_then(Option::as_ref)
            .is_some_and(|mounted| mounted.image().write_protected)
    }

    /// Reads sector data by C/H/R/N near the given track index.
    pub fn read_sector_data(
        &self,
        drive: usize,
        track_index: usize,
        c: u8,
        h: u8,
        r: u8,
        n: u8,
    ) -> Option<&[u8]> {
        self.drives
            .get(drive)?
            .as_ref()
            .and_then(|mounted| {
                mounted
                    .image()
                    .find_sector_near_track_index(track_index, c, h, r, n)
            })
            .map(|sector| sector.data.as_slice())
    }

    /// Returns the full sector record at the given rotational index on a track.
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

    /// Writes sector data by C/H/R/N near the given track index.
    /// Returns `true` if the sector was found and written.
    #[allow(clippy::too_many_arguments)]
    pub fn write_sector_data(
        &mut self,
        drive: usize,
        track_index: usize,
        c: u8,
        h: u8,
        r: u8,
        n: u8,
        data: &[u8],
    ) -> bool {
        match self.drives.get_mut(drive).and_then(Option::as_mut) {
            Some(mounted) => mounted.write_sector_data(track_index, c, h, r, n, data),
            None => false,
        }
    }

    /// Formats a track on the specified drive.
    pub fn format_track(
        &mut self,
        drive: usize,
        track_index: usize,
        chrn: &[(u8, u8, u8, u8)],
        data_n: u8,
        fill_byte: u8,
    ) {
        if let Some(mounted) = self.drives.get_mut(drive).and_then(Option::as_mut) {
            mounted.format_track(track_index, chrn, data_n, fill_byte);
        }
    }

    /// Returns the sector ID (C, H, R, N) at the given rotational index.
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

    /// Returns the number of sectors on a track for the specified drive.
    pub fn sector_count(&self, drive: usize, track_index: usize) -> usize {
        self.drives
            .get(drive)
            .and_then(Option::as_ref)
            .map(|mounted| mounted.image().sector_count(track_index))
            .unwrap_or(0)
    }

    /// Resets the command engine through the shared control-register edge;
    /// head positions and motor bits survive.
    fn perform_reset(&mut self) {
        self.core.write_control(0x80);
        self.core.write_control(0x00);
        self.core.state.drive_has_disk = DRIVES_READY_MASK;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dor_reports_irq_gate_edges() {
        let mut fdc = AtFdc::new();

        let effect = fdc.write_dor(0x0C);
        assert!(effect.reset_released);
        assert!(effect.irq_gate_raised);
        assert!(!effect.irq_gate_dropped);

        let effect = fdc.write_dor(0x1C);
        assert!(!effect.reset_released);
        assert!(!effect.irq_gate_raised);
        assert!(!effect.irq_gate_dropped);

        let effect = fdc.write_dor(0x14);
        assert!(!effect.irq_gate_raised);
        assert!(effect.irq_gate_dropped);
    }
}
