//! AT primary-channel ATA controller on the shared LLE core.
//!
//! Exposes the standard task-file register set decoded by the machine at
//! ports 0x1F0-0x1F7/0x3F6. Only channel 0 with its two hard drives is
//! used. The ATAPI machinery of the shared core stays dormant until a
//! later phase wires the secondary channel.

use std::path::PathBuf;

use super::lle::{self, IdeAction};
use crate::disk::{HddGeometry, HddImage, MountedHdd};

/// Number of drives on the primary channel.
const DRIVE_COUNT: usize = 2;

save_state::runtime_state! {
/// Authoritative AT primary IDE channel state.
#[derive(Clone)]
pub struct AtIdeControllerState {
    controller: lle::Controller,
    media: save_state::MediaManifest,
}}

/// AT primary-channel IDE controller with up to two hard drives.
#[derive(Debug)]
pub struct AtIdeController {
    controller: lle::Controller,
    drives: [Option<MountedHdd>; DRIVE_COUNT],
}

impl Default for AtIdeController {
    fn default() -> Self {
        Self::new()
    }
}

impl AtIdeController {
    /// Creates an idle controller with no drives attached.
    pub fn new() -> Self {
        Self {
            controller: lle::Controller::new(),
            drives: [None, None],
        }
    }

    /// Captures the task file, active transfer, and mounted media bindings.
    pub fn capture_state(&self) -> Result<AtIdeControllerState, save_state::StateValidationError> {
        Ok(AtIdeControllerState {
            controller: self.controller.clone(),
            media: self.media_manifest()?,
        })
    }

    /// Restores controller electronics while retaining mounted disk contents.
    pub fn restore_state(
        &mut self,
        state: AtIdeControllerState,
    ) -> Result<(), save_state::StateValidationError> {
        state.controller.validate_state()?;
        state.media.verify_current(&self.media_manifest()?)?;
        self.controller = state.controller;
        Ok(())
    }

    /// Returns stable bindings for mounted hard disks.
    pub fn media_manifest(
        &self,
    ) -> Result<save_state::MediaManifest, save_state::StateValidationError> {
        let mut bindings = Vec::new();
        for (drive_index, mounted) in self.drives.iter().enumerate() {
            let Some(mounted) = mounted else {
                continue;
            };
            let geometry = mounted.geometry();
            bindings.push(save_state::MediaBinding {
                identifier: save_state::MediaBindingId::new(format!("ide-{drive_index}"))?,
                slot: save_state::MediaSlot::new(
                    save_state::MediaKind::HardDisk,
                    drive_index as u32,
                ),
                source_path: mounted.source_path().cloned(),
                media_type: mounted.image().format_name().to_owned(),
                identity: mounted.identity(),
                geometry: Some(save_state::MediaGeometry::new(
                    u32::from(geometry.cylinders),
                    u32::from(geometry.heads),
                    u32::from(geometry.sectors_per_track),
                    u32::from(geometry.sector_size),
                )?),
                write_protected: false,
                backend_generation: None,
            });
        }
        save_state::MediaManifest::new(bindings)
    }

    /// Inserts a hard disk image into the specified drive (0-1).
    pub fn insert_drive(&mut self, drive: usize, image: HddImage, path: Option<PathBuf>) {
        self.insert_drive_backed(drive, image, path.into());
    }

    /// Inserts a hard disk image with the requested backing (drive 0-1).
    pub fn insert_drive_backed(
        &mut self,
        drive: usize,
        image: HddImage,
        backing: common::MediaBacking,
    ) {
        if drive >= DRIVE_COUNT {
            return;
        }
        let sector_size = image.geometry.sector_size as usize;
        if let Some(mounted) = self.drives[drive].take() {
            mounted.eject();
        }
        self.drives[drive] = Some(crate::disk::mounted_hdd_from_backing(image, backing));
        self.controller.set_drive_sector_size(0, drive, sector_size);
    }

    /// Returns the current in-memory bytes of the disk in `drive`, if mounted.
    pub fn drive_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.drives
            .get(drive)?
            .as_ref()
            .map(MountedHdd::image_bytes)
    }

    /// Ejects the hard disk from the specified drive, flushing if dirty.
    pub fn eject_drive(&mut self, drive: usize) {
        if let Some(mounted) = self.drives.get_mut(drive).and_then(Option::take) {
            mounted.eject();
        }
    }

    /// Flushes all dirty drive images to their backing files.
    pub fn flush(&mut self) {
        for drive in self.drives.iter_mut().flatten() {
            drive.flush();
        }
    }

    /// Returns whether a drive is attached.
    pub fn has_drive(&self, drive: usize) -> bool {
        self.drives.get(drive).is_some_and(Option::is_some)
    }

    /// Returns the geometry of the selected drive, if present.
    pub fn drive_geometry(&self, drive: usize) -> Option<HddGeometry> {
        self.drives.get(drive)?.as_ref().map(MountedHdd::geometry)
    }

    /// Reads the 16-bit data register (port 0x1F0).
    pub fn read_data_word(&mut self) -> (u16, IdeAction) {
        self.controller.read_data_word(&self.drives)
    }

    /// Writes the 16-bit data register (port 0x1F0).
    pub fn write_data_word(&mut self, value: u16) -> IdeAction {
        self.controller.write_data_word(value, &mut self.drives)
    }

    /// Reads the error register (port 0x1F1).
    pub fn read_error(&mut self) -> u8 {
        self.controller.read_error()
    }

    /// Reads the sector count register (port 0x1F2).
    pub fn read_sector_count(&self) -> u8 {
        self.controller.read_sector_count()
    }

    /// Reads the sector number register (port 0x1F3).
    pub fn read_sector_number(&self) -> u8 {
        self.controller.read_sector_number()
    }

    /// Reads the cylinder low register (port 0x1F4).
    pub fn read_cylinder_low(&self) -> u8 {
        self.controller.read_cylinder_low()
    }

    /// Reads the cylinder high register (port 0x1F5).
    pub fn read_cylinder_high(&self) -> u8 {
        self.controller.read_cylinder_high()
    }

    /// Reads the device/head register (port 0x1F6).
    pub fn read_device_head(&self) -> u8 {
        self.controller.read_device_head()
    }

    /// Reads the status register (port 0x1F7). Clears the pending interrupt;
    /// the returned flag asks the bus to deassert the IRQ line.
    pub fn read_status(&mut self) -> (u8, bool) {
        self.controller.read_status()
    }

    /// Reads the alternate status register (port 0x3F6).
    pub fn read_alt_status(&self) -> u8 {
        self.controller.read_alt_status()
    }

    /// Writes the features register (port 0x1F1).
    pub fn write_features(&mut self, value: u8) {
        self.controller.write_features(value);
    }

    /// Writes the sector count register (port 0x1F2).
    pub fn write_sector_count(&mut self, value: u8) {
        self.controller.write_sector_count(value);
    }

    /// Writes the sector number register (port 0x1F3).
    pub fn write_sector_number(&mut self, value: u8) {
        self.controller.write_sector_number(value);
    }

    /// Writes the cylinder low register (port 0x1F4).
    pub fn write_cylinder_low(&mut self, value: u8) {
        self.controller.write_cylinder_low(value);
    }

    /// Writes the cylinder high register (port 0x1F5).
    pub fn write_cylinder_high(&mut self, value: u8) {
        self.controller.write_cylinder_high(value);
    }

    /// Writes the device/head register (port 0x1F6).
    pub fn write_device_head(&mut self, value: u8) {
        self.controller.write_device_head(value);
    }

    /// Writes the command register (port 0x1F7).
    pub fn write_command(&mut self, value: u8) -> IdeAction {
        self.controller.write_command(value, &self.drives)
    }

    /// Writes the device control register (port 0x3F6).
    pub fn write_device_control(&mut self, value: u8) {
        self.controller.write_device_control(value);
    }

    /// Called when the scheduled completion event fires.
    /// Returns true if an interrupt should be raised.
    pub fn complete_operation(&mut self) -> bool {
        self.controller.complete_operation()
    }
}
