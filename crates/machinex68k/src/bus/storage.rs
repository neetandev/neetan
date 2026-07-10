//! Storage-controller glue: address decode, IOC interrupts, and DMA lines.
//!
//! The original X68000 exposes the SASI HDC's four odd-addressed registers
//! in the 0xE96000 window; the SUPER/XVI expose the internal MB89352 SPC's
//! sixteen registers at 0xE96020 with odd addressing. Everything protocol-
//! level lives in the device crate; this module only routes register
//! accesses, mirrors the controllers' interrupt lines into the IOC, and
//! pumps HD63450 channel 1 while a controller requests DMA.

use common::Tracing;
use device::{
    cdrom::CdImage,
    disk::{HddImage, MountedHdd},
    scsi::{ScsiCdrom, ScsiDisk, ScsiTarget},
};

use super::X68kBus;
use crate::{IocSource, X68kStorageController};

/// Number of hard-disk slots (SASI IDs 0/1 or SCSI IDs 0/1).
pub const X68K_HDD_SLOT_COUNT: usize = 2;

/// SASI register offset: data.
const SASI_DATA_OFFSET: u32 = 1;
/// SASI register offset: bus status read / command-phase start write.
const SASI_STATUS_OFFSET: u32 = 3;
/// SASI register offset: controller reset.
const SASI_RESET_OFFSET: u32 = 5;
/// SASI register offset: selection.
const SASI_SELECT_OFFSET: u32 = 7;

/// SCSI ID of the internal CD-ROM drive.
const SCSI_CDROM_ID: usize = 6;

impl<T: Tracing> X68kBus<T> {
    /// Reads a storage-controller register.
    pub(super) fn read_storage_register(&mut self, address: u32) -> u8 {
        let value = match self.model.storage_controller() {
            X68kStorageController::Sasi => match address & 7 {
                SASI_DATA_OFFSET => self.hdc.read_data(),
                SASI_STATUS_OFFSET => self.hdc.read_status(),
                _ => 0xFF,
            },
            X68kStorageController::InternalScsi => {
                if address & 0x20 == 0 {
                    // The low half of the window is not the SPC; the IPL
                    // probes it and relies on this pattern.
                    return if address & 0x02 == 0 { 0 } else { 0xFF };
                }
                self.spc.read_register(((address & 0x1F) >> 1) as usize)
            }
        };
        self.sync_storage_lines();
        value
    }

    /// Writes a storage-controller register.
    pub(super) fn write_storage_register(&mut self, address: u32, value: u8) {
        match self.model.storage_controller() {
            X68kStorageController::Sasi => match address & 7 {
                SASI_DATA_OFFSET => {
                    let now = self.current_cycle;
                    self.hdc.write_data(value, now);
                }
                SASI_STATUS_OFFSET => self.hdc.write_command_start(value),
                SASI_RESET_OFFSET => self.hdc.write_reset(value),
                SASI_SELECT_OFFSET => self.hdc.write_select(value),
                _ => {}
            },
            X68kStorageController::InternalScsi => {
                if address & 0x20 == 0 {
                    return;
                }
                let now = self.current_cycle;
                self.spc
                    .write_register(((address & 0x1F) >> 1) as usize, value, now);
            }
        }
        self.sync_storage_lines();
    }

    /// Runs SASI HDC work due at the current cycle.
    pub(super) fn on_storage_hdc_due(&mut self) {
        let now = self.current_cycle;
        self.hdc.run_due(now);
        self.sync_storage_lines();
    }

    /// Runs internal-SCSI SPC work due at the current cycle.
    pub(super) fn on_storage_spc_due(&mut self) {
        let now = self.current_cycle;
        self.spc.run_due(now);
        self.sync_storage_lines();
    }

    /// Ends the active controller's DMA involvement when channel 1 exhausts
    /// its count; the controllers advance their phases from the data flow
    /// itself, so only the interrupt lines need mirroring here.
    pub(super) fn on_storage_terminal_count(&mut self) {
        self.sync_storage_interrupts();
    }

    /// Mirrors interrupt lines into the IOC and pumps pending DMA.
    pub(super) fn sync_storage_lines(&mut self) {
        self.sync_storage_interrupts();
        self.pump_storage_dma();
        self.schedule_events();
    }

    /// Mirrors the active controller's interrupt line into the IOC.
    fn sync_storage_interrupts(&mut self) {
        match self.model.storage_controller() {
            X68kStorageController::Sasi => {
                if self.hdc.take_completion_interrupt() {
                    self.interrupts.ioc.signal(IocSource::Hdc);
                }
            }
            X68kStorageController::InternalScsi => {
                let line = self.spc.irq_asserted();
                if line && !self.spc_irq_line {
                    self.interrupts.ioc.signal(IocSource::Spc);
                } else if !line && self.spc_irq_line {
                    self.interrupts.ioc.clear(IocSource::Spc);
                }
                self.spc_irq_line = line;
            }
        }
    }

    /// Mounts a hard-disk image into `slot`, validating the sector size the
    /// model's controller requires. On the SASI model the SRAM unit count is
    /// raised so the IPL boot scan covers the attached drives.
    pub fn insert_hdd(
        &mut self,
        slot: usize,
        image: HddImage,
        path: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        if slot >= X68K_HDD_SLOT_COUNT {
            return Err(format!("X68000 hard-disk slot {slot} is not installed"));
        }
        let controller = self.model.storage_controller();
        let required_sector_size = controller.sector_size();
        match controller {
            X68kStorageController::Sasi => {
                if image.geometry.sector_size != required_sector_size {
                    return Err(format!(
                        "{} SASI hard disks need {required_sector_size}-byte sectors, got {}",
                        self.model, image.geometry.sector_size
                    ));
                }
                self.hdc.insert_drive(slot, image, path);
                self.sram.set_sasi_hdmax(self.hdc.drive_count());
            }
            X68kStorageController::InternalScsi => {
                if image.geometry.sector_size != required_sector_size {
                    return Err(format!(
                        "{} SCSI hard disks need {required_sector_size}-byte sectors, got {}",
                        self.model, image.geometry.sector_size
                    ));
                }
                self.spc.insert_target(
                    slot,
                    ScsiTarget::Disk(ScsiDisk::new(MountedHdd::new(image, path))),
                );
            }
        }
        Ok(())
    }

    /// Ejects and flushes the hard disk in `slot`, if any.
    pub fn eject_hdd(&mut self, slot: usize) {
        match self.model.storage_controller() {
            X68kStorageController::Sasi => {
                self.hdc.eject_drive(slot);
                self.sram.set_sasi_hdmax(self.hdc.drive_count());
            }
            X68kStorageController::InternalScsi => self.spc.eject_target(slot),
        }
    }

    /// Flushes every mounted hard disk to its backing file.
    pub fn flush_hdds(&mut self) {
        self.hdc.flush();
        self.spc.flush();
    }

    /// Inserts CD-ROM media into the internal SCSI drive at ID 6, raising
    /// unit attention on a media change.
    pub fn insert_cdrom(&mut self, image: CdImage) -> Result<(), String> {
        match self.model.storage_controller() {
            X68kStorageController::Sasi => Err(format!(
                "{} has no CD-ROM interface; SCSI CD-ROM needs an internal-SCSI model",
                self.model
            )),
            X68kStorageController::InternalScsi => {
                match self.spc.target_mut(SCSI_CDROM_ID) {
                    Some(ScsiTarget::Cdrom(cdrom)) => cdrom.insert_media(image),
                    Some(ScsiTarget::Disk(_)) | None => {
                        let mut cdrom = ScsiCdrom::new(self.sample_rate);
                        cdrom.insert_media(image);
                        self.spc
                            .insert_target(SCSI_CDROM_ID, ScsiTarget::Cdrom(cdrom));
                    }
                }
                Ok(())
            }
        }
    }

    /// Ejects the CD-ROM media, keeping the drive attached to the bus.
    pub fn eject_cdrom(&mut self) {
        if let Some(ScsiTarget::Cdrom(cdrom)) = self.spc.target_mut(SCSI_CDROM_ID) {
            cdrom.eject_media();
        }
    }

    /// Moves operands over DMAC channel 1 while the active controller
    /// requests DMA and the channel is armed.
    fn pump_storage_dma(&mut self) {
        loop {
            let requesting = match self.model.storage_controller() {
                X68kStorageController::Sasi => self.hdc.dma_request(),
                X68kStorageController::InternalScsi => self.spc.dma_request(),
            };
            if !requesting || !self.dmac.channel_active(1) {
                return;
            }
            self.assert_storage_dmac_request();
            self.sync_storage_interrupts();
        }
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, M68000AccessSize, M68000FunctionCode};

    use crate::{
        X68kModel,
        bus::test_support::{access, bus},
    };

    #[test]
    fn original_model_reports_an_empty_sasi_bus() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        let status = access(0xE96003, M68000AccessSize::Byte, supervisor);
        let data = access(0xE96001, M68000AccessSize::Byte, supervisor);

        assert_eq!(bus.m68000_read(status), Ok(0));
        assert_eq!(bus.m68000_read(data), Ok(0));
        bus.m68000_write(status, 0xFF).unwrap();
        assert_eq!(bus.m68000_read(status), Ok(0));
    }

    #[test]
    fn internal_scsi_low_window_keeps_the_probe_pattern() {
        let mut bus = bus(X68kModel::X68000Super);
        let supervisor = M68000FunctionCode::SupervisorData;
        assert_eq!(
            bus.m68000_read(access(0xE96001, M68000AccessSize::Byte, supervisor)),
            Ok(0)
        );
        assert_eq!(
            bus.m68000_read(access(0xE96003, M68000AccessSize::Byte, supervisor)),
            Ok(0xFF)
        );
        bus.m68000_write(access(0xE96003, M68000AccessSize::Byte, supervisor), 0x55)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xE96003, M68000AccessSize::Byte, supervisor)),
            Ok(0xFF)
        );
    }
}
