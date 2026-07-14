//! Built-in (non-intelligent) uPD765A FDC, driven by the main CPU.
//!
//! The PC-6601 wires a uPD765A straight to the main Z80. The CPU can move bytes
//! through the data register (port 0xDD), while the ROM also uses the controller
//! in DMA mode and waits for the command to retire. This layer resolves sectors
//! against the mounted image, paces bytes at the data rate via
//! [`Event60::FdcDrqByte`], and finalizes commands. The FDC interrupt line is
//! polled through port 0xB2 rather than delivered to the vectored controller.

use common::TraceSink;
use device::upd765a_fdc::{
    FdcAction, FdcCommand, FdcPhase, ST0_NOT_READY, ST1_DATA_ERROR, ST1_MISSING_ADDRESS_MARK,
    ST1_NOT_WRITABLE, ST2_CONTROL_MARK, ST2_DATA_ERROR, ST2_MISSING_DATA_ADDRESS_MARK,
};

use super::{FDC_FORCED_READY, OPEN_BUS, Pc6000Bus};
use crate::scheduler::Event60;

/// Delay (main-clock units) before a seek/recalibrate raises its completion
/// interrupt, modelling the head-settle latency the disk BIOS expects before it
/// issues Sense Interrupt Status.
const SEEK_INTERRUPT_DELAY_CYCLES: u64 = 2000;

/// D88 per-sector status byte values (high nibble) for abnormal reads.
const D88_STATUS_ID_CRC_ERROR: u8 = 0xA0;
const D88_STATUS_DATA_CRC_ERROR: u8 = 0xB0;
const D88_STATUS_NO_ADDRESS_MARK: u8 = 0xE0;
const D88_STATUS_NO_DATA_ADDRESS_MARK: u8 = 0xF0;

/// How the current read sector should terminate once its data has drained.
#[derive(Clone, Copy, Default)]
struct FdcSectorEnd {
    /// ST1 bits to report.
    st1: u8,
    /// ST2 bits to report.
    st2: u8,
    /// Whether the command ends with abnormal termination (IC=01).
    abnormal: bool,
    /// Whether the command must end after this sector even before EOT.
    terminate: bool,
}

/// Outcome of loading the sector named by the current command.
enum ReadLoad {
    /// A sector was loaded into the FIFO and is ready to drain.
    Loaded,
    /// No matching sector exists; the caller reports a missing address mark.
    Missing,
    /// SK skipped past EOT, so the command has already completed.
    Done,
}

/// Read-path bookkeeping for deleted-mark / CRC handling and READ TRACK.
#[derive(Clone, Default)]
pub(crate) struct FdcReadState {
    /// Termination status computed for the sector currently draining.
    end: FdcSectorEnd,
    /// Next physical sector index for a READ TRACK transfer.
    phys_index: usize,
    /// Sectors still to transfer for a READ TRACK.
    phys_remaining: usize,
    /// ST1 bits accumulated across a READ TRACK.
    track_st1: u8,
    /// ST2 bits accumulated across a READ TRACK.
    track_st2: u8,
    /// Sector bytes currently being transferred by the built-in DMA path.
    dma_data: Vec<u8>,
    /// Next byte index to transfer at the FDC data rate.
    dma_transfer_index: usize,
    /// Completed DMA pages exposed through ports 0xD0..0xDE.
    dma_pages: Vec<Vec<u8>>,
    /// Next byte exposed from each completed DMA page.
    dma_read_indices: Vec<usize>,
}

impl<T: TraceSink> Pc6000Bus<T> {
    /// Reads a built-in FDC port (0xB2 status, 0xD0 DMA, 0xD4 motor,
    /// 0xDC MSR, 0xDD data).
    /// Only the internal interface is implemented; the external intelligent unit
    /// reports absent so the BIOS falls back to the built-in drive.
    pub(super) fn fdc_read(&mut self, port: u16) -> u8 {
        match port & 0xFF {
            // FDC interrupt-request line: set during the result phase, after a
            // seek/recalibrate, or while a non-DMA data byte is due.
            0xB2 => u8::from(
                self.fdc.state.interrupt_pending
                    || (self.fdc.state.nd && self.fdc.pio_byte_ready()),
            ),
            _ if self.fdc_external_selected => OPEN_BUS,
            // Motor status line (active low); the reference returns zero.
            0xD4 => 0x00,
            0xDC => self.fdc.read_status(),
            0xDD => self.read_fdc_data(),
            0xD0..=0xDE => self.read_fdc_dma_data(port),
            _ => OPEN_BUS,
        }
    }

    /// Writes a built-in FDC port (0xB1 interface select, 0xD6 motor, 0xDD data).
    pub(super) fn fdc_write(&mut self, port: u16, value: u8) {
        match port & 0xFF {
            0xB1 => self.fdc_external_selected = value & 0x04 != 0,
            _ if self.fdc_external_selected => {}
            // Motor control (active low): the built-in drive forces ready while
            // driven so empty-drive reads fail on a missing address mark.
            0xD6 => {
                self.fdc_motor_on = value & 0x01 == 0;
                if self.fdc_motor_on {
                    self.fdc.state.control |= FDC_FORCED_READY;
                } else {
                    self.fdc.state.control &= !FDC_FORCED_READY;
                }
            }
            0xDD => self.write_fdc_data(value),
            _ => {}
        }
    }

    /// Reads an FDC data byte (port 0xDD), then advances the PIO read sequence.
    fn read_fdc_data(&mut self) -> u8 {
        let value = self.fdc.read_data();
        if !(self.fdc.pio_active() && self.fdc.state.exec_reading) {
            return value;
        }
        if !self.fdc.pio_sector_done() {
            self.schedule_drq_byte();
            return value;
        }
        // The CPU consumed the final byte of the current sector.
        if self.fdc.is_read_track() {
            self.finish_read_track_sector();
        } else {
            self.finish_read_sector();
        }
        value
    }

    /// Writes an FDC data byte (port 0xDD): a command/parameter byte, or a PIO
    /// data byte during a write/format execution phase.
    fn write_fdc_data(&mut self, value: u8) {
        let was_pio_write = self.fdc.pio_active() && !self.fdc.state.exec_reading;
        let action = self.fdc.write_data(value);
        if was_pio_write {
            if self.fdc.pio_sector_done() {
                self.advance_pio_write();
            } else {
                self.schedule_drq_byte();
            }
        } else {
            self.on_fdc_action(action);
        }
    }

    /// Dispatches the action returned by the FDC after a command is assembled.
    fn on_fdc_action(&mut self, action: FdcAction) {
        match action {
            FdcAction::None => {}
            FdcAction::ScheduleSeekInterrupt => {
                self.fdc.state.interrupt_pending = false;
                self.scheduler.schedule(
                    Event60::FdcSeekComplete,
                    self.current_cycle + SEEK_INTERRUPT_DELAY_CYCLES,
                );
            }
            FdcAction::StartReadData => self.start_pio_read(),
            FdcAction::StartWriteData => self.start_pio_write(),
            FdcAction::StartReadId => self.handle_read_id(),
            FdcAction::StartFormatTrack => self.start_pio_format(),
            FdcAction::StartScan => unreachable!("SCAN commands are disabled on this FDC"),
        }
    }

    /// Releases the next PIO byte slot at a data-rate DRQ tick.
    pub(super) fn on_fdc_drq_byte(&mut self) {
        if self.dma_read_active() {
            if self.fdc_read.dma_transfer_index >= self.fdc_read.dma_data.len() {
                if self.fdc.is_read_track() {
                    self.advance_read_track();
                } else {
                    self.advance_pio_read();
                }
                if !self.dma_read_active() {
                    return;
                }
            }
            self.transfer_dma_read_byte();
            return;
        }

        if self.fdc.pio_active() && self.fdc.state.exec_reading && self.fdc.pio_sector_done() {
            if self.fdc.is_read_track() {
                self.advance_read_track();
            } else {
                self.advance_pio_read();
            }
        }
        self.fdc.pio_release_byte();
    }

    fn schedule_drq_byte(&mut self) {
        self.scheduler.schedule(
            Event60::FdcDrqByte,
            self.current_cycle + self.fdc_drq_byte_cycles,
        );
    }

    fn dma_read_active(&self) -> bool {
        self.fdc.state.phase == FdcPhase::Execution
            && self.fdc.state.active_command == FdcCommand::ReadData
            && !self.fdc.state.nd
    }

    fn transfer_dma_read_byte(&mut self) {
        if self.fdc_read.dma_transfer_index >= self.fdc_read.dma_data.len() {
            return;
        }

        self.fdc_read.dma_transfer_index += 1;

        if self.fdc_read.dma_transfer_index < self.fdc_read.dma_data.len() {
            self.schedule_drq_byte();
            return;
        }

        self.fdc_read
            .dma_read_indices
            .push(self.fdc_read.dma_data.len());
        self.fdc_read.dma_pages.push(self.fdc_read.dma_data.clone());
        if self.fdc.is_read_track() {
            self.finish_read_track_sector();
        } else {
            self.finish_read_sector();
        }
    }

    /// Completes a data command issued to a drive with no disk. While the motor
    /// drives the drive ready, the read finds no address marks (ST1 MA), which
    /// BASIC reads as "no bootable disk"; an idle drive returns "not ready".
    fn complete_no_disk(&mut self) {
        if self.fdc.forced_ready() {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
        } else {
            self.fdc.complete_error(ST0_NOT_READY, 0, 0);
        }
    }

    fn start_pio_read(&mut self) {
        self.fdc_read.dma_data.clear();
        self.fdc_read.dma_transfer_index = 0;
        self.fdc_read.dma_pages.clear();
        self.fdc_read.dma_read_indices.clear();
        let drive = self.fdc.current_drive();
        if !self.floppy.has_drive(drive) {
            self.complete_no_disk();
            return;
        }
        if self.fdc.is_read_track() {
            self.start_read_track(drive);
            return;
        }
        match self.load_current_read_sector(drive) {
            ReadLoad::Loaded => self.schedule_drq_byte(),
            ReadLoad::Missing => self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0),
            ReadLoad::Done => {}
        }
    }

    /// Loads the sector named by the current FDC C/H/R/N into the PIO FIFO,
    /// skipping deleted-mark mismatches when the SK flag is set.
    fn load_current_read_sector(&mut self, drive: usize) -> ReadLoad {
        loop {
            let track_index = self.fdc.current_track_index();
            let (c, h, r, n) = (
                self.fdc.state.c,
                self.fdc.state.h,
                self.fdc.state.r,
                self.fdc.state.n,
            );
            let Some(sector) = self.floppy.find_sector(drive, track_index, c, h, r, n) else {
                return ReadLoad::Missing;
            };
            let deleted = sector.deleted;
            let status = sector.status;
            let data = sector.data.clone();
            let end = self.sector_end_for(deleted, status);

            // SK set skips a deleted-mark mismatch instead of flagging it.
            if end.st2 & ST2_CONTROL_MARK != 0 && self.fdc.state.sk {
                if self.fdc.advance_sector() {
                    self.fdc.complete_success();
                    return ReadLoad::Done;
                }
                continue;
            }

            self.fdc_read.end = end;
            self.begin_read_transfer(&data);
            return ReadLoad::Loaded;
        }
    }

    fn begin_read_transfer(&mut self, data: &[u8]) {
        if self.fdc.state.nd {
            self.fdc.begin_pio_read(data);
        } else {
            self.fdc_read.dma_data.clear();
            self.fdc_read.dma_data.extend_from_slice(data);
            self.fdc_read.dma_transfer_index = 0;
        }
    }

    /// Reads a built-in DMA data latch. The PC-6601 ROM disk routines transfer
    /// completed pages with INDR from ports 0xD0 upward, so bytes are exposed
    /// from the end of each page toward the beginning.
    fn read_fdc_dma_data(&mut self, port: u16) -> u8 {
        let page = (port & 0x0F) as usize;
        let Some(read_index) = self.fdc_read.dma_read_indices.get_mut(page) else {
            return OPEN_BUS;
        };
        if *read_index == 0 {
            return OPEN_BUS;
        }
        let Some(data) = self.fdc_read.dma_pages.get(page) else {
            return OPEN_BUS;
        };
        *read_index -= 1;
        data[*read_index]
    }

    /// Completes a drained read sector, applying any deleted-mark / CRC status.
    fn finish_read_sector(&mut self) {
        let end = self.fdc_read.end;
        if end.terminate {
            if end.abnormal {
                self.fdc.complete_error(0, end.st1, end.st2);
            } else {
                self.fdc.complete_success_with_status(end.st1, end.st2);
            }
            return;
        }
        if self.fdc.at_last_sector() {
            self.fdc.complete_success_with_status(end.st1, end.st2);
            return;
        }
        // More sectors to read: the next DRQ tick advances the C/H/R cursor.
        self.schedule_drq_byte();
    }

    /// Handles a drained read sector: continue to the next sector or finish at EOT.
    fn advance_pio_read(&mut self) {
        if self.fdc.advance_sector() {
            self.fdc.complete_success();
            return;
        }
        let drive = self.fdc.current_drive();
        match self.load_current_read_sector(drive) {
            ReadLoad::Loaded | ReadLoad::Done => {}
            ReadLoad::Missing => self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0),
        }
    }

    /// Computes how a read sector should terminate given its D88 deleted flag and
    /// status byte, relative to whether the command is READ DELETED DATA.
    fn sector_end_for(&self, sector_deleted: u8, sector_status: u8) -> FdcSectorEnd {
        let mut end = FdcSectorEnd::default();

        // A deleted-data address mark that does not match the command type raises
        // the control mark; the command still reads this sector then stops.
        if (sector_deleted != 0) != self.fdc.is_read_deleted() {
            end.st2 |= ST2_CONTROL_MARK;
            end.terminate = true;
        }

        match sector_status & 0xF0 {
            D88_STATUS_ID_CRC_ERROR => {
                end.st1 |= ST1_DATA_ERROR;
                end.abnormal = true;
                end.terminate = true;
            }
            D88_STATUS_DATA_CRC_ERROR => {
                end.st1 |= ST1_DATA_ERROR;
                end.st2 |= ST2_DATA_ERROR;
                end.abnormal = true;
                end.terminate = true;
            }
            D88_STATUS_NO_ADDRESS_MARK => {
                end.st1 |= ST1_MISSING_ADDRESS_MARK;
                end.abnormal = true;
                end.terminate = true;
            }
            D88_STATUS_NO_DATA_ADDRESS_MARK => {
                end.st1 |= ST1_MISSING_ADDRESS_MARK;
                end.st2 |= ST2_MISSING_DATA_ADDRESS_MARK;
                end.abnormal = true;
                end.terminate = true;
            }
            _ => {}
        }

        end
    }

    /// Begins a READ TRACK (READ DIAGNOSTIC): transfers every sector on the track
    /// in physical order, regardless of the requested C/H/R.
    fn start_read_track(&mut self, drive: usize) {
        let track_index = self.fdc.current_track_index();
        let available = self.floppy.sector_count(drive, track_index);
        let count = (self.fdc.state.eot as usize).min(available);
        if count == 0 {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            return;
        }
        self.fdc_read.phys_index = 0;
        self.fdc_read.phys_remaining = count;
        self.fdc_read.track_st1 = 0;
        self.fdc_read.track_st2 = 0;
        if self.load_read_track_sector(drive) {
            self.schedule_drq_byte();
        } else {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
        }
    }

    /// Loads the physical sector at the current READ TRACK index into the FIFO.
    fn load_read_track_sector(&mut self, drive: usize) -> bool {
        let track_index = self.fdc.current_track_index();
        let index = self.fdc_read.phys_index;
        let Some(sector) = self.floppy.sector_at_index(drive, track_index, index) else {
            return false;
        };
        let (c, h, r, n) = (
            sector.cylinder,
            sector.head,
            sector.record,
            sector.size_code,
        );
        let deleted = sector.deleted;
        let status = sector.status;
        let data = sector.data.clone();
        // The result registers reflect the most recently read sector's ID.
        self.fdc.provide_read_id(c, h, r, n);
        let end = self.sector_end_for(deleted, status);
        self.fdc_read.track_st1 |= end.st1;
        self.fdc_read.track_st2 |= end.st2;
        self.begin_read_transfer(&data);
        true
    }

    /// Completes the current READ TRACK sector, ending the command at the count.
    fn finish_read_track_sector(&mut self) {
        if self.fdc_read.phys_remaining <= 1 {
            self.complete_read_track();
        } else {
            self.schedule_drq_byte();
        }
    }

    /// Advances READ TRACK to the next physical sector on a DRQ tick.
    fn advance_read_track(&mut self) {
        self.fdc_read.phys_remaining = self.fdc_read.phys_remaining.saturating_sub(1);
        self.fdc_read.phys_index += 1;
        let drive = self.fdc.current_drive();
        if !self.load_read_track_sector(drive) {
            self.complete_read_track();
        }
    }

    /// Finalizes a READ TRACK, reporting any CRC / missing-mark errors seen.
    fn complete_read_track(&mut self) {
        let st1 = self.fdc_read.track_st1;
        let st2 = self.fdc_read.track_st2;
        if st1 != 0 || st2 & !ST2_CONTROL_MARK != 0 {
            self.fdc.complete_error(0, st1, st2);
        } else {
            self.fdc.complete_success_with_status(st1, st2);
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
}

#[cfg(test)]
mod tests {
    use device::floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage};

    use super::{OPEN_BUS, Pc6000Bus};
    use crate::{config::Pc6000Model, scheduler::Event60};

    const SECTOR_SIZE: usize = 256;
    const SIZE_CODE_256: u8 = 1;
    /// MSR bit 7: RQM (host may transfer a byte).
    const MSR_RQM: u8 = 0x80;
    /// MSR bit 6: DIO (controller transfers data to host).
    const MSR_DIO: u8 = 0x40;
    /// MSR bit 4: CB (controller busy).
    const MSR_CB: u8 = 0x10;

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

    fn image_with_sectors(name: &str, sectors: Vec<D88Sector>) -> FloppyImage {
        FloppyImage::from_d88(D88Disk::from_tracks(
            String::from(name),
            false,
            D88MediaType::Disk2D,
            vec![Some(sectors)],
        ))
    }

    /// Builds a PC-6601 bus with the disk mounted, the motor running, and the
    /// periodic events cancelled so the FDC DRQ tick is the only event.
    fn bus_with_disk(image: FloppyImage) -> Pc6000Bus {
        let mut bus = Pc6000Bus::new(Pc6000Model::Pc6601, 48_000);
        bus.scheduler.cancel(Event60::Vrtc);
        bus.scheduler.cancel(Event60::KeyScan);
        bus.scheduler.cancel(Event60::Scanline);
        bus.scheduler.cancel(Event60::BusReqEnd);
        bus.insert_floppy(0, image, None);
        // Motor on (port 0xD6 bit 0 active low).
        bus.fdc_write(0xD6, 0x00);
        bus
    }

    /// Issues a READ DATA command for drive 0, head 0, sectors 1..=eot at 256 B.
    fn issue_read(bus: &mut Pc6000Bus, eot: u8) {
        // 0x46 = READ DATA with the MFM flag set.
        bus.fdc_write(0xDD, 0x46);
        for byte in [0x00, 0x00, 0x00, 0x01, SIZE_CODE_256, eot, 0x1B, 0xFF] {
            bus.fdc_write(0xDD, byte);
        }
    }

    fn specify_dma_mode(bus: &mut Pc6000Bus) {
        bus.fdc_write(0xDD, 0x03);
        bus.fdc_write(0xDD, 0xBF);
        bus.fdc_write(0xDD, 0x26);
    }

    fn specify_non_dma_mode(bus: &mut Pc6000Bus) {
        bus.fdc_write(0xDD, 0x03);
        bus.fdc_write(0xDD, 0xBF);
        bus.fdc_write(0xDD, 0x27);
    }

    fn pump_next_event(bus: &mut Pc6000Bus) {
        let fire = bus
            .scheduler
            .next_event_cycle()
            .expect("an event while pumping the FDC");
        bus.set_current_cycle(fire);
        bus.process_events();
    }

    fn pump_until_result_phase(bus: &mut Pc6000Bus, sectors: usize) {
        for _ in 0..=(SECTOR_SIZE * sectors) + 8 {
            pump_next_event(bus);
            if bus.fdc_read(0xDC) & (MSR_RQM | MSR_DIO | MSR_CB) == (MSR_RQM | MSR_DIO | MSR_CB) {
                return;
            }
        }
        panic!("FDC did not enter the result phase");
    }

    fn read_result(bus: &mut Pc6000Bus) -> Vec<u8> {
        (0..7).map(|_| bus.fdc_read(0xDD)).collect()
    }

    fn read_dma_sector(bus: &mut Pc6000Bus, port: u16) -> [u8; SECTOR_SIZE] {
        let mut data = [0; SECTOR_SIZE];
        for address in (0..SECTOR_SIZE).rev() {
            data[address] = bus.fdc_read(port);
        }
        data
    }

    fn assert_sector_pattern(data: &[u8], first_value: u8) {
        for (index, value) in data.iter().enumerate() {
            assert_eq!(*value, first_value.wrapping_add(index as u8));
        }
    }

    /// Pumps the scheduler until the FDC releases the next PIO byte, then reads it.
    fn read_ready_byte(bus: &mut Pc6000Bus) -> u8 {
        for _ in 0..4096 {
            let fire = bus
                .scheduler
                .next_event_cycle()
                .expect("an event while waiting for RQM");
            bus.set_current_cycle(fire);
            bus.process_events();
            if bus.fdc_read(0xDC) & MSR_RQM != 0 {
                return bus.fdc_read(0xDD);
            }
        }
        panic!("FDC did not release a PIO byte");
    }

    #[test]
    fn pio_read_streams_a_sector_then_enters_result_phase() {
        let mut bus = bus_with_disk(image_with_sectors("ONE", vec![sector(1, 1, 0)]));
        specify_non_dma_mode(&mut bus);
        issue_read(&mut bus, 1);

        for expected in 0..SECTOR_SIZE {
            assert_eq!(read_ready_byte(&mut bus), expected as u8);
        }

        // The last byte of the EOT sector terminates straight into the result
        // phase: seven result bytes are readable and report sector 1.
        assert_ne!(bus.fdc_read(0xDC) & MSR_RQM, 0, "result phase asserts RQM");
        let result: Vec<u8> = (0..7).map(|_| bus.fdc_read(0xDD)).collect();
        assert_eq!(result[0] & 0xC0, 0x00, "normal termination");
        assert_eq!(result[5], 1, "result reports the last sector read");
    }

    #[test]
    fn pio_read_continues_across_two_sectors() {
        let mut bus = bus_with_disk(image_with_sectors(
            "TWO",
            vec![sector(1, 2, 0x00), sector(2, 2, 0x80)],
        ));
        specify_non_dma_mode(&mut bus);
        issue_read(&mut bus, 2);

        for expected in 0..SECTOR_SIZE {
            assert_eq!(read_ready_byte(&mut bus), expected as u8);
        }
        for offset in 0..SECTOR_SIZE {
            assert_eq!(read_ready_byte(&mut bus), 0x80u8.wrapping_add(offset as u8));
        }

        let result: Vec<u8> = (0..7).map(|_| bus.fdc_read(0xDD)).collect();
        assert_eq!(result[5], 2, "result reports sector 2 at EOT");
    }

    #[test]
    fn dma_mode_read_can_finish_after_partial_cpu_reads() {
        let mut bus = bus_with_disk(image_with_sectors("DMA", vec![sector(1, 1, 0x10)]));
        specify_dma_mode(&mut bus);
        issue_read(&mut bus, 1);

        assert_eq!(bus.fdc_read(0xDD), 0xFF);
        assert_eq!(bus.fdc_read(0xD0), OPEN_BUS);

        pump_until_result_phase(&mut bus, 1);

        let result = read_result(&mut bus);
        assert_eq!(result[0] & 0xC0, 0x00, "normal termination");
        assert_eq!(result[5], 1, "result reports the last sector read");

        let data = read_dma_sector(&mut bus, 0xD0);
        assert_sector_pattern(&data, 0x10);
        assert_eq!(bus.fdc_read(0xD0), OPEN_BUS);
    }

    #[test]
    fn dma_mode_read_buffers_sectors_on_consecutive_ports() {
        let mut bus = bus_with_disk(image_with_sectors(
            "DMA2",
            vec![sector(1, 2, 0x00), sector(2, 2, 0x80)],
        ));
        specify_dma_mode(&mut bus);
        issue_read(&mut bus, 2);

        pump_until_result_phase(&mut bus, 2);

        let result = read_result(&mut bus);
        assert_eq!(result[0] & 0xC0, 0x00, "normal termination");
        assert_eq!(result[5], 2, "result reports sector 2 at EOT");

        let first_sector = read_dma_sector(&mut bus, 0xD0);
        let second_sector = read_dma_sector(&mut bus, 0xD1);
        assert_sector_pattern(&first_sector, 0x00);
        assert_sector_pattern(&second_sector, 0x80);
        assert_eq!(bus.fdc_read(0xD0), OPEN_BUS);
        assert_eq!(bus.fdc_read(0xD1), OPEN_BUS);
        assert_eq!(bus.fdc_read(0xD2), OPEN_BUS);
    }

    #[test]
    fn pio_write_round_trips_through_the_image() {
        let mut bus = bus_with_disk(image_with_sectors("RW", vec![sector(1, 1, 0)]));
        specify_non_dma_mode(&mut bus);

        // 0x45 = WRITE DATA with the MFM flag set.
        bus.fdc_write(0xDD, 0x45);
        for byte in [0x00, 0x00, 0x00, 0x01, SIZE_CODE_256, 0x01, 0x1B, 0xFF] {
            bus.fdc_write(0xDD, byte);
        }
        for value in 0..SECTOR_SIZE {
            // Wait for the write slot (RQM), then push the byte.
            for _ in 0..4096 {
                let fire = bus.scheduler.next_event_cycle().expect("a write event");
                bus.set_current_cycle(fire);
                bus.process_events();
                if bus.fdc_read(0xDC) & MSR_RQM != 0 {
                    bus.fdc_write(0xDD, 0xA0u8.wrapping_add(value as u8));
                    break;
                }
            }
        }
        // Drain the result phase.
        let _result: Vec<u8> = (0..7).map(|_| bus.fdc_read(0xDD)).collect();

        // Reading the sector back yields the written pattern.
        issue_read(&mut bus, 1);
        for value in 0..SECTOR_SIZE {
            assert_eq!(read_ready_byte(&mut bus), 0xA0u8.wrapping_add(value as u8));
        }
    }

    #[test]
    fn empty_drive_read_reports_a_missing_address_mark() {
        let mut bus = Pc6000Bus::new(Pc6000Model::Pc6601, 48_000);
        bus.scheduler.cancel(Event60::Vrtc);
        bus.scheduler.cancel(Event60::KeyScan);
        bus.fdc_write(0xD6, 0x00);

        specify_non_dma_mode(&mut bus);
        issue_read(&mut bus, 1);
        let result: Vec<u8> = (0..7).map(|_| bus.fdc_read(0xDD)).collect();
        assert_eq!(result[0] & 0xC0, 0x40, "abnormal termination");
        assert_eq!(result[1] & 0x01, 0x01, "ST1 missing address mark");
    }
}
