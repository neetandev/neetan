//! AT FDC glue: ports 0x3F0-0x3F7, IRQ 6, DMA channel 2.
//!
//! Commands execute through the immediate-DMA model: the FDC action arms a
//! short execution delay, the execution event moves whole sectors through
//! DMA channel 2, and a completion event raises IRQ 6 when the DOR gate is
//! open. Seek completion is paced by the SPECIFY step rate time.

use common::{StackVec, TraceSink};
use device::{
    floppy::FloppyImage,
    upd765a_fdc::{
        DorEffect, FdcAction, FdcCommand, ST0_NOT_READY, ST1_MISSING_ADDRESS_MARK,
        ST1_NOT_WRITABLE, ST2_SCAN_EQUAL_HIT, ST2_SCAN_NOT_SATISFIED,
    },
};

use crate::{
    bus::{AtBus, IRQ_FDC},
    cmos::{
        FLOPPY_TYPE_360K, FLOPPY_TYPE_720K, FLOPPY_TYPE_1200K, FLOPPY_TYPE_1440K, FLOPPY_TYPE_NONE,
        set_floppy_drive_types,
    },
    scheduler::EventAt,
};

/// DMA channel wired to the FDC on the AT.
const FDC_DMA_CHANNEL: usize = 2;

/// Number of drive slots on the AT controller.
const FDC_DRIVE_COUNT: usize = 2;

/// Delay from command assembly to execution, in microseconds.
const FDC_EXECUTION_DELAY_MICROS: u64 = 200;

/// Delay from execution completion to the interrupt, in microseconds.
const FDC_INTERRUPT_DELAY_MICROS: u64 = 100;

/// Delay from reset release to the drive polling interrupt, in microseconds.
const FDC_RESET_POLL_MICROS: u64 = 8;

/// Maximum sector IDs accepted by one FORMAT TRACK command.
const FORMAT_TRACK_MAX_SECTORS: usize = 40;

impl<T: TraceSink> AtBus<T> {
    /// Reads one FDC register (ports 0x3F0-0x3F5 and 0x3F7).
    pub(super) fn fdc_io_read(&mut self, port: u16) -> u8 {
        match port {
            0x3F2 => self.fdc.read_dor(),
            0x3F4 => self.fdc.read_main_status(),
            0x3F5 => self.fdc.read_data(),
            0x3F7 => self.fdc.read_dir(),
            _ => super::OPEN_BUS_BYTE,
        }
    }

    /// Writes one FDC register (ports 0x3F0-0x3F5 and 0x3F7).
    pub(super) fn fdc_io_write(&mut self, port: u16, value: u8) {
        match port {
            0x3F2 => {
                let effect = self.fdc.write_dor(value);
                self.on_fdc_dor_effect(effect);
            }
            0x3F4 => {
                if self.fdc.write_dsr(value) {
                    self.schedule_fdc_reset_poll();
                }
            }
            0x3F5 => {
                let cylinders_before = self.fdc.state.drive_cylinder;
                let action = self.fdc.write_data(value);
                self.on_fdc_action(action, cylinders_before);
            }
            0x3F7 => self.fdc.write_ccr(value),
            _ => {}
        }
    }

    /// Applies the bus-side effects of a DOR write.
    fn on_fdc_dor_effect(&mut self, effect: DorEffect) {
        if effect.irq_gate_dropped {
            self.clear_irq(IRQ_FDC);
        }
        if effect.reset_started {
            self.fdc_reset_poll_pending = false;
            self.scheduler.cancel(EventAt::FdcExecution);
            self.scheduler.cancel(EventAt::FdcInterrupt);
            self.update_next_event_cycle();
        }
        if effect.reset_released {
            self.schedule_fdc_reset_poll();
        }
        // Re-opening the gate delivers an interrupt that completed while the
        // gate was closed.
        if effect.irq_gate_raised && self.fdc.irq_enabled() && self.fdc.take_interrupt_pending() {
            self.raise_irq(IRQ_FDC);
        }
    }

    /// Schedules the post-reset drive polling interrupt.
    fn schedule_fdc_reset_poll(&mut self) {
        self.fdc_reset_poll_pending = true;
        let cycles =
            (u64::from(self.clocks.cpu_clock_hz) * FDC_RESET_POLL_MICROS / 1_000_000).max(1);
        self.scheduler
            .schedule(EventAt::FdcInterrupt, self.current_cycle + cycles);
        self.update_next_event_cycle();
    }

    /// Dispatches the action returned by an assembled FDC command.
    fn on_fdc_action(&mut self, action: FdcAction, cylinders_before: [u8; 4]) {
        match action {
            FdcAction::None => {}
            FdcAction::ScheduleSeekInterrupt => self.schedule_fdc_seek_interrupt(cylinders_before),
            FdcAction::StartReadData
            | FdcAction::StartWriteData
            | FdcAction::StartReadId
            | FdcAction::StartFormatTrack
            | FdcAction::StartScan => self.schedule_fdc_execution(),
        }
    }

    /// Defers the seek/recalibrate completion by the stepped distance
    /// at the SPECIFY step rate time (milliseconds per step = 16 - SRT).
    fn schedule_fdc_seek_interrupt(&mut self, cylinders_before: [u8; 4]) {
        self.fdc.state.interrupt_pending = false;
        let drive = (self.fdc.state.params[0] & 0x03) as usize;
        self.fdc.defer_drive_status(drive);
        self.fdc.clear_disk_change_on_step(drive);
        let steps = cylinders_before[drive].abs_diff(self.fdc.state.drive_cylinder[drive]);
        let step_milliseconds = 16 - u64::from(self.fdc.state.srt & 0x0F);
        let cycles =
            u64::from(steps).max(1) * step_milliseconds * u64::from(self.clocks.cpu_clock_hz)
                / 1000;
        self.scheduler
            .schedule(EventAt::FdcInterrupt, self.current_cycle + cycles.max(1));
        self.update_next_event_cycle();
    }

    /// Schedules the command execution event.
    fn schedule_fdc_execution(&mut self) {
        let cycles =
            (u64::from(self.clocks.cpu_clock_hz) * FDC_EXECUTION_DELAY_MICROS / 1_000_000).max(1);
        self.scheduler
            .schedule(EventAt::FdcExecution, self.current_cycle + cycles);
        self.update_next_event_cycle();
    }

    /// Schedules the completion interrupt event.
    fn schedule_fdc_interrupt(&mut self) {
        let cycles =
            (u64::from(self.clocks.cpu_clock_hz) * FDC_INTERRUPT_DELAY_MICROS / 1_000_000).max(1);
        self.scheduler
            .schedule(EventAt::FdcInterrupt, self.current_cycle + cycles);
        self.update_next_event_cycle();
    }

    /// Delivers a deferred FDC interrupt: reset polling statuses, released
    /// seek statuses, and command completions.
    pub(super) fn handle_fdc_interrupt(&mut self) {
        if self.fdc_reset_poll_pending {
            self.fdc_reset_poll_pending = false;
            self.fdc.raise_reset_polling_status();
        }
        if self.fdc.has_waiting_drive_status() {
            self.fdc.release_waiting_drive_statuses();
        }
        if self.fdc.irq_enabled() && self.fdc.take_interrupt_pending() {
            self.raise_irq(IRQ_FDC);
        }
    }

    /// Executes the armed FDC command through DMA channel 2.
    pub(super) fn handle_fdc_execution(&mut self) {
        match self.fdc.state.active_command {
            FdcCommand::ReadData => self.handle_fdc_read_data(),
            FdcCommand::ReadId => self.handle_fdc_read_id(),
            FdcCommand::WriteData => self.handle_fdc_write_data(),
            FdcCommand::FormatTrack => self.handle_fdc_format_track(),
            FdcCommand::Scan => self.handle_fdc_scan(),
            FdcCommand::None => {}
        }
        self.schedule_fdc_interrupt();
    }

    fn handle_fdc_read_data(&mut self) {
        let drive = self.fdc.current_drive();
        let track_index = self.fdc.current_track_index();

        if !self.fdc.has_drive(drive) {
            self.fdc.complete_error(ST0_NOT_READY, 0x00, 0x00);
            return;
        }
        if !self.fdc.data_rate_matches(drive) {
            self.fdc
                .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00);
            return;
        }

        loop {
            let c = self.fdc.state.c;
            let h = self.fdc.state.h;
            let r = self.fdc.state.r;
            let n = self.fdc.state.n;

            let Some(data) = self
                .fdc
                .read_sector_data(drive, track_index, c, h, r, n)
                .map(<[u8]>::to_vec)
            else {
                self.fdc
                    .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00);
                break;
            };

            let dma_result = self.dma.transfer_write_to_memory(FDC_DMA_CHANNEL, &data);
            for (address, byte) in &dma_result.writes {
                self.memory.write_physical(*address, *byte);
            }

            if dma_result.terminal_count {
                self.fdc.signal_terminal_count();
                self.fdc.advance_sector();
                self.fdc.complete_success();
                break;
            }
            if self.fdc.advance_sector() {
                self.fdc.complete_success();
                break;
            }
        }
    }

    fn handle_fdc_read_id(&mut self) {
        let drive = self.fdc.current_drive();

        if !self.fdc.has_drive(drive) {
            self.fdc.complete_error(ST0_NOT_READY, 0x00, 0x00);
            return;
        }
        if !self.fdc.data_rate_matches(drive) {
            self.fdc
                .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00);
            return;
        }

        let track_index = self.fdc.current_track_index();
        let rotation = self.fdc.state.crcn as usize;
        match self.fdc.read_id_at_index(drive, track_index, rotation) {
            Some((c, h, r, n)) => {
                let count = self.fdc.sector_count(drive, track_index);
                self.fdc.provide_read_id(c, h, r, n);
                self.fdc.state.crcn = if count > 0 {
                    ((rotation + 1) % count) as u8
                } else {
                    0
                };
                self.fdc.complete_success();
            }
            None => self
                .fdc
                .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00),
        }
    }

    fn handle_fdc_write_data(&mut self) {
        let drive = self.fdc.current_drive();
        let track_index = self.fdc.current_track_index();

        if !self.fdc.has_drive(drive) {
            self.fdc.complete_error(ST0_NOT_READY, 0x00, 0x00);
            return;
        }
        if self.fdc.is_write_protected(drive) {
            self.fdc.complete_error(0x00, ST1_NOT_WRITABLE, 0x00);
            return;
        }
        if !self.fdc.data_rate_matches(drive) {
            self.fdc
                .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00);
            return;
        }

        loop {
            let c = self.fdc.state.c;
            let h = self.fdc.state.h;
            let r = self.fdc.state.r;
            let n = self.fdc.state.n;
            let sector_size = 128usize << (n as usize).min(7);

            if self
                .fdc
                .read_sector_data(drive, track_index, c, h, r, n)
                .is_none()
            {
                self.fdc
                    .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00);
                break;
            }

            let dma_result = self
                .dma
                .transfer_read_from_memory(FDC_DMA_CHANNEL, sector_size);
            let mut sector_data = Vec::with_capacity(dma_result.addresses.len());
            for &address in &dma_result.addresses {
                sector_data.push(self.memory.read_physical(address));
            }

            self.fdc
                .write_sector_data(drive, track_index, c, h, r, n, &sector_data);

            if dma_result.terminal_count {
                self.fdc.signal_terminal_count();
                self.fdc.advance_sector();
                self.fdc.complete_success();
                break;
            }
            if self.fdc.advance_sector() {
                self.fdc.complete_success();
                break;
            }
        }
    }

    fn handle_fdc_format_track(&mut self) {
        let drive = self.fdc.current_drive();
        let track_index = self.fdc.current_track_index();

        if !self.fdc.has_drive(drive) {
            self.fdc.complete_error(ST0_NOT_READY, 0x00, 0x00);
            return;
        }
        if self.fdc.is_write_protected(drive) {
            self.fdc.complete_error(0x00, ST1_NOT_WRITABLE, 0x00);
            return;
        }

        let data_n = self.fdc.state.n;
        let sector_count = self.fdc.state.eot as usize;
        let fill_byte = self.fdc.state.dtl;

        let mut chrn: StackVec<(u8, u8, u8, u8), FORMAT_TRACK_MAX_SECTORS> = StackVec::new();
        for _ in 0..sector_count.min(FORMAT_TRACK_MAX_SECTORS) {
            let dma_result = self.dma.transfer_read_from_memory(FDC_DMA_CHANNEL, 4);
            let mut identifier = [0u8; 4];
            for (index, &address) in dma_result.addresses.iter().enumerate() {
                identifier[index] = self.memory.read_physical(address);
            }
            chrn.push((identifier[0], identifier[1], identifier[2], identifier[3]));
        }

        self.fdc
            .format_track(drive, track_index, &chrn, data_n, fill_byte);
        self.fdc.complete_success();
    }

    fn handle_fdc_scan(&mut self) {
        let drive = self.fdc.current_drive();
        let track_index = self.fdc.current_track_index();

        if !self.fdc.has_drive(drive) {
            self.fdc.complete_error(ST0_NOT_READY, 0x00, 0x00);
            return;
        }
        if !self.fdc.data_rate_matches(drive) {
            self.fdc
                .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00);
            return;
        }

        loop {
            let c = self.fdc.state.c;
            let h = self.fdc.state.h;
            let r = self.fdc.state.r;
            let n = self.fdc.state.n;

            let Some(data) = self
                .fdc
                .read_sector_data(drive, track_index, c, h, r, n)
                .map(<[u8]>::to_vec)
            else {
                self.fdc
                    .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00);
                break;
            };

            self.fdc.begin_scan_sector(&data);
            let dma_result = self
                .dma
                .transfer_read_from_memory(FDC_DMA_CHANNEL, data.len());
            for &address in &dma_result.addresses {
                let host_byte = self.memory.read_physical(address);
                self.fdc.write_data(host_byte);
            }

            if self.fdc.scan_sector_satisfied() {
                let st2 = if self.fdc.is_scan_equal() {
                    ST2_SCAN_EQUAL_HIT
                } else {
                    0
                };
                self.fdc.complete_success_with_status(0, st2);
                break;
            }
            if dma_result.terminal_count {
                self.fdc.signal_terminal_count();
                self.fdc
                    .complete_success_with_status(0, ST2_SCAN_NOT_SATISFIED);
                break;
            }

            let mut ended = false;
            for _ in 0..self.fdc.scan_step() {
                if self.fdc.advance_sector() {
                    ended = true;
                    break;
                }
            }
            if ended {
                self.fdc
                    .complete_success_with_status(0, ST2_SCAN_NOT_SATISFIED);
                break;
            }
        }
    }

    /// Mounts a floppy image into `drive` and refreshes the CMOS drive types.
    pub fn insert_floppy(
        &mut self,
        drive: usize,
        image: FloppyImage,
        path: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        if drive >= FDC_DRIVE_COUNT {
            return Err(format!("AT floppy drive {drive} is not installed"));
        }
        self.fdc.insert_drive(drive, image, path);
        self.update_cmos_floppy_types();
        Ok(())
    }

    /// Ejects and flushes the floppy in `drive`.
    pub fn eject_floppy(&mut self, drive: usize) {
        self.fdc.eject_drive(drive);
        self.update_cmos_floppy_types();
    }

    /// Flushes every mounted floppy to its backing file.
    pub fn flush_floppies(&mut self) {
        self.fdc.flush_all_drives();
    }

    /// Derives the CMOS floppy drive types from the mounted media. Drive A
    /// defaults to a 1.44 MB drive so the BIOS always reports an A: drive;
    /// drive B only materializes when media is attached.
    fn update_cmos_floppy_types(&mut self) {
        let drive_a = self.floppy_cmos_type(0).unwrap_or(FLOPPY_TYPE_1440K);
        let drive_b = self.floppy_cmos_type(1).unwrap_or(FLOPPY_TYPE_NONE);
        set_floppy_drive_types(&mut self.rtc.cmos, drive_a, drive_b);
    }

    /// Maps the mounted media in `drive` to its CMOS drive type nibble.
    fn floppy_cmos_type(&self, drive: usize) -> Option<u8> {
        use device::floppy::d88::D88MediaType;

        let image = self.fdc.drive(drive)?;
        Some(match image.media_type {
            D88MediaType::Disk2D => FLOPPY_TYPE_360K,
            D88MediaType::Disk2DD => FLOPPY_TYPE_720K,
            D88MediaType::Disk2HD => {
                if image.sector_count(0) == 15 {
                    FLOPPY_TYPE_1200K
                } else {
                    FLOPPY_TYPE_1440K
                }
            }
        })
    }
}
