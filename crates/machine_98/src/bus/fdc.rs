use common::{
    StackVec, TraceContext, TraceDeviceEvent, TraceEvent, TraceEventKey, TraceField, TraceValue,
    trace_id,
};
use device::upd765a_fdc::{FdcCommand, ST0_NOT_READY, ST1_MISSING_ADDRESS_MARK, ST1_NOT_WRITABLE};

use crate::{Pc9801Bus, TraceSink, bus::INTERRUPT_DELAY_CYCLES, scheduler::Event98};

impl<T: TraceSink> Pc9801Bus<T> {
    pub(super) fn handle_fdc_execution(&mut self) {
        let command = self.floppy.active_fdc().state.active_command;
        match command {
            FdcCommand::ReadData => self.handle_fdc_read_data(),
            FdcCommand::ReadId => self.handle_fdc_read_id(),
            FdcCommand::WriteData => self.handle_fdc_write_data(),
            FdcCommand::FormatTrack => self.handle_fdc_format_track(),
            FdcCommand::Scan => unreachable!("SCAN commands are disabled on this FDC"),
            FdcCommand::None => {}
        }
    }

    fn handle_fdc_read_data(&mut self) {
        let drive = self.floppy.active_fdc().current_drive();
        let track_index = self.floppy.active_fdc().current_track_index();
        {
            let fdc = self.floppy.active_fdc();
            if T::ENABLED
                && self.tracer.interested(TraceEventKey::Device {
                    device: trace_id::device::PC98_FDC,
                    action: trace_id::action::READ,
                })
            {
                self.tracer.trace(
                    TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.clocks.cpu_clock_hz)),
                    ),
                    TraceEvent::Device(TraceDeviceEvent {
                        device: trace_id::device::PC98_FDC,
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
                                value: TraceValue::Unsigned(u64::from(fdc.state.c)),
                            },
                            TraceField {
                                name: trace_id::field::HEAD,
                                value: TraceValue::Unsigned(u64::from(fdc.state.h)),
                            },
                            TraceField {
                                name: trace_id::field::RECORD,
                                value: TraceValue::Unsigned(u64::from(fdc.state.r)),
                            },
                            TraceField {
                                name: trace_id::field::SIZE_CODE,
                                value: TraceValue::Unsigned(u64::from(fdc.state.n)),
                            },
                        ],
                    }),
                );
            }
        }

        if !self.floppy.has_drive(drive) {
            self.floppy
                .active_fdc_mut()
                .complete_error(ST0_NOT_READY, 0x00, 0x00);
        } else if !self.floppy.density_matches(drive) {
            self.floppy
                .active_fdc_mut()
                .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00);
        } else {
            let mask_20bit = self.dma_access_ctrl & 0x04 != 0;
            let dma_channel = self.floppy.dma_channel();

            loop {
                let active_fdc = self.floppy.active_fdc_mut();
                let c = active_fdc.state.c;
                let h = active_fdc.state.h;
                let r = active_fdc.state.r;
                let n = active_fdc.state.n;

                let sector_data = self.floppy.read_sector_data(drive, track_index, c, h, r, n);

                match sector_data {
                    Some(data) => {
                        let dma_result = self.dma.transfer_write_to_memory(dma_channel, data);

                        for (addr, byte) in &dma_result.writes {
                            let addr = if mask_20bit { *addr & 0xF_FFFF } else { *addr };
                            self.memory.write_byte(addr, *byte);
                        }

                        let active_fdc = self.floppy.active_fdc_mut();

                        if dma_result.terminal_count {
                            active_fdc.signal_terminal_count();
                            active_fdc.advance_sector();
                            active_fdc.complete_success();
                            break;
                        }

                        let eot_reached = active_fdc.advance_sector();
                        if eot_reached {
                            active_fdc.complete_success();
                            break;
                        }
                    }
                    None => {
                        self.floppy.active_fdc_mut().complete_error(
                            0x00,
                            ST1_MISSING_ADDRESS_MARK,
                            0x00,
                        );
                        break;
                    }
                }
            }
        }

        self.scheduler.schedule(
            Event98::FdcInterrupt,
            self.current_cycle + INTERRUPT_DELAY_CYCLES,
        );
        self.update_next_event_cycle();
    }

    fn handle_fdc_read_id(&mut self) {
        let drive = self.floppy.active_fdc().current_drive();

        if !self.floppy.has_drive(drive) {
            self.floppy
                .active_fdc_mut()
                .complete_error(ST0_NOT_READY, 0x00, 0x00);
        } else if !self.floppy.density_matches(drive) {
            self.floppy
                .active_fdc_mut()
                .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00);
        } else {
            let track_index = self.floppy.active_fdc().current_track_index();
            let crcn = self.floppy.active_fdc().state.crcn as usize;

            let sector_info = self.floppy.read_id_at_index(drive, track_index, crcn);

            match sector_info {
                Some((c, h, r, n)) => {
                    let sector_count = self.floppy.sector_count(drive, track_index);

                    let active_fdc = self.floppy.active_fdc_mut();

                    active_fdc.provide_read_id(c, h, r, n);

                    let next_crcn = if sector_count > 0 {
                        ((crcn + 1) % sector_count) as u8
                    } else {
                        0
                    };
                    active_fdc.state.crcn = next_crcn;
                    active_fdc.complete_success();
                }
                None => {
                    self.floppy.active_fdc_mut().complete_error(
                        0x00,
                        ST1_MISSING_ADDRESS_MARK,
                        0x00,
                    );
                }
            }
        }

        self.scheduler.schedule(
            Event98::FdcInterrupt,
            self.current_cycle + INTERRUPT_DELAY_CYCLES,
        );
        self.update_next_event_cycle();
    }

    fn handle_fdc_write_data(&mut self) {
        let drive = self.floppy.active_fdc().current_drive();
        let track_index = self.floppy.active_fdc().current_track_index();

        if !self.floppy.has_drive(drive) {
            self.floppy
                .active_fdc_mut()
                .complete_error(ST0_NOT_READY, 0x00, 0x00);
        } else if self.floppy.is_write_protected(drive) {
            self.floppy
                .active_fdc_mut()
                .complete_error(0x00, ST1_NOT_WRITABLE, 0x00);
        } else if !self.floppy.density_matches(drive) {
            self.floppy
                .active_fdc_mut()
                .complete_error(0x00, ST1_MISSING_ADDRESS_MARK, 0x00);
        } else {
            let mask_20bit = self.dma_access_ctrl & 0x04 != 0;
            let dma_channel = self.floppy.dma_channel();

            loop {
                let active_fdc = self.floppy.active_fdc_mut();

                let c = active_fdc.state.c;
                let h = active_fdc.state.h;
                let r = active_fdc.state.r;
                let n = active_fdc.state.n;

                let sector_size = 128usize << (n as usize).min(7);

                let sector_exists = self
                    .floppy
                    .read_sector_data(drive, track_index, c, h, r, n)
                    .is_some();

                if !sector_exists {
                    self.floppy.active_fdc_mut().complete_error(
                        0x00,
                        ST1_MISSING_ADDRESS_MARK,
                        0x00,
                    );
                    break;
                }

                let dma_result = self.dma.transfer_read_from_memory(dma_channel, sector_size);

                let mut sector_data = Vec::with_capacity(dma_result.addresses.len());
                for &addr in &dma_result.addresses {
                    let addr = if mask_20bit { addr & 0xF_FFFF } else { addr };
                    sector_data.push(self.memory.read_byte(addr));
                }

                self.floppy
                    .write_sector_data(drive, track_index, c, h, r, n, &sector_data);

                let active_fdc = self.floppy.active_fdc_mut();

                if dma_result.terminal_count {
                    active_fdc.signal_terminal_count();
                    active_fdc.advance_sector();
                    active_fdc.complete_success();
                    break;
                }

                let eot_reached = active_fdc.advance_sector();
                if eot_reached {
                    active_fdc.complete_success();
                    break;
                }
            }
        }

        self.scheduler.schedule(
            Event98::FdcInterrupt,
            self.current_cycle + INTERRUPT_DELAY_CYCLES,
        );
        self.update_next_event_cycle();
    }

    fn handle_fdc_format_track(&mut self) {
        let drive = self.floppy.active_fdc().current_drive();
        let track_index = self.floppy.active_fdc().current_track_index();

        if !self.floppy.has_drive(drive) {
            self.floppy
                .active_fdc_mut()
                .complete_error(ST0_NOT_READY, 0x00, 0x00);
        } else if self.floppy.is_write_protected(drive) {
            self.floppy
                .active_fdc_mut()
                .complete_error(0x00, ST1_NOT_WRITABLE, 0x00);
        } else {
            let mask_20bit = self.dma_access_ctrl & 0x04 != 0;
            let dma_channel = self.floppy.dma_channel();
            let data_n = self.floppy.active_fdc().state.n;
            let sector_count = self.floppy.active_fdc().state.eot as usize;
            let fill_byte = self.floppy.active_fdc().state.dtl;

            // PC-98 floppy formats have at most 26 sectors per track.
            let mut chrn: StackVec<(u8, u8, u8, u8), 26> = StackVec::new();
            for _ in 0..sector_count.min(26) {
                let dma_result = self.dma.transfer_read_from_memory(dma_channel, 4);
                let mut id = [0u8; 4];
                for (i, &addr) in dma_result.addresses.iter().enumerate() {
                    let addr = if mask_20bit { addr & 0xF_FFFF } else { addr };
                    id[i] = self.memory.read_byte(addr);
                }
                chrn.push((id[0], id[1], id[2], id[3]));
            }

            self.floppy
                .format_track(drive, track_index, &chrn, data_n, fill_byte);
            self.floppy.active_fdc_mut().complete_success();
        }

        self.scheduler.schedule(
            Event98::FdcInterrupt,
            self.current_cycle + INTERRUPT_DELAY_CYCLES,
        );
        self.update_next_event_cycle();
    }

    pub(super) fn handle_fdc_interrupt(&mut self) {
        let irq = self.floppy.irq_line();
        if self.floppy.active_fdc_mut().take_interrupt_pending() {
            self.raise_pic_irq(irq);
        }
    }
}
