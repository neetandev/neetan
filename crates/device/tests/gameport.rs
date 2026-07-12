//! Unit tests for the analog game port.

use device::gameport::GamePort;

/// A game port clocked at 1 MHz, so one core cycle is one microsecond and the
/// discharge cycle counts equal the discharge times in microseconds.
fn port() -> GamePort {
    GamePort::new(1_000_000)
}

// Discharge times in microseconds for the documented axis extremes and center.
const MIN_DISCHARGE: u64 = 24; // potentiometer at 0 ohms
const CENTER_DISCHARGE: u64 = 578; // potentiometer at mid-scale
const MAX_DISCHARGE: u64 = 1133; // potentiometer at full scale

#[test]
fn axis_one_shot_scales_with_position() {
    // Minimum: both axes drop just after the base discharge time.
    let mut min = port();
    min.set_present(0, true);
    min.set_axes(0, -32768, -32768);
    min.write(0);
    assert_eq!(min.read(MIN_DISCHARGE - 1) & 0x03, 0x03);
    assert_eq!(min.read(MIN_DISCHARGE) & 0x03, 0x00);

    // Center: mid-scale discharge.
    let mut center = port();
    center.set_present(0, true);
    center.set_axes(0, 0, 0);
    center.write(0);
    assert_eq!(center.read(CENTER_DISCHARGE - 1) & 0x03, 0x03);
    assert_eq!(center.read(CENTER_DISCHARGE) & 0x03, 0x00);

    // Maximum: longest discharge.
    let mut max = port();
    max.set_present(0, true);
    max.set_axes(0, 32767, 32767);
    max.write(0);
    assert_eq!(max.read(MAX_DISCHARGE - 1) & 0x03, 0x03);
    assert_eq!(max.read(MAX_DISCHARGE) & 0x03, 0x00);
}

#[test]
fn axis_reads_high_immediately_after_fire() {
    let mut port = port();
    port.set_present(0, true);
    port.set_axes(0, 0, 0);
    port.write(100);
    assert_eq!(port.read(100) & 0x03, 0x03);
}

#[test]
fn buttons_are_active_low_after_discharge() {
    let mut port = port();
    port.set_present(0, true);
    port.set_axes(0, 0, 0);
    port.set_buttons(0, true, false);
    port.write(0);
    // Read well after every axis has discharged.
    let value = port.read(2000);
    assert_eq!(value & 0x0F, 0); // axis bits low
    assert_eq!(value & 0x10, 0); // button 1 pressed reads low
    assert_ne!(value & 0x20, 0); // button 2 released reads high
}

#[test]
fn buttons_hold_released_while_axes_discharge() {
    let mut port = port();
    port.set_present(0, true);
    port.set_axes(0, 0, 0);
    port.set_buttons(0, true, true);
    port.write(0);
    // While the axes are still discharging, button lines read released.
    let value = port.read(10);
    assert_eq!(value & 0x03, 0x03);
    assert_eq!(value & 0xF0, 0xF0);
}

#[test]
fn absent_stick_reads_low_and_released() {
    let mut port = port();
    // Stick 1 is left absent while its inputs are set.
    port.set_axes(1, 32767, 32767);
    port.set_buttons(1, true, true);
    port.write(0);
    assert_eq!(port.read(10), 0xF0);
}

#[test]
fn reset_clears_state() {
    let mut port = port();
    port.set_present(0, true);
    port.set_axes(0, 32767, 32767);
    port.write(0);
    port.reset();
    // No stick present after reset: axes low, buttons released.
    assert_eq!(port.read(0), 0xF0);
}
