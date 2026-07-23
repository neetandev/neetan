//! Pure-asm hardware IRQ stubs that need no Rust handler: the INT 75h
//! coprocessor error path (IRQ 13).

use common::Bus;

use super::{RESULT, create_machine_dx50, inject_and_run, read_ram_u8};

/// Slave PIC interrupt mask register port.
const SLAVE_IMR_PORT: u16 = 0xA1;
/// Slave PIC IMR bit 5: IRQ 13 (coprocessor error).
const IRQ13_MASK_BIT: u8 = 0x20;

/// Installs IVT vector 02h = 0000:2000 (the NMI vector the INT 75h stub
/// chains to), unmasks IRQ 13, then idles on interrupt wakeups.
#[rustfmt::skip]
const HOOK_NMI_AND_IDLE_CODE: &[u8] = &[
    0xC7, 0x06, 0x08, 0x00, 0x00, 0x20, // MOV WORD [0x0008], 0x2000
    0xC7, 0x06, 0x0A, 0x00, 0x00, 0x00, // MOV WORD [0x000A], 0x0000
    0xE4, 0xA1,                         // IN AL, 0xA1
    0x24, 0xDF,                         // AND AL, 0xDF (unmask IRQ 13)
    0xE6, 0xA1,                         // OUT 0xA1, AL
    0xFB,                               // STI
    0xF4,                               // HLT
    0xEB, 0xFD,                         // JMP short back to the HLT
];

/// INT 02h callback: increments the counter at the result address.
#[rustfmt::skip]
const COUNT_NMI_CALLBACK: &[u8] = &[
    0xFE, 0x06, 0x00, 0x06, // INC BYTE [0x0600]
    0xCF,                   // IRET
];

#[test]
fn coprocessor_error_chains_the_nmi_vector_once() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);
    // The stub must clear the FERR# latch through port 0xF0 before it EOIs,
    // otherwise IRQ 13 stays asserted and the handler runs forever.
    machine.bus.signal_fpu_error();
    inject_and_run(
        &mut machine,
        HOOK_NMI_AND_IDLE_CODE,
        COUNT_NMI_CALLBACK,
        20_000_000,
    );

    assert_eq!(
        read_ram_u8(&machine, RESULT),
        1,
        "the INT 02h hook ran exactly once"
    );
    let pic = &machine.inspection_state().pic;
    assert_eq!(
        pic.chips[1].isr, 0,
        "slave PIC in-service cleared by the EOI"
    );
    assert_eq!(
        pic.chips[0].isr, 0,
        "master PIC in-service cleared by the EOI"
    );
    assert_eq!(
        machine.bus.io_read_byte(SLAVE_IMR_PORT) & IRQ13_MASK_BIT,
        0,
        "IRQ 13 still unmasked, so a stuck request would have re-fired"
    );
}
