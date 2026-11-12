use crate::harness::*;

/// `AL` of `INT 2Fh AX=4300h` with an installed XMS driver.
const XMS_INSTALLED: u8 = 0x80;
/// XMS version 3.00 of `AH=00h`.
const XMS_VERSION: u16 = 0x0300;
/// `AH` of a successful EMS call.
const EMS_STATUS_OK: u8 = 0x00;

#[test]
fn xms_driver_answers_through_the_int_2fh_entry() {
    let mut machine = boot();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0x43,       // MOV AX, 4300h
        0xCD, 0x2F,             // INT 2Fh
        0xA3, 0x00, 0x01,       // MOV [0100h], AX
        0xB8, 0x10, 0x43,       // MOV AX, 4310h
        0xCD, 0x2F,             // INT 2Fh
        0x89, 0x1E, 0x02, 0x01, // MOV [0102h], BX
        0x8C, 0x06, 0x04, 0x01, // MOV [0104h], ES
        0xB4, 0x00,             // MOV AH, 00h
        0xFF, 0x1E, 0x02, 0x01, // CALL FAR [0102h]
        0xA3, 0x06, 0x01,       // MOV [0106h], AX
        0xB4, 0x08,             // MOV AH, 08h
        0xFF, 0x1E, 0x02, 0x01, // CALL FAR [0102h]
        0xA3, 0x08, 0x01,       // MOV [0108h], AX
        0xFA,                   // CLI
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, code);
    let bus = &machine.bus;
    let self_boot = dos::DataArea::Pc98Dos310
        .layout()
        .self_boot
        .expect("the PC-98 3.10 data area has self boot regions");
    assert_eq!(result_word(bus, 0) as u8, XMS_INSTALLED);
    assert_eq!(result_word(bus, 2), self_boot.xms_entry_stub);
    assert_eq!(result_word(bus, 4), DOS_DATA_SEGMENT);
    assert_eq!(result_word(bus, 6), XMS_VERSION);
    assert_ne!(result_word(bus, 8), 0, "XMS reports free memory");
}

#[test]
fn ems_driver_reports_its_status() {
    let mut machine = boot();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x40,       // MOV AH, 40h
        0xCD, 0x67,       // INT 67h
        0xA3, 0x00, 0x01, // MOV [0100h], AX
        0xFA,             // CLI
        0xF4,             // HLT
    ];
    inject_and_run(&mut machine, code);
    let [_, status] = result_word(&machine.bus, 0).to_le_bytes();
    assert_eq!(status, EMS_STATUS_OK);
}
