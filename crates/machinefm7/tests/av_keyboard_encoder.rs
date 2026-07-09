//! FM-77AV keyboard encoder and RTC: the `0xD431`/`0xD432` command handshake,
//! scancode-mode and LED commands, programmable auto-repeat, the embedded
//! calendar clock, and the `0xD40D` INSERT LED register.

mod harness;

use common::Bus;
use harness::{build_av_bus_with_synthetic_roms, run_bus_cycles};
use machinefm7::{BootMode, Fm7Bus, SubBusView};

/// `0xD431` encoder data register.
const ENCODER_DATA: u16 = 0xD431;
/// `0xD432` encoder status register.
const ENCODER_STATUS: u16 = 0xD432;
/// `0xD40D` INSERT LED register.
const KEY_LED: u16 = 0xD40D;
/// `0xD400` sub keyboard high byte (bit 7 carries the ninth keycode bit).
const KEYBOARD_HIGH: u16 = 0xD400;
/// `0xD401` sub keyboard low byte (read clears the key interrupt).
const KEYBOARD_LOW: u16 = 0xD401;

/// Status bit 7 (RXRDY): clear while response data waits.
const STATUS_RXRDY: u8 = 0x80;
/// Status bit 0 (ACK): clear while the encoder is busy.
const STATUS_ACK: u8 = 0x01;

/// Command: set the scancode mode.
const CMD_SET_MODE: u8 = 0x00;
/// Command: get the scancode mode.
const CMD_GET_MODE: u8 = 0x01;
/// Command: set the LED state.
const CMD_SET_LED: u8 = 0x02;
/// Command: get the LED state.
const CMD_GET_LED: u8 = 0x03;
/// Command: set the auto-repeat type.
const CMD_SET_REPEAT_TYPE: u8 = 0x04;
/// Command: set the auto-repeat timing.
const CMD_SET_REPEAT_TIME: u8 = 0x05;
/// Command: access the RTC.
const CMD_RTC: u8 = 0x80;
/// RTC sub-command: read the clock.
const RTC_GET: u8 = 0x00;
/// RTC sub-command: write the clock.
const RTC_SET: u8 = 0x01;

/// Scancode-mode value for the standard keycode mode.
const MODE_STANDARD: u8 = 0x00;
/// Scancode-mode value for the FM-16beta compatible mode.
const MODE_16BETA: u8 = 0x01;
/// Scancode-mode value for raw scan mode.
const MODE_SCAN: u8 = 0x02;
/// LED selector: CAPS on (select CAPS, value bit clear).
const LED_CAPS_ON: u8 = 0x00;
/// LED selector: CAPS off (select CAPS, value bit set).
const LED_CAPS_OFF: u8 = 0x01;
/// LED selector: KANA on (select KANA, value bit clear).
const LED_KANA_ON: u8 = 0x02;
/// Repeat-type argument enabling auto-repeat.
const REPEAT_ENABLE: u8 = 0x00;

/// Physical scancode of the `Q` key (repeatable, latches `q` = 0x71).
const SCANCODE_Q: u8 = 0x11;
/// Physical scancode of the left SHIFT key.
const SCANCODE_SHIFT_LEFT: u8 = 0x53;
/// Physical scancode of the INS key (standard keycode 0x012, 16beta 0x110).
const SCANCODE_INS: u8 = 0x48;
/// Release flag OR-ed into a scancode.
const RELEASE: u8 = 0x80;

/// Main-clock cycles that comfortably exceed the encoder ACK handshake delay.
const ACK_SETTLE_CYCLES: u64 = 512;
/// Poll step used when counting latched keycodes; shorter than a latch period.
const POLL_STEP_CYCLES: u64 = 8_000;

/// Builds an FM-77AV bus with synthetic ROMs.
fn build_av_bus() -> Fm7Bus {
    build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {})
}

/// Reads a sub MMIO byte through a `SubBusView`, honoring read side effects.
fn sub_read(bus: &mut Fm7Bus, address: u16) -> u8 {
    let mut view = SubBusView { bus };
    view.read_byte(u32::from(address))
}

/// Writes a sub MMIO byte through a `SubBusView`.
fn sub_write(bus: &mut Fm7Bus, address: u16, value: u8) {
    let mut view = SubBusView { bus };
    view.write_byte(u32::from(address), value);
}

/// Writes one command byte and lets the ACK handshake settle so the next byte is
/// accepted.
fn send_byte(bus: &mut Fm7Bus, byte: u8) {
    sub_write(bus, ENCODER_DATA, byte);
    run_bus_cycles(bus, ACK_SETTLE_CYCLES);
}

/// Sends a whole command byte by byte.
fn send_command(bus: &mut Fm7Bus, bytes: &[u8]) {
    for &byte in bytes {
        send_byte(bus, byte);
    }
}

/// Drains up to `count` response bytes.
fn read_responses(bus: &mut Fm7Bus, count: usize) -> Vec<u8> {
    (0..count).map(|_| sub_read(bus, ENCODER_DATA)).collect()
}

#[test]
fn scancode_mode_round_trips() {
    let mut bus = build_av_bus();
    send_command(&mut bus, &[CMD_SET_MODE, MODE_SCAN]);
    send_command(&mut bus, &[CMD_GET_MODE]);
    assert_eq!(read_responses(&mut bus, 1), vec![MODE_SCAN]);
}

#[test]
fn led_commands_round_trip() {
    let mut bus = build_av_bus();
    send_command(&mut bus, &[CMD_SET_LED, LED_CAPS_ON]);
    send_command(&mut bus, &[CMD_SET_LED, LED_KANA_ON]);
    send_command(&mut bus, &[CMD_GET_LED]);
    // Response bit 0 = CAPS, bit 1 = KANA; both lit.
    assert_eq!(read_responses(&mut bus, 1), vec![0x03]);

    send_command(&mut bus, &[CMD_SET_LED, LED_CAPS_OFF]);
    send_command(&mut bus, &[CMD_GET_LED]);
    // CAPS cleared, KANA still lit.
    assert_eq!(read_responses(&mut bus, 1), vec![0x02]);
}

#[test]
fn status_reflects_ack_and_rxrdy() {
    let mut bus = build_av_bus();
    // Idle: ACK up, nothing to read.
    let idle = sub_read(&mut bus, ENCODER_STATUS);
    assert_ne!(idle & STATUS_ACK, 0, "ACK is up at rest");
    assert_ne!(idle & STATUS_RXRDY, 0, "no response waiting at rest");

    // A get-mode byte completes immediately and queues a response.
    sub_write(&mut bus, ENCODER_DATA, CMD_GET_MODE);
    let busy = sub_read(&mut bus, ENCODER_STATUS);
    assert_eq!(busy & STATUS_ACK, 0, "ACK drops while busy");
    assert_eq!(busy & STATUS_RXRDY, 0, "RXRDY asserts with data waiting");

    run_bus_cycles(&mut bus, ACK_SETTLE_CYCLES);
    let settled = sub_read(&mut bus, ENCODER_STATUS);
    assert_ne!(
        settled & STATUS_ACK,
        0,
        "ACK returns after the handshake delay"
    );

    // Draining the response clears RXRDY.
    assert_eq!(
        sub_read(&mut bus, ENCODER_DATA),
        0x00,
        "default Standard mode"
    );
    let drained = sub_read(&mut bus, ENCODER_STATUS);
    assert_ne!(drained & STATUS_RXRDY, 0, "RXRDY clears once drained");
}

#[test]
fn rtc_set_and_get_round_trip() {
    let mut bus = build_av_bus();
    // 2023-06-15 (Thursday = 4), 13:45:30, 24-hour form.
    let packed = [0x23, 0x06, 0x15, 0x41, 0x34, 0x53, 0x00];
    let mut command = vec![CMD_RTC, RTC_SET];
    command.extend_from_slice(&packed);
    send_command(&mut bus, &command);

    send_command(&mut bus, &[CMD_RTC, RTC_GET]);
    assert_eq!(read_responses(&mut bus, packed.len()), packed.to_vec());
}

#[test]
fn rtc_advances_one_second() {
    let mut bus = build_av_bus();
    // 2023-06-15 13:45:30.
    let packed = [0x23, 0x06, 0x15, 0x41, 0x34, 0x53, 0x00];
    let mut command = vec![CMD_RTC, RTC_SET];
    command.extend_from_slice(&packed);
    send_command(&mut bus, &command);

    // Advance just past one RTC period (one second of main-clock cycles).
    let one_second = u64::from(bus.cpu_clock_hz()) + POLL_STEP_CYCLES;
    run_bus_cycles(&mut bus, one_second);

    send_command(&mut bus, &[CMD_RTC, RTC_GET]);
    let response = read_responses(&mut bus, packed.len());
    // Second 30 -> 31: byte 5 keeps minute-ones 5 and second-tens 3, byte 6
    // becomes second-ones 1.
    assert_eq!(response[5], 0x53);
    assert_eq!(response[6], 0x10);
}

#[test]
fn insert_led_register_toggles() {
    let mut bus = build_av_bus();
    // Reading 0xD40D lights the INSERT LED.
    sub_read(&mut bus, KEY_LED);
    assert_eq!(bus.keyboard_led_status() & 0x01, 0x01, "INSERT lit by read");
    // Writing 0xD40D clears it.
    sub_write(&mut bus, KEY_LED, 0x00);
    assert_eq!(
        bus.keyboard_led_status() & 0x01,
        0x00,
        "INSERT cleared by write"
    );
}

/// Counts keycodes latched over `cycles`, draining each as it appears. The
/// keyboard IRQ mask defaults to disabled, so a latched keycode surfaces on the
/// sub FIRQ line.
fn count_latched_keycodes(bus: &mut Fm7Bus, cycles: u64) -> usize {
    let end = bus.current_cycle() + cycles;
    let mut count = 0;
    while bus.current_cycle() < end {
        run_bus_cycles(bus, POLL_STEP_CYCLES);
        if bus.sub_has_firq() {
            sub_read(bus, KEYBOARD_LOW);
            count += 1;
        }
    }
    count
}

/// Main-clock cycles spanning roughly 300 ms of observation.
fn observation_window(bus: &Fm7Bus) -> u64 {
    u64::from(bus.cpu_clock_hz()) * 3 / 10
}

#[test]
fn auto_repeat_generates_extra_keycodes() {
    let mut bus = build_av_bus();
    // Without repeat a single press latches exactly one keycode.
    bus.push_keyboard_scancode(SCANCODE_Q);
    let window = observation_window(&bus);
    assert_eq!(count_latched_keycodes(&mut bus, window), 1);
    bus.push_keyboard_scancode(SCANCODE_Q | RELEASE);

    // Enable repeat with short timing (100 ms delay, 20 ms interval).
    send_command(&mut bus, &[CMD_SET_REPEAT_TYPE, REPEAT_ENABLE]);
    send_command(&mut bus, &[CMD_SET_REPEAT_TIME, 10, 2]);

    bus.push_keyboard_scancode(SCANCODE_Q);
    let window = observation_window(&bus);
    let repeated = count_latched_keycodes(&mut bus, window);
    assert!(
        repeated > 1,
        "auto-repeat latches more than one keycode: {repeated}"
    );

    // Releasing the key stops repeats: after flushing, no more keycodes appear.
    bus.push_keyboard_scancode(SCANCODE_Q | RELEASE);
    let flush = observation_window(&bus);
    count_latched_keycodes(&mut bus, flush);
    let window = observation_window(&bus);
    assert_eq!(
        count_latched_keycodes(&mut bus, window),
        0,
        "release cancels repeat"
    );
}

/// Collects the key bytes latched over `cycles`, draining each as it appears.
fn collect_latched_keycodes(bus: &mut Fm7Bus, cycles: u64) -> Vec<u8> {
    let end = bus.current_cycle() + cycles;
    let mut codes = Vec::new();
    while bus.current_cycle() < end {
        run_bus_cycles(bus, POLL_STEP_CYCLES);
        if bus.sub_has_firq() {
            codes.push(sub_read(bus, KEYBOARD_LOW));
        }
    }
    codes
}

#[test]
fn scan_mode_reports_raw_make_and_break_codes() {
    let mut bus = build_av_bus();
    let window = observation_window(&bus);

    // Standard mode: a press latches the translated keycode, a release nothing.
    bus.push_keyboard_scancode(SCANCODE_Q);
    bus.push_keyboard_scancode(SCANCODE_Q | RELEASE);
    assert_eq!(collect_latched_keycodes(&mut bus, window), vec![0x71]);

    send_command(&mut bus, &[CMD_SET_MODE, MODE_SCAN]);

    // Scan mode: presses and releases both latch the physical scancode, with
    // bit 7 marking the release.
    bus.push_keyboard_scancode(SCANCODE_Q);
    assert_eq!(collect_latched_keycodes(&mut bus, window), vec![SCANCODE_Q]);
    bus.push_keyboard_scancode(SCANCODE_Q | RELEASE);
    assert_eq!(
        collect_latched_keycodes(&mut bus, window),
        vec![SCANCODE_Q | RELEASE]
    );

    // Modifier keys report their scancodes too in scan mode.
    bus.push_keyboard_scancode(SCANCODE_SHIFT_LEFT);
    bus.push_keyboard_scancode(SCANCODE_SHIFT_LEFT | RELEASE);
    assert_eq!(
        collect_latched_keycodes(&mut bus, window),
        vec![SCANCODE_SHIFT_LEFT, SCANCODE_SHIFT_LEFT | RELEASE]
    );

    // Back in standard mode the translated keycode path resumes, with the
    // modifier state kept consistent across the scan-mode excursion.
    send_command(&mut bus, &[CMD_SET_MODE, MODE_STANDARD]);
    bus.push_keyboard_scancode(SCANCODE_Q);
    bus.push_keyboard_scancode(SCANCODE_Q | RELEASE);
    assert_eq!(collect_latched_keycodes(&mut bus, window), vec![0x71]);
}

/// Collects the full 9-bit keycodes latched over `cycles`, sampling the high
/// bit before the low-byte read clears the interrupt.
fn collect_latched_keycodes_wide(bus: &mut Fm7Bus, cycles: u64) -> Vec<u16> {
    let end = bus.current_cycle() + cycles;
    let mut codes = Vec::new();
    while bus.current_cycle() < end {
        run_bus_cycles(bus, POLL_STEP_CYCLES);
        if bus.sub_has_firq() {
            let high = sub_read(bus, KEYBOARD_HIGH) & 0x80 != 0;
            let low = sub_read(bus, KEYBOARD_LOW);
            codes.push(u16::from(low) | if high { 0x100 } else { 0 });
        }
    }
    codes
}

#[test]
fn fm16beta_mode_uses_its_own_keycode_tables() {
    let mut bus = build_av_bus();
    let window = observation_window(&bus);

    // Standard mode: INS latches the control code 0x012.
    bus.push_keyboard_scancode(SCANCODE_INS);
    bus.push_keyboard_scancode(SCANCODE_INS | RELEASE);
    assert_eq!(collect_latched_keycodes_wide(&mut bus, window), vec![0x012]);

    send_command(&mut bus, &[CMD_SET_MODE, MODE_16BETA]);

    // FM-16beta mode: INS latches the 9-bit editing keycode 0x110.
    bus.push_keyboard_scancode(SCANCODE_INS);
    bus.push_keyboard_scancode(SCANCODE_INS | RELEASE);
    assert_eq!(collect_latched_keycodes_wide(&mut bus, window), vec![0x110]);

    // Letter keys share their codes between the sets.
    bus.push_keyboard_scancode(SCANCODE_Q);
    bus.push_keyboard_scancode(SCANCODE_Q | RELEASE);
    assert_eq!(collect_latched_keycodes_wide(&mut bus, window), vec![0x071]);

    // Back in standard mode the control code returns.
    send_command(&mut bus, &[CMD_SET_MODE, MODE_STANDARD]);
    bus.push_keyboard_scancode(SCANCODE_INS);
    bus.push_keyboard_scancode(SCANCODE_INS | RELEASE);
    assert_eq!(collect_latched_keycodes_wide(&mut bus, window), vec![0x012]);
}
