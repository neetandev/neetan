//! Sony WD2793 disk-system glue.

use std::path::PathBuf;

use common::{
    TraceContext, TraceDeviceEvent, TraceEvent, TraceEventKey, TraceField, TraceSink, TraceValue,
    trace_id,
};
use device::floppy::FloppyImage;

use super::{MsxBus, OPEN_BUS};
use crate::{FirmwareRegion, scheduler::EventMsx};

/// Status and command register offset in the disk ROM window.
const FDC_STATUS_COMMAND: u16 = 0x3FF8;
/// Track register offset in the disk ROM window.
const FDC_TRACK: u16 = 0x3FF9;
/// Sector register offset in the disk ROM window.
const FDC_SECTOR: u16 = 0x3FFA;
/// Data register offset in the disk ROM window.
const FDC_DATA: u16 = 0x3FFB;
/// Side-select latch offset in the disk ROM window.
const FDC_SIDE: u16 = 0x3FFC;
/// Drive-select and motor latch offset in the disk ROM window.
const FDC_DRIVE_CONTROL: u16 = 0x3FFD;
/// WD2793 line-status offset in the disk ROM window.
const FDC_LINE_STATUS: u16 = 0x3FFF;
/// Drive-control motor bit.
const DRIVE_CONTROL_MOTOR: u8 = 0x80;
/// Drive-control drive-selection mask.
const DRIVE_CONTROL_SELECT_MASK: u8 = 0x03;

impl<T: TraceSink> MsxBus<T> {
    /// Returns whether the disk ROM exposes an FDC register at this address.
    pub(super) fn fdc_is_selected(&self, address: u16) -> bool {
        (FDC_STATUS_COMMAND..=FDC_LINE_STATUS).contains(&(address & 0x3FFF))
            && self.memory.selected_firmware_region(address) == Some(FirmwareRegion::DiskRom)
    }

    /// Reads a selected disk-system register.
    pub(super) fn fdc_read(&mut self, address: u16) -> u8 {
        let offset = address & 0x3FFF;
        let now = self.current_cycle;
        let Some(controller) = self.fdc.as_mut() else {
            return OPEN_BUS;
        };
        let value = match offset {
            FDC_STATUS_COMMAND => controller.read_status(now),
            FDC_TRACK => controller.read_track_register(),
            FDC_SECTOR => controller.read_sector_register(),
            FDC_DATA => controller.read_data_pio(now),
            FDC_SIDE => 0xFE | controller.side(),
            FDC_DRIVE_CONTROL => {
                let disk_changed = controller.read_drive_status() & 0x01 != 0;
                (self.fdc_drive_control & !0x04) | (u8::from(!disk_changed) << 2)
            }
            FDC_LINE_STATUS => {
                0x3F | u8::from(!controller.irq_line()) << 6 | u8::from(!controller.drq()) << 7
            }
            _ => OPEN_BUS,
        };
        self.sync_fdc_schedule();
        value
    }

    /// Emits an FDC read device trace event for a read-sector command.
    fn trace_fdc_read(&mut self, cylinder: u8, record: u8) {
        if !T::ENABLED
            || !self.tracer.interested(TraceEventKey::Device {
                device: trace_id::device::MSX_FDC,
                action: trace_id::action::READ,
            })
        {
            return;
        }
        self.tracer.trace(
            TraceContext::main_cpu(self.current_cycle, Some(u64::from(self.cpu_clock_hz()))),
            TraceEvent::Device(TraceDeviceEvent {
                device: trace_id::device::MSX_FDC,
                action: trace_id::action::READ,
                fields: &[
                    TraceField {
                        name: trace_id::field::CYLINDER,
                        value: TraceValue::Unsigned(u64::from(cylinder)),
                    },
                    TraceField {
                        name: trace_id::field::RECORD,
                        value: TraceValue::Unsigned(u64::from(record)),
                    },
                ],
            }),
        );
    }

    /// Writes a selected disk-system register.
    pub(super) fn fdc_write(&mut self, address: u16, value: u8) {
        let offset = address & 0x3FFF;
        let now = self.current_cycle;
        let mut fdc_read = None;
        let Some(controller) = self.fdc.as_mut() else {
            return;
        };
        match offset {
            FDC_STATUS_COMMAND => {
                controller.write_command(value, now);
                // WD17xx Type II Read Sector opcodes are 0x80..0x9F.
                if value & 0xE0 == 0x80 {
                    fdc_read = Some((
                        controller.read_track_register(),
                        controller.read_sector_register(),
                    ));
                }
            }
            FDC_TRACK => controller.write_track_register(value),
            FDC_SECTOR => controller.write_sector_register(value),
            FDC_DATA => controller.write_data_pio(value, now),
            FDC_SIDE => controller.set_side(value),
            FDC_DRIVE_CONTROL => {
                self.fdc_drive_control = value;
                let selected_drive = match value & DRIVE_CONTROL_SELECT_MASK {
                    0 | 2 => 0,
                    1 => 1,
                    _ => 3,
                };
                controller.select_drive(selected_drive);
                controller.set_msx_motor(value & DRIVE_CONTROL_MOTOR != 0, now);
            }
            _ => {}
        }
        if let Some((cylinder, record)) = fdc_read {
            self.trace_fdc_read(cylinder, record);
        }
        self.sync_fdc_schedule();
    }

    /// Synchronizes WD2793 task and PIO scheduler slots.
    pub(super) fn sync_fdc_schedule(&mut self) {
        let Some(controller) = self.fdc.as_ref() else {
            self.scheduler.cancel(EventMsx::FdcTask);
            self.scheduler.cancel(EventMsx::FdcPio);
            return;
        };
        match controller.next_task_cycle() {
            Some(cycle) => self.scheduler.schedule(EventMsx::FdcTask, cycle),
            None => self.scheduler.cancel(EventMsx::FdcTask),
        }
        match controller.next_pio_event_cycle() {
            Some(cycle) => self.scheduler.schedule(EventMsx::FdcPio, cycle),
            None => self.scheduler.cancel(EventMsx::FdcPio),
        }
    }

    /// Runs a scheduled WD2793 command task.
    pub(super) fn run_fdc_task(&mut self, now: u64) {
        if let Some(controller) = self.fdc.as_mut() {
            let outcome = controller.run_task(now);
            debug_assert!(outcome.dma_read.is_none() && outcome.dma_write_len.is_none());
        }
        self.sync_fdc_schedule();
    }

    /// Runs a scheduled WD2793 PIO event.
    pub(super) fn run_fdc_pio(&mut self, now: u64) {
        if let Some(controller) = self.fdc.as_mut() {
            controller.run_pio_event(now);
        }
        self.sync_fdc_schedule();
    }

    /// Mounts a floppy image in a built-in drive.
    pub fn insert_floppy(&mut self, drive: usize, image: FloppyImage, path: PathBuf) {
        self.insert_floppy_backed(drive, image, path.into());
    }

    /// Mounts a floppy image in a built-in drive with the requested backing.
    pub fn insert_floppy_backed(
        &mut self,
        drive: usize,
        image: FloppyImage,
        backing: common::MediaBacking,
    ) {
        let digest = crate::cartridge::digest_hex(&image.to_bytes());
        if let Some(mapper) = crate::sound_cartridge_for_disk_blake3(&digest)
            && !self.memory.cartridge_present(1)
        {
            self.memory
                .insert_cartridge_with_mapper(1, &[], mapper, None, self.current_cycle)
                .expect("automatic SCC+ cartridge has a valid slot and mapper");
        }
        if let Some(controller) = self.fdc.as_mut() {
            controller.insert_backed(drive, image, backing);
        }
    }

    /// Returns the current in-memory bytes of the floppy in `drive`, if mounted.
    pub fn floppy_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.fdc
            .as_ref()
            .and_then(|controller| controller.drive_image_bytes(drive))
    }

    /// Ejects and flushes a built-in floppy.
    pub fn eject_floppy(&mut self, drive: usize) {
        if let Some(controller) = self.fdc.as_mut() {
            controller.eject(drive);
        }
    }

    /// Flushes every mounted floppy.
    pub fn flush_floppies(&mut self) {
        if let Some(controller) = self.fdc.as_mut() {
            controller.flush_all();
        }
    }
}
