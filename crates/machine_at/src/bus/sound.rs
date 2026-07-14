//! AT sound glue: Sound Blaster 16 (0x220-0x22F), OPL3 AdLib alias
//! (0x388-0x38B), and MPU-401 (0x330-0x331).
//!
//! The SB16 DSP moves PCM through the immediate-batch DMA model shared with the
//! FDC: a `StartDma` action arms the periodic `Sb16DspDma` event, each firing
//! moves one batch of bytes to or from memory on the configured channel (8-bit
//! channel 1 or 16-bit channel 5), and terminal count raises IRQ 5. The MPU-401
//! intelligent-mode timer runs on the `MpuTimer` event and raises IRQ 9.

use common::{StackVec, TraceSink};
use device::sound_blaster_16::{
    DMA_BATCH_SIZE, SoundboardSb16Action, SoundboardSb16Timer, dma_format_bytes_per_sample,
    dma_format_is_16bit,
};

use crate::{
    bus::{AtBus, IRQ_MPU, OPEN_BUS_BYTE},
    scheduler::EventAt,
};

impl<T: TraceSink> AtBus<T> {
    /// Reads a Sound Blaster 16 or OPL3 register (ports 0x220-0x22F, 0x388-0x38B).
    pub(super) fn sound_io_read(&mut self, port: u16) -> u8 {
        match port {
            // OPL3 status (bank-0 and bank-1 addresses both expose it).
            0x0220 | 0x0222 | 0x0228 | 0x0388 | 0x038A => {
                let value = self.sound_blaster_16.read_opl3_status(self.current_cycle);
                self.process_sb16_actions();
                value
            }
            // Mixer address / data.
            0x0224 => self.sound_blaster_16.read_mixer_address(),
            0x0225 => self.sound_blaster_16.read_mixer_data(),
            // DSP reset readback.
            0x0226 => self.sound_blaster_16.read_dsp_reset(),
            // DSP read data.
            0x022A => self.sound_blaster_16.read_dsp_data(),
            // DSP write-buffer status.
            0x022C => self.sound_blaster_16.read_dsp_write_status(),
            // DSP read-buffer status / 8-bit IRQ acknowledge.
            0x022E => {
                let value = self.sound_blaster_16.read_dsp_status_8bit();
                self.process_sb16_actions();
                value
            }
            // DSP 16-bit IRQ acknowledge.
            0x022F => {
                let value = self.sound_blaster_16.read_dsp_status_16bit();
                self.process_sb16_actions();
                value
            }
            _ => OPEN_BUS_BYTE,
        }
    }

    /// Writes a Sound Blaster 16 or OPL3 register (ports 0x220-0x22F, 0x388-0x38B).
    pub(super) fn sound_io_write(&mut self, port: u16, value: u8) {
        match port {
            // OPL3 bank-0 address (native FM and AdLib mirrors).
            0x0220 | 0x0228 | 0x0388 => {
                self.sound_blaster_16
                    .write_opl3_address_lo(value, self.current_cycle);
                self.process_sb16_actions();
            }
            // OPL3 bank-0 data.
            0x0221 | 0x0229 | 0x0389 => {
                self.sound_blaster_16
                    .write_opl3_data(value, self.current_cycle);
                self.process_sb16_actions();
            }
            // OPL3 bank-1 address.
            0x0222 | 0x038A => {
                self.sound_blaster_16
                    .write_opl3_address_hi(value, self.current_cycle);
                self.process_sb16_actions();
            }
            // OPL3 bank-1 data.
            0x0223 | 0x038B => {
                self.sound_blaster_16
                    .write_opl3_data(value, self.current_cycle);
                self.process_sb16_actions();
            }
            // Mixer address / data.
            0x0224 => self.sound_blaster_16.write_mixer_address(value),
            0x0225 => {
                self.sound_blaster_16.write_mixer_data(value);
                self.process_sb16_actions();
            }
            // DSP reset.
            0x0226 => {
                self.sound_blaster_16.write_dsp_reset(value);
                self.process_sb16_actions();
            }
            // DSP write command / data.
            0x022C => {
                self.sound_blaster_16.write_dsp_command(value);
                self.process_sb16_actions();
            }
            _ => {}
        }
    }

    /// Reads an MPU-401 register (ports 0x330 data, 0x331 status).
    pub(super) fn mpu_io_read(&mut self, port: u16) -> u8 {
        match port {
            0x0330 => {
                let value = self.mpu401.read_data();
                self.sync_mpu_irq_and_timer();
                value
            }
            0x0331 => self.mpu401.read_status(),
            _ => OPEN_BUS_BYTE,
        }
    }

    /// Writes an MPU-401 register (ports 0x330 data, 0x331 command).
    pub(super) fn mpu_io_write(&mut self, port: u16, value: u8) {
        match port {
            0x0330 => {
                self.mpu401.write_data(value);
                self.sync_mpu_irq_and_timer();
            }
            0x0331 => {
                self.mpu401.write_command(value);
                self.sync_mpu_irq_and_timer();
            }
            _ => {}
        }
    }

    /// Drains and applies the actions the SB16 emitted (timers, IRQ, DMA).
    pub(crate) fn process_sb16_actions(&mut self) {
        let mut timer_updates: StackVec<(EventAt, Option<u64>), 4> = StackVec::new();
        let mut irq_updates: StackVec<(u8, bool), 8> = StackVec::new();
        let mut start_dma = false;
        let mut stop_dma = false;

        for action in self.sound_blaster_16.drain_actions() {
            match *action {
                SoundboardSb16Action::ScheduleTimer { kind, fire_cycle } => {
                    timer_updates.push((EventAt::from(kind), Some(fire_cycle)));
                }
                SoundboardSb16Action::CancelTimer { kind } => {
                    timer_updates.push((EventAt::from(kind), None));
                }
                SoundboardSb16Action::AssertIrq { irq } => irq_updates.push((irq, true)),
                SoundboardSb16Action::DeassertIrq { irq } => irq_updates.push((irq, false)),
                SoundboardSb16Action::StartDma { channel: _ } => start_dma = true,
                SoundboardSb16Action::StopDma => stop_dma = true,
            }
        }

        for (kind, fire) in timer_updates.iter() {
            match fire {
                Some(cycle) => self.scheduler.schedule(*kind, *cycle),
                None => self.scheduler.cancel(*kind),
            }
        }
        for (irq, assert) in irq_updates.iter() {
            if *assert {
                self.raise_irq(*irq);
            } else {
                self.clear_irq(*irq);
            }
        }
        if stop_dma {
            self.scheduler.cancel(EventAt::Sb16DspDma);
        }
        if start_dma {
            self.schedule_sb16_dma(self.current_cycle);
        }

        self.update_next_event_cycle();
    }

    /// Schedules the next SB16 DSP DMA batch relative to `reference_cycle`.
    fn schedule_sb16_dma(&mut self, reference_cycle: u64) {
        let sample_rate = self.sound_blaster_16.state.dsp.sample_rate.max(1) as u64;
        let dma_format = self.sound_blaster_16.state.dsp.dma_format;
        let bytes_per_sample = dma_format_bytes_per_sample(dma_format).max(1) as u64;
        let byte_rate = sample_rate * bytes_per_sample;
        let interval_cycles =
            DMA_BATCH_SIZE as u64 * self.clocks.cpu_clock_hz as u64 / byte_rate.max(1);
        let fire_cycle = (reference_cycle + interval_cycles.max(1)).max(self.current_cycle + 1);
        self.scheduler.schedule(EventAt::Sb16DspDma, fire_cycle);
    }

    /// Services one SB16 DSP DMA batch when the `Sb16DspDma` event fires.
    pub(super) fn handle_sb16_dma_transfer(&mut self, event_fire_cycle: u64) {
        if !self.sound_blaster_16.dma_transfer_pending() {
            return;
        }

        let channel = self.sound_blaster_16.state.dsp.dma_channel as usize;
        let is_recording = self.sound_blaster_16.state.dsp.dma_is_recording;
        let dma_format = self.sound_blaster_16.state.dsp.dma_format;

        if is_recording {
            // Recording: write silence into memory through the DMA controller.
            let silence_byte = if dma_format_is_16bit(dma_format) {
                0x00u8
            } else {
                0x80u8
            };
            let silence = [silence_byte; DMA_BATCH_SIZE];
            let result = self.dma.transfer_write_to_memory(channel, &silence);
            for &(address, byte) in &result.writes {
                self.memory.write_physical(address, byte);
            }
            self.sound_blaster_16
                .advance_dma_recording(result.writes.len() as u32);
            if result.terminal_count {
                self.sound_blaster_16.dma_terminal_count();
            }
        } else {
            // Playback: read PCM from memory through the DMA controller.
            let result = self.dma.transfer_read_from_memory(channel, DMA_BATCH_SIZE);
            let mut data: StackVec<u8, DMA_BATCH_SIZE> = StackVec::new();
            for &address in &result.addresses {
                data.push(self.memory.read_physical(address));
            }
            self.sound_blaster_16.accept_dma_data(&data);
            if result.terminal_count {
                self.sound_blaster_16.dma_terminal_count();
            }
        }

        self.process_sb16_actions();

        // Reschedule relative to the event fire cycle to avoid drift.
        if self.sound_blaster_16.dma_transfer_pending() {
            self.schedule_sb16_dma(event_fire_cycle);
            self.update_next_event_cycle();
        }
    }

    /// Notifies the SB16 that one OPL3 timer expired and applies the actions.
    pub(super) fn handle_sb16_opl_timer(&mut self, timer: SoundboardSb16Timer) {
        let timer_id = match timer {
            SoundboardSb16Timer::OplTimerA => 0,
            SoundboardSb16Timer::OplTimerB => 1,
        };
        self.sound_blaster_16
            .timer_expired(timer_id, self.current_cycle);
        self.process_sb16_actions();
    }

    /// Advances the MPU-401 intelligent-mode timer and reschedules it.
    pub(super) fn handle_mpu_timer(&mut self) {
        let reschedule = self.mpu401.tick();
        if self.mpu401.take_irq() {
            self.raise_irq(IRQ_MPU);
        }
        if reschedule {
            let step_cycles = self.mpu401.step_clock_cycles(self.clocks.cpu_clock_hz);
            self.scheduler
                .schedule(EventAt::MpuTimer, self.current_cycle + step_cycles);
        }
        self.update_next_event_cycle();
    }

    /// Syncs the MPU-401 IRQ line and timer event after a port access.
    fn sync_mpu_irq_and_timer(&mut self) {
        if self.mpu401.take_irq() {
            self.raise_irq(IRQ_MPU);
        } else {
            self.clear_irq(IRQ_MPU);
        }
        if self.mpu401.timer_active()
            && self.scheduler.state.fire_cycles[EventAt::MpuTimer as usize].is_none()
        {
            let step_cycles = self.mpu401.step_clock_cycles(self.clocks.cpu_clock_hz);
            self.scheduler
                .schedule(EventAt::MpuTimer, self.current_cycle + step_cycles);
        }
        if !self.mpu401.timer_active() {
            self.scheduler.cancel(EventAt::MpuTimer);
        }
        self.update_next_event_cycle();
    }
}
