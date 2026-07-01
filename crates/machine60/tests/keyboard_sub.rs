//! Keyboard sub-controller tests: normal and function key vectors, key release,
//! and the joystick-trigger sub-command.

use machine60::{Pc6000Bus, Pc6000Model};

mod harness;
use harness::{build_machine, fire_next_event};

const NORMAL_KEY_VECTOR: u8 = 0x02;
const FUNCTION_KEY_VECTOR: u8 = 0x14;
const JOYSTICK_VECTOR: u8 = 0x16;
/// Scancode bit 7 marks a key release.
const RELEASE_FLAG: u8 = 0x80;
/// First function-key scancode id.
const FUNCTION_KEY_ID: u8 = 0x60;

fn next_irq_vector(bus: &mut Pc6000Bus) -> u8 {
    for _ in 0..100_000 {
        if let Some(vector) = fire_next_event(bus) {
            return vector;
        }
    }
    panic!("no interrupt was delivered");
}

#[test]
fn normal_key_press_raises_the_sub_vector() {
    let mut machine = build_machine(Pc6000Model::Pc6001);
    let bus = &mut machine.bus;
    bus.push_keyboard_scancode(0x41);
    assert_eq!(next_irq_vector(bus), NORMAL_KEY_VECTOR);
}

#[test]
fn function_key_press_raises_the_function_vector() {
    let mut machine = build_machine(Pc6000Model::Pc6001);
    let bus = &mut machine.bus;
    bus.push_keyboard_scancode(FUNCTION_KEY_ID);
    assert_eq!(next_irq_vector(bus), FUNCTION_KEY_VECTOR);
}

#[test]
fn key_release_raises_a_fresh_scan() {
    let mut machine = build_machine(Pc6000Model::Pc6001);
    let bus = &mut machine.bus;

    bus.push_keyboard_scancode(0x41);
    assert_eq!(next_irq_vector(bus), NORMAL_KEY_VECTOR);

    // Releasing the key scans back to "no key" and signals again.
    bus.push_keyboard_scancode(0x41 | RELEASE_FLAG);
    assert_eq!(next_irq_vector(bus), NORMAL_KEY_VECTOR);
}

#[test]
fn joystick_trigger_subcommand_raises_the_joystick_irq() {
    let mut machine = build_machine(Pc6000Model::Pc6001);
    let bus = &mut machine.bus;

    // Sub-command 0x06 through PPI port A triggers the joystick interrupt.
    bus.io_write(0x90, 0x06);
    assert!(bus.has_irq(), "the joystick trigger raised no interrupt");
    assert_eq!(bus.acknowledge_irq(), JOYSTICK_VECTOR);
}
