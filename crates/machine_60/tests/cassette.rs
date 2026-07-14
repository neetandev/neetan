//! Cassette tests: byte delivery and end-of-tape over the sub-CPU interrupt, the
//! motor gate, and the accepted image formats.

use machine_60::{Pc6000Bus, Pc6000Model};

mod harness;
use harness::{build_machine, fire_next_event};

/// Sub-CPU acknowledge vector for a delivered cassette byte.
const CASSETTE_DATA_VECTOR: u8 = 0x08;
/// Sub-CPU acknowledge vector at end of tape.
const CASSETTE_END_VECTOR: u8 = 0x12;
/// System latch (port 0xB0): cassette motor on (bit 3), timer off (bit 0).
const MOTOR_ON: u8 = 0x09;
/// System latch: motor off, timer off.
const MOTOR_OFF: u8 = 0x01;

/// Pumps events until an interrupt is acknowledged, returning its vector.
fn next_irq_vector(bus: &mut Pc6000Bus) -> u8 {
    for _ in 0..100_000 {
        if let Some(vector) = fire_next_event(bus) {
            return vector;
        }
    }
    panic!("no interrupt was delivered");
}

#[test]
fn cassette_delivers_bytes_then_end_of_tape() {
    let mut machine = build_machine(Pc6000Model::Pc6001);
    let bus = &mut machine.bus;
    bus.insert_cassette("p6", &[0xA1, 0xB2])
        .expect("tape parses");

    bus.io_write(0xB0, MOTOR_ON);

    assert_eq!(next_irq_vector(bus), CASSETTE_DATA_VECTOR);
    assert_eq!(next_irq_vector(bus), CASSETTE_DATA_VECTOR);
    assert_eq!(next_irq_vector(bus), CASSETTE_END_VECTOR);
}

#[test]
fn stopping_the_motor_halts_delivery() {
    let mut machine = build_machine(Pc6000Model::Pc6001);
    let bus = &mut machine.bus;
    bus.insert_cassette("p6", &[0x10, 0x20, 0x30])
        .expect("tape parses");

    bus.io_write(0xB0, MOTOR_ON);
    assert_eq!(next_irq_vector(bus), CASSETTE_DATA_VECTOR);

    // Stopping the motor cancels the remaining byte and end-of-tape deliveries.
    bus.io_write(0xB0, MOTOR_OFF);
    for _ in 0..5_000 {
        if let Some(vector) = fire_next_event(bus) {
            assert!(
                vector != CASSETTE_DATA_VECTOR && vector != CASSETTE_END_VECTOR,
                "a cassette interrupt fired after the motor stopped"
            );
        }
    }
}

#[test]
fn bus_accepts_known_formats_and_rejects_others() {
    let mut machine = build_machine(Pc6000Model::Pc6001);
    let bus = &mut machine.bus;

    assert!(bus.insert_cassette("p6", &[0xD3, 0xD3, 0x69]).is_ok());
    assert!(bus.insert_cassette("cas", &[0xD3, 0xD3, 0x69]).is_ok());
    assert!(bus.insert_cassette("p6t", &[0xD3, 0xD3, 0x69]).is_ok());
    assert!(bus.insert_cassette("wav", &[0x00]).is_err());
    assert!(bus.insert_cassette("p6", &[]).is_err());
}
