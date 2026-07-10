use common::{Bus, CpuMode, HostDateTime, Machine as _, MachineModel};
use machine::{NoTracing, Pc9801Bus, Pc9801Vm};

/// Test time: 2026-03-03 14:30:45, Monday (day_of_week=1).
const TEST_TIME: [u8; 6] = [0x26, 0x31, 0x03, 0x14, 0x30, 0x45];

fn test_time() -> HostDateTime {
    HostDateTime {
        year: 2026,
        month: 3,
        day: 3,
        day_of_week: 1,
        hour: 14,
        minute: 30,
        second: 45,
    }
}

fn make_machine() -> Pc9801Vm {
    let bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    let mut machine = machine::Machine::new(cpu::V30::new(), bus);
    machine.set_host_date_time_provider(test_time);
    machine
}

/// Writes a port 0x20 value to the RTC via the bus.
fn rtc_write(bus: &mut Pc9801Bus<NoTracing>, value: u8) {
    bus.io_write_byte(0x20, value);
}

/// Reads CDAT (bit 0 of port 0x33).
fn read_cdat(bus: &mut Pc9801Bus<NoTracing>) -> u8 {
    bus.io_read_byte(0x33) & 0x01
}

/// Issues a TIME_READ command: DATA phase (cmd=3), STB rising edge, release.
fn time_read(bus: &mut Pc9801Bus<NoTracing>) {
    rtc_write(bus, 0x03); // DATA phase: parallel = 3.
    rtc_write(bus, 0x0B); // STB rising edge (0x03 | 0x08).
    rtc_write(bus, 0x00); // Release.
}

/// Issues a REGISTER_SHIFT command.
fn register_shift(bus: &mut Pc9801Bus<NoTracing>) {
    rtc_write(bus, 0x01); // DATA phase: parallel = 1.
    rtc_write(bus, 0x09); // STB rising edge (0x01 | 0x08).
    rtc_write(bus, 0x00); // Release.
}

/// Pulses CLK once.
fn clock_pulse(bus: &mut Pc9801Bus<NoTracing>) {
    rtc_write(bus, 0x10);
    rtc_write(bus, 0x00);
}

#[test]
fn rtc_time_read_and_shift_out_48_bits() {
    let mut machine = make_machine();
    let bus = &mut machine.bus;

    time_read(bus);
    register_shift(bus);

    // Clock out all 48 BCD time bits from CDAT (positions 63 down to 16).
    let mut bits = Vec::new();
    bits.push(read_cdat(bus));
    for _ in 0..47 {
        clock_pulse(bus);
        bits.push(read_cdat(bus));
    }

    // Reconstruct the 6 BCD bytes.
    // The shift register outputs reg[7] first (seconds), then reg[6] (minutes), etc.
    // Within each byte, bits come out LSB-first due to the bit indexing scheme:
    //   pos 63 -> bit 0 of reg[7], pos 62 -> bit 1, ..., pos 56 -> bit 7.
    let mut reconstructed = [0u8; 6];
    for (i, bit) in bits.iter().enumerate() {
        let byte_idx = 5 - (i / 8);
        let bit_idx = i % 8;
        reconstructed[byte_idx] |= bit << bit_idx;
    }

    assert_eq!(reconstructed, TEST_TIME);
}

#[test]
fn rtc_cdat_starts_at_zero() {
    let mut machine = make_machine();
    assert_eq!(read_cdat(&mut machine.bus), 0);
}

#[test]
fn rtc_cdat_reflects_time_data_after_read() {
    let mut machine = make_machine();
    time_read(&mut machine.bus);

    // After TIME_READ, CDAT should reflect the LSB of the seconds byte (0x45).
    // reg[7] = 0x45 = 0100_0101, bit 0 = 1.
    assert_eq!(read_cdat(&mut machine.bus), 1);
}
