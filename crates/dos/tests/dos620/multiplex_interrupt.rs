use common::Bus;

use crate::harness;

const MUX_HANDLER_OFFSET: u16 = 0x0080;
const OLD_VECTOR_OFFSET: u16 = 0x009E;

fn write_mux_handler(bus: &mut impl Bus) {
    #[rustfmt::skip]
    let handler: &[u8] = &[
        // +0x00: CMP AH, C0h
        0x80, 0xFC, 0xC0,
        // +0x03: JNE chain (+0x19)
        0x75, 0x14,
        // +0x05: CMP AL, 00h (installation check)
        0x3C, 0x00,
        // +0x07: JE install_check (+0x0F)
        0x74, 0x06,
        // +0x09: CMP AL, 01h (custom function)
        0x3C, 0x01,
        // +0x0B: JE custom (+0x15)
        0x74, 0x08,
        // +0x0D: JMP short chain (+0x19)
        0xEB, 0x0A,

        // install_check (+0x0F):
        0xB0, 0xFF,                             // MOV AL, FFh
        0xBB, 0x34, 0x12,                       // MOV BX, 1234h
        0xCF,                                   // IRET

        // custom (+0x15):
        0xBB, 0xCD, 0xAB,                       // MOV BX, ABCDh
        0xCF,                                   // IRET

        // chain (+0x19):
        // JMP FAR [CS:old_vector] (segment offset 0x009E)
        0x2E, 0xFF, 0x2E,
        OLD_VECTOR_OFFSET as u8, (OLD_VECTOR_OFFSET >> 8) as u8,

        // old_vector (+0x1E): 4 bytes, patched by setup prologue via DOS Fn 35h
        0x00, 0x00, 0x00, 0x00,
    ];
    harness::write_bytes(
        bus,
        harness::INJECT_CODE_BASE + MUX_HANDLER_OFFSET as u32,
        handler,
    );
}

const HANDLER_OFF_LO: u8 = MUX_HANDLER_OFFSET as u8;
const HANDLER_OFF_HI: u8 = (MUX_HANDLER_OFFSET >> 8) as u8;
const OLD_VEC_OFF_LO: u8 = OLD_VECTOR_OFFSET as u8;
const OLD_VEC_OFF_HI: u8 = (OLD_VECTOR_OFFSET >> 8) as u8;
const OLD_VEC_SEG_LO: u8 = (OLD_VECTOR_OFFSET + 2) as u8;
const OLD_VEC_SEG_HI: u8 = ((OLD_VECTOR_OFFSET + 2) >> 8) as u8;

// Setup prologue: get old INT 2Fh vector via DOS Fn 35h, store it in handler's
// chain target, then install our handler via DOS Fn 25h. (21 bytes)
#[rustfmt::skip]
const SETUP_PROLOGUE: [u8; 21] = [
    0xB8, 0x2F, 0x35,                           // MOV AX, 352Fh
    0xCD, 0x21,                                 // INT 21h -> ES:BX = old handler
    0x89, 0x1E, OLD_VEC_OFF_LO, OLD_VEC_OFF_HI, // MOV [old_vector], BX
    0x8C, 0x06, OLD_VEC_SEG_LO, OLD_VEC_SEG_HI, // MOV [old_vector+2], ES
    0xBA, HANDLER_OFF_LO, HANDLER_OFF_HI,       // MOV DX, handler_offset
    0xB8, 0x2F, 0x25,                           // MOV AX, 252Fh
    0xCD, 0x21,                                 // INT 21h: set INT 2Fh = DS:DX
];

// Teardown epilogue: restore old INT 2Fh vector via DOS Fn 25h, then halt. (17 bytes)
#[rustfmt::skip]
const TEARDOWN_EPILOGUE: [u8; 17] = [
    0x8B, 0x16, OLD_VEC_OFF_LO, OLD_VEC_OFF_HI,// MOV DX, [old_vector]
    0x1E,                                       // PUSH DS
    0x8E, 0x1E, OLD_VEC_SEG_LO, OLD_VEC_SEG_HI, // MOV DS, [old_vector+2]
    0xB8, 0x2F, 0x25,                           // MOV AX, 252Fh
    0xCD, 0x21,                                 // INT 21h: restore old vector
    0x1F,                                       // POP DS
    0xFA,                                       // CLI
    0xF4,                                       // HLT
];

fn build_hooked_test(test_body: &[u8]) -> Vec<u8> {
    let mut code =
        Vec::with_capacity(SETUP_PROLOGUE.len() + test_body.len() + TEARDOWN_EPILOGUE.len());
    code.extend_from_slice(&SETUP_PROLOGUE);
    code.extend_from_slice(test_body);
    code.extend_from_slice(&TEARDOWN_EPILOGUE);
    code
}

#[test]
fn mux_c0h_not_installed_before_hook() {
    let mut machine = harness::boot_hle();
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB8, 0x00, 0xC0,                       // MOV AX, C000h
        0xCD, 0x2F,                             // INT 2Fh
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX
        0xFA,                                   // CLI
        0xF4,                                   // HLT
    ];
    harness::inject_and_run(&mut machine, code);

    let ax = harness::result_word(&machine.bus, 0);
    assert_eq!(
        ax, 0xC000,
        "Before hook: AX should be unchanged (C000h), got {ax:#06X}"
    );
}

#[test]
fn mux_c0h_installation_check() {
    let mut machine = harness::boot_hle();
    write_mux_handler(&mut machine.bus);

    #[rustfmt::skip]
    let test_body: &[u8] = &[
        0xB8, 0x00, 0xC0,                       // MOV AX, C000h
        0xCD, 0x2F,                             // INT 2Fh
        0xA2, 0x00, 0x01,                       // MOV [0100h], AL
        0x89, 0x1E, 0x02, 0x01,                 // MOV [0102h], BX
    ];
    let code = build_hooked_test(test_body);
    harness::inject_and_run(&mut machine, &code);

    let al = harness::result_byte(&machine.bus, 0);
    let bx = harness::result_word(&machine.bus, 2);
    assert_eq!(
        al, 0xFF,
        "Installation check: AL should be FFh, got {al:#04X}"
    );
    assert_eq!(
        bx, 0x1234,
        "Installation check: BX should be 1234h, got {bx:#06X}"
    );
}

#[test]
fn mux_c0h_custom_subfunction() {
    let mut machine = harness::boot_hle();
    write_mux_handler(&mut machine.bus);

    #[rustfmt::skip]
    let test_body: &[u8] = &[
        0xB8, 0x01, 0xC0,                       // MOV AX, C001h
        0xCD, 0x2F,                             // INT 2Fh
        0x89, 0x1E, 0x00, 0x01,                 // MOV [0100h], BX
    ];
    let code = build_hooked_test(test_body);
    harness::inject_and_run(&mut machine, &code);

    let bx = harness::result_word(&machine.bus, 0);
    assert_eq!(
        bx, 0xABCD,
        "Custom subfunction: BX should be ABCDh, got {bx:#06X}"
    );
}

#[test]
fn mux_chain_to_builtin_doskey() {
    let mut machine = harness::boot_hle();
    write_mux_handler(&mut machine.bus);

    #[rustfmt::skip]
    let test_body: &[u8] = &[
        0xB8, 0x00, 0x48,                       // MOV AX, 4800h
        0xCD, 0x2F,                             // INT 2Fh
        0xA2, 0x00, 0x01,                       // MOV [0100h], AL
    ];
    let code = build_hooked_test(test_body);
    harness::inject_and_run(&mut machine, &code);

    let al = harness::result_byte(&machine.bus, 0);
    assert_eq!(al, 0x00, "DOSKEY chain: AL should be 00h, got {al:#04X}");
}

#[test]
fn mux_chain_unknown_leaves_ax_unchanged() {
    let mut machine = harness::boot_hle();
    write_mux_handler(&mut machine.bus);

    #[rustfmt::skip]
    let test_body: &[u8] = &[
        0xB8, 0x00, 0xC1,                       // MOV AX, C100h
        0xCD, 0x2F,                             // INT 2Fh
        0xA3, 0x00, 0x01,                       // MOV [0100h], AX
    ];
    let code = build_hooked_test(test_body);
    harness::inject_and_run(&mut machine, &code);

    let ax = harness::result_word(&machine.bus, 0);
    assert_eq!(
        ax, 0xC100,
        "Unknown MUX chain: AX should be unchanged (C100h), got {ax:#06X}"
    );
}
