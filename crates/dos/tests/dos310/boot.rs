use crate::harness::*;

#[test]
fn boot_reaches_the_prompt_with_the_dos_310_banner() {
    let machine = boot();
    let rows = screen_rows(&machine.bus);
    assert!(
        rows.iter().any(|row| row.contains("Neetan DOS 3.10")),
        "{rows:#?}"
    );
}

#[test]
fn version_function_reports_dos_310_with_the_nec_oem_number() {
    let mut machine = boot();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x30,             // MOV AH, 30h
        0xCD, 0x21,             // INT 21h
        0xA3, 0x00, 0x01,       // MOV [0100h], AX
        0x89, 0x1E, 0x02, 0x01, // MOV [0102h], BX
        0x89, 0x0E, 0x04, 0x01, // MOV [0104h], CX
        0xFA,                   // CLI
        0xF4,                   // HLT
    ];
    inject_and_run(&mut machine, code);
    assert_eq!(result_word(&machine.bus, 0), 0x0A03);
    assert_eq!(result_word(&machine.bus, 2), 0xFF00);
    assert_eq!(result_word(&machine.bus, 4), 0x0000);
}

#[test]
fn version_command_prints_dos_310() {
    let mut machine = boot();
    type_string(&mut machine.bus, b"VER\r");
    machine.run_for(INJECT_BUDGET);
    let rows = screen_rows(&machine.bus);
    let banner_rows = rows
        .iter()
        .filter(|row| row.contains("Neetan DOS 3.10"))
        .count();
    assert!(banner_rows >= 2, "{rows:#?}");
}
