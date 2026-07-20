//! uPD72065 FDC and drive-control glue: registers, media, and DMA pacing.
//!
//! The FDC data register sits behind DMAC channel 0: each media byte raises
//! one external transfer request at the selected data rate (16 us per byte at
//! 500 kbps), and the DMAC moves it between the data register and memory as a
//! bus-master cycle. Sector lookups, drive readiness, the IOC FDC/FDD
//! interrupt latches, and DIM/XDF/D88 media mounting all live here.

use common::{
    TraceContext, TraceDeviceEvent, TraceEvent, TraceEventKey, TraceField, TraceSink, TraceValue,
    trace_id,
};
use device::{
    floppy::{FloppyImage, MountedFloppy},
    upd765a_fdc::{
        FdcAction, FdcCommand, FdcPhase, ST0_NOT_READY, ST1_MISSING_ADDRESS_MARK, ST1_NOT_WRITABLE,
        ST2_SCAN_EQUAL_HIT, ST2_SCAN_NOT_SATISFIED,
    },
};

use super::X68kBus;
use crate::{IocSource, scheduler::EventX68k};

/// Highest drive unit that accepts media (the X68000 has two internal drives).
const INSERTABLE_DRIVES: usize = 2;

impl<T: TraceSink> X68kBus<T> {
    /// Reads one odd-address FDC block register.
    pub(super) fn read_fdc_register(&mut self, address: u32) -> u8 {
        match address & 7 {
            1 => self.fdc.read_status(),
            3 => {
                let value = self.fdc.read_data();
                self.after_fdc_data_access();
                value
            }
            5 => self.fdd.read_status(),
            _ => 0xFF,
        }
    }

    /// Writes one odd-address FDC block register.
    pub(super) fn write_fdc_register(&mut self, address: u32, value: u8) {
        match address & 7 {
            1 => {
                self.fdc.write_auxiliary_command(value);
                self.sync_fdc_interrupt();
            }
            3 => {
                let executing = self.fdc.execution_active();
                if !executing && self.fdc.state.phase == FdcPhase::Idle {
                    self.interrupts.ioc.clear(IocSource::Fdc);
                }
                let cylinders_before = self.fdc.state.drive_cylinder;
                let action = self.fdc.write_data(value);
                if executing {
                    self.after_fdc_data_access();
                } else {
                    self.on_fdc_action(action, cylinders_before);
                }
            }
            5 => {
                let effects = self.fdd.write_control(value);
                if effects.clear_fdd_interrupt {
                    self.interrupts.ioc.clear(IocSource::Fdd);
                }
                self.process_fdd_ejects(effects.eject_request_mask);
            }
            7 => {
                let effects = self.fdd.write_select(value);
                if effects.clear_fdd_interrupt {
                    self.interrupts.ioc.clear(IocSource::Fdd);
                }
                if effects.motor_changed {
                    self.sync_fdc_ready_lines();
                }
            }
            _ => {}
        }
    }

    /// Dispatches the action returned by an assembled FDC command.
    fn on_fdc_action(&mut self, action: FdcAction, cylinders_before: [u8; 4]) {
        match action {
            FdcAction::None => {}
            FdcAction::ScheduleSeekInterrupt => self.schedule_fdc_seek_interrupt(cylinders_before),
            FdcAction::StartReadData => self.start_fdc_read(),
            FdcAction::StartWriteData => self.start_fdc_write(),
            FdcAction::StartReadId => self.handle_fdc_read_id(),
            FdcAction::StartFormatTrack => self.start_fdc_format(),
            FdcAction::StartScan => self.start_fdc_scan(),
        }
        self.sync_fdc_interrupt();
    }

    /// Defers the seek/recalibrate completion by the stepped distance.
    fn schedule_fdc_seek_interrupt(&mut self, cylinders_before: [u8; 4]) {
        self.fdc.state.interrupt_pending = false;
        let drive = (self.fdc.state.params[0] & 0x03) as usize;
        self.fdc.defer_drive_status(drive);
        let steps = cylinders_before[drive].abs_diff(self.fdc.state.drive_cylinder[drive]);
        let step_milliseconds = 16 - u64::from(self.fdc.state.srt & 0x0F);
        let cycles = u64::from(steps).max(1) * step_milliseconds * self.cpu_clock_hz / 1000;
        self.scheduler
            .schedule(EventX68k::FdcInterrupt, self.current_cycle + cycles.max(1));
    }

    /// Raises the deferred seek/recalibrate completion interrupt.
    pub(super) fn on_fdc_seek_complete(&mut self) {
        if self.fdc.state.phase == FdcPhase::Idle {
            self.fdc.release_waiting_drive_statuses();
            self.interrupts.ioc.signal(IocSource::Fdc);
        }
    }

    /// Serves one data-rate DRQ tick: one DMAC channel-0 external request.
    pub(super) fn on_fdc_drq(&mut self) {
        if !self.fdc.execution_active() {
            return;
        }
        if self.dmac.channel_active(0) {
            self.assert_fdc_dmac_request();
        }
        if self.fdc.execution_active() {
            self.schedule_fdc_byte_event();
        }
    }

    /// Ends the active transfer when DMAC channel 0 exhausts its count.
    pub(super) fn on_fdc_terminal_count(&mut self) {
        self.fdc.signal_terminal_count();
        if !self.fdc.execution_active() {
            return;
        }
        match self.fdc.state.active_command {
            FdcCommand::ReadData | FdcCommand::ReadId | FdcCommand::None => {
                self.fdc.complete_success();
            }
            FdcCommand::WriteData => {
                if self.fdc.state.exec_index > 0 {
                    self.flush_fdc_write_sector();
                }
                self.fdc.complete_success();
            }
            FdcCommand::FormatTrack => self.flush_fdc_format(),
            FdcCommand::Scan => self.complete_fdc_scan_verdict(),
        }
        self.sync_fdc_interrupt();
    }

    /// Handles execution-phase bookkeeping after a data-register access.
    fn after_fdc_data_access(&mut self) {
        if self.fdc.execution_active() && self.fdc.execution_sector_done() {
            match self.fdc.state.active_command {
                FdcCommand::ReadData => self.advance_fdc_read(),
                FdcCommand::WriteData => self.advance_fdc_write(),
                FdcCommand::FormatTrack => self.flush_fdc_format(),
                FdcCommand::Scan => self.advance_fdc_scan(),
                FdcCommand::ReadId | FdcCommand::None => {}
            }
        }
        self.sync_fdc_interrupt();
        if self.fdc.state.phase == FdcPhase::Idle && self.fdc.has_waiting_drive_status() {
            self.scheduler.schedule(
                EventX68k::FdcInterrupt,
                self.current_cycle + self.cpu_clock_hz / 10_000,
            );
        }
    }

    /// Handles a drained read sector: continue to the next or finish at EOT.
    fn advance_fdc_read(&mut self) {
        if self.fdc.state.tc {
            self.fdc.complete_success();
            return;
        }
        if self.fdc.advance_sector() {
            self.fdc.complete_success();
            return;
        }
        let drive = self.fdc.current_drive();
        if !self.load_fdc_read_sector(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
        }
    }

    /// Handles a filled write sector: persist it, then continue or finish.
    fn advance_fdc_write(&mut self) {
        self.flush_fdc_write_sector();
        if self.fdc.state.tc || self.fdc.advance_sector() {
            self.fdc.complete_success();
            return;
        }
        let length = self.fdc_sector_size();
        self.fdc.begin_execution_write(length);
    }

    /// Handles a fully compared scan sector: verdict, step, or continue.
    fn advance_fdc_scan(&mut self) {
        if self.fdc.scan_sector_satisfied() {
            self.complete_fdc_scan_verdict();
            return;
        }
        for _ in 0..self.fdc.scan_step() {
            if self.fdc.advance_sector() {
                self.fdc
                    .complete_success_with_status(0, ST2_SCAN_NOT_SATISFIED);
                return;
            }
        }
        let drive = self.fdc.current_drive();
        if !self.load_fdc_scan_sector(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
        }
    }

    /// Completes a SCAN with the current sector's verdict.
    fn complete_fdc_scan_verdict(&mut self) {
        let satisfied = self.fdc.scan_sector_satisfied() && self.fdc.execution_sector_done();
        if satisfied {
            let st2 = if self.fdc.is_scan_equal() {
                ST2_SCAN_EQUAL_HIT
            } else {
                0
            };
            self.fdc.complete_success_with_status(0, st2);
        } else {
            self.fdc
                .complete_success_with_status(0, ST2_SCAN_NOT_SATISFIED);
        }
    }

    /// Starts a READ DATA transfer through the DMA byte cadence.
    /// Emits an FDC read device trace event for the active command sector.
    fn trace_fdc_read(&mut self, drive: usize) {
        if !T::ENABLED
            || !self.tracer.interested(TraceEventKey::Device {
                device: trace_id::device::X68K_FDC,
                action: trace_id::action::READ,
            })
        {
            return;
        }
        let track_index = self.fdc.current_track_index();
        let state = &self.fdc.state;
        self.tracer.trace(
            TraceContext::main_cpu(self.current_cycle, Some(self.cpu_clock_hz)),
            TraceEvent::Device(TraceDeviceEvent {
                device: trace_id::device::X68K_FDC,
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
                        value: TraceValue::Unsigned(u64::from(state.c)),
                    },
                    TraceField {
                        name: trace_id::field::HEAD,
                        value: TraceValue::Unsigned(u64::from(state.h)),
                    },
                    TraceField {
                        name: trace_id::field::RECORD,
                        value: TraceValue::Unsigned(u64::from(state.r)),
                    },
                    TraceField {
                        name: trace_id::field::SIZE_CODE,
                        value: TraceValue::Unsigned(u64::from(state.n)),
                    },
                ],
            }),
        );
    }

    fn start_fdc_read(&mut self) {
        let drive = self.fdc.current_drive();
        self.trace_fdc_read(drive);
        if !self.fdc_drive_ready(drive) {
            self.fdc.complete_error(ST0_NOT_READY, 0, 0);
            return;
        }
        if !self.load_fdc_read_sector(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            return;
        }
        self.schedule_fdc_byte_event();
    }

    /// Starts a WRITE DATA transfer through the DMA byte cadence.
    fn start_fdc_write(&mut self) {
        let drive = self.fdc.current_drive();
        if !self.fdc_drive_ready(drive) {
            self.fdc.complete_error(ST0_NOT_READY, 0, 0);
            return;
        }
        if self.fdc_drive_write_protected(drive) {
            self.fdc.complete_error(0, ST1_NOT_WRITABLE, 0);
            return;
        }
        let length = self.fdc_sector_size();
        self.fdc.begin_execution_write(length);
        self.schedule_fdc_byte_event();
    }

    /// Starts a FORMAT TRACK: collects four ID bytes per sector by DMA.
    fn start_fdc_format(&mut self) {
        let drive = self.fdc.current_drive();
        if !self.fdc_drive_ready(drive) {
            self.fdc.complete_error(ST0_NOT_READY, 0, 0);
            return;
        }
        if self.fdc_drive_write_protected(drive) {
            self.fdc.complete_error(0, ST1_NOT_WRITABLE, 0);
            return;
        }
        let sector_count = self.fdc.state.eot as usize;
        self.fdc.begin_execution_write(sector_count * 4);
        self.schedule_fdc_byte_event();
    }

    /// Starts a SCAN comparison through the DMA byte cadence.
    fn start_fdc_scan(&mut self) {
        let drive = self.fdc.current_drive();
        if !self.fdc_drive_ready(drive) {
            self.fdc.complete_error(ST0_NOT_READY, 0, 0);
            return;
        }
        if !self.load_fdc_scan_sector(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            return;
        }
        self.schedule_fdc_byte_event();
    }

    /// Serves a READ ID from the current rotational position.
    fn handle_fdc_read_id(&mut self) {
        let drive = self.fdc.current_drive();
        if !self.fdc_drive_ready(drive) {
            self.fdc.complete_error(ST0_NOT_READY, 0, 0);
            return;
        }
        let track_index = self.fdc.current_track_index();
        let rotation = self.fdc.state.crcn as usize;
        let identifier = self.floppy_drives[drive].as_ref().and_then(|mounted| {
            mounted
                .image()
                .sector_at_index(track_index, rotation)
                .map(|sector| {
                    (
                        sector.cylinder,
                        sector.head,
                        sector.record,
                        sector.size_code,
                    )
                })
        });
        match identifier {
            Some((c, h, r, n)) => {
                let count = self.floppy_drives[drive]
                    .as_ref()
                    .map_or(0, |mounted| mounted.image().sector_count(track_index));
                self.fdc.provide_read_id(c, h, r, n);
                self.fdc.state.crcn = if count > 0 {
                    ((rotation + 1) % count) as u8
                } else {
                    0
                };
                self.fdc.complete_success();
            }
            None => self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0),
        }
    }

    /// Schedules the next data-rate byte request.
    fn schedule_fdc_byte_event(&mut self) {
        let cycles = (self.cpu_clock_hz * 8)
            .div_ceil(u64::from(self.fdd.data_rate_hz()))
            .max(1);
        let effective = self
            .current_cycle
            .wrapping_add(self.wait_cycles.max(0) as u64);
        self.scheduler.schedule(EventX68k::Fdc, effective + cycles);
    }

    /// Loads the sector named by C/H/R/N into the read FIFO.
    fn load_fdc_read_sector(&mut self, drive: usize) -> bool {
        let Some(data) = self.fdc_sector_data(drive) else {
            return false;
        };
        self.fdc.begin_execution_read(&data);
        true
    }

    /// Loads the sector named by C/H/R/N into the scan comparator.
    fn load_fdc_scan_sector(&mut self, drive: usize) -> bool {
        let Some(data) = self.fdc_sector_data(drive) else {
            return false;
        };
        self.fdc.begin_scan_sector(&data);
        true
    }

    /// Returns a copy of the sector named by the current command.
    fn fdc_sector_data(&self, drive: usize) -> Option<Vec<u8>> {
        let mounted = self.floppy_drives.get(drive)?.as_ref()?;
        let track_index = self.fdc.current_track_index();
        let state = &self.fdc.state;
        let sector = mounted.image().find_sector_near_track_index(
            track_index,
            state.c,
            state.h,
            state.r,
            state.n,
        )?;
        Some(sector.data.clone())
    }

    /// Persists the accumulated write FIFO to the target sector.
    fn flush_fdc_write_sector(&mut self) {
        let drive = self.fdc.current_drive();
        let track_index = self.fdc.current_track_index();
        let state = &self.fdc.state;
        let (c, h, r, n) = (state.c, state.h, state.r, state.n);
        let data = self.fdc.execution_write_buf().to_vec();
        if let Some(mounted) = self.floppy_drives[drive].as_mut() {
            mounted.write_sector_data(track_index, c, h, r, n, &data);
        }
    }

    /// Builds the track from the streamed ID bytes and formats it.
    fn flush_fdc_format(&mut self) {
        let drive = self.fdc.current_drive();
        let track_index = self.fdc.current_track_index();
        let data_n = self.fdc.state.n;
        let fill_byte = self.fdc.state.dtl;
        let received = self.fdc.state.exec_index & !3;
        let identifiers = self.fdc.execution_write_buf()[..received].to_vec();
        let chrn: Vec<(u8, u8, u8, u8)> = identifiers
            .chunks_exact(4)
            .map(|entry| (entry[0], entry[1], entry[2], entry[3]))
            .collect();
        if !chrn.is_empty()
            && let Some(mounted) = self.floppy_drives[drive].as_mut()
        {
            mounted.format_track(track_index, &chrn, data_n, fill_byte);
        }
        self.fdc.complete_success();
    }

    /// Returns the sector size selected by the command's N parameter.
    fn fdc_sector_size(&self) -> usize {
        128usize << (self.fdc.state.n as usize).min(7)
    }

    /// Returns whether the drive is ready (media inserted, motor running).
    fn fdc_drive_ready(&self, drive: usize) -> bool {
        self.fdc_forced_ready || self.fdd.ready_mask() & (1 << drive) != 0
    }

    /// Applies the OPM CT forced-ready control to the FDC ready lines.
    pub(super) fn set_fdc_forced_ready(&mut self, forced: bool) {
        if self.fdc_forced_ready == forced {
            return;
        }
        self.fdc_forced_ready = forced;
        self.sync_fdc_ready_lines();
    }

    /// Returns whether the mounted media is write-protected.
    fn fdc_drive_write_protected(&self, drive: usize) -> bool {
        self.floppy_drives
            .get(drive)
            .and_then(Option::as_ref)
            .is_some_and(|mounted| mounted.image().write_protected)
    }

    /// Forwards a completed-command edge to the IOC FDC signal.
    fn sync_fdc_interrupt(&mut self) {
        if self.fdc.state.sense_interrupt_result {
            self.interrupts.ioc.clear(IocSource::Fdc);
            return;
        }
        if self.fdc.take_interrupt_pending() {
            self.interrupts.ioc.signal(IocSource::Fdc);
        } else if self.fdc.state.phase == FdcPhase::Idle {
            self.interrupts.ioc.clear(IocSource::Fdc);
        }
    }

    /// Latches an IOC FDD interrupt for a media insert or eject.
    fn sync_fdd_interrupt(&mut self) {
        if self.fdd.take_status_changed() {
            self.interrupts.ioc.signal(IocSource::Fdd);
        }
    }

    /// Propagates drive readiness and write protection into the FDC.
    pub(super) fn sync_fdc_ready_lines(&mut self) {
        let ready_mask = if self.fdc_forced_ready {
            0x0F
        } else {
            self.fdd.ready_mask()
        };
        self.fdc.set_drive_ready_mask(ready_mask);
        let mut protected_mask = 0u8;
        for (index, drive) in self.floppy_drives.iter().enumerate() {
            if drive
                .as_ref()
                .is_some_and(|mounted| mounted.image().write_protected)
            {
                protected_mask |= 1 << index;
            }
        }
        self.fdc.set_drive_write_protected_mask(protected_mask);
    }

    /// Ejects every drive requested by a drive option control write.
    fn process_fdd_ejects(&mut self, mask: u8) {
        if mask == 0 {
            return;
        }
        for drive in 0..self.floppy_drives.len() {
            if mask & (1 << drive) == 0 {
                continue;
            }
            if let Some(mounted) = self.floppy_drives[drive].take() {
                mounted.eject();
            }
            self.fdd.set_inserted(drive, false);
        }
        self.sync_fdc_ready_lines();
        self.sync_fdd_interrupt();
    }

    /// Mounts a floppy image into `drive`, remembering its backing path.
    pub fn insert_floppy(
        &mut self,
        drive: usize,
        image: FloppyImage,
        path: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        self.insert_floppy_backed(drive, image, path.into())
    }

    /// Mounts a floppy image into `drive` with the requested backing.
    pub fn insert_floppy_backed(
        &mut self,
        drive: usize,
        image: FloppyImage,
        backing: common::MediaBacking,
    ) -> Result<(), String> {
        if drive >= INSERTABLE_DRIVES {
            return Err(format!("X68000 drive {drive} is not installed"));
        }
        if let Some(previous) = self.floppy_drives[drive].take() {
            previous.eject();
        }
        self.floppy_drives[drive] = Some(device::floppy::mounted_from_backing(image, backing));
        self.fdd.set_inserted(drive, true);
        self.sync_fdc_ready_lines();
        self.sync_fdd_interrupt();
        Ok(())
    }

    /// Returns the current in-memory bytes of the floppy in `drive`, if mounted.
    pub fn floppy_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.floppy_drives
            .get(drive)?
            .as_ref()
            .map(MountedFloppy::image_bytes)
    }

    /// Ejects and flushes the floppy in `drive`.
    pub fn eject_floppy(&mut self, drive: usize) {
        let Some(mounted) = self.floppy_drives.get_mut(drive).and_then(Option::take) else {
            return;
        };
        mounted.eject();
        self.fdd.set_inserted(drive, false);
        self.sync_fdc_ready_lines();
        self.sync_fdd_interrupt();
    }

    /// Flushes every mounted floppy to its backing file.
    pub fn flush_floppies(&mut self) {
        for drive in self.floppy_drives.iter_mut().flatten() {
            drive.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, CpuMode, M68000AccessSize, M68000FunctionCode};
    use device::floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage};

    use crate::{
        X68kMachine, X68kModel,
        bus::{
            X68kBus, X68kRegion,
            test_support::{access, bus, test_roms},
        },
    };

    /// Sector size in bytes read and written by the boot-chain test.
    const SECTOR_BYTES: usize = 1024;
    /// Size code (128 << N) for a 1024-byte sector.
    const SECTOR_SIZE_CODE: u8 = 3;

    /// Writes one byte register through the supervisor bus.
    fn write_register(bus: &mut X68kBus, address: u32, value: u8) {
        bus.m68000_write(
            access(
                address,
                M68000AccessSize::Byte,
                M68000FunctionCode::SupervisorData,
            ),
            u16::from(value),
        )
        .expect("register write must not raise a CPU bus error");
    }

    /// Reads one byte register through the supervisor bus.
    fn read_register(bus: &mut X68kBus, address: u32) -> u8 {
        bus.m68000_read(access(
            address,
            M68000AccessSize::Byte,
            M68000FunctionCode::SupervisorData,
        ))
        .expect("register read must not raise a CPU bus error") as u8
    }

    /// Builds a single-sector 2HD disk whose sector holds a recognizable ramp.
    fn ramp_disk() -> FloppyImage {
        let sector = D88Sector {
            cylinder: 0,
            head: 0,
            record: 1,
            size_code: SECTOR_SIZE_CODE,
            sector_count: 1,
            mfm_flag: 0x40,
            deleted: 0x00,
            status: 0x00,
            reserved: [0; 5],
            data: (0..SECTOR_BYTES).map(|index| index as u8).collect(),
            source_offset: None,
        };
        FloppyImage::from_d88(D88Disk::from_tracks(
            String::from("RAMP"),
            false,
            D88MediaType::Disk2HD,
            vec![Some(vec![sector])],
        ))
    }

    /// Builds a two-sector 2HD disk with distinct byte patterns.
    fn two_sector_disk() -> FloppyImage {
        let sectors = (1..=2)
            .map(|record| D88Sector {
                cylinder: 0,
                head: 0,
                record,
                size_code: SECTOR_SIZE_CODE,
                sector_count: 2,
                mfm_flag: 0x40,
                deleted: 0x00,
                status: 0x00,
                reserved: [0; 5],
                data: (0..SECTOR_BYTES)
                    .map(|index| (index as u8).wrapping_add(record.wrapping_mul(0x40)))
                    .collect(),
                source_offset: None,
            })
            .collect();
        FloppyImage::from_d88(D88Disk::from_tracks(
            String::from("TWO SECTORS"),
            false,
            D88MediaType::Disk2HD,
            vec![Some(sectors)],
        ))
    }

    #[test]
    fn decoder_places_the_fdc_window() {
        assert_eq!(
            X68kBus::<common::NoTrace>::decode_region(0xE93FFF),
            X68kRegion::Adpcm
        );
        assert_eq!(
            X68kBus::<common::NoTrace>::decode_region(0xE94000),
            X68kRegion::Fdc
        );
        assert_eq!(
            X68kBus::<common::NoTrace>::decode_region(0xE95FFF),
            X68kRegion::Fdc
        );
        assert_eq!(
            X68kBus::<common::NoTrace>::decode_region(0xE96000),
            X68kRegion::StorageController
        );
    }

    #[test]
    fn even_bytes_read_ff_and_registers_mirror_every_eight_bytes() {
        let mut bus = bus(X68kModel::X68000);
        // Even byte reads are open bus.
        assert_eq!(read_register(&mut bus, 0xE94000), 0xFF);
        assert_eq!(read_register(&mut bus, 0xE94002), 0xFF);
        // The MSR reports RQM ready at reset, mirrored across the window.
        assert_eq!(read_register(&mut bus, 0xE94001), 0x80);
        assert_eq!(read_register(&mut bus, 0xE94009), 0x80);
        assert_eq!(read_register(&mut bus, 0xE95001), 0x80);
    }

    #[test]
    fn opm_ct_output_forces_the_drive_ready_line() {
        let mut bus = bus(X68kModel::X68000);
        let sense_device_status = |bus: &mut X68kBus| {
            write_register(bus, 0xE94003, 0x04);
            write_register(bus, 0xE94003, 0x00);
            read_register(bus, 0xE94003)
        };
        assert_eq!(sense_device_status(&mut bus) & 0x20, 0);

        write_register(&mut bus, 0xE90001, 0x1B);
        write_register(&mut bus, 0xE90003, 0x40);
        assert_ne!(sense_device_status(&mut bus) & 0x20, 0);

        write_register(&mut bus, 0xE90001, 0x1B);
        write_register(&mut bus, 0xE90003, 0x00);
        assert_eq!(sense_device_status(&mut bus) & 0x20, 0);
    }

    #[test]
    fn drive_control_and_select_clear_the_fdd_interrupt() {
        let mut bus = bus(X68kModel::X68000);
        bus.signal_ioc_interrupt(crate::IocSource::Fdd);
        assert_ne!(read_register(&mut bus, 0xE9C001) & 0x40, 0);
        write_register(&mut bus, 0xE94005, 0x00);
        assert_eq!(read_register(&mut bus, 0xE9C001) & 0x40, 0);

        bus.signal_ioc_interrupt(crate::IocSource::Fdd);
        write_register(&mut bus, 0xE94007, 0x80);
        assert_eq!(read_register(&mut bus, 0xE9C001) & 0x40, 0);
    }

    #[test]
    fn insert_and_eject_latch_the_fdd_interrupt_with_subvector_one() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE9C001, 0x0F);
        write_register(&mut bus, 0xE9C003, 0x40);

        bus.insert_floppy(0, ramp_disk(), None).unwrap();
        assert_ne!(
            read_register(&mut bus, 0xE9C001) & 0x40,
            0,
            "insert latches FDD"
        );
        assert_eq!(bus.m68000_interrupt_level(), 1);
        assert_eq!(bus.m68000_acknowledge_interrupt(1), 0x41, "FDD subvector 1");

        bus.eject_floppy(0);
        assert_ne!(
            read_register(&mut bus, 0xE9C001) & 0x40,
            0,
            "eject latches FDD"
        );
        assert_eq!(bus.m68000_acknowledge_interrupt(1), 0x41);
    }

    #[test]
    fn drive_status_reports_inserted_media_for_the_control_target() {
        let mut bus = bus(X68kModel::X68000);
        bus.insert_floppy(1, ramp_disk(), None).unwrap();
        write_register(&mut bus, 0xE94005, 0x02);
        assert_ne!(read_register(&mut bus, 0xE94005) & 0x80, 0);
        write_register(&mut bus, 0xE94005, 0x01);
        assert_eq!(read_register(&mut bus, 0xE94005) & 0x80, 0);
    }

    #[test]
    fn recalibrate_defers_its_completion_interrupt() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE9C001, 0x0F);
        bus.fdc.state.drive_cylinder[0] = 5;
        // RECALIBRATE command (0x07) plus its drive-select parameter.
        write_register(&mut bus, 0xE94003, 0x07);
        write_register(&mut bus, 0xE94003, 0x00);
        assert_eq!(bus.fdc.state.drive_cylinder[0], 0, "head steps to track 0");
        assert_eq!(
            read_register(&mut bus, 0xE9C001) & 0x80,
            0,
            "the FDC interrupt is deferred, not immediate"
        );
        // Advance through intermediate device events until the deferred FDC
        // completion interrupt latches in the IOC.
        let mut latched = false;
        for _ in 0..1_000 {
            let deadline = bus.next_event_cycle().expect("a pending device event");
            bus.set_current_cycle(deadline);
            bus.process_due_events();
            if read_register(&mut bus, 0xE9C001) & 0x80 != 0 {
                latched = true;
                break;
            }
        }
        assert!(latched, "the deferred FDC interrupt eventually latches");
    }

    /// Emits a straight-line IPL that performs `writes` then spins in place.
    fn boot_ipl(writes: &[(u32, u8)]) -> Vec<u8> {
        let mut ipl = test_roms(X68kModel::X68000).ipl;
        let mut offset = 0x0008;
        for &(address, value) in writes {
            for word in [
                0x13FCu16,
                u16::from(value),
                (address >> 16) as u16,
                address as u16,
            ] {
                ipl[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
                offset += 2;
            }
        }
        // BRA.S * keeps the CPU running (draining DMAC stall) at interrupt mask 7.
        ipl[offset..offset + 2].copy_from_slice(&0x60FEu16.to_be_bytes());
        ipl
    }

    #[test]
    fn boot_chain_dma_reads_a_sector_and_paces_at_the_data_rate() {
        let buffer = 0x0000_2000u32;
        let mtc = SECTOR_BYTES as u16;
        let writes: [(u32, u8); 29] = [
            // Spin the motor on drive 0.
            (0xE94007, 0x80),
            // DMAC channel 0: dual-address 8-bit port, device->memory byte
            // transfer, external request, MAR up, DAR static at the FDC data
            // register.
            (0xE84004, 0x00),
            (0xE84005, 0x82),
            (0xE84006, 0x04),
            (0xE8400A, (mtc >> 8) as u8),
            (0xE8400B, mtc as u8),
            (0xE8400C, (buffer >> 24) as u8),
            (0xE8400D, (buffer >> 16) as u8),
            (0xE8400E, (buffer >> 8) as u8),
            (0xE8400F, buffer as u8),
            (0xE84014, 0x00),
            (0xE84015, 0xE9),
            (0xE84016, 0x40),
            (0xE84017, 0x03),
            (0xE84025, 0x40),
            (0xE84007, 0x80),
            // IOC: unmask every source, program the vector base.
            (0xE9C001, 0x0F),
            (0xE9C003, 0x40),
            // RECALIBRATE drive 0.
            (0xE94003, 0x07),
            (0xE94003, 0x00),
            // READ DATA (MFM) drive 0, head 0, sector 1.
            (0xE94003, 0x46),
            (0xE94003, 0x00),
            (0xE94003, 0x00),
            (0xE94003, 0x00),
            (0xE94003, 0x01),
            (0xE94003, SECTOR_SIZE_CODE),
            (0xE94003, 0x01),
            (0xE94003, 0x1B),
            (0xE94003, 0xFF),
        ];
        let mut loaded = test_roms(X68kModel::X68000);
        loaded.ipl = boot_ipl(&writes);
        let mut machine: X68kMachine =
            crate::bus::test_support::machine(X68kModel::X68000, CpuMode::High, loaded);
        machine.bus.insert_floppy(0, ramp_disk(), None).unwrap();

        let start = machine.bus.current_cycle();
        let mut completion = None;
        for _ in 0..4000 {
            machine.run_for(2_000);
            if machine.bus.ram_byte(buffer + SECTOR_BYTES as u32 - 1)
                == Some((SECTOR_BYTES - 1) as u8)
                && machine.bus.fdc.state.phase != device::upd765a_fdc::FdcPhase::Execution
            {
                completion = Some(machine.bus.current_cycle());
                break;
            }
        }
        let elapsed = completion.expect("the DMA read must complete") - start;

        // Every sector byte landed in RAM through the DMAC.
        for index in 0..SECTOR_BYTES {
            assert_eq!(
                machine.bus.ram_byte(buffer + index as u32),
                Some(index as u8),
                "sector byte {index}"
            );
        }
        // The FDC completion arrived through the IOC at level 1, subvector 0.
        assert_ne!(read_register(&mut machine.bus, 0xE9C001) & 0x04, 0);
        assert_eq!(machine.bus.m68000_interrupt_level(), 1);
        assert_eq!(machine.bus.m68000_acknowledge_interrupt(1), 0x40);
        // The transfer was paced by the 16 us/byte data rate, not copied instantly.
        let minimum_cycles = SECTOR_BYTES as u64 * 160;
        assert!(
            elapsed >= minimum_cycles,
            "expected at least {minimum_cycles} cycles of DRQ pacing, got {elapsed}"
        );
    }

    #[test]
    fn boot_chain_dma_reads_multiple_sectors_and_reports_terminal_chrn() {
        let buffer = 0x0000_2000u32;
        let transfer_bytes = SECTOR_BYTES * 2;
        let mtc = transfer_bytes as u16;
        let writes: [(u32, u8); 27] = [
            (0xE94007, 0x80),
            (0xE84004, 0x00),
            (0xE84005, 0x82),
            (0xE84006, 0x04),
            (0xE8400A, (mtc >> 8) as u8),
            (0xE8400B, mtc as u8),
            (0xE8400C, (buffer >> 24) as u8),
            (0xE8400D, (buffer >> 16) as u8),
            (0xE8400E, (buffer >> 8) as u8),
            (0xE8400F, buffer as u8),
            (0xE84014, 0x00),
            (0xE84015, 0xE9),
            (0xE84016, 0x40),
            (0xE84017, 0x03),
            (0xE84025, 0x40),
            (0xE84007, 0x80),
            (0xE94003, 0x46),
            (0xE94003, 0x00),
            (0xE94003, 0x00),
            (0xE94003, 0x00),
            (0xE94003, 0x01),
            (0xE94003, SECTOR_SIZE_CODE),
            (0xE94003, 0x02),
            (0xE94003, 0x1B),
            (0xE94003, 0xFF),
            (0xE9C001, 0x0F),
            (0xE9C003, 0x40),
        ];
        let mut loaded = test_roms(X68kModel::X68000);
        loaded.ipl = boot_ipl(&writes);
        let mut machine: X68kMachine =
            crate::bus::test_support::machine(X68kModel::X68000, CpuMode::High, loaded);
        machine
            .bus
            .insert_floppy(0, two_sector_disk(), None)
            .unwrap();

        for _ in 0..8_000 {
            machine.run_for(2_000);
            if machine.bus.fdc.state.phase == device::upd765a_fdc::FdcPhase::Result {
                break;
            }
        }
        assert_eq!(
            machine.bus.fdc.state.phase,
            device::upd765a_fdc::FdcPhase::Result
        );
        for sector in 1..=2u8 {
            for index in 0..SECTOR_BYTES {
                assert_eq!(
                    machine.bus.ram_byte(
                        buffer + ((usize::from(sector - 1) * SECTOR_BYTES + index) as u32)
                    ),
                    Some((index as u8).wrapping_add(sector.wrapping_mul(0x40))),
                    "sector {sector} byte {index}"
                );
            }
        }
        let result: Vec<u8> = (0..7)
            .map(|_| read_register(&mut machine.bus, 0xE94003))
            .collect();
        assert_eq!(&result[..3], &[0, 0, 0]);
        assert_eq!(&result[3..], &[0, 0, 2, SECTOR_SIZE_CODE]);
    }
}
