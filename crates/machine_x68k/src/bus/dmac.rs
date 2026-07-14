//! HD63450 DMAC glue: register window, bus-master port, and transfer pump.
//!
//! The controller is taken out of the bus for the duration of each pump so it
//! can perform bus-master cycles through the [`DmacBusPort`] view of the same
//! bus. Every controller clock consumed by those cycles is charged to the CPU
//! as wait cycles, so DMA transfers genuinely contend with instruction
//! execution instead of completing instantly.

use common::{M68000AccessSize, M68000BusAccess, M68000CycleKind, M68000FunctionCode, TraceSink};
use device::hd63450_dmac::{DmacBusFault, DmacBusPort};

use super::{X68kBus, X68kRegion};
use crate::clock::cycle_to_tick;

/// HD63450 controller clock in Hz, identical on all supported models.
pub(super) const DMAC_CLOCK_HZ: u64 = 10_000_000;

/// Number of DMAC main-RAM accesses that share one DRAM refresh clock.
const DMAC_RAM_ACCESSES_PER_REFRESH_CLOCK: u8 = 8;

/// Returns the DMAC wait penalty of one bus-master access to the region, in
/// whole controller clocks. Main RAM carries only the shared DRAM refresh
/// clock counted separately; regions that never complete an access and the
/// expansion MIDI window carry no penalty.
const fn dmac_access_wait_clocks(region: X68kRegion) -> u64 {
    match region {
        X68kRegion::MainRam
        | X68kRegion::Sram
        | X68kRegion::Cgrom
        | X68kRegion::InternalScsiRom
        | X68kRegion::IplRom => 0,
        X68kRegion::GraphicVram
        | X68kRegion::Crtc
        | X68kRegion::VideoController
        | X68kRegion::Printer
        | X68kRegion::SystemPort
        | X68kRegion::Sprite
        | X68kRegion::StandardSupervisorArea
        | X68kRegion::EnhancedSupervisorArea => 1,
        X68kRegion::TextVram
        | X68kRegion::Rtc
        | X68kRegion::Opm
        | X68kRegion::Adpcm
        | X68kRegion::Fdc
        | X68kRegion::StorageController
        | X68kRegion::Ppi
        | X68kRegion::Ioc => 2,
        X68kRegion::Palette => 3,
        X68kRegion::Mfp => 4,
        X68kRegion::Scc => 6,
        X68kRegion::Dmac
        | X68kRegion::Midi
        | X68kRegion::BuiltinDevice
        | X68kRegion::UserIo
        | X68kRegion::Unmapped => 0,
    }
}

impl<T: TraceSink> X68kBus<T> {
    /// Reads one DMAC register byte; the window mirrors every 0x100 bytes.
    pub(super) fn read_dmac_register(&mut self, address: u32) -> u8 {
        self.dmac.read_register((address & 0xFF) as u8)
    }

    /// Writes one DMAC register byte; a CCR START bit transfers immediately.
    pub(super) fn write_dmac_register(&mut self, address: u32, value: u8) {
        let clock = self.dmac_clock();
        let mut dmac = std::mem::take(&mut self.dmac);
        dmac.write_register((address & 0xFF) as u8, value, self, clock);
        self.dmac = dmac;
        self.finish_dmac_activity();
        self.sync_storage_lines();
    }

    /// Runs auto-request DMAC work that is due at the current time.
    pub(crate) fn pump_dmac(&mut self) {
        if self.dmac.next_work_clock().is_none() {
            return;
        }
        let clock = self.dmac_clock();
        let mut dmac = std::mem::take(&mut self.dmac);
        dmac.run_due(self, clock);
        self.dmac = dmac;
        self.finish_dmac_activity();
        self.sync_storage_lines();
    }

    /// Asserts the FDC external transfer request on DMAC channel 0.
    pub(super) fn assert_fdc_dmac_request(&mut self) {
        let clock = self.dmac_clock();
        let mut dmac = std::mem::take(&mut self.dmac);
        dmac.assert_request(0, self, clock);
        self.dmac = dmac;
        self.finish_dmac_activity();
    }

    /// Asserts the storage-controller external transfer request on DMAC
    /// channel 1, moving one operand through the data register.
    pub(super) fn assert_storage_dmac_request(&mut self) {
        let clock = self.dmac_clock();
        let mut dmac = std::mem::take(&mut self.dmac);
        dmac.assert_request(1, self, clock);
        self.dmac = dmac;
        self.finish_dmac_activity();
    }

    /// Returns the current controller clock, including CPU stall time already
    /// charged during this instruction.
    pub(super) fn dmac_clock(&self) -> u64 {
        let effective = self
            .current_cycle
            .wrapping_add(self.wait_cycles.max(0) as u64);
        cycle_to_tick(effective, DMAC_CLOCK_HZ, self.cpu_clock_hz)
    }

    /// Charges consumed controller clocks to the CPU and reschedules events.
    pub(super) fn finish_dmac_activity(&mut self) {
        let clocks = self.dmac.take_consumed_clocks() + std::mem::take(&mut self.dmac_wait_clocks);
        if clocks != 0 {
            let total = u128::from(clocks) * u128::from(self.cpu_clock_hz)
                + u128::from(self.dmac_stall_remainder);
            self.wait_cycles += (total / u128::from(DMAC_CLOCK_HZ)) as i64;
            self.dmac_stall_remainder = (total % u128::from(DMAC_CLOCK_HZ)) as u64;
        }
        let completions = self.dmac.take_channel_completions();
        if completions & 0x01 != 0 {
            self.on_fdc_terminal_count();
        }
        if completions & 0x02 != 0 {
            self.on_storage_terminal_count();
        }
        self.schedule_events();
    }

    /// Performs one DMAC bus-master read through the checked access path.
    fn dmac_port_read(
        &mut self,
        address: u32,
        size: M68000AccessSize,
    ) -> Result<u16, DmacBusFault> {
        let region = Self::decode_region(address);
        if region == X68kRegion::Dmac {
            return Err(DmacBusFault);
        }
        let value = self
            .read_checked(M68000BusAccess {
                address,
                size,
                function_code: M68000FunctionCode::SupervisorData,
                cycle_kind: M68000CycleKind::Normal,
            })
            .map_err(|_| DmacBusFault)?;
        self.charge_dmac_access_wait(region);
        Ok(value)
    }

    /// Performs one DMAC bus-master write through the checked access path.
    fn dmac_port_write(
        &mut self,
        address: u32,
        size: M68000AccessSize,
        value: u16,
    ) -> Result<(), DmacBusFault> {
        let region = Self::decode_region(address);
        if region == X68kRegion::Dmac {
            return Err(DmacBusFault);
        }
        self.write_checked(
            M68000BusAccess {
                address,
                size,
                function_code: M68000FunctionCode::SupervisorData,
                cycle_kind: M68000CycleKind::Normal,
            },
            value,
        )
        .map_err(|_| DmacBusFault)?;
        self.charge_dmac_access_wait(region);
        Ok(())
    }

    /// Charges the region wait of one bus-master access, counting the DRAM
    /// refresh clock shared by every eight main-RAM accesses.
    fn charge_dmac_access_wait(&mut self, region: X68kRegion) {
        self.dmac_wait_clocks += dmac_access_wait_clocks(region);
        if region == X68kRegion::MainRam {
            self.dmac_refresh_access_count += 1;
            if self.dmac_refresh_access_count == DMAC_RAM_ACCESSES_PER_REFRESH_CLOCK {
                self.dmac_refresh_access_count = 0;
                self.dmac_wait_clocks += 1;
            }
        }
    }
}

impl<T: TraceSink> DmacBusPort for X68kBus<T> {
    /// Reads one byte as a DMAC bus cycle.
    fn read_byte(&mut self, address: u32) -> Result<u8, DmacBusFault> {
        self.dmac_port_read(address, M68000AccessSize::Byte)
            .map(|value| value as u8)
    }

    /// Reads one word as a DMAC bus cycle.
    fn read_word(&mut self, address: u32) -> Result<u16, DmacBusFault> {
        self.dmac_port_read(address, M68000AccessSize::Word)
    }

    /// Writes one byte as a DMAC bus cycle.
    fn write_byte(&mut self, address: u32, value: u8) -> Result<(), DmacBusFault> {
        self.dmac_port_write(address, M68000AccessSize::Byte, u16::from(value))
    }

    /// Writes one word as a DMAC bus cycle.
    fn write_word(&mut self, address: u32, value: u16) -> Result<(), DmacBusFault> {
        self.dmac_port_write(address, M68000AccessSize::Word, value)
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, CpuM68000, CpuMode, M68000AccessSize, M68000FunctionCode};
    use device::hd63450_dmac::{ERROR_DEVICE_BUS, ERROR_MEMORY_BUS};

    use crate::{
        X68kMachine, X68kModel,
        bus::{
            X68kBus, X68kRegion,
            test_support::{access, bus, test_roms},
        },
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
        .expect("DMAC register write must not raise a CPU bus error");
    }

    /// Reads one byte register through the supervisor bus.
    fn read_register(bus: &mut X68kBus, address: u32) -> u8 {
        bus.m68000_read(access(
            address,
            M68000AccessSize::Byte,
            M68000FunctionCode::SupervisorData,
        ))
        .expect("DMAC register read must not raise a CPU bus error") as u8
    }

    /// Programs channel 0 for a dual-address word transfer.
    fn program_word_transfer(bus: &mut X68kBus, source: u32, destination: u32, words: u16) {
        write_register(bus, 0xE84004, 0x08);
        write_register(bus, 0xE84005, 0x11);
        write_register(bus, 0xE84006, 0x05);
        write_register(bus, 0xE8400A, (words >> 8) as u8);
        write_register(bus, 0xE8400B, words as u8);
        for (index, byte) in source.to_be_bytes().into_iter().enumerate() {
            write_register(bus, 0xE8400C + index as u32, byte);
        }
        for (index, byte) in destination.to_be_bytes().into_iter().enumerate() {
            write_register(bus, 0xE84014 + index as u32, byte);
        }
    }

    #[test]
    fn decoder_places_the_dmac_window() {
        assert_eq!(
            X68kBus::<common::NoTrace>::decode_region(0xE83FFF),
            X68kRegion::VideoController
        );
        assert_eq!(
            X68kBus::<common::NoTrace>::decode_region(0xE84000),
            X68kRegion::Dmac
        );
        assert_eq!(
            X68kBus::<common::NoTrace>::decode_region(0xE85FFF),
            X68kRegion::Dmac
        );
        assert_eq!(
            X68kBus::<common::NoTrace>::decode_region(0xE86000),
            X68kRegion::StandardSupervisorArea
        );
    }

    #[test]
    fn registers_use_both_byte_lanes_and_mirror() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        bus.m68000_write(access(0xE8400C, M68000AccessSize::Word, supervisor), 0x1234)
            .unwrap();
        assert_eq!(read_register(&mut bus, 0xE8400C), 0x12);
        assert_eq!(read_register(&mut bus, 0xE8400D), 0x34);
        assert_eq!(
            bus.m68000_read(access(0xE8400C, M68000AccessSize::Word, supervisor)),
            Ok(0x1234)
        );
        assert_eq!(read_register(&mut bus, 0xE8410C), 0x12);
        write_register(&mut bus, 0xE840FF, 0x0F);
        assert_eq!(read_register(&mut bus, 0xE840FF), 0x0F);
        assert!(
            bus.m68000_read(access(
                0xE84000,
                M68000AccessSize::Byte,
                M68000FunctionCode::UserData
            ))
            .is_err()
        );
    }

    #[test]
    fn faulting_transfers_set_cer_instead_of_a_cpu_bus_error() {
        let mut bus = bus(X68kModel::X68000);
        program_word_transfer(&mut bus, 0xED4000, 0x3000, 1);
        write_register(&mut bus, 0xE84007, 0x80);
        assert_ne!(read_register(&mut bus, 0xE84000) & 0x10, 0);
        assert_eq!(read_register(&mut bus, 0xE84001), ERROR_MEMORY_BUS);

        program_word_transfer(&mut bus, 0x2000, 0xED4000, 1);
        write_register(&mut bus, 0xE84047, 0x00);
        write_register(&mut bus, 0xE84044, 0x08);
        write_register(&mut bus, 0xE84045, 0x11);
        write_register(&mut bus, 0xE84046, 0x05);
        write_register(&mut bus, 0xE8404B, 0x01);
        for (index, byte) in 0x2000u32.to_be_bytes().into_iter().enumerate() {
            write_register(&mut bus, 0xE8404C + index as u32, byte);
        }
        for (index, byte) in 0xED4000u32.to_be_bytes().into_iter().enumerate() {
            write_register(&mut bus, 0xE84054 + index as u32, byte);
        }
        write_register(&mut bus, 0xE84047, 0x80);
        assert_eq!(read_register(&mut bus, 0xE84041), ERROR_DEVICE_BUS);
    }

    #[test]
    fn dmac_self_access_faults_as_memory_bus_error() {
        let mut bus = bus(X68kModel::X68000);
        program_word_transfer(&mut bus, 0xE84000, 0x3000, 1);
        write_register(&mut bus, 0xE84007, 0x80);
        assert_eq!(read_register(&mut bus, 0xE84001), ERROR_MEMORY_BUS);
    }

    #[test]
    fn completed_transfer_delivers_a_level_three_normal_vector() {
        let mut bus = bus(X68kModel::X68000);
        bus.ram[0x2000..0x2002].copy_from_slice(&[0xA5, 0x5A]);
        program_word_transfer(&mut bus, 0x2000, 0x3000, 1);
        write_register(&mut bus, 0xE84025, 0x6A);
        write_register(&mut bus, 0xE84007, 0x88);
        assert_eq!(&bus.ram[0x3000..0x3002], &[0xA5, 0x5A]);
        assert_eq!(bus.m68000_interrupt_level(), 3);
        assert_eq!(bus.m68000_acknowledge_interrupt(3), 0x6A);
        assert_eq!(bus.m68000_interrupt_level(), 0);
    }

    /// Builds a machine whose IPL programs the DMAC channel-0 word transfer
    /// and stops; `ccr` selects a started (0x80) or inert (0x00) operation.
    fn transfer_machine(ccr: u8) -> X68kMachine {
        let mut loaded = test_roms(X68kModel::X68000);
        let writes: [(u32, u8); 14] = [
            (0xE84004, 0x08),
            (0xE84005, 0x11),
            (0xE84006, 0x05),
            (0xE8400A, 0x00),
            (0xE8400B, 0x08),
            (0xE8400C, 0x00),
            (0xE8400D, 0x00),
            (0xE8400E, 0x20),
            (0xE8400F, 0x00),
            (0xE84014, 0x00),
            (0xE84015, 0x00),
            (0xE84016, 0x30),
            (0xE84017, 0x00),
            (0xE84007, ccr),
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
        for (index, slot) in machine.bus.ram[0x2000..0x2010].iter_mut().enumerate() {
            *slot = 0xC0 | index as u8;
        }
        machine
    }

    /// Steps the CPU until STOP and returns the consumed cycles.
    fn run_until_stop(machine: &mut X68kMachine) -> u64 {
        let mut total = 0;
        for _ in 0..10_000 {
            let cycles = machine.cpu.step(&mut machine.bus);
            if cycles == 0 {
                return total;
            }
            total += cycles;
        }
        panic!("synthetic IPL program never reached STOP");
    }

    #[test]
    fn memory_to_memory_transfer_copies_and_contends_with_the_cpu() {
        let mut inert = transfer_machine(0x00);
        let baseline_cycles = run_until_stop(&mut inert);
        assert!(inert.bus.ram[0x3000..0x3010].iter().all(|&byte| byte == 0));

        let mut started = transfer_machine(0x80);
        let transfer_cycles = run_until_stop(&mut started);
        let expected: Vec<u8> = (0..16).map(|index| 0xC0 | index as u8).collect();
        assert_eq!(&started.bus.ram[0x3000..0x3010], expected.as_slice());
        assert_ne!(read_register(&mut started.bus, 0xE84000) & 0x80, 0);

        let word_bus_clocks = 80;
        let dram_refresh_clocks = 2;
        let minimum_bus_clocks = word_bus_clocks + dram_refresh_clocks;
        assert!(
            transfer_cycles >= baseline_cycles + minimum_bus_clocks,
            "expected at least {minimum_bus_clocks} stolen cycles, \
             baseline {baseline_cycles}, with transfer {transfer_cycles}"
        );
    }

    #[test]
    fn bus_master_region_waits_match_the_dmac_penalty_table() {
        assert_eq!(super::dmac_access_wait_clocks(X68kRegion::MainRam), 0);
        assert_eq!(super::dmac_access_wait_clocks(X68kRegion::IplRom), 0);
        assert_eq!(super::dmac_access_wait_clocks(X68kRegion::GraphicVram), 1);
        assert_eq!(super::dmac_access_wait_clocks(X68kRegion::TextVram), 2);
        assert_eq!(super::dmac_access_wait_clocks(X68kRegion::Adpcm), 2);
        assert_eq!(
            super::dmac_access_wait_clocks(X68kRegion::StorageController),
            2
        );
        assert_eq!(super::dmac_access_wait_clocks(X68kRegion::Palette), 3);
        assert_eq!(super::dmac_access_wait_clocks(X68kRegion::Mfp), 4);
        assert_eq!(super::dmac_access_wait_clocks(X68kRegion::Scc), 6);
        assert_eq!(super::dmac_access_wait_clocks(X68kRegion::Midi), 0);
    }

    #[test]
    fn every_eighth_ram_access_charges_one_refresh_clock() {
        let mut bus = bus(X68kModel::X68000);
        for _ in 0..8 {
            bus.charge_dmac_access_wait(X68kRegion::MainRam);
        }
        assert_eq!(bus.dmac_wait_clocks, 1);
        bus.charge_dmac_access_wait(X68kRegion::Scc);
        assert_eq!(bus.dmac_wait_clocks, 7);
    }
}
