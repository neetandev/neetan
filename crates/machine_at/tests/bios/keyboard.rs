//! INT 09h keyboard interrupt: scancode translation through the full
//! hardware path (KBC delivery, stub intercept, latch port, BDA buffer).

use common::{Cpu, Machine};
use machine_at::{AT_KEY_CURSOR_UP, AT_KEY_DELETE, AT_KEY_RIGHT_CTRL};

use super::{
    BDA_ALT_NUMPAD_ACCUMULATOR, BDA_BREAK_FLAG, BDA_KEYBOARD_FLAGS_1, BDA_KEYBOARD_HEAD,
    BDA_KEYBOARD_LEDS, BDA_KEYBOARD_MODE, BDA_KEYBOARD_TAIL, IDLE_LOOP_CODE, KEYBOARD_BUFFER,
    RESULT, boot_push_keys_and_run, create_machine_dx50, create_machine_dx66, inject_and_run,
    make_halt_boot_floppy, read_ivt_vector, read_ram_u8, read_ram_u16, write_bytes,
};

/// Shift flags 1: left shift key pressed.
const FLAG1_LEFT_SHIFT: u8 = 0x02;
/// Shift flags 1: either control key pressed.
const FLAG1_CONTROL: u8 = 0x04;
/// Shift flags 1: scroll lock active.
const FLAG1_SCROLL_ACTIVE: u8 = 0x10;
/// Shift flags 1: num lock active.
const FLAG1_NUM_ACTIVE: u8 = 0x20;
/// Shift flags 1: caps lock active.
const FLAG1_CAPS_ACTIVE: u8 = 0x40;
/// Keyboard mode: right control key pressed.
const MODE_RIGHT_CONTROL: u8 = 0x04;
/// Keyboard LED flags: scroll lock.
const LED_SCROLL: u8 = 0x01;
/// Keyboard LED flags: num lock.
const LED_NUM: u8 = 0x02;
/// Keyboard LED flags: caps lock.
const LED_CAPS: u8 = 0x04;
/// Keyboard LED flags: the three lock LED mirror bits.
const LED_STATE_MASK: u8 = 0x07;
/// Keyboard LED flags: the keyboard acknowledged the LED command.
const LED_ACKNOWLEDGED: u8 = 0x10;
/// Keyboard LED flags: an LED update is in progress.
const LED_UPDATE_IN_PROGRESS: u8 = 0x40;
/// Break flag: Ctrl-Break was pressed.
const BREAK_FLAG_PRESSED: u8 = 0x80;
/// Cycle budget for servicing a queued key stream.
const KEY_BUDGET: u64 = 5_000_000;

#[test]
fn keyboard_vectors_use_dedicated_handlers_dx50() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    let (int09h_segment, int09h_offset) = read_ivt_vector(&machine, 0x09);
    let (_, int0ah_offset) = read_ivt_vector(&machine, 0x0A);
    assert_eq!(int09h_segment, 0xF000);
    assert_ne!(int09h_offset, int0ah_offset, "INT 09h left on the EOI stub");

    let (int16h_segment, int16h_offset) = read_ivt_vector(&machine, 0x16);
    let (_, int17h_offset) = read_ivt_vector(&machine, 0x17);
    assert_eq!(int16h_segment, 0xF000);
    assert_ne!(
        int16h_offset, int17h_offset,
        "INT 16h left on the IRET stub"
    );
}

#[test]
fn keyboard_vectors_use_dedicated_handlers_dx66() {
    let mut machine = create_machine_dx66();
    boot_to_halt!(machine);

    let (int09h_segment, int09h_offset) = read_ivt_vector(&machine, 0x09);
    let (_, int0ah_offset) = read_ivt_vector(&machine, 0x0A);
    assert_eq!(int09h_segment, 0xF000);
    assert_ne!(int09h_offset, int0ah_offset, "INT 09h left on the EOI stub");
}

#[test]
fn ascii_key_buffers_scan_ascii_pair_dx50() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(&mut machine, &[0x1E, 0x9E], KEY_BUDGET);

    assert_eq!(read_ram_u16(&machine, KEYBOARD_BUFFER), 0x1E61, "'a' entry");
    assert_eq!(read_ram_u16(&machine, BDA_KEYBOARD_HEAD), 0x001E);
    assert_eq!(read_ram_u16(&machine, BDA_KEYBOARD_TAIL), 0x0020);
}

#[test]
fn ascii_key_buffers_scan_ascii_pair_dx66() {
    let mut machine = create_machine_dx66();
    boot_push_keys_and_run(&mut machine, &[0x1E, 0x9E], KEY_BUDGET);

    assert_eq!(read_ram_u16(&machine, KEYBOARD_BUFFER), 0x1E61, "'a' entry");
    assert_eq!(read_ram_u16(&machine, BDA_KEYBOARD_TAIL), 0x0020);
}

#[test]
fn release_does_not_buffer() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(&mut machine, &[0x1E, 0x9E, 0x9E], KEY_BUDGET);

    assert_eq!(
        read_ram_u16(&machine, BDA_KEYBOARD_TAIL),
        0x0020,
        "one entry"
    );
}

#[test]
fn shift_flag_tracks_press_and_release() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(&mut machine, &[0x2A], KEY_BUDGET);
    assert_ne!(
        read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & FLAG1_LEFT_SHIFT,
        0,
        "left shift pressed"
    );

    machine.push_keyboard_scancode(0xAA);
    machine.run_for(2_000_000);
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & FLAG1_LEFT_SHIFT,
        0,
        "left shift released"
    );
}

#[test]
fn shifted_letter_buffers_uppercase() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(&mut machine, &[0x2A, 0x1E, 0x9E, 0xAA], KEY_BUDGET);

    assert_eq!(read_ram_u16(&machine, KEYBOARD_BUFFER), 0x1E41, "'A' entry");
}

#[test]
fn control_and_alt_letters() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(
        &mut machine,
        &[0x1D, 0x1E, 0x9E, 0x9D, 0x38, 0x1E, 0x9E, 0xB8],
        KEY_BUDGET,
    );

    assert_eq!(read_ram_u16(&machine, KEYBOARD_BUFFER), 0x1E01, "Ctrl-A");
    assert_eq!(read_ram_u16(&machine, KEYBOARD_BUFFER + 2), 0x1E00, "Alt-A");
}

#[test]
fn caps_lock_toggles_flag_and_led() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(&mut machine, &[0x3A, 0xBA, 0x1E, 0x9E], KEY_BUDGET);

    assert_ne!(
        read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & FLAG1_CAPS_ACTIVE,
        0,
        "caps lock active"
    );
    assert_ne!(
        read_ram_u8(&machine, BDA_KEYBOARD_LEDS) & LED_CAPS,
        0,
        "caps LED on"
    );
    assert_eq!(
        read_ram_u16(&machine, KEYBOARD_BUFFER),
        0x1E41,
        "caps letter is uppercase"
    );

    machine.push_keyboard_scancode(0x3A);
    machine.push_keyboard_scancode(0xBA);
    machine.run_for(2_000_000);
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & FLAG1_CAPS_ACTIVE,
        0,
        "second toggle clears"
    );
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_LEDS) & LED_CAPS,
        0,
        "caps LED off"
    );
}

#[test]
fn num_lock_toggles_off_from_post_state() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(&mut machine, &[0x45, 0xC5], KEY_BUDGET);

    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & FLAG1_NUM_ACTIVE,
        0,
        "POST leaves NumLock on, first press turns it off"
    );
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_LEDS) & LED_NUM,
        0,
        "num LED off"
    );
}

#[test]
fn extended_cursor_key_buffers_extended_entry() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(
        &mut machine,
        &[AT_KEY_CURSOR_UP, AT_KEY_CURSOR_UP | 0x80],
        KEY_BUDGET,
    );

    assert_eq!(read_ram_u16(&machine, KEYBOARD_BUFFER), 0x48E0, "grey up");
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_MODE),
        0x10,
        "prefix bits cleared, enhanced keyboard bit intact"
    );
}

#[test]
fn right_control_sets_extended_flag() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(&mut machine, &[AT_KEY_RIGHT_CTRL], KEY_BUDGET);

    assert_ne!(
        read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & FLAG1_CONTROL,
        0,
        "combined control flag"
    );
    assert_ne!(
        read_ram_u8(&machine, BDA_KEYBOARD_MODE) & MODE_RIGHT_CONTROL,
        0,
        "right control bit"
    );

    machine.push_keyboard_scancode(AT_KEY_RIGHT_CTRL | 0x80);
    machine.run_for(2_000_000);
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & FLAG1_CONTROL,
        0,
        "combined flag cleared on release"
    );
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_MODE) & MODE_RIGHT_CONTROL,
        0,
        "right control bit cleared"
    );
}

#[test]
fn keypad_follows_num_lock_state() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(
        &mut machine,
        &[0x48, 0xC8, 0x45, 0xC5, 0x48, 0xC8],
        KEY_BUDGET,
    );

    assert_eq!(
        read_ram_u16(&machine, KEYBOARD_BUFFER),
        0x4838,
        "keypad 8 with NumLock on"
    );
    assert_eq!(
        read_ram_u16(&machine, KEYBOARD_BUFFER + 2),
        0x4800,
        "cursor up with NumLock off"
    );
}

#[test]
fn buffer_overflow_drops_sixteenth_key() {
    let mut machine = create_machine_dx50();
    let keys: Vec<u8> = (0x02..=0x0D).chain(0x10..=0x13).collect();
    assert_eq!(keys.len(), 16);
    boot_push_keys_and_run(&mut machine, &keys, KEY_BUDGET);

    assert_eq!(read_ram_u16(&machine, BDA_KEYBOARD_HEAD), 0x001E);
    assert_eq!(
        read_ram_u16(&machine, BDA_KEYBOARD_TAIL),
        0x003C,
        "fifteen entries buffered, one slot left empty"
    );
    assert_eq!(read_ram_u16(&machine, KEYBOARD_BUFFER), 0x0231, "first '1'");
    assert_eq!(
        read_ram_u16(&machine, KEYBOARD_BUFFER + 28),
        0x1265,
        "fifteenth entry is 'e', the sixteenth was dropped"
    );
}

#[test]
fn alt_numpad_accumulates_decimal_ascii() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(
        &mut machine,
        &[0x38, 0x4F, 0xCF, 0x50, 0xD0, 0xB8],
        KEY_BUDGET,
    );

    assert_eq!(
        read_ram_u16(&machine, KEYBOARD_BUFFER),
        0x000C,
        "Alt-numpad 12 buffers as ASCII 12 with zero scan"
    );
    assert_eq!(read_ram_u16(&machine, BDA_KEYBOARD_TAIL), 0x0020);
    assert_eq!(
        read_ram_u8(&machine, BDA_ALT_NUMPAD_ACCUMULATOR),
        0,
        "accumulator cleared"
    );
}

#[test]
fn japanese_keys_buffer_zero_ascii() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(&mut machine, &[0x79, 0xF9, 0x7D, 0xFD], KEY_BUDGET);

    assert_eq!(read_ram_u16(&machine, KEYBOARD_BUFFER), 0x7900, "henkan");
    assert_eq!(read_ram_u16(&machine, KEYBOARD_BUFFER + 2), 0x7D00, "yen");
}

#[test]
fn ctrl_alt_del_reboots_through_post() {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_halt_boot_floppy(), None)
        .expect("insert boot floppy");
    boot_to_halt!(machine);

    // Dirty a POST-owned BDA field so the reboot is observable.
    write_bytes(&mut machine, 0x413, &[0xAA, 0xAA]);
    machine.push_keyboard_scancode(0x1D);
    machine.push_keyboard_scancode(0x38);
    machine.push_keyboard_scancode(AT_KEY_DELETE);
    inject_and_run(&mut machine, IDLE_LOOP_CODE, &[], 20_000_000);

    assert_eq!(
        read_ram_u16(&machine, 0x413),
        640,
        "POST re-ran and re-seeded the BDA"
    );
    assert!(
        machine.cpu.halted(),
        "the reboot reached the boot sector HLT"
    );
}

#[test]
fn ctrl_break_sets_flag_and_runs_guest_handler() {
    /// Guest INT 1Bh handler: stores a marker at RESULT.
    #[rustfmt::skip]
    const BREAK_HANDLER: &[u8] = &[
        0xC6, 0x06, 0x00, 0x06, 0xA5,   // MOV BYTE [0x0600], 0xA5
        0xCF,                           // IRET
    ];

    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    // Point INT 1Bh at the callback area.
    write_bytes(&mut machine, 0x1B * 4, &[0x00, 0x20, 0x00, 0x00]);
    machine.push_keyboard_scancode(0x1D);
    machine.push_keyboard_scancode(0x46);
    machine.push_keyboard_scancode(0xC6);
    machine.push_keyboard_scancode(0x9D);
    inject_and_run(&mut machine, IDLE_LOOP_CODE, BREAK_HANDLER, KEY_BUDGET);

    assert_eq!(read_ram_u8(&machine, RESULT), 0xA5, "INT 1Bh handler ran");
    assert_ne!(
        read_ram_u8(&machine, BDA_BREAK_FLAG) & BREAK_FLAG_PRESSED,
        0,
        "break flag raised"
    );
    assert_eq!(read_ram_u16(&machine, BDA_KEYBOARD_HEAD), 0x001E);
    assert_eq!(
        read_ram_u16(&machine, BDA_KEYBOARD_TAIL),
        0x0020,
        "buffer flushed down to the null keystroke"
    );
    assert_eq!(
        read_ram_u16(&machine, KEYBOARD_BUFFER),
        0x0000,
        "null keystroke entry"
    );
    let state = machine.inspection_state();
    assert_eq!(
        state.pic.chips[0].isr & 0x02,
        0,
        "IRQ 1 acknowledged after the break chain"
    );
}

#[test]
fn int15h_hook_swallows_intercepted_key() {
    /// Guest INT 15h AH=4Fh hook: consumes the 'a' make code by clearing
    /// CF in its own IRET frame, passes everything else through.
    #[rustfmt::skip]
    const INTERCEPT_HOOK: &[u8] = &[
        0x3C, 0x1E,                     // CMP AL, 0x1E
        0x75, 0x09,                     // JNE keep
        0x55,                           // PUSH BP
        0x89, 0xE5,                     // MOV BP, SP
        0x80, 0x66, 0x06, 0xFE,         // AND BYTE [BP+6], 0xFE (clear CF)
        0x5D,                           // POP BP
        0xCF,                           // IRET
        0xCF,                           // keep: IRET
    ];

    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    // Replace the INT 15h vector with the guest hook.
    write_bytes(&mut machine, 0x15 * 4, &[0x00, 0x20, 0x00, 0x00]);
    machine.push_keyboard_scancode(0x1E);
    machine.push_keyboard_scancode(0x9E);
    machine.push_keyboard_scancode(0x30);
    machine.push_keyboard_scancode(0xB0);
    inject_and_run(&mut machine, IDLE_LOOP_CODE, INTERCEPT_HOOK, KEY_BUDGET);

    assert_eq!(
        read_ram_u16(&machine, BDA_KEYBOARD_TAIL),
        0x0020,
        "only one key survived the intercept"
    );
    assert_eq!(
        read_ram_u16(&machine, KEYBOARD_BUFFER),
        0x3062,
        "the swallowed 'a' never reached the buffer"
    );
}

#[test]
fn ctrl_num_lock_pauses_until_the_next_key() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(
        &mut machine,
        &[0x1D, 0x45, 0xC5, 0x9D, 0x1E, 0x9E, 0x30, 0xB0],
        KEY_BUDGET,
    );

    assert_eq!(
        read_ram_u8(&machine, super::BDA_KEYBOARD_FLAGS_2) & 0x08,
        0,
        "pause ended by the next make code"
    );
    assert_eq!(
        read_ram_u16(&machine, KEYBOARD_BUFFER),
        0x3062,
        "the pause-ending 'a' was consumed, 'b' buffered"
    );
    assert_eq!(read_ram_u16(&machine, BDA_KEYBOARD_TAIL), 0x0020);
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & FLAG1_NUM_ACTIVE,
        0x20,
        "Ctrl-NumLock did not toggle NumLock"
    );
}

#[test]
fn keyboard_interrupt_sends_eoi() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(&mut machine, &[0x1E, 0x9E], KEY_BUDGET);

    let state = machine.inspection_state();
    assert_eq!(state.pic.chips[0].isr & 0x02, 0, "IRQ 1 acknowledged");
}

#[test]
fn lock_keys_program_the_keyboard_leds() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    inject_and_run(&mut machine, IDLE_LOOP_CODE, &[], 1_000);

    // POST leaves num lock active, so every LED state below keeps its bit
    // until num lock itself is pressed.
    for (keys, flag, leds) in [
        (&[0x3A, 0xBA], FLAG1_CAPS_ACTIVE, LED_NUM | LED_CAPS),
        (&[0x45, 0xC5], FLAG1_NUM_ACTIVE, LED_CAPS),
        (&[0x46, 0xC6], FLAG1_SCROLL_ACTIVE, LED_CAPS | LED_SCROLL),
    ] {
        for &key in keys {
            machine.push_keyboard_scancode(key);
        }
        machine.run_for(KEY_BUDGET);

        let expected_active = if flag == FLAG1_NUM_ACTIVE { 0 } else { flag };
        assert_eq!(
            read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & flag,
            expected_active,
            "shift flag after {keys:02X?}"
        );
        assert_eq!(
            read_ram_u8(&machine, BDA_KEYBOARD_LEDS) & LED_STATE_MASK,
            leds,
            "BDA LED mirror after {keys:02X?}"
        );
        assert_eq!(
            machine.inspection_state().keyboard_leds,
            leds,
            "keyboard LED register after {keys:02X?}"
        );
        assert_eq!(
            read_ram_u8(&machine, BDA_KEYBOARD_LEDS) & (LED_ACKNOWLEDGED | LED_UPDATE_IN_PROGRESS),
            LED_ACKNOWLEDGED,
            "update finished after {keys:02X?}"
        );
    }
}

#[test]
fn led_update_is_marked_in_progress_until_the_acknowledge_arrives() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    inject_and_run(&mut machine, IDLE_LOOP_CODE, &[], 1_000);

    machine.push_keyboard_scancode(0x3A);
    // Step until INT 09h has handled the make code, which is the moment the
    // LED command goes out and its acknowledge is still in flight.
    let mut serviced = false;
    for _ in 0..64 {
        machine.run_for(1_000);
        if read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & FLAG1_CAPS_ACTIVE != 0 {
            serviced = true;
            break;
        }
    }
    assert!(serviced, "caps lock make code was serviced");
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_LEDS) & (LED_ACKNOWLEDGED | LED_UPDATE_IN_PROGRESS),
        LED_UPDATE_IN_PROGRESS,
        "update in progress, not yet acknowledged"
    );

    machine.run_for(KEY_BUDGET);
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_LEDS) & (LED_ACKNOWLEDGED | LED_UPDATE_IN_PROGRESS),
        LED_ACKNOWLEDGED,
        "acknowledge consumed the outstanding update"
    );
}

#[test]
fn led_acknowledge_bytes_stay_out_of_the_keyboard_buffer() {
    let mut machine = create_machine_dx50();
    boot_push_keys_and_run(&mut machine, &[0x3A, 0xBA, 0x1E, 0x9E], KEY_BUDGET);

    // Caps lock queues two acknowledge bytes behind the 'a' make code. Only
    // the letter may reach the buffer, and it must be the caps variant.
    assert_eq!(read_ram_u16(&machine, BDA_KEYBOARD_HEAD), 0x001E);
    assert_eq!(
        read_ram_u16(&machine, BDA_KEYBOARD_TAIL),
        0x0020,
        "one buffered entry"
    );
    assert_eq!(read_ram_u16(&machine, KEYBOARD_BUFFER), 0x1E41, "'A' entry");
    assert_eq!(
        read_ram_u8(&machine, BDA_KEYBOARD_FLAGS_1) & FLAG1_LEFT_SHIFT,
        0,
        "the 0xFA bytes were not taken for a shift release"
    );
}
