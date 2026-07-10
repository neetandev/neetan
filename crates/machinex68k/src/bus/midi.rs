//! CZ-6BM1 MIDI board glue: the YM3802 register window, level-4 interrupt
//! vectoring, and routing of transmitted bytes into an installed Roland
//! sound module.

use common::Tracing;
use device::ym3802::{YM3802_CLKM_HZ, Ym3802};

use super::X68kBus;
use crate::clock::cycle_to_tick;

impl<T: Tracing> X68kBus<T> {
    /// Reads a YM3802 register byte at an odd card address.
    pub(super) fn read_midi_register(&mut self, address: u32) -> u8 {
        self.synchronize_devices();
        let tick = cycle_to_tick(self.current_cycle, YM3802_CLKM_HZ, self.cpu_clock_hz);
        let offset = ((address >> 1) & 7) as u8;
        let value = self
            .midi_card
            .as_mut()
            .expect("validated MIDI card")
            .read_register(offset, tick);
        self.schedule_events();
        value
    }

    /// Writes a YM3802 register byte at an odd card address.
    pub(super) fn write_midi_register(&mut self, address: u32, value: u8) {
        self.synchronize_devices();
        let tick = cycle_to_tick(self.current_cycle, YM3802_CLKM_HZ, self.cpu_clock_hz);
        let offset = ((address >> 1) & 7) as u8;
        self.midi_card
            .as_mut()
            .expect("validated MIDI card")
            .write_register(offset, value, tick);
        self.schedule_events();
    }

    /// Installs the CZ-6BM1 MIDI board with transmit-byte capture enabled.
    pub fn install_midi_card(&mut self) {
        let mut chip = Ym3802::new();
        chip.enable_midi_capture();
        self.midi_card = Some(chip);
        self.schedule_events();
    }

    /// Drains captured MIDI transmit bytes into `out`.
    pub fn flush_midi_into(&mut self, out: &mut Vec<u8>) {
        if let Some(chip) = self.midi_card.as_mut() {
            chip.flush_midi_into(out);
        }
    }

    /// Installs a Roland MT-32 sound module driven by the CZ-6BM1 card.
    #[cfg(feature = "mt32")]
    pub fn install_mt32(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::mt32::MuntError> {
        self.install_midi_card();
        self.mt32 = Some(device::mt32::Mt32::new(rom_directory)?);
        Ok(())
    }

    /// Installs a Roland SC-55 sound module driven by the CZ-6BM1 card.
    #[cfg(feature = "sc55")]
    pub fn install_sc55(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::sc55::Sc55Error> {
        self.install_midi_card();
        self.sc55 = Some(device::sc55::Sc55::new(rom_directory)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, CpuMode, M68000AccessSize, M68000FunctionCode};
    use device::ym3802::YM3802_CLKM_HZ;

    use crate::{
        X68kMachine, X68kModel,
        bus::{
            X68kBus,
            test_support::{access, bus, test_roms},
        },
        clock::tick_to_cycle,
    };

    /// Interrupt-vector register address of the primary card.
    const IVR_ADDRESS: u32 = 0xEAFA01;
    /// System-control register address of the primary card.
    const RGR_ADDRESS: u32 = 0xEAFA03;
    /// Interrupt-status register address of the primary card.
    const ISR_ADDRESS: u32 = 0xEAFA05;
    /// Interrupt-clear register address of the primary card.
    const ICR_ADDRESS: u32 = 0xEAFA07;
    /// Banked offset-4 register address of the primary card.
    const BANKED_4_ADDRESS: u32 = 0xEAFA09;
    /// Banked offset-5 register address of the primary card.
    const BANKED_5_ADDRESS: u32 = 0xEAFA0B;
    /// Banked offset-6 register address of the primary card.
    const BANKED_6_ADDRESS: u32 = 0xEAFA0D;
    /// CPU cycles per MIDI byte at the CLKM/32 rate on a 10 MHz model.
    const MIDI_BYTE_CYCLES: u64 = 3200;

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

    /// Advances the bus through its scheduled events up to `target_cycle`.
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

    /// Programs the CLKM/32 MIDI rate and enables the transmitter.
    fn enable_transmitter_at_midi_rate(bus: &mut X68kBus) {
        write_register(bus, RGR_ADDRESS, 0x04);
        write_register(bus, BANKED_4_ADDRESS, 0x08);
        write_register(bus, RGR_ADDRESS, 0x05);
        write_register(bus, BANKED_5_ADDRESS, 0x01);
    }

    /// Drains the captured transmit bytes.
    fn captured(bus: &mut X68kBus) -> Vec<u8> {
        let mut bytes = Vec::new();
        bus.flush_midi_into(&mut bytes);
        bytes
    }

    #[test]
    fn absent_card_bus_errors_across_the_window() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        for address in [0xEAFA01, 0xEAFA0F] {
            assert!(
                bus.m68000_read(access(address, M68000AccessSize::Byte, supervisor))
                    .is_err()
            );
            assert!(
                bus.m68000_write(access(address, M68000AccessSize::Byte, supervisor), 0)
                    .is_err()
            );
        }
    }

    #[test]
    fn installed_card_responds_on_odd_bytes_and_rejects_even() {
        let mut bus = bus(X68kModel::X68000);
        bus.install_midi_card();
        assert_eq!(read_register(&mut bus, IVR_ADDRESS), 0x10);
        let supervisor = M68000FunctionCode::SupervisorData;
        assert!(
            bus.m68000_read(access(0xEAFA00, M68000AccessSize::Byte, supervisor))
                .is_err()
        );
        let word = bus
            .m68000_read(access(0xEAFA00, M68000AccessSize::Word, supervisor))
            .unwrap();
        assert_eq!(word, 0xFF10);
    }

    #[test]
    fn user_mode_access_bus_errors() {
        let mut bus = bus(X68kModel::X68000);
        bus.install_midi_card();
        assert!(
            bus.m68000_read(access(
                IVR_ADDRESS,
                M68000AccessSize::Byte,
                M68000FunctionCode::UserData
            ))
            .is_err()
        );
    }

    #[test]
    fn banked_status_register_reads_through_the_bus() {
        let mut bus = bus(X68kModel::X68000);
        bus.install_midi_card();
        write_register(&mut bus, RGR_ADDRESS, 0x05);
        assert_eq!(read_register(&mut bus, BANKED_4_ADDRESS), 0xC4);
    }

    #[test]
    fn transmit_paces_bytes_at_ten_megahertz() {
        let mut bus = bus(X68kModel::X68000);
        bus.install_midi_card();
        enable_transmitter_at_midi_rate(&mut bus);
        let start = bus.current_cycle();
        let stream = [0x90, 0x40, 0x7F, 0xFE];
        for value in stream {
            write_register(&mut bus, BANKED_6_ADDRESS, value);
        }
        let mut received = Vec::new();
        for (index, expected) in stream.into_iter().enumerate() {
            let count = index as u64 + 1;
            run_events_until(&mut bus, start + MIDI_BYTE_CYCLES * count - 1);
            bus.flush_midi_into(&mut received);
            assert_eq!(received.len(), index, "byte {index} must not be early");
            run_events_until(&mut bus, start + MIDI_BYTE_CYCLES * count);
            bus.flush_midi_into(&mut received);
            assert_eq!(received.len(), index + 1);
            assert_eq!(received[index], expected);
        }
    }

    #[test]
    fn xvi_high_speed_paces_bytes_at_sixteen_megahertz() {
        let mut bus = bus(X68kModel::X68000Xvi);
        let cpu_clock_hz = 16_666_667;
        bus.install_midi_card();
        enable_transmitter_at_midi_rate(&mut bus);
        let start = bus.current_cycle();
        for value in [0x90, 0x40, 0x7F] {
            write_register(&mut bus, BANKED_6_ADDRESS, value);
        }
        for count in 1..=3u64 {
            let byte_cycle = tick_to_cycle(320 * count, YM3802_CLKM_HZ, cpu_clock_hz);
            run_events_until(&mut bus, start + byte_cycle - 2);
            assert_eq!(
                captured(&mut bus).len(),
                0,
                "byte {count} must not be early"
            );
            run_events_until(&mut bus, start + byte_cycle + 1);
            assert_eq!(captured(&mut bus).len(), 1);
        }
    }

    #[test]
    fn fifo_empty_interrupt_asserts_level_four_and_vectors() {
        let mut bus = bus(X68kModel::X68000);
        bus.install_midi_card();
        write_register(&mut bus, RGR_ADDRESS, 0x00);
        write_register(&mut bus, BANKED_4_ADDRESS, 0x40);
        write_register(&mut bus, BANKED_6_ADDRESS, 0x40);
        enable_transmitter_at_midi_rate(&mut bus);
        write_register(&mut bus, BANKED_6_ADDRESS, 0x90);
        assert_eq!(bus.m68000_interrupt_level(), 4);
        assert_eq!(bus.m68000_acknowledge_interrupt(4), 0x4C);
        write_register(&mut bus, ICR_ADDRESS, 0x40);
        assert_eq!(bus.m68000_interrupt_level(), 0);
    }

    #[test]
    fn acknowledge_reflects_current_cause_priority() {
        let mut bus = bus(X68kModel::X68000);
        bus.install_midi_card();
        write_register(&mut bus, RGR_ADDRESS, 0x00);
        write_register(&mut bus, BANKED_4_ADDRESS, 0x40);
        write_register(&mut bus, BANKED_6_ADDRESS, 0xC0);
        enable_transmitter_at_midi_rate(&mut bus);
        write_register(&mut bus, BANKED_6_ADDRESS, 0x90);
        write_register(&mut bus, RGR_ADDRESS, 0x08);
        write_register(&mut bus, BANKED_4_ADDRESS, 4);
        write_register(&mut bus, BANKED_5_ADDRESS, 0x80);
        let start = bus.current_cycle();
        run_events_until(&mut bus, start + 200);
        assert_eq!(read_register(&mut bus, ISR_ADDRESS), 0xC0);
        assert_eq!(bus.m68000_acknowledge_interrupt(4), 0x4C);
        write_register(&mut bus, ICR_ADDRESS, 0x40);
        assert_eq!(bus.m68000_acknowledge_interrupt(4), 0x4E);
        write_register(&mut bus, ICR_ADDRESS, 0x80);
        assert_eq!(bus.m68000_interrupt_level(), 0);
    }

    #[test]
    fn general_timer_interrupts_through_the_cycle_domain() {
        let mut bus = bus(X68kModel::X68000);
        bus.install_midi_card();
        write_register(&mut bus, RGR_ADDRESS, 0x00);
        write_register(&mut bus, BANKED_6_ADDRESS, 0x80);
        write_register(&mut bus, RGR_ADDRESS, 0x06);
        write_register(&mut bus, BANKED_6_ADDRESS, 0x02);
        write_register(&mut bus, RGR_ADDRESS, 0x08);
        write_register(&mut bus, BANKED_4_ADDRESS, 100);
        write_register(&mut bus, BANKED_5_ADDRESS, 0x80);
        let start = bus.current_cycle();
        run_events_until(&mut bus, start + 7_990);
        assert_eq!(bus.m68000_interrupt_level(), 0);
        run_events_until(&mut bus, start + 8_010);
        assert_eq!(bus.m68000_interrupt_level(), 4);
        write_register(&mut bus, ICR_ADDRESS, 0x80);
        assert_eq!(bus.m68000_interrupt_level(), 0);
        run_events_until(&mut bus, start + 16_010);
        assert_eq!(bus.m68000_interrupt_level(), 4);
    }

    #[test]
    fn reset_line_resets_the_card() {
        let mut bus = bus(X68kModel::X68000);
        bus.install_midi_card();
        enable_transmitter_at_midi_rate(&mut bus);
        write_register(&mut bus, BANKED_6_ADDRESS, 0x90);
        bus.m68000_reset_line(true);
        assert_eq!(read_register(&mut bus, IVR_ADDRESS), 0x10);
        write_register(&mut bus, RGR_ADDRESS, 0x05);
        assert_eq!(read_register(&mut bus, BANKED_4_ADDRESS), 0xC4);
    }

    #[test]
    fn cpu_space_iack_reads_the_midi_vector() {
        let mut bus = bus(X68kModel::X68000);
        bus.install_midi_card();
        write_register(&mut bus, RGR_ADDRESS, 0x00);
        write_register(&mut bus, BANKED_4_ADDRESS, 0x40);
        write_register(&mut bus, BANKED_6_ADDRESS, 0x40);
        enable_transmitter_at_midi_rate(&mut bus);
        write_register(&mut bus, BANKED_6_ADDRESS, 0x90);
        let vector = bus
            .m68000_read(access(
                0xFFFFF9,
                M68000AccessSize::Byte,
                M68000FunctionCode::CpuSpace,
            ))
            .unwrap();
        assert_eq!(vector, 0x4C);
    }

    /// Boot-chain variant: the synthetic IPL programs the card and streams a
    /// deterministic MIDI sequence, so pacing runs through the real CPU loop.
    #[test]
    fn synthetic_ipl_transmits_midi_through_the_cpu_loop() {
        let stream = [
            0x90, 0x40, 0x7F, 0xC0, 0x30, 0xF0, 0x41, 0x10, 0x42, 0x12, 0xF7,
        ];
        let mut loaded = test_roms(X68kModel::X68000);
        let mut writes: Vec<(u32, u8)> = vec![
            (RGR_ADDRESS, 0x04),
            (BANKED_4_ADDRESS, 0x08),
            (RGR_ADDRESS, 0x05),
            (BANKED_5_ADDRESS, 0x01),
        ];
        writes.extend(stream.iter().map(|&value| (BANKED_6_ADDRESS, value)));
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
        machine.install_midi_card();
        machine.run_for(MIDI_BYTE_CYCLES * (stream.len() as u64 + 4));
        let mut received = Vec::new();
        machine.flush_midi_into(&mut received);
        assert_eq!(received, stream);
    }
}
