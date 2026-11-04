use common::Bus;

use super::{TEST_CALLBACK, boot_and_run_f, boot_and_run_ra, boot_and_run_vm, boot_and_run_vx};

const RESULT: u32 = 0x0600;
/// Cycle budget of a single software INT test.
const HW_VECTOR_BUDGET: u64 = 2_000_000;

// A software INT to an unclaimed hardware vector returns without hanging.
// The HLE BIOS handler sends no EOI and leaves the PIC ISR unchanged.

// Master PIC vectors: INT 0x0A (IRQ 2), 0x0B (IRQ 3), 0x0D (IRQ 5), 0x0E (IRQ 6).

#[rustfmt::skip]
fn make_master_hwint_code(vector: u8) -> Vec<u8> {
    vec![
        0xCD, vector,                       // INT <vector>
        0xC6, 0x06, 0x00, 0x06, 0xAA,      // MOV BYTE [RESULT], 0xAA
        0xF4,                               // HLT
    ]
}

// Slave PIC vectors: INT 0x10 (IRQ 8), 0x11 (IRQ 9), 0x14..=0x17 (IRQ 12-15).

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

            let state = machine.inspection_state();
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

            let state = machine.inspection_state();
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

/// Cycle budget of a traced test run.
const TRACED_BUDGET: u64 = 4_000_000;
/// Guest address of the interrupt counter the hook increments.
const INTERRUPT_COUNT: u32 = 0x0602;

/// PC-98 machine with an i386 CPU that records interrupts and EOIs.
type TracedMachine = machine_98::Pc98Machine<cpu::I386, EoiRecorder>;

/// Counts the acknowledged IRQ lines and the non-specific EOIs that guest OUT
/// instructions write to the master (port 00h) and the slave (port 08h).
#[derive(Default)]
struct EoiRecorder {
    acknowledges: [usize; 16],
    master_eoi_writes: usize,
    slave_eoi_writes: usize,
}

impl EoiRecorder {
    /// Counts all acknowledged IRQ lines. A slave acknowledge also sets the
    /// cascade line on the master and counts here too.
    fn master_acknowledges(&self) -> usize {
        self.acknowledges.iter().sum()
    }

    /// Counts the acknowledged slave IRQ lines 8 to 15.
    fn slave_acknowledges(&self) -> usize {
        self.acknowledges[8..].iter().sum()
    }
}

impl common::TraceSink for EoiRecorder {
    fn trace(&mut self, _context: common::TraceContext, event: common::TraceEvent<'_>) {
        match event {
            common::TraceEvent::Interrupt(interrupt)
                if interrupt.action == common::TraceInterruptAction::Acknowledge =>
            {
                if let Some(line) = interrupt.line {
                    self.acknowledges[usize::from(line) & 0x0F] += 1;
                }
            }
            common::TraceEvent::Access(access)
                if access.space == common::TraceAddressSpace::MAIN_IO
                    && access.kind == common::TraceAccessKind::Write
                    && access.value == Some(0x20) =>
            {
                match access.address {
                    0x00 => self.master_eoi_writes += 1,
                    0x08 => self.slave_eoi_writes += 1,
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Boots a traced PC-9801RA, runs `prepare`, then runs `code` from TEST_CODE
/// with a fresh recorder.
fn run_traced_ra(code: &[u8], prepare: impl FnOnce(&mut TracedMachine)) -> TracedMachine {
    let mut machine = TracedMachine::new(
        cpu::I386::new(),
        machine_98::Pc9801Bus::new_with_trace_sink(
            common::MachineModel::PC9801RA,
            common::CpuMode::High,
            48_000,
            EoiRecorder::default(),
        ),
    );
    machine.bus.load_font_rom(machine_98::BUILTIN_FONT_ROM);
    boot_to_halt!(machine);
    prepare(&mut machine);
    super::write_bytes(&mut machine.bus, super::TEST_CODE, code);
    machine.cpu.load_state(&{
        let mut state = cpu::I386State {
            ip: super::TEST_CODE as u16,
            ..Default::default()
        };
        state.set_esp(0x4000);
        state
    });
    *machine.bus.tracer_mut() = EoiRecorder::default();
    machine.run_for(TRACED_BUDGET);
    machine
}

/// Checks that the code finished, `line` was acknowledged and every
/// acknowledged interrupt ended with guest OUT EOIs and a clear ISR.
fn assert_guest_eois(machine: &mut TracedMachine, line: usize) {
    assert_eq!(
        machine.bus.read_byte(RESULT),
        0xAA,
        "test code should finish"
    );
    let recorder = machine.bus.tracer();
    assert!(
        recorder.acknowledges[line] >= 1,
        "IRQ {line} should be acknowledged"
    );
    assert!(
        recorder.master_eoi_writes >= recorder.master_acknowledges(),
        "every interrupt should end with a guest OUT 00h,20h ({} EOIs for {} interrupts)",
        recorder.master_eoi_writes,
        recorder.master_acknowledges()
    );
    assert!(
        recorder.slave_eoi_writes >= recorder.slave_acknowledges(),
        "every slave interrupt should end with a guest OUT 08h,20h ({} EOIs for {} interrupts)",
        recorder.slave_eoi_writes,
        recorder.slave_acknowledges()
    );
    let state = machine.inspection_state();
    assert_eq!(state.pic.chips[0].isr, 0, "master ISR should be clear");
    assert_eq!(state.pic.chips[1].isr, 0, "slave ISR should be clear");
}

/// Builds code that sets the master and slave IMR, runs `setup`, waits for
/// `waits` interrupts with HLT, runs `teardown` and marks RESULT.
#[rustfmt::skip]
fn make_wait_code(
    master_mask: u8,
    slave_mask: u8,
    setup: &[u8],
    waits: usize,
    teardown: &[u8],
) -> Vec<u8> {
    let mut code = vec![
        0xFA,                           // CLI
        0xB0, master_mask,              // MOV AL, master_mask
        0xE6, 0x02,                     // OUT 0x02, AL
        0xB0, slave_mask,               // MOV AL, slave_mask
        0xE6, 0x0A,                     // OUT 0x0A, AL
    ];
    code.extend_from_slice(setup);
    code.push(0xFB);                    // STI
    code.extend(std::iter::repeat_n(0xF4, waits)); // HLT
    code.extend_from_slice(teardown);
    code.extend_from_slice(&[
        0xFA,                           // CLI
        0xC6, 0x06, 0x00, 0x06, 0xAA,   // MOV BYTE [RESULT], 0xAA
        0xF4,                           // HLT
    ]);
    code
}

// Every BIOS interrupt handler ends the interrupt with guest OUT instructions,
// so a V86 monitor that traps the PIC ports sees each EOI.

#[test]
fn vsync_interrupt_eoi_is_a_guest_io_write_ra() {
    let code = make_wait_code(0xFB, 0xFF, &[0xE6, 0x64], 2, &[]); // OUT 0x64, AL (arm VSYNC)
    let mut machine = run_traced_ra(&code, |_| {});
    assert_guest_eois(&mut machine, 2);
}

#[test]
fn keyboard_interrupt_eoi_is_a_guest_io_write_ra() {
    let code = make_wait_code(0xFD, 0xFF, &[], 1, &[]);
    let mut machine = run_traced_ra(&code, |machine| {
        machine.bus.push_keyboard_scancode(0x1C);
    });
    assert_guest_eois(&mut machine, 1);
}

#[test]
fn serial_interrupt_eoi_is_a_guest_io_write_ra() {
    #[rustfmt::skip]
    let setup = [
        0x31, 0xC0,                     // XOR AX, AX
        0x8E, 0xC0,                     // MOV ES, AX
        0xBF, 0x00, 0x30,               // MOV DI, 0x3000          (buffer control block)
        0xBA, 0x00, 0x01,               // MOV DX, 0x0100          (256-byte data buffer)
        0xBB, 0x40, 0x04,               // MOV BX, 0x0440          (timeouts)
        0xB9, 0x27, 0x4E,               // MOV CX, 0x4E27          (8N1, TxEN|DTR|RxEN|RTS)
        0xB8, 0x07, 0x00,               // MOV AX, 0x0007          (init, 9600 baud)
        0xCD, 0x19,                     // INT 0x19
        0xFA,                           // CLI
        0xB0, 0xEF,                     // MOV AL, 0xEF            (only IR4 unmasked)
        0xE6, 0x02,                     // OUT 0x02, AL
    ];
    let code = make_wait_code(0xFF, 0xFF, &setup, 1, &[]);
    let mut machine = run_traced_ra(&code, |machine| {
        machine.bus.push_serial_byte(0x41);
    });
    assert_guest_eois(&mut machine, 4);
}

#[test]
fn fdc_1mb_interrupt_eoi_is_a_guest_io_write_ra() {
    #[rustfmt::skip]
    let setup = [
        0xB0, 0x07,                     // MOV AL, 0x07            (RECALIBRATE)
        0xE6, 0x92,                     // OUT 0x92, AL
        0xB0, 0x00,                     // MOV AL, 0x00            (drive 0)
        0xE6, 0x92,                     // OUT 0x92, AL
    ];
    let code = make_wait_code(0x7F, 0xF7, &setup, 1, &[]);
    let mut machine = run_traced_ra(&code, |_| {});
    assert_guest_eois(&mut machine, 11);
}

/// Hooks INT 15h with a counter that chains to the BIOS handler, enables the
/// mouse timer interrupt (IR13) and waits with interrupts enabled. The
/// original vector is saved at 0000:0604h.
#[rustfmt::skip]
const MOUSE_TIMER_CODE: &[u8] = &[
    0xFA,                               // CLI
    0xA1, 0x54, 0x00,                   // MOV AX, [0x0054]
    0xA3, 0x04, 0x06,                   // MOV [0x0604], AX
    0xA1, 0x56, 0x00,                   // MOV AX, [0x0056]
    0xA3, 0x06, 0x06,                   // MOV [0x0606], AX
    0xC7, 0x06, 0x54, 0x00, 0x00, 0x20, // MOV WORD [0x0054], 0x2000
    0xC7, 0x06, 0x56, 0x00, 0x00, 0x00, // MOV WORD [0x0056], 0x0000
    0xC7, 0x06, 0x02, 0x06, 0x00, 0x00, // MOV WORD [INTERRUPT_COUNT], 0
    0xE4, 0x0A,                         // IN AL, 0x0A
    0x24, 0xDF,                         // AND AL, 0xDF            (unmask IR13)
    0xE6, 0x0A,                         // OUT 0x0A, AL
    0xE4, 0x02,                         // IN AL, 0x02
    0x24, 0x7F,                         // AND AL, 0x7F            (unmask IR7 cascade)
    0xE6, 0x02,                         // OUT 0x02, AL
    0xBA, 0xDD, 0x7F,                   // MOV DX, 0x7FDD
    0xB0, 0x10,                         // MOV AL, 0x10            (mouse timer interrupt off)
    0xEE,                               // OUT DX, AL
    0xB0, 0x00,                         // MOV AL, 0x00            (mouse timer interrupt on)
    0xEE,                               // OUT DX, AL
    0xFB,                               // STI
    0xB9, 0xFF, 0xFF,                   // MOV CX, 0xFFFF
    0xE2, 0xFE,                         // LOOP $
    0xFA,                               // CLI
    0xB0, 0x10,                         // MOV AL, 0x10            (mouse timer interrupt off)
    0xEE,                               // OUT DX, AL
    0xC6, 0x06, 0x00, 0x06, 0xAA,       // MOV BYTE [RESULT], 0xAA
    0xF4,                               // HLT
];

/// INT 15h hook: counts the interrupt and chains to the saved BIOS vector.
#[rustfmt::skip]
const COUNTING_HOOK: &[u8] = &[
    0x2E, 0xFF, 0x06, 0x02, 0x06,       // INC WORD CS:[INTERRUPT_COUNT]
    0x2E, 0xFF, 0x2E, 0x04, 0x06,       // JMP FAR CS:[0x0604]
];

// A hardware interrupt on an unclaimed slave line (IR13, INT 15h) reaches the
// BIOS default handler. The handler must end the interrupt on both PICs, or
// the cascade stays in service and blocks every later slave interrupt.
macro_rules! test_unclaimed_slave_irq {
    ($name:ident, $run_fn:ident) => {
        #[test]
        fn $name() {
            let (mut machine, _cycles) = $run_fn(MOUSE_TIMER_CODE, COUNTING_HOOK, TRACED_BUDGET);
            assert_eq!(
                machine.bus.read_byte(RESULT),
                0xAA,
                "test code should finish"
            );
            let count = u16::from(machine.bus.read_byte(INTERRUPT_COUNT))
                | u16::from(machine.bus.read_byte(INTERRUPT_COUNT + 1)) << 8;
            assert!(
                count >= 2,
                "IR13 should be delivered repeatedly (count {count})"
            );
            let state = machine.inspection_state();
            assert_eq!(state.pic.chips[1].isr, 0, "slave ISR should be clear");
            assert_eq!(state.pic.chips[0].isr, 0, "master ISR should be clear");
        }
    };
}

test_unclaimed_slave_irq!(unclaimed_irq13_ends_interrupt_vm, boot_and_run_vm);
test_unclaimed_slave_irq!(unclaimed_irq13_ends_interrupt_ra, boot_and_run_ra);

#[test]
fn unclaimed_irq13_eoi_is_a_guest_io_write_ra() {
    #[rustfmt::skip]
    let setup = [
        0xBA, 0xDD, 0x7F,               // MOV DX, 0x7FDD
        0xB0, 0x10,                     // MOV AL, 0x10            (mouse timer interrupt off)
        0xEE,                           // OUT DX, AL
        0xB0, 0x00,                     // MOV AL, 0x00            (mouse timer interrupt on)
        0xEE,                           // OUT DX, AL
    ];
    // MOV AL, 0x10; OUT DX, AL (mouse timer interrupt off)
    let code = make_wait_code(0x7F, 0xDF, &setup, 2, &[0xB0, 0x10, 0xEE]);
    let mut machine = run_traced_ra(&code, |_| {});
    assert_guest_eois(&mut machine, 13);
}

#[test]
fn unclaimed_software_interrupts_send_no_eoi_ra() {
    #[rustfmt::skip]
    let setup = [
        0xCD, 0x0B,                     // INT 0x0B
        0xCD, 0x0D,                     // INT 0x0D
        0xCD, 0x0F,                     // INT 0x0F
        0xCD, 0x11,                     // INT 0x11
        0xCD, 0x15,                     // INT 0x15
        0xCD, 0x17,                     // INT 0x17
    ];
    let code = make_wait_code(0xFF, 0xFF, &setup, 0, &[]);
    let mut machine = run_traced_ra(&code, |_| {});
    assert_eq!(
        machine.bus.read_byte(RESULT),
        0xAA,
        "test code should finish"
    );
    let recorder = machine.bus.tracer();
    assert_eq!(recorder.master_eoi_writes, 0, "no master EOI expected");
    assert_eq!(recorder.slave_eoi_writes, 0, "no slave EOI expected");
}

/// Guest address of the master ISR the hook reads.
const MASTER_ISR_SEEN: u32 = 0x0608;
/// Guest address of the slave ISR the hook reads.
const SLAVE_ISR_SEEN: u32 = 0x0609;

// A software INT 11h inside the VSYNC handler finds IR2 in service. The
// unclaimed slave handler must leave the outer interrupt in service.
#[test]
fn unclaimed_slave_vector_inside_master_interrupt_keeps_it_in_service_ra() {
    #[rustfmt::skip]
    let setup = [
        0xC7, 0x06, 0x28, 0x00, TEST_CALLBACK as u8, (TEST_CALLBACK >> 8) as u8, // MOV WORD [0x0028], TEST_CALLBACK (INT 0Ah hook)
        0xC7, 0x06, 0x2A, 0x00, 0x00, 0x00, // MOV WORD [0x002A], 0x0000
        0xE6, 0x64,                         // OUT 0x64, AL            (arm VSYNC)
    ];
    #[rustfmt::skip]
    let hook = [
        0xCD, 0x11,                         // INT 0x11
        0xB0, 0x0B,                         // MOV AL, 0x0B            (OCW3: read ISR)
        0xE6, 0x00,                         // OUT 0x00, AL
        0xE4, 0x00,                         // IN AL, 0x00
        0xA2, 0x08, 0x06,                   // MOV [MASTER_ISR_SEEN], AL
        0xB0, 0x20,                         // MOV AL, 0x20
        0xE6, 0x00,                         // OUT 0x00, AL            (EOI)
        0xCF,                               // IRET
    ];
    let code = make_wait_code(0xFB, 0xFF, &setup, 1, &[]);
    let (mut machine, _cycles) = boot_and_run_ra(&code, &hook, TRACED_BUDGET);
    assert_eq!(
        machine.bus.read_byte(RESULT),
        0xAA,
        "test code should finish"
    );
    assert_eq!(
        machine.bus.read_byte(MASTER_ISR_SEEN),
        0x04,
        "IR2 should stay in service across the software INT 11h"
    );
}

// A software INT 0Bh inside a mouse timer (IR13) handler finds the slave
// interrupt in service. The unclaimed master handler must leave it in service.
#[test]
fn unclaimed_master_vector_inside_slave_interrupt_keeps_it_in_service_ra() {
    #[rustfmt::skip]
    let setup = [
        0xC7, 0x06, 0x54, 0x00, TEST_CALLBACK as u8, (TEST_CALLBACK >> 8) as u8, // MOV WORD [0x0054], TEST_CALLBACK (INT 15h hook)
        0xC7, 0x06, 0x56, 0x00, 0x00, 0x00, // MOV WORD [0x0056], 0x0000
        0xBA, 0xDD, 0x7F,                   // MOV DX, 0x7FDD
        0xB0, 0x10,                         // MOV AL, 0x10            (mouse timer interrupt off)
        0xEE,                               // OUT DX, AL
        0xB0, 0x00,                         // MOV AL, 0x00            (mouse timer interrupt on)
        0xEE,                               // OUT DX, AL
    ];
    #[rustfmt::skip]
    let hook = [
        0xCD, 0x0B,                         // INT 0x0B
        0xB0, 0x0B,                         // MOV AL, 0x0B            (OCW3: read ISR)
        0xE6, 0x08,                         // OUT 0x08, AL
        0xE4, 0x08,                         // IN AL, 0x08
        0xA2, 0x09, 0x06,                   // MOV [SLAVE_ISR_SEEN], AL
        0xB0, 0x0B,                         // MOV AL, 0x0B            (OCW3: read ISR)
        0xE6, 0x00,                         // OUT 0x00, AL
        0xE4, 0x00,                         // IN AL, 0x00
        0xA2, 0x08, 0x06,                   // MOV [MASTER_ISR_SEEN], AL
        0xBA, 0xDD, 0x7F,                   // MOV DX, 0x7FDD
        0xB0, 0x10,                         // MOV AL, 0x10            (mouse timer interrupt off)
        0xEE,                               // OUT DX, AL
        0xB0, 0x20,                         // MOV AL, 0x20
        0xE6, 0x08,                         // OUT 0x08, AL            (slave EOI)
        0xE6, 0x00,                         // OUT 0x00, AL            (master EOI)
        0xCF,                               // IRET
    ];
    let code = make_wait_code(0x7F, 0xDF, &setup, 1, &[]);
    let (mut machine, _cycles) = boot_and_run_ra(&code, &hook, TRACED_BUDGET);
    assert_eq!(
        machine.bus.read_byte(RESULT),
        0xAA,
        "test code should finish"
    );
    assert_eq!(
        machine.bus.read_byte(SLAVE_ISR_SEEN),
        0x20,
        "IR13 should stay in service across the software INT 0Bh"
    );
    assert_eq!(
        machine.bus.read_byte(MASTER_ISR_SEEN),
        0x80,
        "the cascade should stay in service across the software INT 0Bh"
    );
}
