//! AT secondary-channel ATAPI CD-ROM controller on the shared LLE core.
//!
//! Exposes the standard task-file register set decoded by the machine at
//! ports 0x170-0x177/0x376. A PC/AT decodes two physically separate IDE port
//! ranges, so the secondary channel is its own controller instance with its
//! own interrupt and DRQ state, independent of the primary HDD channel. The
//! ATAPI drive lives on the core's channel 1, which this controller pins as
//! the active channel once a disc is inserted.

use super::{
    atapi,
    lle::{self, IdeAction, IdePhase},
};
use crate::{cd_audio::CdAudioPlayer, cdrom::CdImage, disk::MountedHdd, scsi::cdrom::ScsiCdrom};

save_state::runtime_state! {
/// Authoritative AT secondary ATAPI channel state.
#[derive(Clone)]
pub struct AtAtapiControllerState {
    controller: lle::Controller,
    optical: crate::scsi::cdrom::ScsiCdromState,
    atapi: atapi::AtapiState,
    media: save_state::MediaManifest,
}}

/// AT secondary-channel ATAPI CD-ROM controller.
#[derive(Debug)]
pub struct AtAtapiController {
    controller: lle::Controller,
    // The ATAPI channel carries no hard disks, but the shared core's
    // data-register path takes the HDD array by reference; it stays empty.
    drives: [Option<MountedHdd>; 2],
    optical: ScsiCdrom,
    atapi_state: atapi::AtapiState,
}

impl AtAtapiController {
    /// Creates an idle controller with no disc inserted.
    pub fn new(output_sample_rate: u32) -> Self {
        Self {
            controller: lle::Controller::new(),
            drives: [None, None],
            optical: ScsiCdrom::new(output_sample_rate),
            atapi_state: atapi::AtapiState::new(),
        }
    }

    /// Captures packet transport, CD audio, and mounted disc identity.
    pub fn capture_state(
        &self,
    ) -> Result<AtAtapiControllerState, save_state::StateValidationError> {
        Ok(AtAtapiControllerState {
            controller: self.controller.clone(),
            optical: self.optical.capture_state(),
            atapi: self.atapi_state.clone(),
            media: self.media_manifest()?,
        })
    }

    /// Restores packet transport and CD audio while retaining disc contents.
    pub fn restore_state(
        &mut self,
        state: AtAtapiControllerState,
    ) -> Result<(), save_state::StateValidationError> {
        state.controller.validate_state()?;
        state.atapi.validate_state()?;
        self.optical.validate_state(&state.optical)?;
        state.media.verify_current(&self.media_manifest()?)?;
        self.optical.restore_state(state.optical)?;
        self.controller = state.controller;
        self.atapi_state = state.atapi;
        Ok(())
    }

    /// Returns the mounted disc binding.
    pub fn media_manifest(
        &self,
    ) -> Result<save_state::MediaManifest, save_state::StateValidationError> {
        let bindings = self
            .optical
            .media()
            .map(|media| {
                Ok(save_state::MediaBinding {
                    identifier: save_state::MediaBindingId::new("atapi-0")?,
                    slot: save_state::MediaSlot::new(save_state::MediaKind::CdRom, 0),
                    source_path: media.source_path().cloned(),
                    media_type: "cdrom".to_owned(),
                    identity: media.identity(),
                    geometry: None,
                    write_protected: true,
                    backend_generation: None,
                })
            })
            .transpose()?
            .into_iter()
            .collect();
        save_state::MediaManifest::new(bindings)
    }

    /// Inserts a CD-ROM image and activates the ATAPI drive on channel 1.
    pub fn insert_cdrom(&mut self, image: CdImage) {
        self.optical.insert_media(image);
        self.atapi_state.media_inserted();
        self.controller.initialize_atapi_drive();
        self.controller.select_channel(1);
    }

    /// Ejects the CD-ROM image.
    pub fn eject_cdrom(&mut self) {
        self.optical.eject_media();
        self.atapi_state.media_ejected();
    }

    /// Returns true if a CD-ROM image is loaded.
    pub fn has_cdrom(&self) -> bool {
        self.optical.has_media()
    }

    /// Returns a reference to the loaded CD-ROM image, if any.
    pub fn cdrom_image(&self) -> Option<&CdImage> {
        self.optical.media()
    }

    /// Returns a reference to the CD audio player.
    pub fn cd_audio_player(&self) -> &CdAudioPlayer {
        self.optical.audio()
    }

    /// Returns a mutable reference to the CD audio player.
    pub fn cd_audio_player_mut(&mut self) -> &mut CdAudioPlayer {
        self.optical.audio_mut()
    }

    /// Mixes CD audio into the output buffer for one audio frame.
    pub fn generate_cd_audio_samples(&mut self, volume: f32, output: &mut [f32]) {
        self.optical
            .generate_audio_samples([volume, volume], output);
    }

    /// Reads the 16-bit data register (port 0x170).
    pub fn read_data_word(&mut self) -> (u16, IdeAction) {
        if self.controller.is_atapi_channel_active()
            && self.controller.atapi_phase() == IdePhase::PacketDataIn
        {
            return atapi::route_read_data_word(&mut self.controller, &mut self.atapi_state);
        }
        self.controller.read_data_word(&self.drives)
    }

    /// Writes the 16-bit data register (port 0x170).
    pub fn write_data_word(&mut self, value: u16) -> IdeAction {
        if self.controller.is_atapi_channel_active() {
            return atapi::route_write_data_word(
                &mut self.controller,
                &mut self.atapi_state,
                &mut self.optical,
                value,
            );
        }
        self.controller.write_data_word(value, &mut self.drives)
    }

    /// Reads the error register (port 0x171).
    pub fn read_error(&mut self) -> u8 {
        self.controller.read_error()
    }

    /// Reads the sector count register (port 0x172).
    pub fn read_sector_count(&self) -> u8 {
        self.controller.read_sector_count()
    }

    /// Reads the sector number register (port 0x173).
    pub fn read_sector_number(&self) -> u8 {
        self.controller.read_sector_number()
    }

    /// Reads the cylinder low register (port 0x174).
    pub fn read_cylinder_low(&self) -> u8 {
        self.controller.read_cylinder_low()
    }

    /// Reads the cylinder high register (port 0x175).
    pub fn read_cylinder_high(&self) -> u8 {
        self.controller.read_cylinder_high()
    }

    /// Reads the device/head register (port 0x176).
    pub fn read_device_head(&self) -> u8 {
        self.controller.read_device_head()
    }

    /// Reads the status register (port 0x177). Clears the pending interrupt;
    /// the returned flag asks the bus to deassert the IRQ line.
    pub fn read_status(&mut self) -> (u8, bool) {
        self.controller.read_status()
    }

    /// Reads the alternate status register (port 0x376).
    pub fn read_alt_status(&self) -> u8 {
        self.controller.read_alt_status()
    }

    /// Writes the features register (port 0x171).
    pub fn write_features(&mut self, value: u8) {
        self.controller.write_features(value);
    }

    /// Writes the sector count register (port 0x172).
    pub fn write_sector_count(&mut self, value: u8) {
        self.controller.write_sector_count(value);
    }

    /// Writes the sector number register (port 0x173).
    pub fn write_sector_number(&mut self, value: u8) {
        self.controller.write_sector_number(value);
    }

    /// Writes the cylinder low register (port 0x174).
    pub fn write_cylinder_low(&mut self, value: u8) {
        self.controller.write_cylinder_low(value);
    }

    /// Writes the cylinder high register (port 0x175).
    pub fn write_cylinder_high(&mut self, value: u8) {
        self.controller.write_cylinder_high(value);
    }

    /// Writes the device/head register (port 0x176).
    pub fn write_device_head(&mut self, value: u8) {
        self.controller.write_device_head(value);
    }

    /// Writes the command register (port 0x177).
    pub fn write_command(&mut self, value: u8) -> IdeAction {
        if self.controller.is_atapi_channel_active() {
            return atapi::route_command(&mut self.controller, &mut self.atapi_state, value);
        }
        self.controller.write_command(value, &self.drives)
    }

    /// Writes the device control register (port 0x376).
    pub fn write_device_control(&mut self, value: u8) {
        self.controller.write_device_control(value);
    }

    /// Called when the scheduled completion event fires.
    /// Returns true if an interrupt should be raised.
    pub fn complete_operation(&mut self) -> bool {
        self.controller.complete_operation()
    }
}
