//! Disk sub-CPU FDC orchestration: non-DMA (PIO) byte transfers.
//!
//! Mirrors the PC-98 driver (`crates/machine_98/src/bus/fdc.rs`) but for the single
//! PC80S31K uPD765A driven by programmed I/O: the sub CPU moves each data byte
//! through port 0xFB while polling the MSR, and asserts terminal count through
//! port 0xF8. The device is a passive byte FIFO (see [`device::upd765a_fdc`]);
//! this layer resolves sectors, paces bytes at the data rate via
//! `Event88::FdcDrqByte`, and finalizes commands.

use common::{
    TraceContext, TraceDeviceEvent, TraceEvent, TraceEventKey, TraceField, TraceSink, TraceValue,
    trace_id,
};
use device::{
    floppy::D88MediaType,
    upd765a_fdc::{
        FdcAction, FdcCommand, ST0_NOT_READY, ST1_MISSING_ADDRESS_MARK, ST1_NOT_WRITABLE,
    },
};

use super::Pc8801Bus;
use crate::scheduler::Event88;

/// Delay (main-clock units) before a seek/recalibrate raises its completion
/// interrupt. Calibration item: models the head-settle latency the disk ROM
/// expects before issuing Sense Interrupt Status.
const SEEK_INTERRUPT_DELAY_CYCLES: u64 = 2000;

impl<T: TraceSink> Pc8801Bus<T> {
    /// Reads an FDC data byte (port 0xFB), then advances the PIO read sequence.
    pub(crate) fn read_fdc_data(&mut self) -> u8 {
        let value = self.fdc.read_data();
        if self.fdc.pio_active() && self.fdc.state.exec_reading {
            if self.fdc.pio_sector_done() && self.fdc.at_last_sector() {
                // The CPU consumed the final byte of the last (EOT) sector: the
                // uPD765A terminates into the result phase at once, without a
                // further DRQ tick or a host terminal-count pulse.
                self.fdc.complete_success();
            } else {
                self.schedule_drq_byte();
            }
        }
        value
    }

    /// Writes an FDC data byte (port 0xFB): a command/parameter byte, or a PIO
    /// data byte during a write/format execution phase.
    pub(crate) fn write_fdc_data(&mut self, value: u8) {
        let was_pio_write = self.fdc.pio_active() && !self.fdc.state.exec_reading;
        let action = self.fdc.write_data(value);
        if was_pio_write {
            if self.fdc.pio_sector_done() {
                self.advance_pio_write();
            } else {
                self.schedule_drq_byte();
            }
        } else {
            let context = TraceContext::sub_cpu(
                self.current_cycle,
                self.sub_cycle,
                Some(u64::from(self.sub_clock_hz())),
            );
            self.on_fdc_action(action, context);
        }
    }

    /// Dispatches the action returned by the FDC after a command is assembled.
    fn on_fdc_action(&mut self, action: FdcAction, context: TraceContext) {
        match action {
            FdcAction::None => {}
            FdcAction::ScheduleSeekInterrupt => {
                // Defer the seek/recalibrate completion interrupt by the head-settle
                // delay, then raise it for the sub CPU to service.
                self.fdc.state.interrupt_pending = false;
                self.scheduler.schedule(
                    Event88::FdcSeekComplete,
                    self.current_cycle + SEEK_INTERRUPT_DELAY_CYCLES,
                );
                self.update_next_event_cycle();
            }
            FdcAction::StartReadData => self.start_pio_read(context),
            FdcAction::StartWriteData => self.start_pio_write(),
            FdcAction::StartReadId => self.handle_read_id(),
            FdcAction::StartFormatTrack => self.start_pio_format(),
            FdcAction::StartScan => unreachable!("SCAN commands are disabled on this FDC"),
        }
    }

    /// Releases the next PIO byte slot at a data-rate DRQ tick.
    pub(crate) fn on_fdc_drq_byte(&mut self, _fire_cycle: u64) {
        if self.fdc.pio_active() && self.fdc.state.exec_reading && self.fdc.pio_sector_done() {
            self.advance_pio_read(TraceContext::scheduler_main(
                self.current_cycle,
                Some(u64::from(self.cpu_clock_hz())),
            ));
        }
        self.fdc.pio_release_byte();
    }

    /// Asserts the FDC terminal count (port 0xF8 read): ends the active PIO
    /// transfer and schedules the line to deassert after the TC pulse.
    pub(crate) fn assert_fdc_terminal_count(&mut self) {
        self.tc_active = true;
        self.fdc.signal_terminal_count();
        if self.fdc.pio_active() {
            if !self.fdc.state.exec_reading {
                self.flush_pio_write_sector();
            }
            self.fdc.complete_success();
        }
        // TC pulse width ~50 us.
        let pulse = (u64::from(self.clocks.main_clock_hz) / 20_000).max(1);
        self.scheduler
            .schedule(Event88::FdcTcClear, self.current_cycle + pulse);
        self.update_next_event_cycle();
    }

    fn schedule_drq_byte(&mut self) {
        self.scheduler.schedule(
            Event88::FdcDrqByte,
            self.current_cycle + self.drq_byte_cycles,
        );
        self.update_next_event_cycle();
    }

    /// Completes a data command issued to a drive with no disk. The PC80S31K
    /// forces the drive ready while the motor is driven, so the µPD765A does not
    /// report "not ready"; the read instead finds no address marks (ST1 MA),
    /// which N88 reads as "no bootable disk" and falls through to ROM BASIC.
    /// Only an unforced, genuinely idle drive returns "not ready".
    fn complete_no_disk(&mut self) {
        if self.fdc.forced_ready() {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
        } else {
            self.fdc.complete_error(ST0_NOT_READY, 0, 0);
        }
    }

    fn start_pio_read(&mut self, context: TraceContext) {
        let drive = self.fdc.current_drive();
        if !self.floppy.has_drive(drive) {
            self.complete_no_disk();
            return;
        }
        if !self.pc88_density_matches(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            return;
        }
        if !self.load_current_read_sector(drive, context) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            return;
        }
        self.schedule_drq_byte();
    }

    /// Loads the sector named by the current FDC C/H/R/N into the PIO FIFO.
    fn load_current_read_sector(&mut self, drive: usize, context: TraceContext) -> bool {
        let track_index = self.fdc.current_track_index();
        let (c, h, r, n) = (
            self.fdc.state.c,
            self.fdc.state.h,
            self.fdc.state.r,
            self.fdc.state.n,
        );
        match self.floppy.read_sector_data(drive, track_index, c, h, r, n) {
            Some(data) => {
                if T::ENABLED
                    && self.tracer.interested(TraceEventKey::Device {
                        device: trace_id::device::PC88_FDC,
                        action: trace_id::action::READ,
                    })
                {
                    self.tracer.trace(
                        context,
                        TraceEvent::Device(TraceDeviceEvent {
                            device: trace_id::device::PC88_FDC,
                            action: trace_id::action::READ,
                            fields: &[
                                TraceField {
                                    name: trace_id::field::DRIVE,
                                    value: TraceValue::Unsigned(drive as u64),
                                },
                                TraceField {
                                    name: trace_id::field::TRACK_INDEX,
                                    value: TraceValue::Unsigned(track_index as u64),
                                },
                                TraceField {
                                    name: trace_id::field::CYLINDER,
                                    value: TraceValue::Unsigned(u64::from(c)),
                                },
                                TraceField {
                                    name: trace_id::field::HEAD,
                                    value: TraceValue::Unsigned(u64::from(h)),
                                },
                                TraceField {
                                    name: trace_id::field::RECORD,
                                    value: TraceValue::Unsigned(u64::from(r)),
                                },
                                TraceField {
                                    name: trace_id::field::SIZE_CODE,
                                    value: TraceValue::Unsigned(u64::from(n)),
                                },
                            ],
                        }),
                    );
                }
                let data = data.to_vec();
                self.fdc.begin_pio_read(&data);
                true
            }
            None => false,
        }
    }

    /// Handles a drained read sector: continue to the next sector or finish at EOT.
    fn advance_pio_read(&mut self, context: TraceContext) {
        if self.fdc.advance_sector() {
            self.fdc.complete_success();
            return;
        }
        let drive = self.fdc.current_drive();
        if !self.load_current_read_sector(drive, context) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
        }
    }

    fn start_pio_write(&mut self) {
        let drive = self.fdc.current_drive();
        if !self.floppy.has_drive(drive) {
            self.complete_no_disk();
            return;
        }
        if self.floppy.is_write_protected(drive) {
            self.fdc.complete_error(0, ST1_NOT_WRITABLE, 0);
            return;
        }
        if !self.pc88_density_matches(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            return;
        }
        let sector_size = self.fdc_sector_size();
        self.fdc.begin_pio_write(sector_size);
        self.schedule_drq_byte();
    }

    /// Handles a full write sector: persist it, then continue or finish at EOT.
    fn advance_pio_write(&mut self) {
        self.flush_pio_write_sector();
        if self.fdc.state.active_command == FdcCommand::FormatTrack {
            return;
        }
        if self.fdc.advance_sector() {
            self.fdc.complete_success();
            return;
        }
        let sector_size = self.fdc_sector_size();
        self.fdc.begin_pio_write(sector_size);
        self.schedule_drq_byte();
    }

    /// Persists the accumulated PIO write bytes to the target sector.
    fn flush_pio_write_sector(&mut self) {
        if self.fdc.state.active_command == FdcCommand::FormatTrack {
            self.flush_pio_format();
            return;
        }
        let drive = self.fdc.current_drive();
        let track_index = self.fdc.current_track_index();
        let (c, h, r, n) = (
            self.fdc.state.c,
            self.fdc.state.h,
            self.fdc.state.r,
            self.fdc.state.n,
        );
        let data = self.fdc.take_pio_write_buf().to_vec();
        self.floppy
            .write_sector_data(drive, track_index, c, h, r, n, &data);
    }

    fn start_pio_format(&mut self) {
        let drive = self.fdc.current_drive();
        if !self.floppy.has_drive(drive) {
            self.complete_no_disk();
            return;
        }
        if self.floppy.is_write_protected(drive) {
            self.fdc.complete_error(0, ST1_NOT_WRITABLE, 0);
            return;
        }
        if !self.pc88_density_matches(drive) {
            if let Some(image) = self.floppy.drive(drive) {
                let selected = self.pc88_drive_media_type(drive);
                common::warn!(
                    "PC-88 FDC FORMAT TRACK density mismatch: drive={drive} selected={selected:?} image={:?}",
                    image.media_type
                );
            }
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            return;
        }
        // FORMAT TRACK streams four ID bytes (C, H, R, N) per sector via PIO.
        let sector_count = self.fdc.state.eot as usize;
        self.fdc.begin_pio_write(sector_count * 4);
        self.schedule_drq_byte();
    }

    /// Builds the track from the streamed ID bytes and formats it.
    fn flush_pio_format(&mut self) {
        let drive = self.fdc.current_drive();
        let track_index = self.fdc.current_track_index();
        let data_n = self.fdc.state.n;
        let fill_byte = self.fdc.state.dtl;
        let ids = self.fdc.take_pio_write_buf().to_vec();
        let mut chrn: Vec<(u8, u8, u8, u8)> = Vec::with_capacity(ids.len() / 4);
        for entry in ids.chunks_exact(4) {
            chrn.push((entry[0], entry[1], entry[2], entry[3]));
        }
        self.floppy
            .format_track(drive, track_index, &chrn, data_n, fill_byte);
        self.fdc.complete_success();
    }

    fn handle_read_id(&mut self) {
        let drive = self.fdc.current_drive();
        if !self.floppy.has_drive(drive) {
            self.complete_no_disk();
            return;
        }
        if !self.pc88_density_matches(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            return;
        }
        let track_index = self.fdc.current_track_index();
        let crcn = self.fdc.state.crcn as usize;
        match self.floppy.read_id_at_index(drive, track_index, crcn) {
            Some((c, h, r, n)) => {
                let sector_count = self.floppy.sector_count(drive, track_index);
                self.fdc.provide_read_id(c, h, r, n);
                self.fdc.state.crcn = if sector_count > 0 {
                    ((crcn + 1) % sector_count) as u8
                } else {
                    0
                };
                self.fdc.complete_success();
            }
            None => self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0),
        }
    }

    /// Sector size in bytes for the current command's N parameter.
    fn fdc_sector_size(&self) -> usize {
        128usize << (self.fdc.state.n as usize).min(7)
    }

    /// Returns the drive type selected by the PC80S31K port 0xF4 latch.
    fn pc88_drive_media_type(&self, drive: usize) -> D88MediaType {
        let two_hd_bit = if drive == 0 { 0x01 } else { 0x02 };
        let two_dd_bit = if drive == 0 { 0x04 } else { 0x08 };
        if self.drive_mode & two_hd_bit != 0 {
            D88MediaType::Disk2HD
        } else if self.drive_mode & two_dd_bit != 0 {
            D88MediaType::Disk2DD
        } else {
            D88MediaType::Disk2D
        }
    }

    /// Whether the drive-mode (port 0xF4) density expectation matches the disk.
    fn pc88_density_matches(&self, drive: usize) -> bool {
        let Some(image) = self.floppy.drive(drive) else {
            return true;
        };
        match self.pc88_drive_media_type(drive) {
            D88MediaType::Disk2D => image.media_type == D88MediaType::Disk2D,
            D88MediaType::Disk2DD => {
                image.media_type == D88MediaType::Disk2D
                    || image.media_type == D88MediaType::Disk2DD
            }
            D88MediaType::Disk2HD => image.media_type == D88MediaType::Disk2HD,
        }
    }
}

#[cfg(test)]
mod tests {
    use device::{
        floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage},
        upd765a_fdc::FdcPhase,
    };

    use super::*;
    use crate::config::{ClockSelect, Pc8801Model};

    const SECTOR_SIZE: usize = 256;
    const SIZE_CODE_256: u8 = 1;

    #[derive(Default)]
    struct DeviceContextTrace {
        contexts: Vec<TraceContext>,
    }

    impl TraceSink for DeviceContextTrace {
        fn trace(&mut self, context: TraceContext, event: TraceEvent<'_>) {
            if let TraceEvent::Device(device) = event
                && device.device == trace_id::device::PC88_FDC
                && device.action == trace_id::action::READ
            {
                self.contexts.push(context);
            }
        }
    }

    fn sector(record: u8, sector_count: u16, first_value: u8) -> D88Sector {
        D88Sector {
            cylinder: 0,
            head: 0,
            record,
            size_code: SIZE_CODE_256,
            sector_count,
            mfm_flag: 0x00,
            deleted: 0x00,
            status: 0x00,
            reserved: [0; 5],
            data: (0..SECTOR_SIZE)
                .map(|index| first_value.wrapping_add(index as u8))
                .collect(),
            source_offset: None,
        }
    }

    fn single_sector_image_with_media(media_type: D88MediaType) -> FloppyImage {
        let sector = sector(1, 1, 0);
        FloppyImage::from_d88(D88Disk::from_tracks(
            String::from("DRQ-IRQ"),
            false,
            media_type,
            vec![Some(vec![sector])],
        ))
    }

    fn single_sector_image() -> FloppyImage {
        single_sector_image_with_media(D88MediaType::Disk2D)
    }

    fn two_sector_image() -> FloppyImage {
        FloppyImage::from_d88(D88Disk::from_tracks(
            String::from("TC-BOUNDARY"),
            false,
            D88MediaType::Disk2D,
            vec![Some(vec![sector(1, 2, 0), sector(2, 2, 0x80)])],
        ))
    }

    fn read_ready_pio_byte<T: TraceSink>(bus: &mut Pc8801Bus<T>) -> u8 {
        for _ in 0..1024 {
            let event_cycle = bus.next_event_cycle().expect("event while waiting for RQM");
            bus.set_current_cycle(event_cycle);
            if bus.sub_io_read(0xFA).0 & 0x80 != 0 {
                return bus.sub_io_read(0xFB).0;
            }
        }
        panic!("FDC did not release a PIO byte");
    }

    fn write_ready_pio_byte<T: TraceSink>(bus: &mut Pc8801Bus<T>, value: u8) {
        for _ in 0..1024 {
            let event_cycle = bus.next_event_cycle().expect("event while waiting for RQM");
            bus.set_current_cycle(event_cycle);
            if bus.sub_io_read(0xFA).0 & 0x80 != 0 {
                bus.sub_io_write(0xFB, value);
                return;
            }
        }
        panic!("FDC did not release a PIO byte slot");
    }

    #[test]
    fn pio_read_drq_wakes_the_disk_sub_cpu() {
        let mut bus =
            Pc8801Bus::<common::NoTrace>::new(Pc8801Model::PC8801MC, ClockSelect::FourMhz, 48_000);
        bus.insert_floppy(0, single_sector_image(), None);
        bus.sub_io_write(0xF4, 0x00);

        bus.sub_io_write(0xFB, 0x46);
        for byte in [0x00, 0x00, 0x00, 0x01, SIZE_CODE_256, 0x01, 0x1B, 0xFF] {
            bus.sub_io_write(0xFB, byte);
        }

        assert!(!bus.sub_irq_pending());
        let drq_cycle = bus.next_event_cycle().expect("DRQ event");
        bus.set_current_cycle(drq_cycle);

        assert_ne!(bus.sub_io_read(0xFA).0 & 0x80, 0, "RQM is set");
        assert!(bus.sub_irq_pending(), "DRQ asserts the disk sub-CPU IRQ");
        bus.acknowledge_sub_irq();
        assert!(
            bus.sub_irq_pending(),
            "PIO byte-ready IRQ remains asserted until the byte is consumed"
        );
        assert_eq!(bus.sub_io_read(0xFB).0, 0x00, "first sector byte");
        assert!(
            !bus.sub_irq_pending(),
            "PIO byte-ready IRQ clears after the byte is consumed"
        );
    }

    #[test]
    fn pio_read_terminal_count_after_sector_completes_current_sector() {
        let mut bus =
            Pc8801Bus::<common::NoTrace>::new(Pc8801Model::PC8801MC, ClockSelect::FourMhz, 48_000);
        bus.insert_floppy(0, two_sector_image(), None);
        bus.sub_io_write(0xF4, 0x00);

        bus.sub_io_write(0xFB, 0x46);
        for byte in [0x00, 0x00, 0x00, 0x01, SIZE_CODE_256, 0x02, 0x1B, 0xFF] {
            bus.sub_io_write(0xFB, byte);
        }

        for expected_index in 0..SECTOR_SIZE {
            assert_eq!(read_ready_pio_byte(&mut bus), expected_index as u8);
        }
        assert_eq!(bus.fdc.state.r, 1);
        assert!(bus.fdc.pio_sector_done());

        bus.sub_io_read(0xF8);
        assert!(matches!(bus.fdc.state.phase, FdcPhase::Result));
        let result_bytes: Vec<u8> = (0..7).map(|_| bus.sub_io_read(0xFB).0).collect();
        assert_eq!(result_bytes[5], 1);
    }

    #[test]
    fn read_trace_uses_initiating_clock_context() {
        let mut bus = Pc8801Bus::new_with_trace_sink(
            Pc8801Model::PC8801MC,
            ClockSelect::FourMhz,
            48_000,
            DeviceContextTrace::default(),
        );
        bus.insert_floppy(0, two_sector_image(), None);
        bus.sub_io_write(0xF4, 0x00);
        bus.current_cycle = 37;
        bus.sub_cycle = 19;
        let main_clock_hz = u64::from(bus.cpu_clock_hz());
        let sub_clock_hz = u64::from(bus.sub_clock_hz());

        bus.sub_io_write(0xFB, 0x46);
        for byte in [0x00, 0x00, 0x00, 0x01, SIZE_CODE_256, 0x02, 0x1B, 0xFF] {
            bus.sub_io_write(0xFB, byte);
        }

        assert_eq!(
            bus.tracer().contexts,
            [TraceContext::sub_cpu(37, 19, Some(sub_clock_hz))]
        );

        for _ in 0..SECTOR_SIZE {
            read_ready_pio_byte(&mut bus);
        }
        read_ready_pio_byte(&mut bus);

        assert_eq!(bus.tracer().contexts.len(), 2);
        let scheduler_context = bus.tracer().contexts[1];
        assert_eq!(
            scheduler_context,
            TraceContext::scheduler_main(scheduler_context.tick, Some(main_clock_hz))
        );
    }

    #[test]
    fn pio_format_track_completes_after_streamed_id_bytes() {
        let mut bus =
            Pc8801Bus::<common::NoTrace>::new(Pc8801Model::PC8801MC, ClockSelect::FourMhz, 48_000);
        bus.insert_floppy(0, single_sector_image(), None);
        bus.sub_io_write(0xF4, 0x00);

        bus.sub_io_write(0xFB, 0x4D);
        for byte in [0x00, SIZE_CODE_256, 0x02, 0x1B, 0xE5] {
            bus.sub_io_write(0xFB, byte);
        }
        for byte in [
            0x00,
            0x00,
            0x01,
            SIZE_CODE_256,
            0x00,
            0x00,
            0x02,
            SIZE_CODE_256,
        ] {
            write_ready_pio_byte(&mut bus, byte);
        }

        assert!(matches!(bus.fdc.state.phase, FdcPhase::Result));
        assert!(!bus.fdc.pio_active());
        assert_eq!(bus.floppy.sector_count(0, 0), 2);
        let result_bytes: Vec<u8> = (0..7).map(|_| bus.sub_io_read(0xFB).0).collect();
        assert_eq!(result_bytes[1], 0x00);
        assert_eq!(result_bytes[2], 0x00);
    }

    #[test]
    fn pio_format_track_rejects_density_mismatch_before_streaming_ids() {
        let mut bus =
            Pc8801Bus::<common::NoTrace>::new(Pc8801Model::PC8801MC, ClockSelect::FourMhz, 48_000);
        bus.insert_floppy(
            0,
            single_sector_image_with_media(D88MediaType::Disk2HD),
            None,
        );
        bus.sub_io_write(0xF4, 0x00);

        bus.sub_io_write(0xFB, 0x4D);
        for byte in [0x00, SIZE_CODE_256, 0x02, 0x1B, 0xE5] {
            bus.sub_io_write(0xFB, byte);
        }

        assert!(matches!(bus.fdc.state.phase, FdcPhase::Result));
        assert!(!bus.fdc.pio_active());
        assert_eq!(bus.floppy.sector_count(0, 0), 1);
        assert!(!bus.floppy.is_drive_dirty(0));
        let result_bytes: Vec<u8> = (0..7).map(|_| bus.sub_io_read(0xFB).0).collect();
        assert_eq!(result_bytes[0] & 0xC0, 0x40);
        assert_eq!(
            result_bytes[1] & ST1_MISSING_ADDRESS_MARK,
            ST1_MISSING_ADDRESS_MARK
        );
    }

    #[test]
    fn pio_format_track_accepts_matching_2hd_mode() {
        let mut bus =
            Pc8801Bus::<common::NoTrace>::new(Pc8801Model::PC8801MC, ClockSelect::FourMhz, 48_000);
        bus.insert_floppy(
            0,
            single_sector_image_with_media(D88MediaType::Disk2HD),
            None,
        );
        bus.sub_io_write(0xF4, 0x01);

        bus.sub_io_write(0xFB, 0x4D);
        for byte in [0x00, SIZE_CODE_256, 0x02, 0x1B, 0xE5] {
            bus.sub_io_write(0xFB, byte);
        }
        for byte in [
            0x00,
            0x00,
            0x01,
            SIZE_CODE_256,
            0x00,
            0x00,
            0x02,
            SIZE_CODE_256,
        ] {
            write_ready_pio_byte(&mut bus, byte);
        }

        assert!(matches!(bus.fdc.state.phase, FdcPhase::Result));
        assert!(!bus.fdc.pio_active());
        assert_eq!(bus.floppy.sector_count(0, 0), 2);
        assert!(bus.floppy.is_drive_dirty(0));
        let result_bytes: Vec<u8> = (0..7).map(|_| bus.sub_io_read(0xFB).0).collect();
        assert_eq!(result_bytes[1], 0x00);
        assert_eq!(result_bytes[2], 0x00);
    }
}
