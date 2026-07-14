//! AT IDE secondary channel glue: ports 0x170-0x177 and 0x376, IRQ 15.
//!
//! The secondary channel carries the ATAPI CD-ROM. Register accesses go to a
//! dedicated ATAPI controller so its interrupt and DRQ state stay independent
//! of the primary HDD channel. Completions are deferred through the scheduler
//! like the primary channel, using its own event slots.

use common::TraceSink;
use device::{cdrom::CdImage, ide::IdeAction};

use crate::{
    bus::{
        AtBus, IRQ_IDE_SECONDARY,
        hdd::{IDE_EXECUTION_DELAY_MICROS, IDE_INTERRUPT_DELAY_MICROS},
    },
    scheduler::EventAt,
};

impl<T: TraceSink> AtBus<T> {
    /// Reads one secondary-channel register (ports 0x170-0x177 and 0x376).
    pub(super) fn ide_secondary_io_read(&mut self, port: u16) -> u8 {
        match port {
            0x170 => {
                let (word, action) = self.ide_secondary.read_data_word();
                self.process_ide_secondary_action(action);
                word as u8
            }
            0x171 => self.ide_secondary.read_error(),
            0x172 => self.ide_secondary.read_sector_count(),
            0x173 => self.ide_secondary.read_sector_number(),
            0x174 => self.ide_secondary.read_cylinder_low(),
            0x175 => self.ide_secondary.read_cylinder_high(),
            0x176 => self.ide_secondary.read_device_head(),
            0x177 => {
                let (status, clear_irq) = self.ide_secondary.read_status();
                if clear_irq {
                    self.clear_irq(IRQ_IDE_SECONDARY);
                }
                status
            }
            0x376 => self.ide_secondary.read_alt_status(),
            _ => super::OPEN_BUS_BYTE,
        }
    }

    /// Writes one secondary-channel register (ports 0x170-0x177 and 0x376).
    pub(super) fn ide_secondary_io_write(&mut self, port: u16, value: u8) {
        match port {
            0x170 => {
                let action = self.ide_secondary.write_data_word(u16::from(value));
                self.process_ide_secondary_action(action);
            }
            0x171 => self.ide_secondary.write_features(value),
            0x172 => self.ide_secondary.write_sector_count(value),
            0x173 => self.ide_secondary.write_sector_number(value),
            0x174 => self.ide_secondary.write_cylinder_low(value),
            0x175 => self.ide_secondary.write_cylinder_high(value),
            0x176 => self.ide_secondary.write_device_head(value),
            0x177 => {
                let action = self.ide_secondary.write_command(value);
                self.process_ide_secondary_action(action);
            }
            0x376 => self.ide_secondary.write_device_control(value),
            _ => {}
        }
    }

    /// Reads the 16-bit data register (port 0x170).
    pub(super) fn ide_secondary_read_data_word(&mut self) -> u16 {
        let (word, action) = self.ide_secondary.read_data_word();
        self.process_ide_secondary_action(action);
        word
    }

    /// Writes the 16-bit data register (port 0x170).
    pub(super) fn ide_secondary_write_data_word(&mut self, value: u16) {
        let action = self.ide_secondary.write_data_word(value);
        self.process_ide_secondary_action(action);
    }

    /// Schedules the completion event when the ATAPI core requests it.
    fn process_ide_secondary_action(&mut self, action: IdeAction) {
        match action {
            IdeAction::None => {}
            IdeAction::ScheduleCompletion => {
                let cycles = (u64::from(self.clocks.cpu_clock_hz) * IDE_EXECUTION_DELAY_MICROS
                    / 1_000_000)
                    .max(1);
                self.scheduler
                    .schedule(EventAt::IdeSecondaryExecution, self.current_cycle + cycles);
                self.update_next_event_cycle();
            }
        }
    }

    /// Completes the pending ATAPI operation and arms the interrupt event.
    pub(crate) fn handle_ide_secondary_execution(&mut self) {
        if self.ide_secondary.complete_operation() {
            let cycles = (u64::from(self.clocks.cpu_clock_hz) * IDE_INTERRUPT_DELAY_MICROS
                / 1_000_000)
                .max(1);
            self.scheduler
                .schedule(EventAt::IdeSecondaryInterrupt, self.current_cycle + cycles);
            self.update_next_event_cycle();
        }
    }

    /// Raises the secondary-channel interrupt.
    pub(crate) fn handle_ide_secondary_interrupt(&mut self) {
        self.raise_irq(IRQ_IDE_SECONDARY);
    }

    /// Inserts a CD-ROM image into the ATAPI drive.
    pub fn insert_cdrom(&mut self, image: CdImage) -> Result<(), String> {
        self.ide_secondary.insert_cdrom(image);
        Ok(())
    }

    /// Ejects the CD-ROM image from the ATAPI drive.
    pub fn eject_cdrom(&mut self) {
        self.ide_secondary.eject_cdrom();
    }

    /// Returns whether a CD-ROM image is loaded.
    pub fn has_cdrom(&self) -> bool {
        self.ide_secondary.has_cdrom()
    }
}
