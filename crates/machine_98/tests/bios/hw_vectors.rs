use common::Bus;

use super::{boot_and_run_f, boot_and_run_ra, boot_and_run_vm, boot_and_run_vx};

const RESULT: u32 = 0x0600;
const HW_VECTOR_BUDGET: u64 = 2_000_000;

// Software INT to an unhandled hardware vector must complete without hanging.
// The HLE BIOS handler sends EOI and returns, so execution continues.
// Without the handler, the ISR bit stays set, blocking lower-priority IRQs.

// Master PIC vectors: INT 0x0A (IRQ 2), 0x0B (IRQ 3), 0x0D (IRQ 5), 0x0E (IRQ 6).
// These require a master PIC EOI.

#[rustfmt::skip]
fn make_master_hwint_code(vector: u8) -> Vec<u8> {
    vec![
        0xCD, vector,                       // INT <vector>
        0xC6, 0x06, 0x00, 0x06, 0xAA,      // MOV BYTE [RESULT], 0xAA
        0xF4,                               // HLT
    ]
}

// Slave PIC vectors: INT 0x10 (IRQ 8), 0x11 (IRQ 9), 0x14..=0x17 (IRQ 12-15).
// These require both slave and master PIC EOI.

#[rustfmt::skip]
fn make_slave_hwint_code(vector: u8) -> Vec<u8> {
    vec![
        0xCD, vector,                       // INT <vector>
        0xC6, 0x06, 0x00, 0x06, 0xAA,      // MOV BYTE [RESULT], 0xAA
        0xF4,                               // HLT
    ]
}

macro_rules! test_master_hwint {
    ($name:ident, $vector:expr, $run_fn:ident) => {
        #[test]
        fn $name() {
            let code = make_master_hwint_code($vector);
            let (mut machine, _cycles) = $run_fn(&code, &[], HW_VECTOR_BUDGET);
            let marker = machine.bus.read_byte(RESULT);
            assert_eq!(
                marker, 0xAA,
                "INT {:#04X} handler should complete and execution should continue",
                $vector
            );

            let state = machine.save_state();
            assert_eq!(
                state.pic.chips[0].isr, 0,
                "Master PIC ISR should be clear after INT {:#04X} (ISR={:#04X})",
                $vector, state.pic.chips[0].isr
            );
        }
    };
}

macro_rules! test_slave_hwint {
    ($name:ident, $vector:expr, $run_fn:ident) => {
        #[test]
        fn $name() {
            let code = make_slave_hwint_code($vector);
            let (mut machine, _cycles) = $run_fn(&code, &[], HW_VECTOR_BUDGET);
            let marker = machine.bus.read_byte(RESULT);
            assert_eq!(
                marker, 0xAA,
                "INT {:#04X} handler should complete and execution should continue",
                $vector
            );

            let state = machine.save_state();
            assert_eq!(
                state.pic.chips[0].isr, 0,
                "Master PIC ISR should be clear after INT {:#04X} (ISR={:#04X})",
                $vector, state.pic.chips[0].isr
            );
            assert_eq!(
                state.pic.chips[1].isr, 0,
                "Slave PIC ISR should be clear after INT {:#04X} (ISR={:#04X})",
                $vector, state.pic.chips[1].isr
            );
        }
    };
}

// Master PIC: INT 0x0A (IRQ 2 - CRTC VSYNC)
test_master_hwint!(hwint_0a_completes_f, 0x0A, boot_and_run_f);
test_master_hwint!(hwint_0a_completes_vm, 0x0A, boot_and_run_vm);
test_master_hwint!(hwint_0a_completes_vx, 0x0A, boot_and_run_vx);
test_master_hwint!(hwint_0a_completes_ra, 0x0A, boot_and_run_ra);

// Master PIC: INT 0x0B (IRQ 3 - INT0 expansion)
test_master_hwint!(hwint_0b_completes_f, 0x0B, boot_and_run_f);
test_master_hwint!(hwint_0b_completes_vm, 0x0B, boot_and_run_vm);
test_master_hwint!(hwint_0b_completes_vx, 0x0B, boot_and_run_vx);
test_master_hwint!(hwint_0b_completes_ra, 0x0B, boot_and_run_ra);

// Master PIC: INT 0x0D (IRQ 5 - INT1 expansion)
test_master_hwint!(hwint_0d_completes_f, 0x0D, boot_and_run_f);
test_master_hwint!(hwint_0d_completes_vm, 0x0D, boot_and_run_vm);
test_master_hwint!(hwint_0d_completes_vx, 0x0D, boot_and_run_vx);
test_master_hwint!(hwint_0d_completes_ra, 0x0D, boot_and_run_ra);

// Master PIC: INT 0x0E (IRQ 6 - FDC)
test_master_hwint!(hwint_0e_completes_f, 0x0E, boot_and_run_f);
test_master_hwint!(hwint_0e_completes_vm, 0x0E, boot_and_run_vm);
test_master_hwint!(hwint_0e_completes_vx, 0x0E, boot_and_run_vx);
test_master_hwint!(hwint_0e_completes_ra, 0x0E, boot_and_run_ra);

// Slave PIC: INT 0x10 (IRQ 8 - INT2 expansion)
test_slave_hwint!(hwint_10_completes_f, 0x10, boot_and_run_f);
test_slave_hwint!(hwint_10_completes_vm, 0x10, boot_and_run_vm);
test_slave_hwint!(hwint_10_completes_vx, 0x10, boot_and_run_vx);
test_slave_hwint!(hwint_10_completes_ra, 0x10, boot_and_run_ra);

// Slave PIC: INT 0x11 (IRQ 9 - INT3 expansion)
test_slave_hwint!(hwint_11_completes_f, 0x11, boot_and_run_f);
test_slave_hwint!(hwint_11_completes_vm, 0x11, boot_and_run_vm);
test_slave_hwint!(hwint_11_completes_vx, 0x11, boot_and_run_vx);
test_slave_hwint!(hwint_11_completes_ra, 0x11, boot_and_run_ra);

// Slave PIC: INT 0x14 (IRQ 12 - INT5 expansion)
test_slave_hwint!(hwint_14_completes_f, 0x14, boot_and_run_f);
test_slave_hwint!(hwint_14_completes_vm, 0x14, boot_and_run_vm);
test_slave_hwint!(hwint_14_completes_vx, 0x14, boot_and_run_vx);
test_slave_hwint!(hwint_14_completes_ra, 0x14, boot_and_run_ra);

// Slave PIC: INT 0x15 (IRQ 13 - INT6 expansion / SCSI)
test_slave_hwint!(hwint_15_completes_f, 0x15, boot_and_run_f);
test_slave_hwint!(hwint_15_completes_vm, 0x15, boot_and_run_vm);
test_slave_hwint!(hwint_15_completes_vx, 0x15, boot_and_run_vx);
test_slave_hwint!(hwint_15_completes_ra, 0x15, boot_and_run_ra);

// Slave PIC: INT 0x16 (IRQ 14 - Reserved)
test_slave_hwint!(hwint_16_completes_f, 0x16, boot_and_run_f);
test_slave_hwint!(hwint_16_completes_vm, 0x16, boot_and_run_vm);
test_slave_hwint!(hwint_16_completes_vx, 0x16, boot_and_run_vx);
test_slave_hwint!(hwint_16_completes_ra, 0x16, boot_and_run_ra);

// Slave PIC: INT 0x17 (IRQ 15 - Reserved)
test_slave_hwint!(hwint_17_completes_f, 0x17, boot_and_run_f);
test_slave_hwint!(hwint_17_completes_vm, 0x17, boot_and_run_vm);
test_slave_hwint!(hwint_17_completes_vx, 0x17, boot_and_run_vx);
test_slave_hwint!(hwint_17_completes_ra, 0x17, boot_and_run_ra);
