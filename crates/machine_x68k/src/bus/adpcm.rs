//! MSM6258 ADPCM glue: registers, DMAC channel-3 requests, and byte pacing.
//!
//! Playback runs on a scheduled byte cadence at twice the selected sampling
//! period. Each tick decodes the byte delivered by DMAC channel 3 (or repeats
//! the held sample on an under-run) and immediately requests the next byte,
//! mirroring the chip's data-request handshake.

use common::TraceSink;
use device::msm6258::Msm6258Command;

use super::X68kBus;
use crate::scheduler::EventX68k;

/// Offset of the ADPCM command and status port within its four-byte mirror.
const ADPCM_COMMAND_PORT_OFFSET: u32 = 1;
/// Offset of the ADPCM data port within its four-byte mirror.
const ADPCM_DATA_PORT_OFFSET: u32 = 3;
/// DMAC channel connected to the ADPCM data request line.
const ADPCM_DMAC_CHANNEL: usize = 3;

impl<T: TraceSink> X68kBus<T> {
    /// Reads an ADPCM register byte at an odd address.
    pub(super) fn read_adpcm_register(&mut self, address: u32) -> u8 {
        if address & 3 == ADPCM_COMMAND_PORT_OFFSET {
            self.adpcm.read_status()
        } else {
            0xFF
        }
    }

    /// Writes an ADPCM register byte at an odd address.
    pub(super) fn write_adpcm_register(&mut self, address: u32, value: u8) {
        match address & 3 {
            ADPCM_COMMAND_PORT_OFFSET => match self.adpcm.write_command(value) {
                Msm6258Command::Started => {
                    self.adpcm_cycle_remainder = 0;
                    if self.adpcm.divider_ratio().is_some() {
                        self.schedule_adpcm_byte_event();
                        self.request_adpcm_byte();
                    } else {
                        self.dmac
                            .set_peripheral_control_line(ADPCM_DMAC_CHANNEL, true);
                    }
                }
                Msm6258Command::Stopped => {
                    self.scheduler.cancel(EventX68k::Adpcm);
                    self.dmac
                        .set_peripheral_control_line(ADPCM_DMAC_CHANNEL, true);
                }
                Msm6258Command::Unchanged => {}
            },
            ADPCM_DATA_PORT_OFFSET if self.adpcm.write_data(value) => {
                self.dmac
                    .set_peripheral_control_line(ADPCM_DMAC_CHANNEL, true);
            }
            _ => {}
        }
    }

    /// Serves one playback byte tick: decode, then request the next byte.
    /// The next tick is scheduled from `fire_cycle` so CPU overshoot and DMA
    /// stalls never stretch the sampling cadence.
    pub(super) fn on_adpcm_byte_tick(&mut self, fire_cycle: u64) {
        if !self.adpcm.consume_byte_tick() {
            return;
        }
        self.schedule_adpcm_byte_event_from(fire_cycle);
        self.request_adpcm_byte();
    }

    /// Requests the next encoded byte through DMAC channel 3.
    fn request_adpcm_byte(&mut self) {
        self.dmac
            .set_peripheral_control_line(ADPCM_DMAC_CHANNEL, false);
        if self.dmac.channel_active(ADPCM_DMAC_CHANNEL) {
            let clock = self.dmac_clock();
            let mut dmac = std::mem::take(&mut self.dmac);
            dmac.assert_request(ADPCM_DMAC_CHANNEL, self, clock);
            self.dmac = dmac;
            self.finish_dmac_activity();
        }
    }

    /// Returns the current byte-period numerator and denominator in CPU cycles.
    pub(super) fn adpcm_byte_period(&self) -> Option<(u128, u128)> {
        self.adpcm.divider_ratio().map(|divider| {
            (
                u128::from(self.cpu_clock_hz) * 2 * u128::from(divider),
                u128::from(self.adpcm.master_clock_hz()),
            )
        })
    }

    /// Applies a clock or divider change to an active byte cadence.
    pub(super) fn retime_adpcm_byte_event(&mut self, old_period: Option<(u128, u128)>) {
        let new_period = self.adpcm_byte_period();
        if old_period == new_period {
            return;
        }
        self.adpcm_cycle_remainder = 0;
        if !self.adpcm.playing() {
            return;
        }
        let effective = self
            .current_cycle
            .wrapping_add(self.wait_cycles.max(0) as u64);
        match (old_period, new_period) {
            (Some((old_numerator, old_denominator)), Some((new_numerator, new_denominator))) => {
                let Some(deadline) = self.scheduler.event_cycle(EventX68k::Adpcm) else {
                    self.schedule_adpcm_byte_event();
                    return;
                };
                let remaining = u128::from(deadline.saturating_sub(effective));
                let numerator = remaining * new_numerator * old_denominator;
                let denominator = old_numerator * new_denominator;
                let scaled = numerator.div_ceil(denominator).max(1) as u64;
                self.scheduler
                    .schedule(EventX68k::Adpcm, effective.wrapping_add(scaled));
            }
            (Some(_), None) => {
                self.scheduler.cancel(EventX68k::Adpcm);
                self.dmac
                    .set_peripheral_control_line(ADPCM_DMAC_CHANNEL, true);
            }
            (None, Some(_)) => {
                self.schedule_adpcm_byte_event();
                if !self.adpcm.data_pending() {
                    self.request_adpcm_byte();
                }
            }
            (None, None) => {}
        }
    }

    /// Schedules the next byte tick one encoded-byte period after the
    /// pending CPU stall window (used when starting a fresh cadence).
    pub(super) fn schedule_adpcm_byte_event(&mut self) {
        let effective = self
            .current_cycle
            .wrapping_add(self.wait_cycles.max(0) as u64);
        self.schedule_adpcm_byte_event_from(effective);
    }

    /// Schedules the next byte tick one encoded-byte period after
    /// `base_cycle`, carrying the fractional cycle remainder so the cadence
    /// never drifts.
    pub(super) fn schedule_adpcm_byte_event_from(&mut self, base_cycle: u64) {
        let Some((period_numerator, denominator)) = self.adpcm_byte_period() else {
            self.scheduler.cancel(EventX68k::Adpcm);
            return;
        };
        let numerator = period_numerator + u128::from(self.adpcm_cycle_remainder);
        let cycles = (numerator / denominator) as u64;
        self.adpcm_cycle_remainder = (numerator % denominator) as u64;
        self.scheduler
            .schedule(EventX68k::Adpcm, base_cycle + cycles.max(1));
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, CpuMode, M68000AccessSize, M68000FunctionCode};

    use crate::{
        X68kMachine, X68kModel,
        bus::{
            X68kBus,
            test_support::{access, bus, test_roms},
        },
        scheduler::EventX68k,
    };

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

    /// Programs DMAC channel 3 for a byte transfer from RAM to the ADPCM
    /// data port with external request generation, starting it with `ccr`.
    fn program_adpcm_dma_with_ccr(bus: &mut X68kBus, source: u32, bytes: u16, ccr: u8) {
        write_register(bus, 0xE840C4, 0x80);
        write_register(bus, 0xE840C5, 0x02);
        write_register(bus, 0xE840C6, 0x04);
        write_register(bus, 0xE840CA, (bytes >> 8) as u8);
        write_register(bus, 0xE840CB, bytes as u8);
        for (index, byte) in source.to_be_bytes().into_iter().enumerate() {
            write_register(bus, 0xE840CC + index as u32, byte);
        }
        for (index, byte) in 0x00E92003u32.to_be_bytes().into_iter().enumerate() {
            write_register(bus, 0xE840D4 + index as u32, byte);
        }
        write_register(bus, 0xE840C7, ccr);
    }

    /// Programs and starts DMAC channel 3 without a completion interrupt.
    fn program_adpcm_dma(bus: &mut X68kBus, source: u32, bytes: u16) {
        program_adpcm_dma_with_ccr(bus, source, bytes, 0x80);
    }

    /// Advances the bus through its scheduled events up to `target_cycle`,
    /// draining stall cycles the way an executing CPU would.
    fn run_events_until(bus: &mut X68kBus, target_cycle: u64) {
        while let Some(event_cycle) = bus.next_event_cycle() {
            if event_cycle > target_cycle {
                break;
            }
            bus.set_current_cycle(event_cycle);
            bus.process_due_events();
            let _ = bus.drain_wait_cycles();
        }
        bus.set_current_cycle(target_cycle);
    }

    #[test]
    fn status_and_data_ports_mirror_and_reject_even_bytes() {
        let mut bus = bus(X68kModel::X68000);
        assert_eq!(read_register(&mut bus, 0xE92001), 0x40);
        assert_eq!(read_register(&mut bus, 0xE93FFD), 0x40);
        assert_eq!(read_register(&mut bus, 0xE92003), 0xFF);
        let supervisor = M68000FunctionCode::SupervisorData;
        assert!(
            bus.m68000_read(access(0xE92000, M68000AccessSize::Byte, supervisor))
                .is_err()
        );
        assert!(
            bus.m68000_write(access(0xE92002, M68000AccessSize::Byte, supervisor), 0)
                .is_err()
        );
    }

    #[test]
    fn commands_toggle_the_playback_status_bit() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE92001, 0x02);
        assert_eq!(read_register(&mut bus, 0xE92001), 0xC0);
        write_register(&mut bus, 0xE92001, 0x01);
        assert_eq!(read_register(&mut bus, 0xE92001), 0x40);
    }

    #[test]
    fn playback_pulls_bytes_through_dmac_channel_three_at_the_data_rate() {
        let mut bus = bus(X68kModel::X68000);
        let byte_count = 32u16;
        for index in 0..byte_count as usize {
            bus.ram[0x2000 + index] = 0x11;
        }
        program_adpcm_dma(&mut bus, 0x2000, byte_count);
        assert!(bus.dmac.channel_active(3));

        let start_cycle = bus.current_cycle();
        write_register(&mut bus, 0xE92001, 0x02);
        // 15.625 kHz: one byte per 1280 cycles at 10 MHz.
        run_events_until(&mut bus, start_cycle + 1280 * u64::from(byte_count) + 640);
        assert!(
            !bus.dmac.channel_active(3),
            "the transfer count must be exhausted"
        );
        let expected_cycles = 1280 * u64::from(byte_count);
        let mut output = vec![0.0f32; 4096];
        assert_eq!(
            bus.generate_audio_samples(1.0, &mut output),
            4096,
            "audio generation must cover the transfer window"
        );
        // Port C has not enabled the output lines yet, so the decoded stream
        // stays silent even though every byte was consumed.
        assert!(output.iter().all(|sample| *sample == 0.0));
        assert!(
            bus.current_cycle() >= start_cycle + expected_cycles,
            "pacing must span the full transfer"
        );
    }

    #[test]
    fn slow_divider_paces_bytes_at_a_quarter_of_the_fast_rate() {
        let fast = pacing_cycles(false, None);
        let slow = pacing_cycles(true, Some(0x03));
        assert!(
            slow > fast * 3 && slow < fast * 5,
            "4 MHz / 1024 must run near a quarter of 8 MHz / 512: fast {fast}, slow {slow}"
        );
    }

    #[test]
    fn medium_divider_paces_bytes_at_one_and_a_half_times_the_fast_rate() {
        let fast = pacing_cycles(false, None);
        let medium = pacing_cycles(false, Some(0x07));
        assert!(
            medium > fast * 5 / 4 && medium < fast * 7 / 4,
            "8 MHz / 768 must run at 1.5 times the 8 MHz / 512 span: \
             fast {fast}, medium {medium}"
        );
    }

    #[test]
    fn divider_change_rescales_the_pending_byte_deadline() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE9A007, 0x92);
        write_register(&mut bus, 0xE9A005, 0x08);
        let _ = bus.drain_wait_cycles();
        write_register(&mut bus, 0xE92001, 0x02);
        let _ = bus.drain_wait_cycles();
        let old_deadline = bus
            .scheduler
            .event_cycle(EventX68k::Adpcm)
            .expect("playback must schedule a byte event");

        bus.set_current_cycle(old_deadline - 650);
        write_register(&mut bus, 0xE9A005, 0x00);
        let effective = bus.current_cycle;
        let new_deadline = bus
            .scheduler
            .event_cycle(EventX68k::Adpcm)
            .expect("valid divider must keep the byte event armed");
        assert_eq!(new_deadline, effective + (old_deadline - effective) * 2);
    }

    #[test]
    fn ct1_change_rescales_the_pending_byte_deadline() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE90001, 0x1B);
        let _ = bus.drain_wait_cycles();
        write_register(&mut bus, 0xE92001, 0x02);
        let _ = bus.drain_wait_cycles();
        let old_deadline = bus
            .scheduler
            .event_cycle(EventX68k::Adpcm)
            .expect("playback must schedule a byte event");

        bus.set_current_cycle(old_deadline - 650);
        write_register(&mut bus, 0xE90003, 0x80);
        let effective = bus.current_cycle;
        let new_deadline = bus
            .scheduler
            .event_cycle(EventX68k::Adpcm)
            .expect("valid clock must keep the byte event armed");
        assert_eq!(new_deadline, effective + (old_deadline - effective) * 2);
    }

    #[test]
    fn reserved_divider_pauses_and_resumes_with_the_latched_byte() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE9A007, 0x92);
        write_register(&mut bus, 0xE9A005, 0x08);
        bus.ram[0x2000] = 0x11;
        bus.ram[0x2001] = 0x22;
        program_adpcm_dma(&mut bus, 0x2000, 2);
        write_register(&mut bus, 0xE92001, 0x02);
        assert!(bus.adpcm.data_pending());
        assert!(bus.dmac.channel_active(3));

        write_register(&mut bus, 0xE9A005, 0x0C);
        assert_eq!(bus.adpcm.divider_ratio(), None);
        assert_eq!(read_register(&mut bus, 0xE92001), 0xC0);
        assert_eq!(bus.scheduler.event_cycle(EventX68k::Adpcm), None);
        assert!(bus.adpcm.data_pending());
        assert!(bus.dmac.channel_active(3));

        write_register(&mut bus, 0xE9A005, 0x08);
        assert!(bus.scheduler.event_cycle(EventX68k::Adpcm).is_some());
        assert!(bus.adpcm.data_pending());
        assert!(bus.dmac.channel_active(3));
        let deadline = bus.scheduler.event_cycle(EventX68k::Adpcm).unwrap();
        bus.set_current_cycle(deadline);
        bus.process_due_events();
        assert!(!bus.dmac.channel_active(3));
    }

    #[test]
    fn playback_started_while_inhibited_waits_for_a_valid_divider() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE9A007, 0x92);
        write_register(&mut bus, 0xE9A005, 0x0C);
        write_register(&mut bus, 0xE92001, 0x02);
        assert_eq!(read_register(&mut bus, 0xE92001), 0xC0);
        assert_eq!(bus.scheduler.event_cycle(EventX68k::Adpcm), None);
        write_register(&mut bus, 0xE9A005, 0x08);
        assert!(bus.scheduler.event_cycle(EventX68k::Adpcm).is_some());
    }

    /// Measures the cycle span needed to drain an eight-byte transfer with
    /// an optional 4 MHz master clock and port C divider selection.
    fn pacing_cycles(four_megahertz_clock: bool, port_c: Option<u8>) -> u64 {
        let mut bus = bus(X68kModel::X68000);
        if four_megahertz_clock {
            write_register(&mut bus, 0xE90001, 0x1B);
            write_register(&mut bus, 0xE90003, 0x80);
        }
        if let Some(value) = port_c {
            write_register(&mut bus, 0xE9A005, value);
        }
        for index in 0..8 {
            bus.ram[0x2000 + index] = 0x11;
        }
        program_adpcm_dma(&mut bus, 0x2000, 8);
        let start_cycle = bus.current_cycle();
        write_register(&mut bus, 0xE92001, 0x02);
        for _ in 0..1_000 {
            if !bus.dmac.channel_active(3) {
                return bus.current_cycle() - start_cycle;
            }
            let Some(event_cycle) = bus.next_event_cycle() else {
                break;
            };
            bus.set_current_cycle(event_cycle);
            bus.process_due_events();
            let _ = bus.drain_wait_cycles();
        }
        panic!("the ADPCM transfer never completed");
    }

    #[test]
    fn dmac_completion_interrupts_at_level_three_with_the_channel_vector() {
        let mut bus = bus(X68kModel::X68000);
        for index in 0..4 {
            bus.ram[0x2000 + index] = 0x11;
        }
        write_register(&mut bus, 0xE840E5, 0x72);
        program_adpcm_dma_with_ccr(&mut bus, 0x2000, 4, 0x88);
        write_register(&mut bus, 0xE92001, 0x02);
        let target = bus.current_cycle() + 1280 * 6;
        run_events_until(&mut bus, target);
        assert_eq!(bus.m68000_interrupt_level(), 3);
        assert_eq!(bus.m68000_acknowledge_interrupt(3), 0x72);
    }

    #[test]
    fn under_run_keeps_the_cadence_until_the_stop_command() {
        let mut bus = bus(X68kModel::X68000);
        write_register(&mut bus, 0xE92001, 0x02);
        let target = bus.current_cycle() + 1280 * 8;
        run_events_until(&mut bus, target);
        assert_eq!(
            read_register(&mut bus, 0xE92001),
            0xC0,
            "starved playback must stay active"
        );
        write_register(&mut bus, 0xE92001, 0x01);
        assert_eq!(read_register(&mut bus, 0xE92001), 0x40);
        let stopped_target = bus.current_cycle() + 1280 * 8;
        run_events_until(&mut bus, stopped_target);
        assert_eq!(read_register(&mut bus, 0xE92001), 0x40);
    }

    /// Boot-chain variant: the synthetic IPL programs channel 3 and starts
    /// playback, so pacing runs through the real CPU loop.
    #[test]
    fn synthetic_ipl_plays_adpcm_through_the_cpu_loop() {
        let mut loaded = test_roms(X68kModel::X68000);
        let writes: [(u32, u8); 16] = [
            (0xE840C4, 0x80),
            (0xE840C5, 0x02),
            (0xE840C6, 0x04),
            (0xE840CA, 0x00),
            (0xE840CB, 0x08),
            (0xE840CC, 0x00),
            (0xE840CD, 0x00),
            (0xE840CE, 0x20),
            (0xE840CF, 0x00),
            (0xE840D4, 0x00),
            (0xE840D5, 0xE9),
            (0xE840D6, 0x20),
            (0xE840D7, 0x03),
            (0xE840C7, 0x80),
            (0xE9A005, 0x08),
            (0xE92001, 0x02),
        ];
        let mut offset = 0x0008;
        for (address, value) in writes {
            let words = [
                0x13FC,
                u16::from(value),
                (address >> 16) as u16,
                address as u16,
            ];
            for word in words {
                loaded.ipl[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
                offset += 2;
            }
        }
        for word in [0x4E72u16, 0x2700] {
            loaded.ipl[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
            offset += 2;
        }
        let mut machine: X68kMachine =
            crate::bus::test_support::machine(X68kModel::X68000, CpuMode::High, loaded);
        for slot in machine.bus.ram[0x2000..0x2008].iter_mut() {
            *slot = 0x22;
        }
        machine.run_for(1280 * 200);
        assert!(
            !machine.bus.dmac.channel_active(3),
            "the CPU loop must drain the eight-byte transfer"
        );
        let mut output = vec![0.0f32; 4096];
        machine.bus.generate_audio_samples(1.0, &mut output);
        assert!(
            output.iter().any(|sample| *sample != 0.0),
            "decoded ADPCM audio must reach the mixed output"
        );
    }
}
