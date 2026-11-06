//! Interval timer countdown under timer interrupts of the PIT that reach the
//! BIOS INT 08h handler in real mode and in V86 mode.

use common::{Bus, Cpu as _};

use super::{
    TEST_CALLBACK, create_machine_ra, read_ivt_vector,
    v86_monitor::{
        Register, TICK_COUNT, TICK_LOG, TrappedAccess, load_register, run_real_mode,
        run_under_v86_monitor_reflecting_irq0,
    },
    write_bytes,
};

/// Interval timer count that INT 1Ch AH=02h starts.
const TIMER_COUNT: u16 = 3;
/// Address of the counter that the INT 07h callback increments.
const CALLBACK_COUNT: u32 = 0x0630;
/// INT 07h callback: INC BYTE [0630h]; IRET.
const COUNTING_CALLBACK: &[u8] = &[0xFE, 0x06, 0x30, 0x06, 0xCF];
/// Master PIC IMR with every IR line masked.
const IMR_ALL_MASKED: u8 = 0xFF;
/// Master PIC IMR with only IR0 unmasked.
const IMR_IR0_UNMASKED: u8 = 0xFE;
/// Linear address of the real mode INT 08h hook.
const REAL_MODE_HOOK: u16 = 0x1200;
/// Number of tick log entries that the tests clear and read.
const TICK_LOG_ENTRIES: u32 = 16;

/// Real mode INT 08h hook at 0000:1200. It logs the interval timer count at
/// 058Ah and jumps to the BIOS handler that the code saved at 0000:06F0.
#[rustfmt::skip]
const REAL_MODE_HOOK_CODE: &[u8] = &[
    0x50,                                           // PUSH AX
    0x53,                                           // PUSH BX
    0x30, 0xFF,                                     // XOR BH, BH
    0x2E, 0x8A, 0x1E, 0x80, 0x06,                   // MOV BL, CS:[0x0680]
    0xD1, 0xE3,                                     // SHL BX, 1
    0x2E, 0xA1, 0x8A, 0x05,                         // MOV AX, CS:[0x058A]
    0x2E, 0x89, 0x87, 0x90, 0x06,                   // MOV CS:[BX+0x0690], AX
    0x2E, 0xFE, 0x06, 0x80, 0x06,                   // INC BYTE CS:[0x0680]
    0x5B,                                           // POP BX
    0x58,                                           // POP AX
    0x2E, 0xFF, 0x2E, 0xF0, 0x06,                   // JMP FAR CS:[0x06F0]
];

/// Real mode code: masks every IR line, hooks INT 08h with the logging hook
/// and saves the BIOS vector at 0000:06F0.
#[rustfmt::skip]
const REAL_MODE_HOOK_INSTALL: &[u8] = &[
    0xFA,                                           // CLI
    0xB0, IMR_ALL_MASKED,                           // MOV AL, 0xFF
    0xE6, 0x02,                                     // OUT 0x02, AL
    0xA1, 0x20, 0x00,                               // MOV AX, [0x0020]
    0xA3, 0xF0, 0x06,                               // MOV [0x06F0], AX
    0xA1, 0x22, 0x00,                               // MOV AX, [0x0022]
    0xA3, 0xF2, 0x06,                               // MOV [0x06F2], AX
    0xC7, 0x06, 0x20, 0x00,
    REAL_MODE_HOOK as u8, (REAL_MODE_HOOK >> 8) as u8, // MOV WORD [0x0020], REAL_MODE_HOOK
    0xC7, 0x06, 0x22, 0x00, 0x00, 0x00,             // MOV WORD [0x0022], 0x0000
];

/// Code that enables interrupts, waits until `ticks` timer interrupts were
/// logged, waits about four more tick periods, writes the return marker and
/// halts.
#[rustfmt::skip]
fn make_tick_wait_code(ticks: u8) -> Vec<u8> {
    vec![
        0xFB,                                       // STI
        0x80, 0x3E, TICK_COUNT as u8, (TICK_COUNT >> 8) as u8, ticks, // CMP BYTE [TICK_COUNT], ticks
        0x72, 0xF9,                                 // JB -7
        0xB9, 0xFF, 0xFF,                           // MOV CX, 0xFFFF
        0xE2, 0xFE,                                 // LOOP -2
        0xFA,                                       // CLI
        0xC6, 0x06, 0x50, 0x06, 0x77,               // MOV BYTE [0x0650], 0x77
        0xF4,                                       // HLT
    ]
}

/// Loads AX=0200h, BX=TEST_CALLBACK and CX=TIMER_COUNT for INT 1Ch AH=02h.
fn load_set_interval_timer_registers() -> Vec<u8> {
    let mut code = Vec::new();
    code.extend_from_slice(&load_register(Register::Ax, 0x0200));
    code.extend_from_slice(&load_register(Register::Bx, TEST_CALLBACK as u16));
    code.extend_from_slice(&load_register(Register::Cx, TIMER_COUNT));
    code
}

/// Boots an RA, clears the tick log and the callback counter and writes the
/// INT 07h callback.
fn boot_ra_for_countdown() -> machine_98::Pc9801Ra {
    let mut machine = create_machine_ra();
    boot_to_halt!(machine);
    write_bytes(&mut machine.bus, TEST_CALLBACK, COUNTING_CALLBACK);
    for address in TICK_COUNT..TICK_LOG + TICK_LOG_ENTRIES * 2 {
        machine.bus.write_byte(address, 0);
    }
    machine.bus.write_byte(CALLBACK_COUNT, 0);
    machine
}

/// Returns the interval timer counts that the tick log holds.
fn read_tick_log(machine: &mut machine_98::Pc9801Ra) -> Vec<u16> {
    let count = u32::from(machine.bus.read_byte(TICK_COUNT));
    assert!(count <= TICK_LOG_ENTRIES, "tick log overflow: {count}");
    (0..count)
        .map(|index| {
            let entry = TICK_LOG + index * 2;
            u16::from_le_bytes([
                machine.bus.read_byte(entry),
                machine.bus.read_byte(entry + 1),
            ])
        })
        .collect()
}

/// Reads the interval timer count at 058Ah.
fn read_interval_timer_count(machine: &mut machine_98::Pc9801Ra) -> u16 {
    u16::from_le_bytes([machine.bus.read_byte(0x058A), machine.bus.read_byte(0x058B)])
}

#[test]
fn interval_timer_counts_down_once_per_tick_in_real_mode_ra() {
    let mut machine = boot_ra_for_countdown();
    write_bytes(
        &mut machine.bus,
        u32::from(REAL_MODE_HOOK),
        REAL_MODE_HOOK_CODE,
    );
    let mut code = REAL_MODE_HOOK_INSTALL.to_vec();
    code.extend_from_slice(&[0x31, 0xC0, 0x8E, 0xC0]); // XOR AX, AX; MOV ES, AX
    code.extend_from_slice(&load_set_interval_timer_registers());
    code.extend_from_slice(&[0xCD, 0x1C]); // INT 0x1C
    code.extend_from_slice(&make_tick_wait_code(TIMER_COUNT as u8));

    run_real_mode(&mut machine, &code);

    assert!(machine.cpu.halted(), "the real mode code did not halt");
    assert_eq!(read_tick_log(&mut machine), [3, 2, 1]);
    assert_eq!(read_interval_timer_count(&mut machine), 0);
    assert_eq!(machine.bus.read_byte(CALLBACK_COUNT), 1);
    assert_eq!(machine.inspection_state().pic.chips[0].imr, IMR_ALL_MASKED);
}

#[test]
fn interval_timer_counts_down_once_per_reflected_tick_in_v86_ra() {
    let mut machine = boot_ra_for_countdown();
    let state = machine.inspection_state();
    let (segment, offset) = read_ivt_vector(&state.memory.ram, 0x1C);

    let mut code = load_set_interval_timer_registers();
    code.extend_from_slice(&[
        0x9C, // PUSHF
        0x9A,
        offset as u8,
        (offset >> 8) as u8,
        segment as u8,
        (segment >> 8) as u8, // CALL FAR segment:offset
    ]);
    code.extend_from_slice(&make_tick_wait_code(TIMER_COUNT as u8));

    let accesses = run_under_v86_monitor_reflecting_irq0(&mut machine, IMR_ALL_MASKED, &code);

    assert_eq!(read_tick_log(&mut machine), [3, 2, 1]);
    assert_eq!(read_interval_timer_count(&mut machine), 0);
    assert_eq!(machine.bus.read_byte(CALLBACK_COUNT), 1);
    let imr_update = |before: u8, after: u8| {
        [
            TrappedAccess::In {
                port: 0x02,
                value: before,
            },
            TrappedAccess::Out {
                port: 0x02,
                value: after,
            },
        ]
    };
    let expected: Vec<TrappedAccess> = [
        imr_update(IMR_ALL_MASKED, IMR_IR0_UNMASKED),
        imr_update(IMR_IR0_UNMASKED, IMR_IR0_UNMASKED),
        imr_update(IMR_IR0_UNMASKED, IMR_IR0_UNMASKED),
        imr_update(IMR_IR0_UNMASKED, IMR_ALL_MASKED),
    ]
    .into_iter()
    .flatten()
    .collect();
    assert_eq!(accesses, expected);
    assert_eq!(machine.inspection_state().pic.chips[0].imr, IMR_ALL_MASKED);
}
