//! YM2151 OPM register access, timer scheduling, IRQ routing, and audio
//! mixing.

use common::Tracing;
use device::opn_fm::FmTimerAction;

use super::X68kBus;
use crate::scheduler::EventX68k;

/// OPM master clock of 4 MHz.
pub(super) const OPM_CLOCK_HZ: u32 = 4_000_000;

/// Offset of the OPM address port within its four-byte mirror.
const OPM_ADDRESS_PORT_OFFSET: u32 = 1;
/// Offset of the OPM data and status port within its four-byte mirror.
const OPM_DATA_PORT_OFFSET: u32 = 3;
/// CT output bit controlling the FDC forced-ready line.
const OPM_CT_FDC_FORCED_READY: u8 = 0x01;
/// CT output bit selecting the MSM6258 4 MHz master clock.
const OPM_CT_ADPCM_CLOCK_LOW: u8 = 0x02;

impl<T: Tracing> X68kBus<T> {
    /// Reads an OPM register byte at an odd address.
    pub(super) fn read_opm_register(&mut self, address: u32) -> u8 {
        if address & 3 == OPM_DATA_PORT_OFFSET {
            let value = self.opm.read_status(self.current_cycle);
            self.apply_sound_timers();
            value
        } else {
            0xFF
        }
    }

    /// Writes an OPM register byte at an odd address.
    pub(super) fn write_opm_register(&mut self, address: u32, value: u8) {
        match address & 3 {
            OPM_ADDRESS_PORT_OFFSET => self.opm.write_address(value, self.current_cycle),
            OPM_DATA_PORT_OFFSET => self.opm.write_data(value, self.current_cycle),
            _ => return,
        }
        self.apply_sound_timers();
        self.apply_opm_ct_outputs();
    }

    /// Applies an expired OPM timer and its follow-up scheduling.
    pub(super) fn on_opm_timer_expired(&mut self, timer_id: u32, fire_cycle: u64) {
        self.opm.timer_expired(timer_id, fire_cycle);
        self.apply_sound_timers();
    }

    /// Drains pending OPM timer requests onto the scheduler and routes the
    /// OPM IRQ edge to MFP GPIP3 (active low).
    pub(super) fn apply_sound_timers(&mut self) {
        let timers: [Option<FmTimerAction>; 2] = {
            let actions = self.opm.drain_timers();
            let mut pending = [None, None];
            for (slot, action) in pending.iter_mut().zip(actions.iter()) {
                *slot = Some(*action);
            }
            pending
        };
        for action in timers.into_iter().flatten() {
            let (timer_id, fire_cycle) = match action {
                FmTimerAction::Schedule {
                    timer_id,
                    fire_cycle,
                } => (timer_id, Some(fire_cycle)),
                FmTimerAction::Cancel { timer_id } => (timer_id, None),
            };
            let kind = if timer_id == 0 {
                EventX68k::OpmTimerA
            } else {
                EventX68k::OpmTimerB
            };
            match fire_cycle {
                Some(cycle) => self
                    .scheduler
                    .schedule(kind, cycle.max(self.current_cycle + 1)),
                None => self.scheduler.cancel(kind),
            }
        }
        if self.opm.take_irq_change().is_some() {
            self.update_device_pins();
            self.schedule_events();
        }
    }

    /// Routes latched OPM CT output changes to their board functions.
    fn apply_opm_ct_outputs(&mut self) {
        let Some(ct) = self.opm.take_ct_change() else {
            return;
        };
        self.set_fdc_forced_ready(ct & OPM_CT_FDC_FORCED_READY != 0);
        let old_period = self.adpcm_byte_period();
        self.adpcm.set_clock_low(ct & OPM_CT_ADPCM_CLOCK_LOW != 0);
        self.retime_adpcm_byte_event(old_period);
    }

    /// Generates one frame of mixed motherboard audio into `output`
    /// (interleaved stereo), returning the number of samples written.
    pub fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        output.fill(0.0);
        let cpu_clock_hz = self.cpu_clock_hz as u32;
        self.opm
            .generate_samples(self.current_cycle, cpu_clock_hz, volume, output);
        self.apply_sound_timers();
        self.adpcm.generate_samples(volume, output);
        self.spc.generate_cd_audio_samples([volume, volume], output);
        #[cfg(feature = "mt32")]
        if let Some(ref mt32) = self.mt32 {
            mt32.exchange(volume, output, |buffer| {
                if let Some(chip) = self.midi_card.as_mut() {
                    chip.flush_midi_into(buffer);
                }
            });
        }
        #[cfg(feature = "sc55")]
        if let Some(ref sc55) = self.sc55 {
            sc55.exchange(volume, output, |buffer| {
                if let Some(chip) = self.midi_card.as_mut() {
                    chip.flush_midi_into(buffer);
                }
            });
        }
        output.len() & !1
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, M68000AccessSize, M68000FunctionCode};

    use crate::{
        X68kModel,
        bus::test_support::{access, bus, read_word},
    };

    const OPM_ADDRESS_PORT: u32 = 0xE90001;
    const OPM_DATA_PORT: u32 = 0xE90003;

    fn write_byte(bus: &mut crate::X68kBus, address: u32, value: u8) {
        bus.m68000_write(
            access(
                address,
                M68000AccessSize::Byte,
                M68000FunctionCode::SupervisorData,
            ),
            u16::from(value),
        )
        .unwrap();
    }

    fn read_byte(bus: &mut crate::X68kBus, address: u32) -> u8 {
        bus.m68000_read(access(
            address,
            M68000AccessSize::Byte,
            M68000FunctionCode::SupervisorData,
        ))
        .unwrap() as u8
    }

    fn write_opm(bus: &mut crate::X68kBus, register: u8, value: u8) {
        write_byte(bus, OPM_ADDRESS_PORT, register);
        let settle_cycle = bus.current_cycle() + 200;
        bus.set_current_cycle(settle_cycle);
        write_byte(bus, OPM_DATA_PORT, value);
        let settle_cycle = bus.current_cycle() + 200;
        bus.set_current_cycle(settle_cycle);
    }

    #[test]
    fn status_reads_report_the_busy_window() {
        let mut bus = bus(X68kModel::X68000);
        assert_eq!(read_byte(&mut bus, OPM_DATA_PORT), 0);
        write_byte(&mut bus, OPM_ADDRESS_PORT, 0x08);
        write_byte(&mut bus, OPM_DATA_PORT, 0x00);
        assert_eq!(read_byte(&mut bus, OPM_DATA_PORT) & 0x80, 0x80);
        let settled_cycle = bus.current_cycle() + 400;
        bus.set_current_cycle(settled_cycle);
        assert_eq!(read_byte(&mut bus, OPM_DATA_PORT) & 0x80, 0);
    }

    #[test]
    fn address_port_reads_return_open_bus() {
        let mut bus = bus(X68kModel::X68000);
        assert_eq!(read_byte(&mut bus, OPM_ADDRESS_PORT), 0xFF);
        assert_eq!(read_byte(&mut bus, 0xE90005), 0xFF);
    }

    #[test]
    fn word_reads_pair_open_upper_lane_with_the_status() {
        let mut bus = bus(X68kModel::X68000);
        assert_eq!(read_word(&mut bus, 0xE90002), 0xFF00);
    }

    #[test]
    fn registers_mirror_across_the_window() {
        let mut bus = bus(X68kModel::X68000);
        assert_eq!(read_byte(&mut bus, 0xE91FFF), 0);
        write_byte(&mut bus, 0xE91FFD, 0x14);
        write_byte(&mut bus, 0xE91FFF, 0x00);
        assert_eq!(read_byte(&mut bus, 0xE91FFF) & 0x80, 0x80);
    }

    #[test]
    fn even_byte_accesses_raise_bus_errors() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        assert!(
            bus.m68000_read(access(0xE90000, M68000AccessSize::Byte, supervisor))
                .is_err()
        );
        assert!(
            bus.m68000_write(access(0xE90002, M68000AccessSize::Byte, supervisor), 0)
                .is_err()
        );
    }

    #[test]
    fn timer_a_interrupts_through_mfp_gpip3() {
        let mut bus = bus(X68kModel::X68000);
        for (address, value) in [(0xE88009, 0x08), (0xE88015, 0x08), (0xE88017, 0x40)] {
            write_byte(&mut bus, address, value);
        }
        write_opm(&mut bus, 0x10, 0xFF);
        write_opm(&mut bus, 0x11, 0x03);
        write_opm(&mut bus, 0x14, 0x05);
        assert_eq!(bus.m68000_interrupt_level(), 0);

        let expired_cycle = bus.current_cycle() + 20_000;
        bus.set_current_cycle(expired_cycle);
        bus.process_due_events();
        assert_eq!(read_byte(&mut bus, OPM_DATA_PORT) & 0x03, 0x01);
        assert_eq!(bus.m68000_interrupt_level(), 6);
        assert_eq!(bus.m68000_acknowledge_interrupt(6), 0x43);

        write_opm(&mut bus, 0x14, 0x10);
        assert_eq!(read_byte(&mut bus, OPM_DATA_PORT) & 0x03, 0);
        assert_eq!(bus.m68000_interrupt_level(), 0);
    }

    #[test]
    fn keyed_channel_produces_audio_and_idle_output_stays_silent() {
        let mut bus = bus(X68kModel::X68000);
        let mut output = [0.5; 1024];
        assert_eq!(bus.generate_audio_samples(1.0, &mut output), 1024);
        assert!(output.iter().all(|sample| *sample == 0.0));

        for (register, value) in [
            (0x20, 0xC7),
            (0x40, 0x01),
            (0x60, 0x00),
            (0x80, 0x1F),
            (0xA0, 0x00),
            (0xC0, 0x00),
            (0xE0, 0x0F),
            (0x28, 0x4A),
            (0x08, 0x78),
        ] {
            write_opm(&mut bus, register, value);
        }
        let elapsed_cycle = bus.current_cycle() + 200_000;
        bus.set_current_cycle(elapsed_cycle);
        let written = bus.generate_audio_samples(1.0, &mut output);
        assert_eq!(written, 1024);
        assert!(output.iter().any(|sample| *sample != 0.0));
    }

    #[test]
    fn ct_outputs_latch_the_adpcm_clock_selection() {
        let mut bus = bus(X68kModel::X68000);
        assert_eq!(bus.adpcm.master_clock_hz(), 8_000_000);
        write_opm(&mut bus, 0x1B, 0x80);
        assert_eq!(bus.adpcm.master_clock_hz(), 4_000_000);
        write_opm(&mut bus, 0x1B, 0x00);
        assert_eq!(bus.adpcm.master_clock_hz(), 8_000_000);
    }
}
