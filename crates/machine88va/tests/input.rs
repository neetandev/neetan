//! Input device integration tests: the HLE keyboard (IRQ1, the data port and the
//! active-low scan matrix) and the bus mouse read through the SSG ports, driven
//! through the public bus surface.

use common::{Bus, JoystickState, Machine};
use machine88va::Pc88VaMachine;

#[path = "common/harness.rs"]
mod harness;
use harness::*;

/// Steps the mouse nibble machine one phase by toggling port 0x040 bit 6.
fn mouse_strobe(machine: &mut Pc88VaMachine, level: u8) {
    machine.bus.io_write_byte(0x040, level << 6);
}

#[test]
fn keyboard_push_raises_irq1() {
    let mut machine = machine();
    machine.push_keyboard_scancode(0x3A);
    assert!(machine.bus.has_irq());
    // Master IR1 acknowledges as vector 0x09.
    assert_eq!(machine.bus.acknowledge_irq(), 0x09);
}

#[test]
fn keyboard_data_port_returns_code_and_clears_irq() {
    let mut machine = machine();
    machine.push_keyboard_scancode(0x12);
    machine.push_keyboard_scancode(0x34);

    assert_eq!(machine.bus.io_read_byte(0x1C1), 0x12);
    // A code is still queued, so the interrupt stays asserted.
    assert!(machine.bus.has_irq());
    assert_eq!(machine.bus.io_read_byte(0x1C1), 0x34);
    // The queue is now empty; the interrupt clears.
    assert!(!machine.bus.has_irq());
    assert_eq!(machine.bus.io_read_byte(0x1C1), 0x00);
}

#[test]
fn queued_keycodes_each_get_their_own_irq_edge() {
    // The keyboard IRQ is edge-triggered, and acknowledging it clears the request.
    // When several codes are queued (rapid typing sends make+break together), each
    // must be delivered by its own edge, mirroring how the ISR drains port 0x1C1.
    let mut machine = machine();
    machine.push_keyboard_scancode(0x1D); // 'A' make
    machine.push_keyboard_scancode(0x9D); // 'A' break
    machine.push_keyboard_scancode(0x1E); // 'S' make

    for expected in [0x1D, 0x9D, 0x1E] {
        assert!(
            machine.bus.has_irq(),
            "keyboard IRQ missing before {expected:#04x}"
        );
        assert_eq!(machine.bus.acknowledge_irq(), 0x09);
        assert_eq!(machine.bus.io_read_byte(0x1C1), expected);
        // Acknowledging cleared the edge; the read must re-assert it while codes
        // remain so the next one is delivered.
        machine.bus.io_write_byte(0x188, 0x20); // non-specific EOI to the master
    }
    assert!(
        !machine.bus.has_irq(),
        "keyboard IRQ stuck after draining the queue"
    );
    assert_eq!(machine.bus.io_read_byte(0x1C1), 0x00);
}

#[test]
fn keyboard_scancode_drives_the_active_low_matrix() {
    let mut machine = machine();
    // 'A' is VA keycode 0x1D; it maps to matrix row 2, column 1.
    machine.push_keyboard_scancode(0x1D);
    assert_eq!(machine.bus.io_read_byte(0x002), 0xFD);
    // F8 is VA keycode 0x69; it maps to row 0x0D, column 2. The boot ROM reads
    // this bit to enter the setup menu.
    machine.push_keyboard_scancode(0x69);
    assert_eq!(machine.bus.io_read_byte(0x00D), 0xFB);
    // Releasing 'A' (bit 7 set) restores its row.
    machine.push_keyboard_scancode(0x80 | 0x1D);
    assert_eq!(machine.bus.io_read_byte(0x002), 0xFF);
}

#[test]
fn mouse_delta_and_buttons_read_through_ssg_ports() {
    let mut machine = machine();
    machine.push_mouse_delta(-33, -18);
    machine.set_mouse_buttons(true, false, false);

    // Select the SSG data-nibble register (0x0E) and walk XH, XL, YH, YL.
    machine.bus.io_write_byte(0x044, 0x0E);
    // The first strobe edge latches the delta and exposes the XH nibble.
    // latch_x = -33 -> x = 33 = 0x21; latch_y = -18 -> y = 18 = 0x12.
    mouse_strobe(&mut machine, 1);
    assert_eq!(machine.bus.io_read_byte(0x045), 0xF0 | 0x02); // XH
    mouse_strobe(&mut machine, 0);
    assert_eq!(machine.bus.io_read_byte(0x045), 0xF0 | 0x01); // XL
    mouse_strobe(&mut machine, 1);
    assert_eq!(machine.bus.io_read_byte(0x045), 0xF0 | 0x01); // YH
    mouse_strobe(&mut machine, 0);
    assert_eq!(machine.bus.io_read_byte(0x045), 0xF0 | 0x02); // YL

    // Buttons read through register 0x0F: bit1 = left pressed.
    machine.bus.io_write_byte(0x044, 0x0F);
    assert_eq!(machine.bus.io_read_byte(0x045), 0xFC | 0x02);
}

#[test]
fn joystick_directions_read_active_low_on_port_a() {
    let mut machine = machine();
    machine.set_joystick(
        0,
        JoystickState {
            up: true,
            right: true,
            ..JoystickState::default()
        },
    );

    // No mouse strobe has happened, so the joystick owns SSG port A (0x0E).
    machine.bus.io_write_byte(0x044, 0x0E);
    let port_a = machine.bus.io_read_byte(0x045);
    assert_eq!(port_a & 0x01, 0, "up reads active low");
    assert_eq!(port_a & 0x08, 0, "right reads active low");
    assert_eq!(port_a & 0x02, 0x02, "down stays released");
    assert_eq!(port_a & 0x04, 0x04, "left stays released");
    assert_eq!(port_a & 0xF0, 0xF0, "the high nibble is forced to 1");
}

#[test]
fn joystick_triggers_read_active_low_on_port_b() {
    let mut machine = machine();
    machine.set_joystick(
        0,
        JoystickState {
            trigger1: true,
            ..JoystickState::default()
        },
    );

    // Triggers read through register 0x0F while the joystick is the active device.
    machine.bus.io_write_byte(0x044, 0x0F);
    let port_b = machine.bus.io_read_byte(0x045);
    assert_eq!(port_b & 0x01, 0, "trigger 1 reads active low");
    assert_eq!(port_b & 0x02, 0x02, "trigger 2 stays released");
}

#[test]
fn mouse_and_joystick_swap_ownership_of_the_shared_port() {
    // The VA connects one device at a time; the port reports whichever device
    // last received input. An idle mouse must not look like joystick movement.
    let mut machine = machine();

    // Pressing a joystick direction selects the joystick on the shared port.
    machine.set_joystick(
        0,
        JoystickState {
            left: true,
            ..JoystickState::default()
        },
    );
    machine.bus.io_write_byte(0x044, 0x0E);
    assert_eq!(
        machine.bus.io_read_byte(0x045) & 0x04,
        0,
        "joystick left visible while the joystick is selected"
    );

    // Real mouse motion hands the port back to the mouse, which then reads its
    // movement nibble. delta_x = -64 -> x = 0x40, so the first (XH) nibble is 0x4.
    machine.push_mouse_delta(-64, 0);
    mouse_strobe(&mut machine, 1);
    assert_eq!(
        machine.bus.io_read_byte(0x045),
        0xF4,
        "mouse owns the port after real motion"
    );

    // Pressing the joystick again selects it back.
    machine.set_joystick(
        0,
        JoystickState {
            left: true,
            ..JoystickState::default()
        },
    );
    assert_eq!(
        machine.bus.io_read_byte(0x045) & 0x04,
        0,
        "joystick reclaims the port on the next press"
    );
}

#[test]
fn idle_mouse_reads_zero_nibbles_not_joystick_lines() {
    // Regression: with the mouse selected (the default) and no movement, every
    // nibble must read zero. If the idle joystick lines (0xF) leaked onto the
    // port the driver would decode a phantom -1 delta and the cursor would drift.
    let mut machine = machine();

    machine.bus.io_write_byte(0x044, 0x0E);
    for level in [1, 0, 1, 0] {
        mouse_strobe(&mut machine, level);
        assert_eq!(
            machine.bus.io_read_byte(0x045),
            0xF0,
            "an unmoved mouse must read a zero nibble"
        );
    }
}
