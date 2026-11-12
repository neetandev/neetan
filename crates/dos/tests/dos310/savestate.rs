use common::{HleDosVersion, Machine as _};

use crate::harness::*;

/// `INT 21h AH=30h` result of MS-DOS 3.10.
const DOS310_VERSION: u16 = 0x0A03;
/// `INT 21h AH=30h` result of MS-DOS 6.20.
const DOS620_VERSION: u16 = 0x1406;
/// Offset of the list of lists of the DOS 3.10 data area.
const DOS310_LIST_OF_LISTS_OFFSET: u16 = 0x0026;
/// Offset that `INT 21h AH=52h` returns for the DOS 6.20 data area.
const DOS620_LIST_OF_LISTS_OFFSET: u16 = 0x0010;

/// Returns the `AX` result of `INT 21h AH=30h`.
fn reported_version(machine: &mut Machine) -> u16 {
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x30,       // MOV AH, 30h
        0xCD, 0x21,       // INT 21h
        0xA3, 0x00, 0x01, // MOV [0100h], AX
        0xFA,             // CLI
        0xF4,             // HLT
    ];
    inject_and_run(machine, code);
    result_word(&machine.bus, 0)
}

#[test]
fn save_state_round_trip_keeps_dos_310() {
    let mut machine = boot();
    let saved = machine.capture_state().expect("capture");

    machine.restore_state(&saved).expect("restore");
    assert_eq!(reported_version(&mut machine), DOS310_VERSION);
    assert_eq!(
        list_of_lists_pointer(&mut machine).1,
        DOS310_LIST_OF_LISTS_OFFSET
    );

    let mut restored = create_machine_with_version(HleDosVersion::Dos620);
    restored.restore_state(&saved).expect("restore");
    assert_eq!(reported_version(&mut restored), DOS310_VERSION);
    assert_eq!(
        list_of_lists_pointer(&mut restored).1,
        DOS310_LIST_OF_LISTS_OFFSET
    );
    let replayed = restored.capture_state().expect("capture");
    let mut original = boot();
    original.restore_state(&replayed).expect("restore");
    assert_eq!(reported_version(&mut original), DOS310_VERSION);
}

#[test]
fn dos_620_save_state_keeps_dos_620_on_a_machine_configured_for_dos_310() {
    let mut machine = boot_with_version(HleDosVersion::Dos620);
    assert_eq!(reported_version(&mut machine), DOS620_VERSION);
    let saved = machine.capture_state().expect("capture");

    let mut restored = create_machine();
    restored.restore_state(&saved).expect("restore");
    assert_eq!(reported_version(&mut restored), DOS620_VERSION);
    assert_eq!(
        list_of_lists_pointer(&mut restored).1,
        DOS620_LIST_OF_LISTS_OFFSET
    );
    restored.run_for(INJECT_BUDGET);
    let recaptured = restored.capture_state().expect("capture");
    let mut dos310 = boot();
    dos310.restore_state(&recaptured).expect("restore");
    assert_eq!(reported_version(&mut dos310), DOS620_VERSION);
}
