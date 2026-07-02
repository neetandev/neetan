//! PC-98 IDE (ATA) controller with ATAPI CD-ROM support.
//!
//! Consolidates the IDE HLE (High-Level Emulation) and LLE (Low-Level
//! Emulation) into a single controller module. The HLE intercepts INT 1Bh
//! via an expansion ROM stub at 0xD8000, while the LLE handles direct
//! hardware register access at ports 0x0640-0x064E and 0x074C-0x074E.
//!
//! The controller supports two channels via bank switching (port 0x0432):
//! - Channel 0: Up to 2 ATA hard drives (master/slave)
//! - Channel 1: ATAPI CD-ROM drive

pub(crate) mod atapi;
mod hle;
mod lle;

use std::{cell::Cell, path::PathBuf};

pub use lle::{IdeAction, IdePhase};

pub use crate::disk_hle::{buffer_address, drive_index, sector_position, transfer_size};
use crate::{
    cd_audio::CdAudioPlayer,
    cdrom::CdImage,
    disk::{HddGeometry, HddImage, MountedHdd},
};

/// Size of the expansion ROM window mapped at 0xD8000.
pub const ROM_SIZE: usize = 8192;

/// 8192-byte expansion ROM image.
static ROM_IMAGE: &[u8; ROM_SIZE] = include_bytes!("../../../utils/ide/ide.rom");

/// PC-98 IDE (ATA) controller with ATAPI CD-ROM support.
#[derive(Debug)]
pub struct IdeController {
    lle_controller: lle::Controller,
    hle_pending: bool,
    yield_requested: Cell<bool>,
    rom: Option<Box<[u8; ROM_SIZE]>>,
    // Channel 0: HDD drives (master/slave).
    drives: [Option<MountedHdd>; 2],
    // Channel 1: ATAPI CD-ROM.
    cdrom: Option<CdImage>,
    atapi_state: atapi::AtapiState,
    cd_audio_player: CdAudioPlayer,
    work_area_mapped: bool,
}

impl Default for IdeController {
    fn default() -> Self {
        Self::new(44100)
    }
}

impl IdeController {
    /// Creates a new idle IDE controller.
    pub fn new(output_sample_rate: u32) -> Self {
        Self {
            lle_controller: lle::Controller::new(),
            hle_pending: false,
            yield_requested: Cell::new(false),
            rom: None,
            drives: [None, None],
            cdrom: None,
            atapi_state: atapi::AtapiState::new(),
            cd_audio_player: CdAudioPlayer::new(output_sample_rate),
            work_area_mapped: false,
        }
    }

    /// Inserts a hard disk image into the specified drive (0-1) on channel 0.
    /// Installs the expansion ROM on the first insertion.
    pub fn insert_drive(&mut self, drive: usize, image: HddImage, path: Option<PathBuf>) {
        let sector_size = image.geometry.sector_size as usize;
        if let Some(mounted) = self.drives[drive].take() {
            mounted.eject();
        }
        self.drives[drive] = Some(MountedHdd::new(image, path));
        self.lle_controller
            .set_drive_sector_size(0, drive, sector_size);
        self.install_rom();
    }

    /// Inserts a CD-ROM image on channel 1.
    /// Installs the expansion ROM on the first insertion.
    pub fn insert_cdrom(&mut self, image: CdImage) {
        self.cdrom = Some(image);
        self.atapi_state.media_inserted();
        self.lle_controller.initialize_atapi_drive();
        self.install_rom();
    }

    /// Ejects the CD-ROM image from channel 1.
    pub fn eject_cdrom(&mut self) {
        self.cd_audio_player.reset();
        self.cdrom = None;
        self.atapi_state.media_ejected();
    }

    /// Returns true if a CD-ROM image is loaded.
    pub fn has_cdrom(&self) -> bool {
        self.cdrom.is_some()
    }

    /// Returns a reference to the loaded CD-ROM image, if any.
    pub fn cdrom_image(&self) -> Option<&CdImage> {
        self.cdrom.as_ref()
    }

    /// Returns a mutable reference to the CD audio player.
    pub fn cd_audio_player_mut(&mut self) -> &mut CdAudioPlayer {
        &mut self.cd_audio_player
    }

    /// Returns a reference to the CD audio player.
    pub fn cd_audio_player(&self) -> &CdAudioPlayer {
        &self.cd_audio_player
    }

    /// Generates CD audio samples, borrowing the CD image and audio player
    /// simultaneously from within the controller.
    pub fn generate_cd_audio_samples(&mut self, volume: f32, output: &mut [f32]) {
        if let Some(ref cdrom) = self.cdrom {
            self.cd_audio_player
                .generate_samples(cdrom, [volume, volume], output);
        }
    }

    /// Starts CD audio playback, splitting the internal borrow.
    pub fn play_cd_audio(&mut self, start_lba: u32, sector_count: u32) {
        if let Some(ref cdrom) = self.cdrom {
            self.cd_audio_player.play(cdrom, start_lba, sector_count);
        }
    }

    /// Resumes CD audio playback, splitting the internal borrow.
    pub fn resume_cd_audio(&mut self) {
        if let Some(ref cdrom) = self.cdrom {
            self.cd_audio_player.resume(cdrom);
        }
    }

    /// Returns true if the specified HDD slot (0 or 1) has a drive image loaded.
    pub fn has_hdd(&self, slot: usize) -> bool {
        self.drives.get(slot).is_some_and(Option::is_some)
    }

    /// Returns true if any HDD image is loaded on channel 0.
    pub fn has_any_hdd(&self) -> bool {
        self.drives[0].is_some() || self.drives[1].is_some()
    }

    /// Returns the BIOS IDE geometry flags stored at memory address `0x0457`.
    pub fn bios_capacity_byte(&self) -> u8 {
        let primary_drive = if self.drives[0].is_some() { 0x90 } else { 0x38 };
        let secondary_drive = if self.drives[1].is_some() { 0x42 } else { 0x07 };

        primary_drive | secondary_drive
    }

    fn install_rom(&mut self) {
        if self.rom.is_none() {
            self.rom = Some(Box::new(*ROM_IMAGE));
        }
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

    /// Reads the boot sector from the specified HDD drive into a local buffer.
    /// For 256-byte sector (SASI-compat) images, reads 1024 bytes (4 sectors).
    /// For 512-byte sector images, reads 512 bytes (1 sector).
    pub fn read_boot_sector(&self, drive_idx: usize) -> Option<Vec<u8>> {
        let geometry = self.drive_geometry(drive_idx)?;
        let pos = crate::disk_hle::sector_position(0x80 | drive_idx as u8, 0, 0, &geometry);
        let boot_size: usize = if geometry.sector_size == 256 {
            0x0400
        } else {
            0x0200
        };
        let mut buf = vec![0u8; boot_size];
        let mut offset = 0usize;
        let result = self.execute_read(drive_idx, boot_size as u32, pos, 0, |_addr, byte| {
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
        let sector_size = self.drives[drive_idx]
            .as_ref()
            .map(|d| d.geometry().sector_size)
            .unwrap_or(512);
        match sector_size {
            256 => crate::disk_hle::execute_write::<256>(
                drive_idx,
                xfer_size,
                sector_pos,
                buf_addr,
                &mut self.drives,
                read_byte,
            ),
            _ => crate::disk_hle::execute_write::<512>(
                drive_idx,
                xfer_size,
                sector_pos,
                buf_addr,
                &mut self.drives,
                read_byte,
            ),
        }
    }

    /// Executes a BIOS sense: returns the IDE media type.
    pub fn execute_sense(&self, drive_idx: usize) -> u8 {
        hle::execute_sense(drive_idx, &self.drives)
    }

    /// Executes a BIOS sense for the CD-ROM unit.
    /// Returns 0x0F (present) if a CD-ROM image is loaded, 0x60 otherwise.
    pub fn execute_cdrom_sense(&self) -> u8 {
        if self.cdrom.is_some() { 0x0F } else { 0x60 }
    }

    /// Reads the boot sector from the CD-ROM (LBA 0, first 1024 bytes).
    /// Returns the 1024-byte buffer if a CD-ROM is loaded and sector 0 is readable.
    pub fn read_cdrom_boot_sector(&self) -> Option<[u8; 1024]> {
        let cdrom = self.cdrom.as_ref()?;
        let mut sector_buf = [0u8; 2048];
        cdrom.read_sector(0, &mut sector_buf)?;
        let mut boot_buf = [0u8; 1024];
        boot_buf.copy_from_slice(&sector_buf[..1024]);
        Some(boot_buf)
    }

    /// Executes a BIOS init: returns the disk equipment word.
    /// Preserves non-IDE bits from `current_equip`.
    pub fn execute_init(&self, current_equip: u16) -> u16 {
        crate::disk_hle::execute_init(&self.drives, current_equip)
    }

    /// Executes a BIOS format on a track.
    pub fn execute_format(&mut self, drive_idx: usize, sector_pos: u32) -> u8 {
        crate::disk_hle::execute_format(drive_idx, sector_pos, &mut self.drives)
    }

    /// Executes a BIOS mode set (function 0x0E): returns 0x00 if the drive
    /// exists, 0x60 otherwise.
    pub fn execute_mode_set(&self, drive_idx: usize) -> u8 {
        if self.drives[drive_idx].is_some() {
            0x00
        } else {
            0x60
        }
    }

    /// Executes IDE Check Power Mode (function D0h).
    pub fn execute_check_power_mode(&self, drive_idx: usize) -> u8 {
        hle::execute_check_power_mode(drive_idx, &self.drives)
    }

    /// Executes IDE Motor ON (function E0h).
    pub fn execute_motor_on(&self, drive_idx: usize) -> u8 {
        hle::execute_motor_on(drive_idx, &self.drives)
    }

    /// Executes IDE Motor OFF (function F0h).
    pub fn execute_motor_off(&self, drive_idx: usize) -> u8 {
        hle::execute_motor_off(drive_idx, &self.drives)
    }

    /// Computes the IDE device connection flags for BIOS memory 0x05BA.
    /// Each bit represents one IDE slot: bit 0 = slot 0 (ch0 master),
    /// bit 1 = slot 1 (ch0 slave), bit 2 = slot 2 (ch1 master / CD-ROM),
    /// bit 3 = slot 3 (ch1 slave).
    ///
    /// In compatibility mode (CD-ROM only on channel 1 master), only HDD
    /// slots set their bits. Otherwise all connected devices set bits.
    pub fn compute_connection_flags(&self) -> u8 {
        // In compatibility mode (CD-ROM only on channel 1 master), only HDD
        // slots set their bits. In neetan, CD-ROM is always on channel 1, so
        // compmode is always true when a CD-ROM is present. When no CD-ROM is
        // present, non-compmode applies but there's no CD-ROM to set bit 2.
        // Either way, only HDD slots contribute to the flags.
        let mut flags = 0u8;
        if self.drives[0].is_some() {
            flags |= 0x01;
        }
        if self.drives[1].is_some() {
            flags |= 0x02;
        }
        flags
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

    /// Writes a byte to the trap port (0x07EE).
    /// Any single byte triggers the HLE trap.
    pub fn write_trap_port(&mut self, _value: u8) {
        self.hle_pending = true;
        self.yield_requested.set(true);
    }

    /// Returns true if an IDE HLE trap is pending.
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

    /// Reads the 16-bit data register (port 0x0640).
    /// Returns `(word, action)` where action indicates if an interrupt should be scheduled
    /// (e.g. at chunk boundaries during multi-sector ATAPI reads).
    pub fn read_data_word(&mut self) -> (u16, IdeAction) {
        if self.lle_controller.is_atapi_channel_active() {
            let phase = self.lle_controller.atapi_phase();
            if phase == IdePhase::PacketDataIn {
                return self.atapi_read_data_word();
            }
            // DataIn phase (IDENTIFY PACKET DEVICE) uses the drive's buffer.
            // Fall through to standard read_data_word handling.
        }
        self.lle_controller.read_data_word(&self.drives)
    }

    /// Writes the 16-bit data register (port 0x0640).
    pub fn write_data_word(&mut self, value: u16) -> IdeAction {
        if self.lle_controller.is_atapi_channel_active() {
            return self.atapi_write_data_word(value);
        }
        self.lle_controller.write_data_word(value, &mut self.drives)
    }

    /// Reads the error register (port 0x0642). Clears ERR bit in status.
    pub fn read_error(&mut self) -> u8 {
        self.lle_controller.read_error()
    }

    /// Reads the sector count register (port 0x0644).
    pub fn read_sector_count(&self) -> u8 {
        self.lle_controller.read_sector_count()
    }

    /// Reads the sector number register (port 0x0646).
    pub fn read_sector_number(&self) -> u8 {
        self.lle_controller.read_sector_number()
    }

    /// Reads the cylinder low register (port 0x0648).
    pub fn read_cylinder_low(&self) -> u8 {
        self.lle_controller.read_cylinder_low()
    }

    /// Reads the cylinder high register (port 0x064A).
    pub fn read_cylinder_high(&self) -> u8 {
        self.lle_controller.read_cylinder_high()
    }

    /// Reads the device/head register (port 0x064C).
    pub fn read_device_head(&self) -> u8 {
        self.lle_controller.read_device_head()
    }

    /// Reads the status register (port 0x064E). Clears pending interrupt.
    /// Returns (status, clear_irq) where clear_irq signals PIC IRQ deassertion.
    pub fn read_status(&mut self) -> (u8, bool) {
        self.lle_controller.read_status()
    }

    /// Reads the alternate status register (port 0x074C). Does NOT clear interrupt.
    pub fn read_alt_status(&self) -> u8 {
        self.lle_controller.read_alt_status()
    }

    /// Writes the features register (port 0x0642).
    pub fn write_features(&mut self, value: u8) {
        self.lle_controller.write_features(value);
    }

    /// Writes the sector count register (port 0x0644).
    pub fn write_sector_count(&mut self, value: u8) {
        self.lle_controller.write_sector_count(value);
    }

    /// Writes the sector number register (port 0x0646).
    pub fn write_sector_number(&mut self, value: u8) {
        self.lle_controller.write_sector_number(value);
    }

    /// Writes the cylinder low register (port 0x0648).
    pub fn write_cylinder_low(&mut self, value: u8) {
        self.lle_controller.write_cylinder_low(value);
    }

    /// Writes the cylinder high register (port 0x064A).
    pub fn write_cylinder_high(&mut self, value: u8) {
        self.lle_controller.write_cylinder_high(value);
    }

    /// Writes the device/head register (port 0x064C).
    pub fn write_device_head(&mut self, value: u8) {
        self.lle_controller.write_device_head(value);
    }

    /// Writes the command register (port 0x064E).
    pub fn write_command(&mut self, value: u8) -> IdeAction {
        if self.lle_controller.is_atapi_channel_active() {
            return self.atapi_write_command(value);
        }
        self.lle_controller.write_command(value, &self.drives)
    }

    /// Writes the device control register (port 0x074C).
    pub fn write_device_control(&mut self, value: u8) {
        self.lle_controller.write_device_control(value);
    }

    /// Reads the bank 1 select register (port 0x0432).
    /// Clears the interrupt pending flag (bit 7) after reading.
    pub fn read_bank(&mut self, index: usize) -> u8 {
        self.lle_controller.read_bank(index)
    }

    /// Reads the bank 0 status register (port 0x0430).
    /// Returns computed status instead of raw bank value.
    pub fn read_bank0_status(&mut self) -> u8 {
        self.lle_controller
            .read_bank0_status(&self.drives, self.cdrom.is_some())
    }

    /// Writes the bank select register.
    pub fn write_bank(&mut self, index: usize, value: u8) {
        self.lle_controller.write_bank(index, value);
    }

    /// Reads the IDE presence detection register (port 0x0433).
    pub fn read_presence(&self) -> u8 {
        self.lle_controller.read_presence(self.cdrom.is_some())
    }

    /// Reads the additional status register (port 0x0435).
    pub fn read_additional_status(&self) -> u8 {
        self.lle_controller.read_additional_status(&self.drives)
    }

    /// Reads the digital input register (port 0x074E).
    pub fn read_digital_input(&self) -> u8 {
        self.lle_controller.read_digital_input()
    }

    /// Called when the scheduled completion event fires.
    /// Returns true if an interrupt should be raised.
    pub fn complete_operation(&mut self) -> bool {
        self.lle_controller.complete_operation()
    }

    /// Reads the work area mapping port (0x1E8E).
    /// Returns 0x81 if DA000-DBFFF is mapped, 0x80 otherwise.
    pub fn read_work_area_port(&self) -> u8 {
        if self.work_area_mapped { 0x81 } else { 0x80 }
    }

    /// Writes the work area mapping port (0x1E8E).
    /// 0x81 maps RAM at DA000-DBFFF, 0x00/0x80 unmaps it.
    pub fn write_work_area_port(&mut self, value: u8) {
        self.work_area_mapped = value == 0x81;
    }

    // --- ATAPI command routing ---

    fn atapi_write_command(&mut self, command: u8) -> IdeAction {
        match command {
            // DEVICE RESET
            0x08 => {
                self.atapi_state.reset();
                self.lle_controller.atapi_device_reset();
                IdeAction::ScheduleCompletion
            }
            // EXECUTE DEVICE DIAGNOSTIC (0x90)
            0x90 => {
                self.atapi_state.reset();
                self.lle_controller.atapi_device_reset();
                IdeAction::ScheduleCompletion
            }
            // PACKET (0xA0)
            0xA0 => {
                let cyl_lo = self.lle_controller.read_cylinder_low();
                let cyl_hi = self.lle_controller.read_cylinder_high();
                self.atapi_state.start_packet_command(cyl_lo, cyl_hi);
                self.lle_controller.atapi_start_packet();
                IdeAction::None
            }
            // IDENTIFY PACKET DEVICE (0xA1)
            0xA1 => {
                self.lle_controller
                    .atapi_identify_packet_device(&self.atapi_state);
                IdeAction::ScheduleCompletion
            }
            // MEDIA LOCK (0xDE)
            0xDE => {
                self.lle_controller.atapi_set_ready();
                IdeAction::ScheduleCompletion
            }
            // MEDIA UNLOCK (0xDF)
            0xDF => {
                self.lle_controller.atapi_set_ready();
                IdeAction::ScheduleCompletion
            }
            // IDENTIFY DEVICE (0xEC) - abort with ATAPI signature
            0xEC => {
                self.lle_controller.atapi_identify_device_abort();
                IdeAction::ScheduleCompletion
            }
            // SET FEATURES (0xEF)
            0xEF => {
                let features = self.lle_controller.read_atapi_features();
                match features {
                    0x02 | 0x82 | 0x03 => {
                        self.lle_controller.atapi_set_ready();
                        IdeAction::ScheduleCompletion
                    }
                    _ => {
                        self.lle_controller.atapi_abort();
                        IdeAction::ScheduleCompletion
                    }
                }
            }
            // All other ATA commands abort on ATAPI
            _ => {
                self.lle_controller.atapi_abort();
                IdeAction::ScheduleCompletion
            }
        }
    }

    fn atapi_write_data_word(&mut self, value: u16) -> IdeAction {
        let phase = self.lle_controller.atapi_phase();
        match phase {
            IdePhase::PacketCommand => {
                let complete = self.atapi_state.receive_packet_word(value);
                if complete {
                    let (has_data, is_error) = self
                        .atapi_state
                        .execute_packet(self.cdrom.as_ref(), &mut self.cd_audio_player);
                    if is_error {
                        self.lle_controller.atapi_command_error(&self.atapi_state);
                    } else if has_data {
                        let transfer_size = self.atapi_state.current_transfer_size();
                        self.lle_controller.atapi_start_data_in(transfer_size);
                    } else {
                        self.lle_controller.atapi_command_done();
                    }
                    IdeAction::ScheduleCompletion
                } else {
                    IdeAction::None
                }
            }
            _ => IdeAction::None,
        }
    }

    fn atapi_read_data_word(&mut self) -> (u16, IdeAction) {
        let phase = self.lle_controller.atapi_phase();
        if phase != IdePhase::PacketDataIn {
            return (0xFFFF, IdeAction::None);
        }

        let word = self.atapi_state.read_data_word();
        self.atapi_state.chunk_position += 2;

        if self.atapi_state.transfer_complete() {
            self.lle_controller.atapi_command_done();
            (word, IdeAction::ScheduleCompletion)
        } else if self.atapi_state.chunk_complete() {
            self.atapi_state.start_next_chunk();
            let transfer_size = self.atapi_state.current_transfer_size();
            self.lle_controller.atapi_start_data_in(transfer_size);
            (word, IdeAction::ScheduleCompletion)
        } else {
            (word, IdeAction::None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cdrom::CdImage,
        disk::{HddFormat, HddGeometry},
    };

    fn make_test_drive() -> HddImage {
        let geometry = HddGeometry {
            cylinders: 20,
            heads: 4,
            sectors_per_track: 17,
            sector_size: 512,
        };
        let data = vec![0u8; geometry.total_bytes() as usize];
        HddImage::from_raw(geometry, HddFormat::Hdi, data)
    }

    fn make_test_cdimage() -> CdImage {
        let cue = r#"FILE "test.bin" BINARY
  TRACK 01 MODE1/2048
    INDEX 01 00:00:00
"#;
        let mut bin_data = vec![0u8; 2048 * 100];
        for i in 0..100u32 {
            let offset = i as usize * 2048;
            bin_data[offset] = (i >> 8) as u8;
            bin_data[offset + 1] = i as u8;
        }
        CdImage::from_cue(cue, bin_data).unwrap()
    }

    #[test]
    fn trap_triggers_on_single_byte() {
        let mut ide = IdeController::new(44100);
        assert!(!ide.hle_pending());
        ide.write_trap_port(0x00);
        assert!(ide.hle_pending());
        assert!(ide.take_yield_requested());
        assert!(!ide.take_yield_requested());
    }

    #[test]
    fn rom_image_has_correct_signature() {
        let mut ide = IdeController::new(44100);
        assert!(!ide.rom_installed());

        ide.insert_drive(0, make_test_drive(), None);

        assert!(ide.rom_installed());
        assert_eq!(ide.read_rom_byte(9), 0x55);
        assert_eq!(ide.read_rom_byte(10), 0xAA);
        assert_eq!(ide.read_rom_byte(11), 0x10);
    }

    #[test]
    fn work_area_port_read_write() {
        let mut ide = IdeController::new(44100);
        assert_eq!(ide.read_work_area_port(), 0x80);

        ide.write_work_area_port(0x81);
        assert_eq!(ide.read_work_area_port(), 0x81);

        ide.write_work_area_port(0x80);
        assert_eq!(ide.read_work_area_port(), 0x80);

        ide.write_work_area_port(0x00);
        assert_eq!(ide.read_work_area_port(), 0x80);
    }

    #[test]
    fn cdrom_insert_installs_rom() {
        let mut ide = IdeController::new(44100);
        assert!(!ide.rom_installed());

        ide.insert_cdrom(make_test_cdimage());
        assert!(ide.rom_installed());
        assert!(ide.has_cdrom());
    }

    #[test]
    fn cdrom_eject() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        assert!(ide.has_cdrom());

        ide.eject_cdrom();
        assert!(!ide.has_cdrom());
    }

    #[test]
    fn atapi_identify_device_returns_signature() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());

        // Switch to channel 1.
        ide.write_bank(1, 0x01);

        // IDENTIFY DEVICE (0xEC) should abort with ATAPI signature.
        let action = ide.write_command(0xEC);
        assert_eq!(action, IdeAction::ScheduleCompletion);

        // Check ATAPI signature in registers.
        assert_eq!(ide.read_cylinder_low(), 0x14);
        assert_eq!(ide.read_cylinder_high(), 0xEB);

        // Status should show error.
        assert_ne!(ide.read_alt_status() & 0x01, 0);
    }

    #[test]
    fn atapi_identify_packet_device() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());

        // Switch to channel 1.
        ide.write_bank(1, 0x01);

        // IDENTIFY PACKET DEVICE (0xA1).
        let action = ide.write_command(0xA1);
        assert_eq!(action, IdeAction::ScheduleCompletion);

        // Read 256 words.
        let mut data = vec![0u16; 256];
        let mut final_action = IdeAction::None;
        for word in data.iter_mut() {
            let (value, action) = ide.read_data_word();
            *word = value;
            final_action = action;
        }

        // Word 0: 0x8580 (ATAPI CD-ROM).
        assert_eq!(data[0], 0x8580);
        assert_eq!(final_action, IdeAction::ScheduleCompletion);
        assert_eq!(ide.read_alt_status() & 0x18, 0x10);
    }

    #[test]
    fn atapi_packet_inquiry() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());

        // Switch to channel 1.
        ide.write_bank(1, 0x01);

        // Set byte count limit in cylinder registers.
        ide.write_cylinder_low(0xFE);
        ide.write_cylinder_high(0xFF);

        // Send PACKET command (sets DRQ synchronously, no completion needed).
        let action = ide.write_command(0xA0);
        assert_eq!(action, IdeAction::None);

        // Write INQUIRY CDB (12 bytes = 6 words).
        // INQUIRY: opcode=0x12, allocation_length=36 in byte 4.
        ide.write_data_word(0x0012); // byte[0]=0x12, byte[1]=0x00
        ide.write_data_word(0x0000); // byte[2]=0x00, byte[3]=0x00
        ide.write_data_word(0x0024); // byte[4]=0x24 (36), byte[5]=0x00
        ide.write_data_word(0x0000);
        ide.write_data_word(0x0000);
        ide.write_data_word(0x0000);

        // Read INQUIRY response data.
        let (first_word, _) = ide.read_data_word();
        // Byte 0 = device type 0x05 (CD-ROM), Byte 1 = 0x80 (removable).
        assert_eq!(first_word & 0xFF, 0x05);
        assert_eq!(first_word >> 8, 0x80);
    }

    #[test]
    fn atapi_packet_read_sector() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());

        // Directly clear the media change state so we can test READ without
        // the UNIT_ATTENTION handshake.
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        // Switch to channel 1.
        ide.write_bank(1, 0x01);

        // Send PACKET command with READ(10) for LBA 42, count 1.
        ide.write_cylinder_low(0xFE);
        ide.write_cylinder_high(0xFF);
        ide.write_command(0xA0);

        // READ(10) CDB: opcode=0x28, LBA=42 (bytes 2-5 BE), count=1 (bytes 7-8 BE).
        // receive_packet_word(v): packet[pos]=v as u8, packet[pos+1]=(v>>8) as u8.
        ide.write_data_word(0x0028); // byte[0]=0x28 (opcode), byte[1]=0x00
        ide.write_data_word(0x0000); // byte[2]=0x00, byte[3]=0x00
        ide.write_data_word(0x2A00); // byte[4]=0x00, byte[5]=0x2A (LBA low = 42)
        ide.write_data_word(0x0000); // byte[6]=0x00, byte[7]=0x00
        ide.write_data_word(0x0001); // byte[8]=0x01, byte[9]=0x00 (count = 1)
        ide.write_data_word(0x0000); // byte[10]=0x00, byte[11]=0x00 (control)

        // Read first word of sector data.
        let (first_word, _) = ide.read_data_word();
        // Sector 42 marker: byte[0]=0, byte[1]=42.
        assert_eq!(first_word & 0xFF, 0);
        assert_eq!(first_word >> 8, 42);
    }

    #[test]
    fn hdd_still_works_with_cdrom() {
        let mut ide = IdeController::new(44100);
        ide.insert_drive(0, make_test_drive(), None);
        ide.insert_cdrom(make_test_cdimage());

        // Channel 0 (HDD): IDENTIFY DEVICE should work.
        let action = ide.write_command(0xEC);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        let mut data = vec![0u16; 256];
        for word in data.iter_mut() {
            *word = ide.read_data_word().0;
        }
        assert_eq!(data[0], 0x0040); // HDD general config.

        // Switch to channel 1, verify ATAPI.
        ide.write_bank(1, 0x01);
        ide.write_command(0xEC); // Should abort with ATAPI signature.
        assert_eq!(ide.read_cylinder_low(), 0x14);
        assert_eq!(ide.read_cylinder_high(), 0xEB);

        // Switch back to channel 0, still HDD.
        ide.write_bank(1, 0x00);
        let action = ide.write_command(0xEC);
        assert_eq!(action, IdeAction::ScheduleCompletion);
    }

    #[test]
    fn presence_returns_0x02_when_channel1_selected_and_cdrom() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        // Channel 1 not yet selected: returns 0x00.
        assert_eq!(ide.read_presence(), 0x00);
        // Select channel 1, CD-ROM present: returns 0x02.
        ide.write_bank(1, 0x01);
        assert_eq!(ide.read_presence(), 0x02);
    }

    #[test]
    fn connection_flags_hdd_and_cdrom() {
        let mut ide = IdeController::new(44100);
        ide.insert_drive(0, make_test_drive(), None);
        ide.insert_cdrom(make_test_cdimage());
        // Compatibility mode: only HDD bits set.
        assert_eq!(ide.compute_connection_flags(), 0x01);
    }

    #[test]
    fn connection_flags_hdd_only() {
        let mut ide = IdeController::new(44100);
        ide.insert_drive(0, make_test_drive(), None);
        // No CD-ROM, non-compmode: HDD on slot 0.
        assert_eq!(ide.compute_connection_flags(), 0x01);
    }

    #[test]
    fn connection_flags_two_hdds_and_cdrom() {
        let mut ide = IdeController::new(44100);
        ide.insert_drive(0, make_test_drive(), None);
        ide.insert_drive(1, make_test_drive(), None);
        ide.insert_cdrom(make_test_cdimage());
        // Compatibility mode: both HDD bits set.
        assert_eq!(ide.compute_connection_flags(), 0x03);
    }

    #[test]
    fn connection_flags_cdrom_only() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        // Compatibility mode, no HDDs: 0x00.
        assert_eq!(ide.compute_connection_flags(), 0x00);
    }

    #[test]
    fn connection_flags_nothing() {
        let ide = IdeController::new(44100);
        assert_eq!(ide.compute_connection_flags(), 0x00);
    }

    #[test]
    fn cdrom_insert_sets_atapi_signature_on_channel1() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());

        ide.write_bank(1, 0x01);
        assert_eq!(ide.read_cylinder_low(), 0x14);
        assert_eq!(ide.read_cylinder_high(), 0xEB);
        assert_eq!(ide.read_sector_count(), 0x01);
        assert_eq!(ide.read_sector_number(), 0x01);
        // Power-on status: 0x00 (not DRDY|DSC).
        assert_eq!(ide.read_alt_status(), 0x00);
    }

    #[test]
    fn execute_diagnostic_on_atapi_sets_signature() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.write_bank(1, 0x01);

        // Clobber registers.
        ide.write_cylinder_low(0x00);
        ide.write_cylinder_high(0x00);

        let action = ide.write_command(0x90);
        assert_eq!(action, IdeAction::ScheduleCompletion);

        assert_eq!(ide.read_cylinder_low(), 0x14);
        assert_eq!(ide.read_cylinder_high(), 0xEB);
    }

    #[test]
    fn set_features_write_cache_succeeds_on_atapi() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.write_bank(1, 0x01);

        // SET FEATURES with subcommand 0x02 (enable write cache).
        ide.write_features(0x02);
        let action = ide.write_command(0xEF);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert_ne!(ide.read_alt_status() & 0x40, 0); // DRDY set.
        assert_eq!(ide.read_alt_status() & 0x01, 0); // No error.
    }

    #[test]
    fn set_features_transfer_mode_succeeds_on_atapi() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.write_bank(1, 0x01);

        ide.write_features(0x03);
        let action = ide.write_command(0xEF);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert_eq!(ide.read_alt_status() & 0x01, 0);
    }

    #[test]
    fn set_features_invalid_aborts_on_atapi() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.write_bank(1, 0x01);

        ide.write_features(0xFF);
        let action = ide.write_command(0xEF);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert_ne!(ide.read_alt_status() & 0x01, 0); // Error set.
    }

    #[test]
    fn media_lock_succeeds_on_atapi() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.write_bank(1, 0x01);

        let action = ide.write_command(0xDE);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert_ne!(ide.read_alt_status() & 0x40, 0); // DRDY set.
        assert_eq!(ide.read_alt_status() & 0x01, 0); // No error.
    }

    #[test]
    fn media_unlock_succeeds_on_atapi() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.write_bank(1, 0x01);

        let action = ide.write_command(0xDF);
        assert_eq!(action, IdeAction::ScheduleCompletion);
        assert_ne!(ide.read_alt_status() & 0x40, 0);
        assert_eq!(ide.read_alt_status() & 0x01, 0);
    }

    #[test]
    fn atapi_packet_command_returns_none_action() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.write_bank(1, 0x01);
        ide.write_cylinder_low(0xFE);
        ide.write_cylinder_high(0xFF);

        let action = ide.write_command(0xA0);
        assert_eq!(action, IdeAction::None);

        // DRQ should be set (ready for CDB).
        assert_ne!(ide.read_alt_status() & 0x08, 0);
    }

    #[test]
    fn atapi_read_status_signals_irq_deassertion() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.write_bank(1, 0x01);

        // IDENTIFY PACKET DEVICE sets interrupt_pending.
        let action = ide.write_command(0xA1);
        assert_eq!(action, IdeAction::ScheduleCompletion);

        // NIEN=0 (default): read_status should signal clear_irq.
        let (status, clear_irq) = ide.read_status();
        assert_ne!(status & 0x08, 0); // DRQ set
        assert!(clear_irq);
    }

    #[test]
    fn srst_on_atapi_channel_preserves_signature() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.write_bank(1, 0x01);

        // Software reset.
        ide.write_device_control(0x04); // SRST
        ide.write_device_control(0x00); // Clear SRST

        // ATAPI signature must be present after reset.
        assert_eq!(ide.read_cylinder_low(), 0x14);
        assert_eq!(ide.read_cylinder_high(), 0xEB);
        // Post-SRST status: DRDY|DSC|ERR.
        assert_ne!(ide.read_alt_status() & 0x51, 0);
    }

    #[test]
    fn packet_command_status_has_drdy_dsc_drq() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.write_bank(1, 0x01);
        ide.write_cylinder_low(0xFE);
        ide.write_cylinder_high(0xFF);

        ide.write_command(0xA0);

        let status = ide.read_alt_status();
        // DRDY (0x40), DSC (0x10), DRQ (0x08) must all be set.
        assert_ne!(status & 0x40, 0, "DRDY not set");
        assert_ne!(status & 0x10, 0, "DSC not set");
        assert_ne!(status & 0x08, 0, "DRQ not set");
    }

    #[test]
    fn additional_status_no_drives() {
        let ide = IdeController::new(44100);
        assert_eq!(ide.read_additional_status(), 0x02);
    }

    #[test]
    fn additional_status_with_hdd() {
        let mut ide = IdeController::new(44100);
        ide.insert_drive(0, make_test_drive(), None);
        assert_eq!(ide.read_additional_status(), 0x00);
    }

    #[test]
    fn additional_status_cdrom_only() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        assert_eq!(ide.read_additional_status(), 0x02);
    }

    #[test]
    fn has_any_hdd_empty() {
        let ide = IdeController::new(44100);
        assert!(!ide.has_any_hdd());
    }

    #[test]
    fn has_any_hdd_with_drive() {
        let mut ide = IdeController::new(44100);
        ide.insert_drive(0, make_test_drive(), None);
        assert!(ide.has_any_hdd());
    }

    #[test]
    fn has_any_hdd_cdrom_only() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        assert!(!ide.has_any_hdd());
    }

    #[test]
    fn data_in_phase_has_dsc_drq() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());

        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        ide.write_bank(1, 0x01);
        ide.write_cylinder_low(0xFE);
        ide.write_cylinder_high(0xFF);
        ide.write_command(0xA0);

        // Send INQUIRY CDB.
        ide.write_data_word(0x0012);
        ide.write_data_word(0x0000);
        ide.write_data_word(0x0024);
        ide.write_data_word(0x0000);
        ide.write_data_word(0x0000);
        ide.write_data_word(0x0000);

        let status = ide.read_alt_status();
        // DSC (0x10) and DRQ (0x08) must be set during data-in.
        assert_ne!(status & 0x10, 0, "DSC not set during data-in");
        assert_ne!(status & 0x08, 0, "DRQ not set during data-in");
    }

    fn setup_atapi_for_packet(ide: &mut IdeController, byte_count_limit: u16) {
        ide.write_bank(1, 0x01);
        ide.write_cylinder_low(byte_count_limit as u8);
        ide.write_cylinder_high((byte_count_limit >> 8) as u8);
        ide.write_command(0xA0);
    }

    fn send_read10_cdb(ide: &mut IdeController, lba: u32, count: u16) {
        // READ(10): opcode=0x28, LBA in bytes 2-5 BE, count in bytes 7-8 BE.
        ide.write_data_word(0x0028);
        ide.write_data_word(((lba >> 8) & 0xFF) as u16 | (((lba >> 24) & 0xFF) as u16) << 8);
        ide.write_data_word((lba & 0xFF) as u16 | (((lba >> 16) & 0xFF) as u16) << 8);
        ide.write_data_word((count >> 8) & 0xFF);
        ide.write_data_word(count & 0xFF);
        ide.write_data_word(0x0000);
    }

    #[test]
    fn multi_sector_read_fires_interrupt_per_chunk() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        // Set byte_count_limit to 2048 (one sector per chunk).
        setup_atapi_for_packet(&mut ide, 2048);
        send_read10_cdb(&mut ide, 0, 3);

        // Read sector 0 (1024 words = 2048 bytes).
        for i in 0..1024u32 {
            let (_, action) = ide.read_data_word();
            if i < 1023 {
                assert_eq!(action, IdeAction::None);
            } else {
                // Last word of chunk: fires interrupt for next chunk.
                assert_eq!(action, IdeAction::ScheduleCompletion);
            }
        }

        // Read sector 1.
        for i in 0..1024u32 {
            let (_, action) = ide.read_data_word();
            if i < 1023 {
                assert_eq!(action, IdeAction::None);
            } else {
                assert_eq!(action, IdeAction::ScheduleCompletion);
            }
        }

        // Read sector 2 (last sector - fires completion).
        for i in 0..1024u32 {
            let (_, action) = ide.read_data_word();
            if i < 1023 {
                assert_eq!(action, IdeAction::None);
            } else {
                // Last word of last sector: transfer complete.
                assert_eq!(action, IdeAction::ScheduleCompletion);
            }
        }
    }

    #[test]
    fn single_sector_read_fires_completion_at_end() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        setup_atapi_for_packet(&mut ide, 0xFFFE);
        send_read10_cdb(&mut ide, 0, 1);

        for i in 0..1024u32 {
            let (_, action) = ide.read_data_word();
            if i < 1023 {
                assert_eq!(action, IdeAction::None);
            } else {
                assert_eq!(action, IdeAction::ScheduleCompletion);
            }
        }
    }

    #[test]
    fn large_byte_count_limit_delivers_per_sector_chunks() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        // Even with byte_count_limit=0xFFFE, multi-sector reads deliver one
        // CD sector (2048 bytes = 1024 words) per DRQ assertion - matching
        // real ATAPI CD-ROM drive behavior.
        setup_atapi_for_packet(&mut ide, 0xFFFE);
        send_read10_cdb(&mut ide, 0, 3);

        let words_per_sector = 1024;
        for sector in 0..3u32 {
            for word in 0..words_per_sector {
                let (_, action) = ide.read_data_word();
                let is_last_word = word == words_per_sector - 1;
                if is_last_word {
                    assert_eq!(
                        action,
                        IdeAction::ScheduleCompletion,
                        "sector {sector} last word should trigger completion"
                    );
                } else {
                    assert_eq!(
                        action,
                        IdeAction::None,
                        "sector {sector} word {word} should be None"
                    );
                }
            }
        }
    }

    /// Encodes two CDB bytes into one word for write_data_word().
    /// receive_packet_word(v) stores v as little-endian: packet[pos]=v as u8, packet[pos+1]=(v>>8).
    fn cdb_word(even_byte: u8, odd_byte: u8) -> u16 {
        u16::from(even_byte) | (u16::from(odd_byte) << 8)
    }

    fn send_sub_channel_cdb(
        ide: &mut IdeController,
        sub_q: bool,
        format: u8,
        alloc_len: u16,
        msf: bool,
    ) {
        let sub_q_byte = if sub_q { 0x40u8 } else { 0x00u8 };
        let byte1 = if msf { 0x02u8 } else { 0x00u8 };
        ide.write_data_word(cdb_word(0x42, byte1));
        ide.write_data_word(cdb_word(sub_q_byte, format));
        ide.write_data_word(cdb_word(0x00, 0x00));
        ide.write_data_word(cdb_word(0x00, (alloc_len >> 8) as u8));
        ide.write_data_word(cdb_word(alloc_len as u8, 0x00));
        ide.write_data_word(cdb_word(0x00, 0x00));
    }

    fn send_read_cd_cdb(ide: &mut IdeController, lba: u32, count: u32, flags: u8) {
        ide.write_data_word(cdb_word(0xBE, 0x00));
        ide.write_data_word(cdb_word((lba >> 24) as u8, (lba >> 16) as u8));
        ide.write_data_word(cdb_word((lba >> 8) as u8, lba as u8));
        ide.write_data_word(cdb_word((count >> 16) as u8, (count >> 8) as u8));
        ide.write_data_word(cdb_word(count as u8, flags));
        ide.write_data_word(cdb_word(0x00, 0x00));
    }

    #[test]
    fn read_sub_channel_no_subq_returns_header_only() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        setup_atapi_for_packet(&mut ide, 0xFFFE);
        send_sub_channel_cdb(&mut ide, false, 0x00, 16, false);

        // Should return 4-byte minimal response.
        let (word0, _) = ide.read_data_word();
        // byte[0]=0x00 (reserved), byte[1]=0x15 (no audio status).
        assert_eq!(word0, 0x1500);
        let (word1, _) = ide.read_data_word();
        // bytes[2-3]: sub-channel data length = 0.
        assert_eq!(word1, 0x0000);
    }

    #[test]
    fn read_sub_channel_format_01_returns_16_bytes() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        setup_atapi_for_packet(&mut ide, 0xFFFE);
        send_sub_channel_cdb(&mut ide, true, 0x01, 16, false);

        // Read 8 words (16 bytes).
        let mut data = [0u16; 8];
        for word in data.iter_mut() {
            *word = ide.read_data_word().0;
        }

        // byte[0]=0x00, byte[1]=0x15.
        assert_eq!(data[0], 0x1500);
        // byte[2..3]: data length = 0x000C.
        assert_eq!(data[1], 0x0C00);
        // byte[4]=0x01 (format), byte[5]=ADR/CTL (0x14 for data track).
        assert_eq!(data[2] & 0xFF, 0x01);
        assert_eq!(data[2] >> 8, 0x14);
        // byte[6]=track number (1), byte[7]=index (1).
        assert_eq!(data[3], 0x0101);
    }

    #[test]
    fn read_sub_channel_format_01_bcd_mode() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;
        ide.atapi_state.bcd_msf_mode = true;

        setup_atapi_for_packet(&mut ide, 0xFFFE);
        send_sub_channel_cdb(&mut ide, true, 0x01, 16, true);

        let mut data = [0u16; 8];
        for word in data.iter_mut() {
            *word = ide.read_data_word().0;
        }

        // Absolute position at LBA 0 = MSF 0:02:00 with BCD encoding:
        // store_address writes: buf[0]=0x00, buf[1]=BCD(0)=0x00, buf[2]=BCD(2)=0x02, buf[3]=BCD(0)=0x00.
        // data_buffer[8..12] = [0x00, 0x00, 0x02, 0x00].
        // word[4] = byte[8] | (byte[9] << 8) = 0x0000.
        // word[5] = byte[10] | (byte[11] << 8) = 0x0002.
        assert_eq!(data[4], 0x0000);
        assert_eq!(data[5], 0x0002);
    }

    #[test]
    fn read_toc_clears_media_changed() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        setup_atapi_for_packet(&mut ide, 0xFFFE);
        // READ TOC CDB: format 0, MSF=0, alloc_length=0x00FF.
        ide.write_data_word(cdb_word(0x43, 0x00));
        ide.write_data_word(cdb_word(0x00, 0x00));
        ide.write_data_word(cdb_word(0x00, 0x00));
        ide.write_data_word(cdb_word(0x00, 0x00));
        ide.write_data_word(cdb_word(0xFF, 0x00));
        ide.write_data_word(cdb_word(0x00, 0x00));

        // Drain response.
        for _ in 0..64 {
            let (_, action) = ide.read_data_word();
            if action == IdeAction::ScheduleCompletion {
                break;
            }
        }

        assert!(!ide.atapi_state.media_changed);
    }

    #[test]
    fn read_cd_header_only_flag() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        setup_atapi_for_packet(&mut ide, 0xFFFE);
        // flags=0x20: header_code=01 (header only). Transfer = 4 bytes.
        send_read_cd_cdb(&mut ide, 0, 1, 0x20);

        let (_, _) = ide.read_data_word();
        let (_, action) = ide.read_data_word();
        assert_eq!(action, IdeAction::ScheduleCompletion);
    }

    #[test]
    fn read_cd_sync_without_header_not_returned() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        setup_atapi_for_packet(&mut ide, 0xFFFE);
        // flags=0x90: sync=1, header_code=00, user_data=1.
        // Quirk: sync suppressed without header. Only user data (2048 bytes).
        send_read_cd_cdb(&mut ide, 0, 1, 0x90);

        for i in 0..1024u32 {
            let (_, action) = ide.read_data_word();
            if i < 1023 {
                assert_eq!(action, IdeAction::None);
            } else {
                assert_eq!(action, IdeAction::ScheduleCompletion);
            }
        }
    }

    #[test]
    fn read_cd_sync_with_header_both_returned() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        setup_atapi_for_packet(&mut ide, 0xFFFE);
        // flags=0xB0: sync=1, header_code=01 (header), user_data=1.
        // Transfer = 12 (sync) + 4 (header) + 2048 (user data) = 2064 bytes = 1032 words.
        send_read_cd_cdb(&mut ide, 0, 1, 0xB0);

        let total_words = (12 + 4 + 2048) / 2;
        for i in 0..total_words {
            let (_, action) = ide.read_data_word();
            if i < total_words - 1 {
                assert_eq!(action, IdeAction::None);
            } else {
                assert_eq!(action, IdeAction::ScheduleCompletion);
            }
        }
    }

    #[test]
    fn read_cd_sub_header_only() {
        let mut ide = IdeController::new(44100);
        ide.insert_cdrom(make_test_cdimage());
        ide.atapi_state.media_loaded = true;
        ide.atapi_state.media_changed = false;

        setup_atapi_for_packet(&mut ide, 0xFFFE);
        // flags=0x40: header_code=10 (sub-header only). Transfer = 8 bytes = 4 words.
        send_read_cd_cdb(&mut ide, 0, 1, 0x40);

        for i in 0..4u32 {
            let (_, action) = ide.read_data_word();
            if i < 3 {
                assert_eq!(action, IdeAction::None);
            } else {
                assert_eq!(action, IdeAction::ScheduleCompletion);
            }
        }
    }
}
