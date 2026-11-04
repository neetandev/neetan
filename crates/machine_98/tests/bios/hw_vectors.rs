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

const VSYNC_BUDGET: u64 = 4_000_000;

/// Counts the IR2 acknowledges and the master PIC EOIs written by guest OUT
/// instructions.
#[derive(Default)]
struct EoiRecorder {
    vsync_acknowledges: usize,
    master_eoi_writes: usize,
}

impl common::TraceSink for EoiRecorder {
    fn trace(&mut self, _context: common::TraceContext, event: common::TraceEvent<'_>) {
        match event {
            common::TraceEvent::Interrupt(interrupt)
                if interrupt.action == common::TraceInterruptAction::Acknowledge
                    && interrupt.line == Some(2) =>
            {
                self.vsync_acknowledges += 1;
            }
            common::TraceEvent::Access(access)
                if access.space == common::TraceAddressSpace::MAIN_IO
                    && access.kind == common::TraceAccessKind::Write
                    && access.address == 0x00
                    && access.value == Some(0x20) =>
            {
                self.master_eoi_writes += 1;
            }
            _ => {}
        }
    }
}

/// Unmasks only IR2, arms the VSYNC interrupt and waits for two of them.
#[rustfmt::skip]
const VSYNC_WAIT_CODE: &[u8] = &[
    0xFA,                               // CLI
    0xB0, 0xFB,                         // MOV AL, 0xFB            (only IR2 unmasked)
    0xE6, 0x02,                         // OUT 0x02, AL
    0xB0, 0xFF,                         // MOV AL, 0xFF
    0xE6, 0x0A,                         // OUT 0x0A, AL            (slave masked)
    0xE6, 0x64,                         // OUT 0x64, AL            (arm VSYNC interrupt)
    0xFB,                               // STI
    0xF4,                               // HLT
    0xF4,                               // HLT
    0xFA,                               // CLI
    0xC6, 0x06, 0x00, 0x06, 0xAA,       // MOV BYTE [RESULT], 0xAA
    0xF4,                               // HLT
];

// The BIOS INT 0Ah handler ends the VSYNC interrupt with a guest OUT to port
// 00h, so a V86 monitor that traps the PIC ports sees the EOI.
#[test]
fn vsync_interrupt_eoi_is_a_guest_io_write_ra() {
    let mut machine = machine_98::Pc98Machine::<cpu::I386, EoiRecorder>::new(
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
    super::write_bytes(&mut machine.bus, super::TEST_CODE, VSYNC_WAIT_CODE);
    machine.cpu.load_state(&{
        let mut state = cpu::I386State {
            ip: super::TEST_CODE as u16,
            ..Default::default()
        };
        state.set_esp(0x4000);
        state
    });
    *machine.bus.tracer_mut() = EoiRecorder::default();
    machine.run_for(VSYNC_BUDGET);

    assert_eq!(
        machine.bus.read_byte(RESULT),
        0xAA,
        "test code should finish"
    );
    let recorder = machine.bus.tracer();
    assert!(
        recorder.vsync_acknowledges >= 2,
        "two VSYNC interrupts should be acknowledged (got {})",
        recorder.vsync_acknowledges
    );
    assert!(
        recorder.master_eoi_writes >= recorder.vsync_acknowledges,
        "each VSYNC interrupt should end with a guest OUT 00h,20h ({} EOIs for {} interrupts)",
        recorder.master_eoi_writes,
        recorder.vsync_acknowledges
    );
    assert_eq!(
        machine.inspection_state().pic.chips[0].isr,
        0,
        "master ISR should be clear"
    );
}
