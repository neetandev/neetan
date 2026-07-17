//! MB8877 floppy controller glue for the FM-7.
//!
//! The MB8877 (WD1793 family) hangs off the main 6809 at `0xFD18-0xFD1F` and moves
//! sector data one byte at a time through the data register: the base FM-7 has no
//! DMA, so software polls the `0xFD1F` DRQ line to transfer each byte and the
//! `0xFD1F` IRQ line / `0xFD18` status for completion. Motor spin-up is deferred
//! 0.3 s and spin-down 0.05 s after the `0xFD1D` motor bit changes; the controller
//! only sees the motor once the delay elapses.

use std::path::PathBuf;

use common::TraceSink;
use device::{
    floppy::{FloppyImage, MountedFloppy},
    wd17xx_fdc::{WD17XX_PLATFORM_FM7, Wd17xxFdc},
};

use super::{Fm7Bus, OPEN_BUS};
use crate::scheduler::EventFm7;

/// `0xFD18` MB8877 status (read) and command (write) register.
const PORT_FDC_STATUS_COMMAND: u8 = 0x18;
/// `0xFD19` MB8877 track register.
const PORT_FDC_TRACK: u8 = 0x19;
/// `0xFD1A` MB8877 sector register.
const PORT_FDC_SECTOR: u8 = 0x1A;
/// `0xFD1B` MB8877 data register (PIO transfer).
const PORT_FDC_DATA: u8 = 0x1B;
/// `0xFD1C` side-select register.
const PORT_FDC_SIDE: u8 = 0x1C;
/// `0xFD1D` drive-select and motor register.
const PORT_FDC_DRIVE_MOTOR: u8 = 0x1D;
/// `0xFD1E` register present only on the AV40 (out of scope); reads open bus.
const PORT_FDC_UNUSED: u8 = 0x1E;
/// `0xFD1F` DRQ/IRQ status register (read-only).
const PORT_FDC_DRQ_IRQ: u8 = 0x1F;

/// `0xFD1C` write bit selecting the head/side.
const SIDE_SELECT_MASK: u8 = 0x01;
/// `0xFD1C` read base with only the side bit meaningful; upper bits read as one.
const SIDE_READBACK_BASE: u8 = 0xFE;

/// `0xFD1D` write bits selecting the active drive.
const DRIVE_SELECT_MASK: u8 = 0x03;
/// `0xFD1D` write bit requesting the motor to spin.
const MOTOR_REQUEST_BIT: u8 = 0x80;
/// `0xFD1D` read base with the drive-select and motor bits overlaid.
const DRIVE_MOTOR_READBACK_BASE: u8 = 0x3C;
/// `0xFD1D` read bit reporting the motor as spinning on a fitted drive.
const MOTOR_READY_BIT: u8 = 0x80;

/// `0xFD1F` read base; the low six bits always read as one.
const DRQ_IRQ_READBACK_BASE: u8 = 0x3F;
/// `0xFD1F` read bit reporting a pending data request (DRQ).
const DRQ_STATUS_BIT: u8 = 0x80;
/// `0xFD1F` read bit reporting a pending controller interrupt (IRQ).
const IRQ_STATUS_BIT: u8 = 0x40;

/// Motor spin-up delay before the controller sees the motor, in microseconds.
const MOTOR_SPIN_UP_MICROS: u64 = 300_000;
/// Motor spin-down delay after the motor request clears, in microseconds.
const MOTOR_SPIN_DOWN_MICROS: u64 = 50_000;

/// Builds the MB8877 controller with the FM-7 wiring.
pub(super) fn new_fdc(cpu_clock_hz: u32) -> Wd17xxFdc<WD17XX_PLATFORM_FM7> {
    let mut fdc = Wd17xxFdc::new(cpu_clock_hz);
    // The real WD1793 always asserts its interrupt line on command completion;
    // the FM-7 gates delivery to the CPU with the `0xFD02` bit 4 mask instead. So
    // the chip output stays enabled and `0xFD1F` bit 6 reports the raw line.
    fdc.set_irq_enable(true);
    fdc
}

impl<T: TraceSink> Fm7Bus<T> {
    /// Reads an FDC port (`0xFD18-0xFD1F`).
    pub(crate) fn fdc_read(&mut self, port: u8) -> u8 {
        let now = self.current_cycle();
        match port {
            PORT_FDC_STATUS_COMMAND => {
                let value = self.fdc.read_status(now);
                self.sync_fdc_interrupt();
                value
            }
            PORT_FDC_TRACK => self.fdc.read_track_register(),
            PORT_FDC_SECTOR => self.fdc.read_sector_register(),
            PORT_FDC_DATA => {
                let value = self.fdc.read_data_pio(now);
                self.sync_fdc_schedule();
                self.sync_fdc_interrupt();
                value
            }
            PORT_FDC_SIDE => SIDE_READBACK_BASE | self.fdc_side,
            PORT_FDC_DRIVE_MOTOR => self.fdc_drive_motor_readback(),
            PORT_FDC_UNUSED => OPEN_BUS,
            PORT_FDC_DRQ_IRQ => {
                let mut value = DRQ_IRQ_READBACK_BASE;
                if self.fdc.drq_line(now) {
                    value |= DRQ_STATUS_BIT;
                }
                if self.fdc.irq_line() {
                    value |= IRQ_STATUS_BIT;
                }
                value
            }
            _ => OPEN_BUS,
        }
    }

    /// Writes an FDC port (`0xFD18-0xFD1F`).
    pub(crate) fn fdc_write(&mut self, port: u8, value: u8) {
        let now = self.current_cycle();
        match port {
            PORT_FDC_STATUS_COMMAND => {
                self.fdc.write_command(value, now);
                self.sync_fdc_schedule();
                self.sync_fdc_interrupt();
            }
            PORT_FDC_TRACK => self.fdc.write_track_register(value),
            PORT_FDC_SECTOR => self.fdc.write_sector_register(value),
            PORT_FDC_DATA => {
                self.fdc.write_data_pio(value, now);
                self.sync_fdc_schedule();
                self.sync_fdc_interrupt();
            }
            PORT_FDC_SIDE => {
                self.fdc_side = value & SIDE_SELECT_MASK;
                self.fdc.set_side(self.fdc_side);
            }
            PORT_FDC_DRIVE_MOTOR => {
                self.fdc_drive_select = value & DRIVE_SELECT_MASK;
                self.fdc.select_drive(usize::from(self.fdc_drive_select));
                self.request_fdc_motor(value & MOTOR_REQUEST_BIT != 0);
            }
            PORT_FDC_UNUSED | PORT_FDC_DRQ_IRQ => {}
            _ => {}
        }
    }

    /// The `0xFD1D` readback: drive-select bits over the fixed base, plus the motor
    /// bit when the motor is requested and the selected drive is fitted. The bit
    /// reflects the written control latch immediately; the boot ROM checks it a
    /// few microseconds after switching the motor on and performs the physical
    /// spin-up wait itself.
    fn fdc_drive_motor_readback(&self) -> u8 {
        let mut value = DRIVE_MOTOR_READBACK_BASE | self.fdc_drive_select;
        if self.fdc_motor_requested && self.fdc_drive_select < self.model().drive_count() {
            value |= MOTOR_READY_BIT;
        }
        value
    }

    /// Requests the motor on or off, scheduling the spin-up/spin-down delay after
    /// which the controller sees the change. A new request cancels any pending
    /// transition so rapid toggles do not stack.
    fn request_fdc_motor(&mut self, requested: bool) {
        if requested == self.fdc_motor_requested {
            return;
        }
        self.fdc_motor_requested = requested;
        self.scheduler.cancel(EventFm7::FdcMotorOn);
        self.scheduler.cancel(EventFm7::FdcMotorOff);
        if requested {
            let delay = self.micros_to_main_cycles(MOTOR_SPIN_UP_MICROS);
            self.scheduler
                .schedule(EventFm7::FdcMotorOn, self.current_cycle() + delay);
        } else {
            let delay = self.micros_to_main_cycles(MOTOR_SPIN_DOWN_MICROS);
            self.scheduler
                .schedule(EventFm7::FdcMotorOff, self.current_cycle() + delay);
        }
    }

    /// Handles the motor spin-up event: the controller now sees the motor.
    pub(crate) fn on_fdc_motor_on(&mut self) {
        self.fdc_motor_on = true;
        self.fdc.set_motor(true);
    }

    /// Handles the motor spin-down event: the controller now sees the motor stop.
    pub(crate) fn on_fdc_motor_off(&mut self) {
        self.fdc_motor_on = false;
        self.fdc.set_motor(false);
    }

    /// Schedules (or cancels) the controller's next command-completion task.
    pub(crate) fn sync_fdc_schedule(&mut self) {
        match self.fdc.next_task_cycle() {
            Some(cycle) => self.scheduler.schedule(EventFm7::FdcSeekComplete, cycle),
            None => self.scheduler.cancel(EventFm7::FdcSeekComplete),
        }
    }

    /// Runs the controller's scheduled command task (seek settle, sector fetch, or
    /// the next record of a multi-sector transfer). Transfer commands stage their
    /// bytes behind the DRQ line for the CPU-polled data register to drain.
    pub(crate) fn on_fdc_seek_complete(&mut self, now: u64) {
        let outcome = self.fdc.run_task(now);
        debug_assert!(outcome.dma_read.is_none() && outcome.dma_write_len.is_none());
        self.sync_fdc_schedule();
        self.sync_fdc_interrupt();
    }

    /// Mirrors the controller's interrupt line into the main IRQ controller, where
    /// the `0xFD02` bit 4 mask gates delivery to the CPU.
    fn sync_fdc_interrupt(&mut self) {
        let line = self.fdc.irq_line();
        self.interrupts.set_fdc_pending(
            line,
            common::TraceContext::main_cpu(
                self.current_cycle,
                Some(u64::from(self.cpu_clock_hz())),
            ),
            &mut self.tracer,
        );
    }

    /// Mounts a floppy image into `drive`, remembering its backing path.
    pub fn insert_floppy(&mut self, drive: usize, image: FloppyImage, path: PathBuf) {
        self.fdc
            .insert(drive, MountedFloppy::new(image, Some(path)));
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
