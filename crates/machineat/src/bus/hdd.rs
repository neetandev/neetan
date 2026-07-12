//! AT IDE primary channel glue: ports 0x1F0-0x1F7 and 0x3F6, IRQ 14.
//!
//! Register accesses go straight to the shared LLE ATA core. Operations
//! that signal completion are deferred through the scheduler so the
//! interrupt arrives after the command returns to the guest.

use common::Tracing;
use device::{
    disk::{HddFormat, HddImage},
    ide::IdeAction,
};

use crate::{
    bus::{AtBus, IRQ_IDE_PRIMARY},
    cmos::{set_boot_sequence, set_hard_disk_user_type},
    config::AtBootDevice,
    scheduler::EventAt,
};

/// Delay from command acceptance to completion, in microseconds.
pub(super) const IDE_EXECUTION_DELAY_MICROS: u64 = 100;

/// Delay from completion to the interrupt, in microseconds.
pub(super) const IDE_INTERRUPT_DELAY_MICROS: u64 = 25;

impl<T: Tracing> AtBus<T> {
    /// Reads one IDE task-file register (ports 0x1F0-0x1F7 and 0x3F6).
    pub(super) fn ide_io_read(&mut self, port: u16) -> u8 {
        match port {
            0x1F0 => {
                let (word, action) = self.ide.read_data_word();
                self.process_ide_action(action);
                word as u8
            }
            0x1F1 => self.ide.read_error(),
            0x1F2 => self.ide.read_sector_count(),
            0x1F3 => self.ide.read_sector_number(),
            0x1F4 => self.ide.read_cylinder_low(),
            0x1F5 => self.ide.read_cylinder_high(),
            0x1F6 => self.ide.read_device_head(),
            0x1F7 => {
                let (status, clear_irq) = self.ide.read_status();
                if clear_irq {
                    self.clear_irq(IRQ_IDE_PRIMARY);
                }
                status
            }
            0x3F6 => self.ide.read_alt_status(),
            _ => super::OPEN_BUS_BYTE,
        }
    }

    /// Writes one IDE task-file register (ports 0x1F0-0x1F7 and 0x3F6).
    pub(super) fn ide_io_write(&mut self, port: u16, value: u8) {
        match port {
            0x1F0 => {
                let action = self.ide.write_data_word(u16::from(value));
                self.process_ide_action(action);
            }
            0x1F1 => self.ide.write_features(value),
            0x1F2 => self.ide.write_sector_count(value),
            0x1F3 => self.ide.write_sector_number(value),
            0x1F4 => self.ide.write_cylinder_low(value),
            0x1F5 => self.ide.write_cylinder_high(value),
            0x1F6 => self.ide.write_device_head(value),
            0x1F7 => {
                let action = self.ide.write_command(value);
                self.process_ide_action(action);
            }
            0x3F6 => self.ide.write_device_control(value),
            _ => {}
        }
    }

    /// Reads the 16-bit data register (port 0x1F0).
    pub(super) fn ide_read_data_word(&mut self) -> u16 {
        let (word, action) = self.ide.read_data_word();
        self.process_ide_action(action);
        word
    }

    /// Writes the 16-bit data register (port 0x1F0).
    pub(super) fn ide_write_data_word(&mut self, value: u16) {
        let action = self.ide.write_data_word(value);
        self.process_ide_action(action);
    }

    /// Schedules the completion event when the core requests it.
    fn process_ide_action(&mut self, action: IdeAction) {
        match action {
            IdeAction::None => {}
            IdeAction::ScheduleCompletion => {
                let cycles = (u64::from(self.clocks.cpu_clock_hz) * IDE_EXECUTION_DELAY_MICROS
                    / 1_000_000)
                    .max(1);
                self.scheduler
                    .schedule(EventAt::IdeExecution, self.current_cycle + cycles);
                self.update_next_event_cycle();
            }
        }
    }

    /// Completes the pending IDE operation and arms the interrupt event.
    pub(crate) fn handle_ide_execution(&mut self) {
        if self.ide.complete_operation() {
            let cycles = (u64::from(self.clocks.cpu_clock_hz) * IDE_INTERRUPT_DELAY_MICROS
                / 1_000_000)
                .max(1);
            self.scheduler
                .schedule(EventAt::IdeInterrupt, self.current_cycle + cycles);
            self.update_next_event_cycle();
        }
    }

    /// Raises the primary-channel interrupt.
    pub(crate) fn handle_ide_interrupt(&mut self) {
        self.raise_irq(IRQ_IDE_PRIMARY);
    }

    /// Attaches a hard disk image to `drive` and writes its user-type
    /// geometry into the CMOS.
    pub fn insert_hdd(
        &mut self,
        drive: usize,
        image: HddImage,
        path: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        if drive >= 2 {
            return Err(format!("AT IDE drive {drive} is not installed"));
        }
        if image.format != HddFormat::AtFlat {
            return Err(format!(
                "the AT machine only accepts flat .hdd images, got {}",
                image.format_name()
            ));
        }
        let geometry = image.geometry;
        self.ide.insert_drive(drive, image, path);
        set_hard_disk_user_type(&mut self.rtc.cmos, drive, &geometry);
        Ok(())
    }

    /// Flushes every attached hard disk to its backing file.
    pub fn flush_hdds(&mut self) {
        self.ide.flush();
    }

    /// Selects the BIOS boot device order in the CMOS.
    pub fn set_boot_device(&mut self, device: AtBootDevice) {
        let floppy_first = match device {
            AtBootDevice::FloppyFirst => true,
            AtBootDevice::HddFirst => false,
        };
        set_boot_sequence(&mut self.rtc.cmos, floppy_first);
    }
}
