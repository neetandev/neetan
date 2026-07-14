//! Cassette transport and EAR-playback tests (driven through the sub-CPU).

mod harness;

use harness::{build_machine, run_bus_cycles};
use machine_x1::X1Model;

/// Sub-CPU mailbox port.
const SUB_MAILBOX: u16 = 0x1900;
/// PPI port B (bit 1 carries the cassette EAR level).
const PPI_PORT_B: u16 = 0x1A01;
const PORT_B_CASSETTE: u8 = 0x02;

/// CMT control command and its transport sub-commands.
const CMD_CMT_CONTROL: u8 = 0xE9;
const CMD_CMT_SENSOR: u8 = 0xEB;
const CMT_STOP: u8 = 0x01;
const CMT_PLAY: u8 = 0x02;

/// Builds an "old" X1 `.tap` image: a 4-byte sample rate then the sample bytes.
fn old_tap(sample_rate: u32, samples: &[u8]) -> Vec<u8> {
    let mut image = sample_rate.to_le_bytes().to_vec();
    image.extend_from_slice(samples);
    image
}

/// Sends a sub-CPU byte and lets several poll ticks run.
fn send(bus: &mut machine_x1::X1Bus, value: u8) {
    bus.io_write(SUB_MAILBOX, value);
    run_bus_cycles(bus, 8_000);
}

fn read_result(bus: &mut machine_x1::X1Bus) -> u8 {
    let value = bus.io_read(SUB_MAILBOX).0;
    run_bus_cycles(bus, 2_000);
    value
}

#[test]
fn sensor_reports_a_loaded_tape() {
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;
    bus.insert_cassette("tap", &old_tap(4000, &[0xAA; 8]))
        .expect("tape loads");

    send(bus, CMD_CMT_SENSOR);
    let sensor = read_result(bus);
    assert_ne!(sensor, 0x00, "a loaded tape must report a nonzero sensor");
}

#[test]
fn play_streams_the_waveform_onto_port_b() {
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;
    // 0xAA = alternating sample bits at 4 kHz -> one bit per 1000 CPU cycles.
    bus.insert_cassette("tap", &old_tap(4000, &[0xAA; 64]))
        .expect("tape loads");

    send(bus, CMD_CMT_CONTROL);
    send(bus, CMT_PLAY);

    let mut saw_high = false;
    let mut saw_low = false;
    for _ in 0..30 {
        run_bus_cycles(bus, 500);
        if bus.io_read(PPI_PORT_B).0 & PORT_B_CASSETTE != 0 {
            saw_high = true;
        } else {
            saw_low = true;
        }
    }
    assert!(
        saw_high && saw_low,
        "the EAR line must toggle while playing"
    );
}

#[test]
fn stop_freezes_the_waveform() {
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;
    // A constant-high lead-in so the frozen level is unambiguous.
    bus.insert_cassette("tap", &old_tap(4000, &[0xFF; 128]))
        .expect("tape loads");

    send(bus, CMD_CMT_CONTROL);
    send(bus, CMT_PLAY);
    run_bus_cycles(bus, 4_000);
    let playing = bus.io_read(PPI_PORT_B).0 & PORT_B_CASSETTE;
    assert_eq!(
        playing, PORT_B_CASSETTE,
        "playing a high lead-in reads high"
    );

    send(bus, CMD_CMT_CONTROL);
    send(bus, CMT_STOP);
    run_bus_cycles(bus, 100_000);
    // Past the (short) tape the head would run off the end; stopping holds it,
    // so the level stays at the last sample rather than the end-of-tape low.
    let stopped = bus.io_read(PPI_PORT_B).0 & PORT_B_CASSETTE;
    assert_eq!(stopped, PORT_B_CASSETTE);
}
