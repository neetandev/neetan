//! Bidirectional cursor sync across HLE DOS syscall dispatch.
//!
//! The BIOS owns the master GDC cursor state and the HLE DOS owns the IOSYS
//! cursor fields. `NeetanDos::dispatch` reconciles them at entry (GDC -> IOSYS)
//! and exit (IOSYS -> GDC) so BIOS-side updates survive intervening DOS calls
//! and OS-side updates (e.g. ANSI ESC sequences) propagate to the hardware.

use crate::harness;

#[test]
fn bios_hide_cursor_survives_dos_syscall() {
    let mut machine = harness::boot_hle();

    // Show the cursor via BIOS, hide it via BIOS, then trigger a harmless
    // HLE DOS syscall (INT 2Ah is a no-op stub but still goes through the
    // NeetanDos dispatch path). Without the pre-dispatch GDC -> IOSYS sync,
    // the post-dispatch IOSYS -> GDC sync would read a stale IOSYS
    // (cursor_visible = 1, initialised at boot) and revert the BIOS hide.
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x11, 0xCD, 0x18,     // INT 18h AH=11h  (show cursor)
        0xB4, 0x12, 0xCD, 0x18,     // INT 18h AH=12h  (hide cursor)
        0xCD, 0x2A,                 // INT 2Ah         (no-op HLE DOS syscall)
        0xFA, 0xF4,                 // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let state = machine.inspection_state();
    assert!(
        !state.gdc_master.cursor_display,
        "BIOS AH=12h hide must survive intervening HLE DOS syscall dispatch",
    );
}

#[test]
fn bios_cursor_position_survives_dos_syscall() {
    let mut machine = harness::boot_hle();

    // AH=13h sets the master-GDC cursor position (DX is the byte offset;
    // ead = DX / 2). DX=0x0140 -> ead = 0xA0 = row 2, col 0. Without the
    // pre-dispatch sync, the post-dispatch IOSYS -> GDC sync would read
    // IOSYS_OFF_CURSOR_X/Y (still 0, initialised at boot) and move the
    // hardware cursor back to (0, 0).
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x13, 0xBA, 0x40, 0x01, 0xCD, 0x18,   // INT 18h AH=13h DX=0x0140
        0xCD, 0x2A,                                 // INT 2Ah (no-op)
        0xFA, 0xF4,                                 // CLI; HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let state = machine.inspection_state();
    assert_eq!(
        state.gdc_master.ead, 0xA0,
        "BIOS AH=13h cursor position must survive intervening HLE DOS syscall dispatch",
    );
}

#[test]
fn os_cursor_show_escape_propagates_to_gdc() {
    let mut machine = harness::boot_hle();

    // Hide the cursor via BIOS, then feed ESC[>5l through INT 29h. The
    // HLE DOS ESC handler sets IOSYS_OFF_CURSOR_VISIBLE = 0x01. The
    // post-dispatch IOSYS -> GDC sync must propagate the change back to
    // the master GDC.
    let mut code: Vec<u8> = vec![
        0xB4, 0x12, 0xCD, 0x18, // INT 18h AH=12h (hide cursor)
    ];
    for byte in [0x1B, b'[', b'>', b'5', b'l'] {
        code.extend_from_slice(&[0xB0, byte, 0xCD, 0x29]); // MOV AL,byte; INT 29h
    }
    code.extend_from_slice(&[0xFA, 0xF4]); // CLI; HLT
    harness::inject_and_run(&mut machine, &code);

    let state = machine.inspection_state();
    assert!(
        state.gdc_master.cursor_display,
        "OS-level ESC[>5l must reach the hardware via post-dispatch sync",
    );
}

#[test]
fn os_cursor_hide_escape_propagates_to_gdc() {
    let mut machine = harness::boot_hle();

    // Show the cursor via BIOS, then feed ESC[>5h through INT 29h. The
    // HLE DOS ESC handler sets IOSYS_OFF_CURSOR_VISIBLE = 0x00. The
    // post-dispatch IOSYS -> GDC sync must propagate the change back to
    // the master GDC.
    let mut code: Vec<u8> = vec![
        0xB4, 0x11, 0xCD, 0x18, // INT 18h AH=11h (show cursor)
    ];
    for byte in [0x1B, b'[', b'>', b'5', b'h'] {
        code.extend_from_slice(&[0xB0, byte, 0xCD, 0x29]);
    }
    code.extend_from_slice(&[0xFA, 0xF4]);
    harness::inject_and_run(&mut machine, &code);

    let state = machine.inspection_state();
    assert!(
        !state.gdc_master.cursor_display,
        "OS-level ESC[>5h must reach the hardware via post-dispatch sync",
    );
}
