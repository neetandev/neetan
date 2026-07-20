//! MB8877 floppy controller glue for the X1.
//!
//! The MB8877 (WD1793 family) hangs straight off the main Z80 and moves sector
//! data one byte at a time through the data register, paced by the DRQ line:
//! the BIOS polls the status register for BUSY / DRQ, while X1 turbo software
//! arms the Z80 DMA, whose ready input the same DRQ line feeds. The
//! controller's completion interrupt is not routed to the daisy chain; the
//! BIOS polls instead.
//!
//! Registers live at `0x0FF8-0x0FFB`; `0x0FFC` is the drive/side/motor control
//! latch. On the turbo, reads of `0x0FFC`/`0x0FFD` select FM / MFM density.

use common::{
    TraceContext, TraceDeviceEvent, TraceEvent, TraceEventKey, TraceField, TraceSink, TraceValue,
    trace_id,
};
use device::floppy::FloppyImage;

use super::{OPEN_BUS, X1Bus};
use crate::scheduler::EventX1;

/// Status register (read) and command register (write).
const FDC_STATUS_COMMAND: u16 = 0x0FF8;
/// Track register.
const FDC_TRACK: u16 = 0x0FF9;
/// Sector register.
const FDC_SECTOR: u16 = 0x0FFA;
/// Data register (PIO transfer).
const FDC_DATA: u16 = 0x0FFB;
/// Drive/side/motor control latch (write); FM density select (read).
const FDC_CONTROL_FM: u16 = 0x0FFC;
/// MFM density select (read).
const FDC_CONTROL_MFM: u16 = 0x0FFD;

/// Control-latch bit fields at `0x0FFC`.
const CONTROL_DRIVE_MASK: u8 = 0x03;
const CONTROL_SIDE: u8 = 0x10;
const CONTROL_MOTOR: u8 = 0x80;

impl<T: TraceSink> X1Bus<T> {
    /// Reads an FDC port (`0x0FF8-0x0FFF`).
    pub(super) fn fdc_read(&mut self, port: u16) -> u8 {
        let now = self.current_cycle;
        match port {
            FDC_STATUS_COMMAND => self.fdc.read_status(now),
            FDC_TRACK => self.fdc.read_track_register(),
            FDC_SECTOR => self.fdc.read_sector_register(),
            FDC_DATA => {
                let value = self.fdc.read_data_pio(now);
                self.sync_fdc_schedule();
                value
            }
            FDC_CONTROL_FM if self.model.is_turbo() => {
                self.fdc.set_double_density(false);
                OPEN_BUS
            }
            FDC_CONTROL_MFM if self.model.is_turbo() => {
                self.fdc.set_double_density(true);
                OPEN_BUS
            }
            _ => OPEN_BUS,
        }
    }

    /// Emits an FDC read device trace event for a read-sector command.
    fn trace_fdc_read_command(&mut self, command: u8) {
        // WD17xx Type II Read Sector opcodes are 0x80..0x9F.
        if command & 0xE0 != 0x80 {
            return;
        }
        if !T::ENABLED
            || !self.tracer.interested(TraceEventKey::Device {
                device: trace_id::device::X1_FDC,
                action: trace_id::action::READ,
            })
        {
            return;
        }
        let cylinder = self.fdc.read_track_register();
        let record = self.fdc.read_sector_register();
        self.tracer.trace(
            TraceContext::main_cpu(self.current_cycle, Some(u64::from(self.cpu_clock_hz()))),
            TraceEvent::Device(TraceDeviceEvent {
                device: trace_id::device::X1_FDC,
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

    /// Writes an FDC port (`0x0FF8-0x0FFF`).
    pub(super) fn fdc_write(&mut self, port: u16, value: u8) {
        let now = self.current_cycle;
        match port {
            FDC_STATUS_COMMAND => {
                self.fdc.write_command(value, now);
                self.trace_fdc_read_command(value);
                self.sync_fdc_schedule();
            }
            FDC_TRACK => self.fdc.write_track_register(value),
            FDC_SECTOR => self.fdc.write_sector_register(value),
            FDC_DATA => {
                self.fdc.write_data_pio(value, now);
                self.sync_fdc_schedule();
            }
            FDC_CONTROL_FM => {
                self.fdc.select_drive((value & CONTROL_DRIVE_MASK) as usize);
                self.fdc.set_side(u8::from(value & CONTROL_SIDE != 0));
                self.fdc.set_motor(value & CONTROL_MOTOR != 0);
            }
            _ => {}
        }
    }

    /// Schedules (or cancels) the controller's next command-completion task.
    pub(super) fn sync_fdc_schedule(&mut self) {
        match self.fdc.next_task_cycle() {
            Some(cycle) => self.scheduler.schedule(EventX1::FdcSeekComplete, cycle),
            None => self.scheduler.cancel(EventX1::FdcSeekComplete),
        }
    }

    /// Runs the controller's scheduled command task (seek settle, sector fetch,
    /// or the next record of a multi-sector transfer). Transfer commands stage
    /// their bytes behind the DRQ line; the CPU-polled data register and the
    /// turbo Z80 DMA both drain them from there.
    pub(super) fn on_fdc_seek_complete(&mut self, now: u64) {
        let outcome = self.fdc.run_task(now);
        debug_assert!(outcome.dma_read.is_none() && outcome.dma_write_len.is_none());
        self.sync_fdc_schedule();
        self.sync_interrupts();
        self.sync_dma_tick();
    }

    /// Mounts a floppy image into `drive`, remembering its backing path.
    pub fn insert_floppy(&mut self, drive: usize, image: FloppyImage, path: std::path::PathBuf) {
        self.insert_floppy_backed(drive, image, path.into());
    }

    /// Mounts a floppy image into `drive` with the requested backing.
    pub fn insert_floppy_backed(
        &mut self,
        drive: usize,
        image: FloppyImage,
        backing: common::MediaBacking,
    ) {
        self.fdc.insert_backed(drive, image, backing);
    }

    /// Returns the current in-memory bytes of the floppy in `drive`, if mounted.
    pub fn floppy_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.fdc.drive_image_bytes(drive)
    }

    /// Ejects and flushes the floppy in `drive`.
    pub fn eject_floppy(&mut self, drive: usize) {
        self.fdc.eject(drive);
    }

    /// Flushes every mounted floppy to its backing file.
    pub fn flush_floppies(&mut self) {
        self.fdc.flush_all();
    }
}
