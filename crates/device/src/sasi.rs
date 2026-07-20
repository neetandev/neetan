//! PC-9801-27 SASI hard disk controller.
//!
//! Consolidates the SASI HLE (High-Level Emulation) and LLE (Low-Level
//! Emulation) into a single controller module. The HLE intercepts INT 1Bh
//! via an expansion ROM stub at 0xD7000, while the LLE handles direct
//! hardware register access at ports 0x80/0x82.

mod hle;
mod lle;
mod target;
mod x68k_hdc;

use std::{cell::Cell, path::PathBuf};

pub use lle::{SasiAction, SasiPhase};
pub use x68k_hdc::{X68K_SASI_DRIVE_COUNT, X68kSasiHdc, X68kSasiHdcState};

use crate::disk::{HddGeometry, HddImage, MountedHdd};
pub use crate::disk_hle::{buffer_address, drive_index, sector_position, transfer_size};

/// Size of the expansion ROM window mapped at 0xD7000.
pub const ROM_SIZE: usize = 4096;

/// 4096-byte expansion ROM image.
static ROM_IMAGE: &[u8; ROM_SIZE] = include_bytes!("../../../utils/sasi/sasi.rom");

save_state::runtime_state! {
/// Authoritative PC-98 SASI electronics and media binding state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SasiControllerState {
    lle: lle::ControllerState,
    hle_pending: bool,
    yield_requested: bool,
    rom_installed: bool,
    media: save_state::MediaManifest,
}}

/// Offset of drive-0 full-height parameter table inside the expansion ROM.
const PARAM_TABLE_D0_FULL_OFFSET: usize = 0x0200;
/// Offset of drive-0 half-height parameter table inside the expansion ROM.
const PARAM_TABLE_D0_HALF_OFFSET: usize = 0x0220;
/// Offset of drive-1 full-height parameter table inside the expansion ROM.
const PARAM_TABLE_D1_FULL_OFFSET: usize = 0x0240;
/// Offset of drive-1 half-height parameter table inside the expansion ROM.
const PARAM_TABLE_D1_HALF_OFFSET: usize = 0x0260;
/// Parameter table size in bytes.
const PARAM_TABLE_SIZE: usize = 0x20;

/// PC-9801-27 SASI hard disk controller.
#[derive(Debug)]
pub struct SasiController {
    lle_controller: lle::Controller,
    hle_pending: bool,
    yield_requested: Cell<bool>,
    rom: Option<Box<[u8; ROM_SIZE]>>,
    drives: [Option<MountedHdd>; 2],
}

impl Default for SasiController {
    fn default() -> Self {
        Self::new()
    }
}

impl SasiController {
    /// Creates a new idle SASI controller.
    pub fn new() -> Self {
        Self {
            lle_controller: lle::Controller::new(),
            hle_pending: false,
            yield_requested: Cell::new(false),
            rom: None,
            drives: [None, None],
        }
    }

    /// Captures SASI electronics and stable mounted image identities.
    pub fn capture_state(&self) -> Result<SasiControllerState, save_state::StateValidationError> {
        Ok(SasiControllerState {
            lle: self.lle_controller.capture_state(),
            hle_pending: self.hle_pending,
            yield_requested: self.yield_requested.get(),
            rom_installed: self.rom.is_some(),
            media: self.media_manifest()?,
        })
    }

    /// Restores SASI electronics while retaining hard disk contents.
    pub fn restore_state(
        &mut self,
        state: SasiControllerState,
    ) -> Result<(), save_state::StateValidationError> {
        lle::Controller::validate_state(&state.lle)?;
        state.media.verify_current(&self.media_manifest()?)?;
        if state.rom_installed != self.rom.is_some() {
            return Err(save_state::StateValidationError::new(
                "SASI expansion ROM installation differs",
            ));
        }
        self.lle_controller.restore_state(state.lle);
        self.hle_pending = state.hle_pending;
        self.yield_requested.set(state.yield_requested);
        Ok(())
    }

    /// Returns stable bindings for mounted SASI hard disks.
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
                identifier: save_state::MediaBindingId::new(format!("sasi-{drive_index}"))?,
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
    /// Installs the expansion ROM on the first insertion.
    pub fn insert_drive(&mut self, drive: usize, image: HddImage, path: Option<PathBuf>) {
        self.insert_drive_backed(drive, image, path.into());
    }

    /// Inserts a hard disk image with the requested backing.
    /// Installs the expansion ROM on the first insertion.
    pub fn insert_drive_backed(
        &mut self,
        drive: usize,
        image: HddImage,
        backing: common::MediaBacking,
    ) {
        if let Some(mounted) = self.drives[drive].take() {
            mounted.eject();
        }
        self.drives[drive] = Some(crate::disk::mounted_hdd_from_backing(image, backing));
        if self.rom.is_none() {
            self.rom = Some(Box::new(*ROM_IMAGE));
        }
        self.refresh_parameter_tables(drive);
    }

    /// Returns the current in-memory bytes of the disk in `drive`, if mounted.
    pub fn drive_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.drives
            .get(drive)?
            .as_ref()
            .map(MountedHdd::image_bytes)
    }

    /// Flushes the HDD image to its source file.
    pub fn flush_drive(&mut self, drive: usize) {
        if let Some(mounted) = self.drives[drive].as_mut() {
            mounted.flush();
        }
    }

    /// Flushes all dirty HDD images to disk.
    pub fn flush_all_drives(&mut self) {
        for drive in 0..2 {
            self.flush_drive(drive);
        }
    }

    /// Executes a BIOS read: reads sectors and writes to memory via closure.
    pub fn execute_read(
        &self,
        drive_idx: usize,
        xfer_size: u32,
        sector_pos: u32,
        buf_addr: u32,
        write_byte: impl FnMut(u32, u8),
    ) -> u8 {
        crate::disk_hle::execute_read(
            drive_idx,
            xfer_size,
            sector_pos,
            buf_addr,
            &self.drives,
            write_byte,
        )
    }

    /// Reads the boot sector (LBA 0, 1024 bytes) from the specified drive
    /// into a local buffer and returns it.
    pub fn read_boot_sector(&self, drive_idx: usize) -> Option<Vec<u8>> {
        let geometry = self.drive_geometry(drive_idx)?;
        let pos = crate::disk_hle::sector_position(0x80 | drive_idx as u8, 0, 0, &geometry);
        let mut buf = vec![0u8; 0x0400];
        let mut offset = 0usize;
        let result = self.execute_read(drive_idx, 0x0400, pos, 0, |_addr, byte| {
            buf[offset] = byte;
            offset += 1;
        });
        if result < 0x20 { Some(buf) } else { None }
    }

    /// Executes a BIOS write: reads from memory via closure and writes sectors.
    pub fn execute_write(
        &mut self,
        drive_idx: usize,
        xfer_size: u32,
        sector_pos: u32,
        buf_addr: u32,
        read_byte: impl FnMut(u32) -> u8,
    ) -> u8 {
        crate::disk_hle::execute_write::<256>(
            drive_idx,
            xfer_size,
            sector_pos,
            buf_addr,
            &mut self.drives,
            read_byte,
        )
    }

    /// Executes a BIOS sense: returns the SASI media type.
    pub fn execute_sense(&self, drive_idx: usize) -> u8 {
        hle::execute_sense(drive_idx, &self.drives)
    }

    /// Executes a BIOS init: returns the disk equipment word.
    /// Preserves non-SASI bits from `current_equip`.
    pub fn execute_init(&self, current_equip: u16) -> u16 {
        crate::disk_hle::execute_init(&self.drives, current_equip)
    }

    /// Executes a BIOS format on a track.
    pub fn execute_format(&mut self, drive_idx: usize, sector_pos: u32) -> u8 {
        crate::disk_hle::execute_format(drive_idx, sector_pos, &mut self.drives)
    }

    /// Executes a BIOS mode set (function 0x0E): selects drive mode
    /// (half-height / full-height). Returns 0x00 if the drive exists, 0x60 otherwise.
    pub fn execute_mode_set(&self, drive_idx: usize) -> u8 {
        if self.drives[drive_idx].is_some() {
            0x00
        } else {
            0x60
        }
    }

    /// Returns the HDD parameter table pointer `(offset, segment)` selected by
    /// the mode-set call for the specified drive.
    pub fn mode_set_parameter_pointer(
        &self,
        drive_idx: usize,
        function_code: u8,
    ) -> Option<(u16, u16)> {
        if self.drives.get(drive_idx)?.is_none() {
            return None;
        }

        let half_height_mode = function_code & 0x80 != 0;
        let offset = match (drive_idx, half_height_mode) {
            (0, false) => PARAM_TABLE_D0_FULL_OFFSET,
            (0, true) => PARAM_TABLE_D0_HALF_OFFSET,
            (1, false) => PARAM_TABLE_D1_FULL_OFFSET,
            (1, true) => PARAM_TABLE_D1_HALF_OFFSET,
            _ => return None,
        };
        Some((offset as u16, 0xD700))
    }

    /// Returns true if the expansion ROM is installed.
    pub fn rom_installed(&self) -> bool {
        self.rom.is_some()
    }

    /// Returns the geometry of the selected drive, if present.
    pub fn drive_geometry(&self, drive: usize) -> Option<HddGeometry> {
        self.drives.get(drive)?.as_ref().map(MountedHdd::geometry)
    }

    /// Reads a single sector by LBA, returning a copy of the data.
    pub fn read_sector_raw(&self, drive: usize, lba: u32) -> Option<Vec<u8>> {
        self.drives
            .get(drive)?
            .as_ref()?
            .read_sector(lba)
            .map(|data| data.to_vec())
    }

    /// Reads a single sector by LBA as a borrowed slice.
    pub fn read_sector(&self, drive: usize, lba: u32) -> Option<&[u8]> {
        self.drives.get(drive)?.as_ref()?.read_sector(lba)
    }

    /// Writes a single sector by LBA.
    pub fn write_sector_raw(&mut self, drive: usize, lba: u32, data: &[u8]) -> bool {
        match self.drives.get_mut(drive).and_then(Option::as_mut) {
            Some(mounted) => mounted.write_sector(lba, data),
            None => false,
        }
    }

    /// Returns the sector size for the given drive.
    pub fn sector_size_for_drive(&self, drive: usize) -> Option<u16> {
        self.drives
            .get(drive)?
            .as_ref()
            .map(|m| m.geometry().sector_size)
    }

    /// Returns the total sector count for the given drive.
    pub fn total_sectors_for_drive(&self, drive: usize) -> Option<u32> {
        self.drives
            .get(drive)?
            .as_ref()
            .map(|m| m.geometry().total_sectors())
    }

    /// Reads a byte from the expansion ROM at the given offset.
    pub fn read_rom_byte(&self, offset: usize) -> u8 {
        self.rom.as_ref().map_or(0xFF, |rom| rom[offset])
    }

    /// Writes a byte to the trap port (0x07EF).
    /// Any single byte triggers the HLE trap.
    pub fn write_trap_port(&mut self, _value: u8) {
        self.hle_pending = true;
        self.yield_requested.set(true);
    }

    /// Returns true if a SASI HLE trap is pending.
    pub fn hle_pending(&self) -> bool {
        self.hle_pending
    }

    /// Clears the HLE pending flag after execution.
    pub fn clear_hle_pending(&mut self) {
        self.hle_pending = false;
    }

    /// Returns and clears the yield-requested flag.
    pub fn take_yield_requested(&self) -> bool {
        self.yield_requested.replace(false)
    }

    /// Reads from port 0x80 (data register).
    pub fn read_data(&mut self) -> u8 {
        self.lle_controller.read_data(&self.drives)
    }

    /// Writes to port 0x80 (data register).
    /// Handles pending sector writes internally.
    pub fn write_data(&mut self, value: u8) -> SasiAction {
        let action = self.lle_controller.write_data(value, &self.drives);
        if let Some((unit, sector, data)) = self.lle_controller.pending_write_data()
            && let Some(drive) = &mut self.drives[unit as usize]
        {
            drive.write_sector(sector, data);
        }
        if action == SasiAction::FormatTrack {
            self.do_format_track();
            return SasiAction::ScheduleCompletion;
        }
        action
    }

    /// Performs a format track operation on the currently selected drive.
    fn do_format_track(&mut self) {
        let unit = self.lle_controller.current_unit() as usize;
        let sector = self.lle_controller.current_sector();
        if let Some(drive) = &mut self.drives[unit] {
            drive.format_track(sector);
        }
    }

    /// Reads from port 0x82 (status register).
    pub fn read_status(&mut self) -> u8 {
        self.lle_controller.read_status(&self.drives)
    }

    /// Writes to port 0x82 (control register).
    pub fn write_control(&mut self, value: u8) -> SasiAction {
        self.lle_controller.write_control(value)
    }

    /// Returns the current controller phase.
    pub fn phase(&self) -> SasiPhase {
        self.lle_controller.phase()
    }

    /// Returns the currently selected drive number.
    pub fn current_unit(&self) -> u8 {
        self.lle_controller.current_unit()
    }

    /// Returns whether DMA transfer is active.
    pub fn dma_ready(&self) -> bool {
        self.lle_controller.dma_ready()
    }

    /// DMA read: transfers one byte from disk to host.
    pub fn dma_read_byte(&mut self) -> (u8, SasiAction) {
        self.lle_controller.dma_read_byte(&self.drives)
    }

    /// DMA write: transfers one byte from host to disk.
    pub fn dma_write_byte(&mut self, value: u8) -> SasiAction {
        self.lle_controller.dma_write_byte(value, &mut self.drives)
    }

    /// Called when the scheduled completion event fires.
    /// Returns true if an interrupt should be raised.
    pub fn complete_operation(&mut self) -> bool {
        self.lle_controller.complete_operation()
    }

    fn refresh_parameter_tables(&mut self, drive: usize) {
        let Some(rom) = self.rom.as_mut() else {
            return;
        };
        let Some(mounted) = &self.drives[drive] else {
            return;
        };
        let geometry = mounted.geometry();

        let full_offset = match drive {
            0 => PARAM_TABLE_D0_FULL_OFFSET,
            1 => PARAM_TABLE_D1_FULL_OFFSET,
            _ => return,
        };
        let half_offset = full_offset + PARAM_TABLE_SIZE;

        Self::write_parameter_table(rom.as_mut_slice(), full_offset, geometry);
        Self::write_parameter_table(rom.as_mut_slice(), half_offset, geometry);
    }

    fn write_parameter_table(rom: &mut [u8], offset: usize, geometry: HddGeometry) {
        if offset + PARAM_TABLE_SIZE > rom.len() {
            return;
        }

        let table = &mut rom[offset..offset + PARAM_TABLE_SIZE];
        table.fill(0x00);

        let cylinders_minus_one = geometry.cylinders.saturating_sub(1);
        let max_head_address = geometry.heads.saturating_sub(1);
        let legacy_sense = geometry.sasi_legacy_sense_type().unwrap_or(0x0F);
        let new_sense = geometry.sasi_new_sense_type().unwrap_or(0x0F);

        // SASI parameter table fields used by software that inspects BIOS data.
        table[0x03] = max_head_address;
        table[0x04] = (cylinders_minus_one >> 8) as u8;
        table[0x05] = cylinders_minus_one as u8;
        table[0x0D] = max_head_address;
        table[0x0E] = (cylinders_minus_one >> 8) as u8;
        table[0x0F] = cylinders_minus_one as u8;
        table[0x14] = legacy_sense;
        table[0x18] = geometry.heads;
        table[0x19] = 0x00;
        table[0x1C] = new_sense;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{HddFormat, HddGeometry};

    fn tempfile_with(bytes: &[u8], suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        let unique = format!(
            "neetan_sasi_test_{}_{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            suffix
        );
        let path = dir.join(unique);
        std::fs::write(&path, bytes).expect("write temp file");
        path
    }

    fn make_test_drive() -> HddImage {
        let geometry = HddGeometry {
            cylinders: 153,
            heads: 4,
            sectors_per_track: 33,
            sector_size: 256,
        };
        let data = vec![0u8; geometry.total_bytes() as usize];
        HddImage::from_raw(geometry, HddFormat::Thd, data)
    }

    #[test]
    fn trap_triggers_on_single_byte() {
        let mut sasi = SasiController::new();
        assert!(!sasi.hle_pending());
        sasi.write_trap_port(0x00);
        assert!(sasi.hle_pending());
        assert!(sasi.take_yield_requested());
        assert!(!sasi.take_yield_requested());
    }

    #[test]
    fn rom_image_has_correct_signature() {
        let mut sasi = SasiController::new();
        // ROM not installed until a drive is inserted.
        assert!(!sasi.rom_installed());

        sasi.insert_drive(0, make_test_drive(), None);

        assert!(sasi.rom_installed());
        // Expansion ROM signature at offset 9.
        assert_eq!(sasi.read_rom_byte(9), 0x55);
        assert_eq!(sasi.read_rom_byte(10), 0xAA);
        // ROM size code: 2 (= 2 * 512 = 1024 bytes).
        assert_eq!(sasi.read_rom_byte(11), 0x02);
    }

    #[test]
    fn flush_all_drives_persists_successful_write_before_drop() {
        let image = make_test_drive();
        let path = tempfile_with(&image.to_bytes(), ".thd");

        let mut sasi = SasiController::new();
        sasi.insert_drive(0, image, Some(path.clone()));

        let pattern = vec![0x39u8; 256];
        assert!(sasi.write_sector_raw(0, 12, &pattern));
        sasi.flush_all_drives();

        let raw = std::fs::read(&path).unwrap();
        let offset = 256 + 12 * 256;
        assert_eq!(&raw[offset..offset + 256], &pattern[..]);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn state_restores_electronics_and_retains_hard_disk_writes() {
        let mut controller = SasiController::new();
        controller.insert_drive(0, make_test_drive(), None);
        assert_eq!(controller.write_data(1), SasiAction::None);
        assert_eq!(controller.write_data(0x08), SasiAction::None);
        let encoded = save_state::encode_runtime_state(&controller.capture_state().unwrap());
        let state =
            save_state::decode_runtime_state::<SasiControllerState>(&encoded, 1 << 20).unwrap();

        let replacement = [0x6Du8; 256];
        assert!(controller.write_sector_raw(0, 4, &replacement));
        controller.write_control(0x08);
        controller.write_control(0x00);
        controller.restore_state(state).unwrap();

        assert_eq!(controller.phase(), SasiPhase::Command);
        assert_eq!(controller.read_sector(0, 4).unwrap(), replacement);
    }

    #[test]
    fn state_rejects_a_different_hard_disk() {
        let mut source = SasiController::new();
        source.insert_drive(0, make_test_drive(), None);
        let state = source.capture_state().unwrap();

        let mut different_image = make_test_drive();
        assert!(different_image.write_sector(0, &[0xA5; 256]));
        let mut target = SasiController::new();
        target.insert_drive(0, different_image, None);
        assert!(target.restore_state(state).is_err());
    }
}
