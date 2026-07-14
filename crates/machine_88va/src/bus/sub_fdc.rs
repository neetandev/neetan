//! Floppy sub-CPU FDC orchestration: non-DMA (PIO) byte transfers.
//!
//! The single PC80S31K uPD765A is driven by programmed I/O: the sub CPU moves
//! each data byte through port 0xFB while polling the MSR, and asserts terminal
//! count through port 0xF8. The device is a passive byte FIFO (see
//! [`device::upd765a_fdc`]); this layer resolves sectors, paces bytes at the
//! data rate via `Event88Va::FdcDrqByte`, and finalizes commands.

use device::{
    floppy::D88MediaType,
    upd765a_fdc::{
        FdcAction, FdcCommand, ST0_NOT_READY, ST1_MISSING_ADDRESS_MARK, ST1_NOT_WRITABLE,
    },
};

use super::Pc88VaBus;
use crate::scheduler::Event88Va;

/// Delay (main-clock units) before a seek/recalibrate raises its completion
/// interrupt. Calibration item: models the head-settle latency the disk ROM
/// expects before issuing Sense Interrupt Status.
const SEEK_INTERRUPT_DELAY_CYCLES: u64 = 2000;

impl Pc88VaBus {
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
        self.update_main_fdc_irq();
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
            self.on_fdc_action(action);
        }
        self.update_main_fdc_irq();
    }

    /// Dispatches the action returned by the FDC after a command is assembled.
    fn on_fdc_action(&mut self, action: FdcAction) {
        match action {
            FdcAction::None => {}
            FdcAction::ScheduleSeekInterrupt => {
                // Defer the seek/recalibrate completion interrupt by the head-settle
                // delay, then raise it for the sub CPU to service.
                self.fdc.state.interrupt_pending = false;
                self.scheduler.schedule(
                    Event88Va::FdcSeekComplete,
                    self.current_cycle + SEEK_INTERRUPT_DELAY_CYCLES,
                );
                self.update_next_event_cycle();
            }
            FdcAction::StartReadData => {
                if self.fdc_dma_mode {
                    self.start_dma_read();
                } else {
                    self.start_pio_read();
                }
            }
            FdcAction::StartWriteData => {
                if self.fdc_dma_mode {
                    self.start_dma_write();
                } else {
                    self.start_pio_write();
                }
            }
            FdcAction::StartReadId => self.handle_read_id(),
            FdcAction::StartFormatTrack => self.start_pio_format(),
            FdcAction::StartScan => unreachable!("SCAN commands are disabled on this FDC"),
        }
    }

    /// Releases the next PIO byte slot at a data-rate DRQ tick.
    pub(crate) fn on_fdc_drq_byte(&mut self, _fire_cycle: u64) {
        if self.fdc.pio_active() && self.fdc.state.exec_reading && self.fdc.pio_sector_done() {
            self.advance_pio_read();
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
            .schedule(Event88Va::FdcTcClear, self.current_cycle + pulse);
        self.update_next_event_cycle();
    }

    fn schedule_drq_byte(&mut self) {
        self.scheduler.schedule(
            Event88Va::FdcDrqByte,
            self.current_cycle + self.drq_byte_cycles,
        );
        self.update_next_event_cycle();
    }

    /// Completes a data command issued to a drive with no disk. The PC80S31K
    /// forces the drive ready while the motor is driven, so the µPD765A does not
    /// report "not ready"; the read instead finds no address marks (ST1 MA),
    /// which the BIOS reads as "no bootable disk". Only an unforced, genuinely
    /// idle drive returns "not ready".
    fn complete_no_disk(&mut self) {
        if self.fdc.forced_ready() {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
        } else {
            self.fdc.complete_error(ST0_NOT_READY, 0, 0);
        }
    }

    /// uPD71071 DMA channel wired to the uPD765A in native (DMA) mode.
    const FDC_DMA_CHANNEL: usize = 2;

    /// Arms the FDC data-command completion interrupt for the main-CPU DMA path,
    /// after the same head-settle delay the seek path uses.
    fn finish_dma_command(&mut self) {
        self.fdc.state.interrupt_pending = false;
        self.scheduler.schedule(
            Event88Va::FdcResultComplete,
            self.current_cycle + SEEK_INTERRUPT_DELAY_CYCLES,
        );
        self.update_next_event_cycle();
    }

    /// Performs a native-mode READ DATA as a block DMA transfer: streams sector
    /// bytes into memory at the channel-2 address up to the programmed count,
    /// spanning sectors until the count or the command's EOT is reached.
    fn start_dma_read(&mut self) {
        let drive = self.fdc.current_drive();
        if !self.floppy.has_drive(drive) {
            self.complete_no_disk();
            self.finish_dma_command();
            return;
        }
        if !self.pc88_density_matches(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            self.finish_dma_command();
            return;
        }

        let mut remaining = self.dmac.transfer_length(Self::FDC_DMA_CHANNEL);
        let mut address = self.dmac.address(Self::FDC_DMA_CHANNEL);
        let mut transferred = 0usize;
        loop {
            let track_index = self.fdc.current_track_index();
            let (c, h, r, n) = (
                self.fdc.state.c,
                self.fdc.state.h,
                self.fdc.state.r,
                self.fdc.state.n,
            );
            match self.floppy.read_sector_data(drive, track_index, c, h, r, n) {
                Some(data) => {
                    let data = data.to_vec();
                    for &byte in &data {
                        if remaining == 0 {
                            break;
                        }
                        self.memory.write_byte(address, byte);
                        address = address.wrapping_add(1);
                        remaining -= 1;
                        transferred += 1;
                    }
                }
                None => {
                    self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
                    self.dmac.advance(Self::FDC_DMA_CHANNEL, transferred);
                    self.finish_dma_command();
                    return;
                }
            }
            // The uPD765A advances its sector ID after every sector it reads,
            // so the result phase reports the next sector even when the host
            // terminal count ends the command. Mirror that here.
            let command_ended = self.fdc.advance_sector();
            if remaining == 0 || command_ended {
                break;
            }
        }
        self.dmac.advance(Self::FDC_DMA_CHANNEL, transferred);
        self.fdc.complete_success();
        self.finish_dma_command();
    }

    /// Performs a native-mode WRITE DATA as a block DMA transfer: streams bytes
    /// from memory at the channel-2 address into sectors up to the programmed
    /// count, spanning sectors until the count or the command's EOT is reached.
    fn start_dma_write(&mut self) {
        let drive = self.fdc.current_drive();
        if !self.floppy.has_drive(drive) {
            self.complete_no_disk();
            self.finish_dma_command();
            return;
        }
        if self.floppy.is_write_protected(drive) {
            self.fdc.complete_error(0, ST1_NOT_WRITABLE, 0);
            self.finish_dma_command();
            return;
        }
        if !self.pc88_density_matches(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            self.finish_dma_command();
            return;
        }

        let sector_size = self.fdc_sector_size();
        let mut remaining = self.dmac.transfer_length(Self::FDC_DMA_CHANNEL);
        let mut address = self.dmac.address(Self::FDC_DMA_CHANNEL);
        let mut transferred = 0usize;
        loop {
            let mut sector = vec![0u8; sector_size];
            for byte in sector.iter_mut() {
                if remaining == 0 {
                    break;
                }
                *byte = self.memory.read_byte(address);
                address = address.wrapping_add(1);
                remaining -= 1;
                transferred += 1;
            }
            let track_index = self.fdc.current_track_index();
            let (c, h, r, n) = (
                self.fdc.state.c,
                self.fdc.state.h,
                self.fdc.state.r,
                self.fdc.state.n,
            );
            self.floppy
                .write_sector_data(drive, track_index, c, h, r, n, &sector);
            let command_ended = self.fdc.advance_sector();
            if remaining == 0 || command_ended {
                break;
            }
        }
        self.dmac.advance(Self::FDC_DMA_CHANNEL, transferred);
        self.fdc.complete_success();
        self.finish_dma_command();
    }

    fn start_pio_read(&mut self) {
        let drive = self.fdc.current_drive();
        if !self.floppy.has_drive(drive) {
            self.complete_no_disk();
            return;
        }
        if !self.pc88_density_matches(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            return;
        }
        if !self.load_current_read_sector(drive) {
            self.fdc.complete_error(0, ST1_MISSING_ADDRESS_MARK, 0);
            return;
        }
        self.schedule_drq_byte();
    }

    /// Loads the sector named by the current FDC C/H/R/N into the PIO FIFO.
    fn load_current_read_sector(&mut self, drive: usize) -> bool {
        let track_index = self.fdc.current_track_index();
        let (c, h, r, n) = (
            self.fdc.state.c,
            self.fdc.state.h,
            self.fdc.state.r,
            self.fdc.state.n,
        );
        match self.floppy.read_sector_data(drive, track_index, c, h, r, n) {
            Some(data) => {
                let data = data.to_vec();
                self.fdc.begin_pio_read(&data);
                true
            }
            None => false,
        }
    }

    /// Handles a drained read sector: continue to the next sector or finish at EOT.
    fn advance_pio_read(&mut self) {
        if self.fdc.advance_sector() {
            self.fdc.complete_success();
            return;
        }
        let drive = self.fdc.current_drive();
        if !self.load_current_read_sector(drive) {
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
                    "PC-88VA2 FDC FORMAT TRACK density mismatch: drive={drive} selected={selected:?} image={:?}",
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
    use common::Bus;
    use device::{
        floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage},
        upd765a_fdc::FdcPhase,
    };

    use super::*;
    use crate::{bus::test_support::test_bus, scheduler::Event88Va};

    const SECTOR_SIZE: usize = 256;
    const SIZE_CODE_256: u8 = 1;

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

    fn read_ready_pio_byte(bus: &mut Pc88VaBus) -> u8 {
        for _ in 0..1024 {
            let event_cycle = bus.next_event_cycle().expect("event while waiting for RQM");
            bus.set_current_cycle(event_cycle);
            if bus.sub_io_read(0xFA) & 0x80 != 0 {
                return bus.sub_io_read(0xFB);
            }
        }
        panic!("FDC did not release a PIO byte");
    }

    fn write_ready_pio_byte(bus: &mut Pc88VaBus, value: u8) {
        for _ in 0..1024 {
            let event_cycle = bus.next_event_cycle().expect("event while waiting for RQM");
            bus.set_current_cycle(event_cycle);
            if bus.sub_io_read(0xFA) & 0x80 != 0 {
                bus.sub_io_write(0xFB, value);
                return;
            }
        }
        panic!("FDC did not release a PIO byte slot");
    }

    #[test]
    fn pio_read_drq_wakes_the_disk_sub_cpu() {
        let mut bus = test_bus();
        bus.insert_floppy(0, single_sector_image(), None);
        bus.sub_io_write(0xF4, 0x00);

        bus.sub_io_write(0xFB, 0x46);
        for byte in [0x00, 0x00, 0x00, 0x01, SIZE_CODE_256, 0x01, 0x1B, 0xFF] {
            bus.sub_io_write(0xFB, byte);
        }

        assert!(!bus.sub_irq_pending());
        let drq_cycle = bus.next_event_cycle().expect("DRQ event");
        bus.set_current_cycle(drq_cycle);

        assert_ne!(bus.sub_io_read(0xFA) & 0x80, 0, "RQM is set");
        assert!(bus.sub_irq_pending(), "DRQ asserts the disk sub-CPU IRQ");
        bus.acknowledge_sub_irq();
        assert!(
            bus.sub_irq_pending(),
            "PIO byte-ready IRQ remains asserted until the byte is consumed"
        );
        assert_eq!(bus.sub_io_read(0xFB), 0x00, "first sector byte");
        assert!(
            !bus.sub_irq_pending(),
            "PIO byte-ready IRQ clears after the byte is consumed"
        );
    }

    #[test]
    fn pio_read_terminal_count_after_sector_completes_current_sector() {
        let mut bus = test_bus();
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
        let result_bytes: Vec<u8> = (0..7).map(|_| bus.sub_io_read(0xFB)).collect();
        assert_eq!(result_bytes[5], 1);
    }

    #[test]
    fn pio_format_track_completes_after_streamed_id_bytes() {
        let mut bus = test_bus();
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
        let result_bytes: Vec<u8> = (0..7).map(|_| bus.sub_io_read(0xFB)).collect();
        assert_eq!(result_bytes[1], 0x00);
        assert_eq!(result_bytes[2], 0x00);
    }

    #[test]
    fn pio_format_track_rejects_density_mismatch_before_streaming_ids() {
        let mut bus = test_bus();
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
        let result_bytes: Vec<u8> = (0..7).map(|_| bus.sub_io_read(0xFB)).collect();
        assert_eq!(result_bytes[0] & 0xC0, 0x40);
        assert_eq!(
            result_bytes[1] & ST1_MISSING_ADDRESS_MARK,
            ST1_MISSING_ADDRESS_MARK
        );
    }

    #[test]
    fn pio_format_track_accepts_matching_2hd_mode() {
        let mut bus = test_bus();
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
        let result_bytes: Vec<u8> = (0..7).map(|_| bus.sub_io_read(0xFB)).collect();
        assert_eq!(result_bytes[1], 0x00);
        assert_eq!(result_bytes[2], 0x00);
    }

    // --- Main-CPU (native DMA) floppy path ---

    const HD_SECTOR_SIZE: usize = 1024;
    const SIZE_CODE_1024: u8 = 3;
    /// uPD765A interrupt vector on the main 8259 (slave IR3 = IRQ 11).
    const FDC_IRQ_VECTOR: u8 = 0x13;
    /// DMA target inside plain main RAM (page 3).
    const DMA_TARGET: u32 = 0x3_0000;

    /// A 1024-byte (N=3) 2HD sector whose bytes encode its record number.
    fn hd_sector(record: u8, sector_count: u16) -> D88Sector {
        D88Sector {
            cylinder: 0,
            head: 0,
            record,
            size_code: SIZE_CODE_1024,
            sector_count,
            mfm_flag: 0x00,
            deleted: 0x00,
            status: 0x00,
            reserved: [0; 5],
            data: (0..HD_SECTOR_SIZE)
                .map(|index| record.wrapping_mul(0x20).wrapping_add(index as u8))
                .collect(),
            source_offset: None,
        }
    }

    /// A 2HD disk with `num_sectors` 1024-byte sectors on track 0 (R1..=N).
    fn hd_track0_image(num_sectors: u8) -> FloppyImage {
        let sectors: Vec<D88Sector> = (1..=num_sectors)
            .map(|record| hd_sector(record, u16::from(num_sectors)))
            .collect();
        FloppyImage::from_d88(D88Disk::from_tracks(
            String::from("DMA"),
            false,
            D88MediaType::Disk2HD,
            vec![Some(sectors)],
        ))
    }

    /// Masks the 8259s so only the FDC interrupt (slave IR3 / IRQ 11) can be
    /// delivered, making IRQ assertions deterministic.
    fn isolate_fdc_irq(bus: &mut Pc88VaBus) {
        bus.io_write(0x18A, 0x7F); // master: only the slave cascade (IR7) open
        bus.io_write(0x186, 0xF7); // slave: only IR3 (IRQ 11) open
    }

    /// Selects 2HD DMA-mode access and arms uPD71071 channel 2 for `length` bytes.
    fn setup_dma(bus: &mut Pc88VaBus, address: u32, length: u16) {
        bus.io_write(0x1B2, 0x01); // 2HD density, drive 0
        bus.io_write(0x1B0, 0x01); // FDC DMA mode
        bus.io_write(0x161, 0x02); // select DMA channel 2
        let count = length - 1;
        bus.io_write(0x162, count as u8);
        bus.io_write(0x163, (count >> 8) as u8);
        bus.io_write(0x164, address as u8);
        bus.io_write(0x165, (address >> 8) as u8);
        bus.io_write(0x166, (address >> 16) as u8);
        bus.io_write(0x16F, 0xFB); // unmask DMA channel 2
        isolate_fdc_irq(bus);
    }

    /// Issues a main-path data command (opcode includes the MFM bit) for drive 0.
    fn issue_data_command(bus: &mut Pc88VaBus, opcode: u8, head: u8, c: u8, r: u8, n: u8, eot: u8) {
        bus.io_write(0x1BA, opcode);
        let hd_us = (head & 1) << 2;
        for param in [hd_us, c, head, r, n, eot, 0x1B, 0xFF] {
            bus.io_write(0x1BA, param);
        }
    }

    /// Advances the scheduler to the given FDC completion event so the deferred
    /// main IRQ asserts.
    fn fire_fdc_completion(bus: &mut Pc88VaBus, event: Event88Va) {
        let fire = bus.scheduler.state.fire_cycles[event as usize].expect("completion scheduled");
        bus.set_current_cycle(fire);
    }

    /// Reads the seven-byte result phase through the main data port.
    fn read_result(bus: &mut Pc88VaBus) -> [u8; 7] {
        let mut result = [0u8; 7];
        for byte in result.iter_mut() {
            *byte = bus.io_read(0x1BA);
        }
        result
    }

    #[test]
    fn dma_read_transfers_sector_to_memory() {
        let mut bus = test_bus();
        bus.insert_floppy(0, hd_track0_image(8), None);
        setup_dma(&mut bus, DMA_TARGET, HD_SECTOR_SIZE as u16);
        issue_data_command(&mut bus, 0x46, 0, 0, 2, SIZE_CODE_1024, 8);

        // The block transfer runs synchronously: sector 2 lands at the target.
        for index in 0..HD_SECTOR_SIZE {
            let expected = 2u8.wrapping_mul(0x20).wrapping_add(index as u8);
            assert_eq!(
                bus.read_byte(DMA_TARGET + index as u32),
                expected,
                "byte {index}"
            );
        }
        let result = read_result(&mut bus);
        assert_eq!(result[0] & 0xC0, 0x00, "ST0 reports normal termination");
    }

    #[test]
    fn dma_read_advances_result_sector() {
        let mut bus = test_bus();
        bus.insert_floppy(0, hd_track0_image(8), None);
        setup_dma(&mut bus, DMA_TARGET, HD_SECTOR_SIZE as u16);
        issue_data_command(&mut bus, 0x46, 0, 0, 2, SIZE_CODE_1024, 8);

        let result = read_result(&mut bus);
        assert_eq!(result[3], 0, "C unchanged");
        assert_eq!(result[5], 3, "result reports the next sector after R2");
    }

    #[test]
    fn dma_read_spans_multiple_sectors() {
        let mut bus = test_bus();
        bus.insert_floppy(0, hd_track0_image(8), None);
        setup_dma(&mut bus, DMA_TARGET, (HD_SECTOR_SIZE * 2) as u16);
        issue_data_command(&mut bus, 0x46, 0, 0, 1, SIZE_CODE_1024, 8);

        assert_eq!(bus.read_byte(DMA_TARGET), 1u8.wrapping_mul(0x20));
        assert_eq!(
            bus.read_byte(DMA_TARGET + HD_SECTOR_SIZE as u32),
            2u8.wrapping_mul(0x20)
        );
        let result = read_result(&mut bus);
        assert_eq!(result[5], 3, "result reports the next sector after R1+R2");
    }

    #[test]
    fn dma_read_missing_sector_reports_error() {
        let mut bus = test_bus();
        bus.insert_floppy(0, hd_track0_image(8), None);
        setup_dma(&mut bus, DMA_TARGET, HD_SECTOR_SIZE as u16);
        // Sector 9 does not exist on the eight-sector track.
        issue_data_command(&mut bus, 0x46, 0, 0, 9, SIZE_CODE_1024, 9);

        let result = read_result(&mut bus);
        assert_eq!(result[0] & 0x40, 0x40, "ST0 abnormal termination");
        assert_eq!(result[1] & 0x01, 0x01, "ST1 missing address mark");
    }

    #[test]
    fn dma_read_raises_main_irq_11() {
        let mut bus = test_bus();
        bus.insert_floppy(0, hd_track0_image(8), None);
        setup_dma(&mut bus, DMA_TARGET, HD_SECTOR_SIZE as u16);
        issue_data_command(&mut bus, 0x46, 0, 0, 2, SIZE_CODE_1024, 8);

        assert!(
            !bus.pic.has_pending_irq(),
            "completion interrupt is deferred until the event fires"
        );
        fire_fdc_completion(&mut bus, Event88Va::FdcResultComplete);
        assert!(bus.pic.has_pending_irq());
        assert_eq!(bus.pic.acknowledge(), FDC_IRQ_VECTOR);
    }

    #[test]
    fn dma_write_transfers_memory_to_sector() {
        let mut bus = test_bus();
        bus.insert_floppy(0, hd_track0_image(8), None);
        for index in 0..HD_SECTOR_SIZE {
            bus.write_byte(DMA_TARGET + index as u32, 0xC0u8.wrapping_add(index as u8));
        }
        setup_dma(&mut bus, DMA_TARGET, HD_SECTOR_SIZE as u16);
        issue_data_command(&mut bus, 0x45, 0, 0, 3, SIZE_CODE_1024, 8);

        let written = bus
            .floppy
            .read_sector_data(0, 0, 0, 0, 3, SIZE_CODE_1024)
            .expect("sector 3 exists")
            .to_vec();
        for (index, byte) in written.iter().enumerate() {
            assert_eq!(
                *byte,
                0xC0u8.wrapping_add(index as u8),
                "written byte {index}"
            );
        }
    }

    #[test]
    fn recalibrate_raises_main_irq_in_dma_mode() {
        let mut bus = test_bus();
        bus.insert_floppy(0, hd_track0_image(8), None);
        bus.io_write(0x1B0, 0x01); // DMA mode
        isolate_fdc_irq(&mut bus);

        // SPECIFY timing, then RECALIBRATE drive 0.
        for byte in [0x03, 0xDF, 0x30] {
            bus.io_write(0x1BA, byte);
        }
        bus.io_write(0x1BA, 0x07);
        bus.io_write(0x1BA, 0x00);

        assert!(!bus.pic.has_pending_irq(), "seek interrupt is deferred");
        fire_fdc_completion(&mut bus, Event88Va::FdcSeekComplete);
        assert!(bus.pic.has_pending_irq());
        assert_eq!(bus.pic.acknowledge(), FDC_IRQ_VECTOR);

        // The ISR's SENSE INTERRUPT STATUS returns seek-end ST0 and PCN 0.
        bus.io_write(0x1BA, 0x08);
        let st0 = bus.io_read(0x1BA);
        let pcn = bus.io_read(0x1BA);
        assert_eq!(st0 & 0x20, 0x20, "ST0 seek-end set");
        assert_eq!(pcn, 0, "recalibrated to track 0");
    }

    #[test]
    fn sub_irq_gated_off_in_dma_mode() {
        let mut bus = test_bus();
        bus.fdc.state.interrupt_pending = true;

        bus.io_write(0x1B0, 0x01); // DMA mode: the main CPU owns the FDC
        assert!(
            !bus.sub_irq_pending(),
            "sub-CPU must not consume the main-path FDC interrupt"
        );

        bus.io_write(0x1B0, 0x00); // PIO mode: the sub-CPU owns the FDC again
        assert!(bus.sub_irq_pending());
    }
}
