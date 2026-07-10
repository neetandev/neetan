//! Full-machine input tests: host keyboard, joystick, and mouse input
//! reaching CPU-visible registers through the `Machine` trait entry points.

#[path = "common/harness.rs"]
mod harness;

use common::{JoystickState, Machine};
use harness::{STOP_MASKED, byte_write_script, scripted_machine};
use machinex68k::X68kModel;

/// One mouse byte time in CPU cycles at 10 MHz.
const MOUSE_BYTE_CYCLES: u64 = 23_232;

#[test]
fn keyboard_scancodes_reach_a_polling_cpu_through_the_mfp_usart() {
    // MFP USART receive setup mirroring the IPL: UCR, RSR/TSR enables, the
    // keyboard-control port, and one keyboard command byte.
    let mut program = byte_write_script(&[
        (0xE88021, 13),
        (0xE8802B, 1),
        (0xE8802D, 1),
        (0xE8E007, 8),
        (0xE8802F, 0x49),
    ]);
    // Poll RSR buffer-full, then store the received byte:
    //   loop: tst.b (0xE8802B).l ; bpl.s loop
    program.extend([0x4A39, 0x00E8, 0x802B, 0x6AF8]);
    // move.b (0xE8802F).l, (0x2000).l
    program.extend([0x13F9, 0x00E8, 0x802F, 0x0000, 0x2000]);
    program.extend(STOP_MASKED);

    let mut machine = scripted_machine(X68kModel::X68000, &program);
    machine.run_for(20_000);
    assert_eq!(machine.bus.ram_byte(0x2000), Some(0));
    machine.push_keyboard_scancode(0x1E);
    machine.run_for(100_000);
    assert_eq!(
        machine.bus.ram_byte(0x2000),
        Some(0x1E),
        "the polled scancode must land in RAM"
    );
}

#[test]
fn joystick_states_reach_the_cpu_through_the_ppi_ports() {
    // PPI control: ports A and B as inputs, then copy both joystick ports.
    let mut program = byte_write_script(&[(0xE9A007, 0x92)]);
    // move.b (0xE9A001).l, (0x2000).l and move.b (0xE9A003).l, (0x2001).l
    program.extend([0x13F9, 0x00E9, 0xA001, 0x0000, 0x2000]);
    program.extend([0x13F9, 0x00E9, 0xA003, 0x0000, 0x2001]);
    program.extend(STOP_MASKED);

    let mut machine = scripted_machine(X68kModel::X68000, &program);
    machine.set_joystick(
        0,
        JoystickState {
            up: true,
            trigger1: true,
            ..JoystickState::default()
        },
    );
    machine.set_joystick(
        1,
        JoystickState {
            right: true,
            trigger2: true,
            ..JoystickState::default()
        },
    );
    machine.run_for(5_000);
    assert_eq!(
        machine.bus.ram_byte(0x2000),
        Some(0xDE),
        "port A active-low"
    );
    assert_eq!(
        machine.bus.ram_byte(0x2001),
        Some(0xB7),
        "port B active-low"
    );
}

#[test]
fn mouse_input_reaches_a_polling_cpu_as_a_paced_packet() {
    // SCC channel B mouse setup ending in the MSCTRL request edge.
    let mut program = byte_write_script(&[
        (0xE98001, 9),
        (0xE98001, 0x09),
        (0xE98001, 2),
        (0xE98001, 0x40),
        (0xE98001, 1),
        (0xE98001, 0x10),
        (0xE98001, 5),
        (0xE98001, 0x60),
        (0xE98001, 5),
        (0xE98001, 0x62),
    ]);
    // Three polled receives, each waiting on RR0 bit 0:
    //   loop: btst #0, (0xE98001).l ; beq.s loop
    //   move.b (0xE98003).l, (0x2000+N).l
    for index in 0..3u16 {
        program.extend([0x0839, 0x0000, 0x00E9, 0x8001, 0x67F6]);
        program.extend([0x13F9, 0x00E9, 0x8003, 0x0000, 0x2000 + index]);
    }
    program.extend(STOP_MASKED);

    let mut machine = scripted_machine(X68kModel::X68000, &program);
    machine.push_mouse_delta(5, -3);
    machine.set_mouse_buttons(true, false, false);
    machine.run_for(5 * MOUSE_BYTE_CYCLES);
    assert_eq!(machine.bus.ram_byte(0x2000), Some(0x01), "left button held");
    assert_eq!(machine.bus.ram_byte(0x2001), Some(0x05), "X delta");
    assert_eq!(machine.bus.ram_byte(0x2002), Some(0xFD), "Y delta");
}
