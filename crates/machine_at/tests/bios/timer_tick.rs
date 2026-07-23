//! INT 08h timer tick: tick rate, INT 1Ch chaining, midnight rollover and
//! the floppy motor shutoff countdown.

use common::Bus;

use super::{
    RESULT, create_machine_dx50, create_machine_dx66, inject_and_run, read_ivt_vector, read_ram_u8,
    read_ram_u16, read_ram_u32, write_bytes,
};

/// BIOS data area: timer tick count (dword).
const BDA_TIMER_COUNT: u32 = 0x46C;
/// BIOS data area: timer 24-hour rollover flag.
const BDA_TIMER_OVERFLOW: u32 = 0x470;
/// BIOS data area: diskette motor status.
const BDA_FLOPPY_MOTOR: u32 = 0x43F;
/// BIOS data area: diskette motor shutoff counter.
const BDA_FLOPPY_MOTOR_COUNT: u32 = 0x440;
/// FDC digital output register port.
const FDC_DOR_PORT: u16 = 0x3F2;

/// STI, then a HLT loop that wakes on every timer tick.
const TICK_LOOP_CODE: &[u8] = &[
    0xFB, // STI
    0xF4, // HLT
    0xEB, 0xFD, // JMP short back to the HLT
];

/// Installs IVT vector 1Ch = 0000:2000, then enters the tick loop.
#[rustfmt::skip]
const HOOK_INT1CH_CODE: &[u8] = &[
    0xC7, 0x06, 0x70, 0x00, 0x00, 0x20, // MOV WORD [0x0070], 0x2000
    0xC7, 0x06, 0x72, 0x00, 0x00, 0x00, // MOV WORD [0x0072], 0x0000
    0xFB,                               // STI
    0xF4,                               // HLT
    0xEB, 0xFD,                         // JMP short back to the HLT
];

/// INT 1Ch callback: increments the counter at the result address.
#[rustfmt::skip]
const COUNT_TICKS_CALLBACK: &[u8] = &[
    0xFF, 0x06, 0x00, 0x06, // INC WORD [0x0600]
    0xCF,                   // IRET
];

#[test]
fn tick_rate_is_18_2_hz_dx50() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    let clock_hz = u64::from(machine.bus.cpu_clock_hz());
    let ticks_before = read_ram_u32(&machine, BDA_TIMER_COUNT);
    inject_and_run(&mut machine, TICK_LOOP_CODE, &[], clock_hz);
    let delta = read_ram_u32(&machine, BDA_TIMER_COUNT) - ticks_before;

    assert!(
        (17..=19).contains(&delta),
        "expected about 18 ticks in one second, got {delta}"
    );
}

#[test]
fn tick_rate_is_18_2_hz_dx66() {
    let mut machine = create_machine_dx66();
    boot_to_halt!(machine);

    let clock_hz = u64::from(machine.bus.cpu_clock_hz());
    let ticks_before = read_ram_u32(&machine, BDA_TIMER_COUNT);
    inject_and_run(&mut machine, TICK_LOOP_CODE, &[], clock_hz);
    let delta = read_ram_u32(&machine, BDA_TIMER_COUNT) - ticks_before;

    assert!(
        (17..=19).contains(&delta),
        "expected about 18 ticks in one second, got {delta}"
    );
}

#[test]
fn int1ch_hook_is_invoked_per_tick() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    let clock_hz = u64::from(machine.bus.cpu_clock_hz());
    let ticks_before = read_ram_u32(&machine, BDA_TIMER_COUNT);
    inject_and_run(
        &mut machine,
        HOOK_INT1CH_CODE,
        COUNT_TICKS_CALLBACK,
        clock_hz / 2,
    );
    let delta = read_ram_u32(&machine, BDA_TIMER_COUNT) - ticks_before;
    let hook_count = u32::from(read_ram_u16(&machine, RESULT));

    assert!(hook_count >= 2, "hook must run, got {hook_count}");
    // The budget may expire between the Rust tick work and the chained
    // INT 1Ch of the last tick, so allow the hook to be one behind.
    assert!(
        hook_count == delta || hook_count + 1 == delta,
        "hook count {hook_count} must match the tick delta {delta}"
    );
}

#[test]
fn midnight_rollover_resets_count_and_sets_flag() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    // One tick short of the 24-hour count 0x0018_00B0.
    write_bytes(&mut machine, BDA_TIMER_COUNT, &[0xAF, 0x00, 0x18, 0x00]);
    write_bytes(&mut machine, BDA_TIMER_OVERFLOW, &[0x00]);
    inject_and_run(&mut machine, TICK_LOOP_CODE, &[], 10_000_000);

    assert!(
        read_ram_u32(&machine, BDA_TIMER_COUNT) < 4,
        "tick count must restart from zero"
    );
    assert_eq!(
        read_ram_u8(&machine, BDA_TIMER_OVERFLOW),
        1,
        "midnight flag must be set"
    );
}

#[test]
fn motor_countdown_stops_the_motor() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    // Motor A on: drive select 0, controller enabled, DMA gate open.
    machine.bus.io_write_byte(FDC_DOR_PORT, 0x1C);
    write_bytes(&mut machine, BDA_FLOPPY_MOTOR, &[0x01]);
    write_bytes(&mut machine, BDA_FLOPPY_MOTOR_COUNT, &[0x02]);
    inject_and_run(&mut machine, TICK_LOOP_CODE, &[], 10_000_000);

    assert_eq!(
        read_ram_u8(&machine, BDA_FLOPPY_MOTOR_COUNT),
        0,
        "shutoff counter must reach zero"
    );
    assert_eq!(
        read_ram_u8(&machine, BDA_FLOPPY_MOTOR) & 0x0F,
        0,
        "BDA motor running flags must clear"
    );
    assert_eq!(
        machine.bus.io_read_byte(FDC_DOR_PORT) & 0xF0,
        0,
        "DOR motor enables must clear"
    );
}

#[test]
fn int08h_vector_points_at_the_chain_stub() {
    let mut machine = create_machine_dx50();
    boot_to_halt!(machine);

    let (timer_segment, timer_offset) = read_ivt_vector(&machine, 0x08);
    let (_, cascade_offset) = read_ivt_vector(&machine, 0x0A);
    let (user_hook_segment, user_hook_offset) = read_ivt_vector(&machine, 0x1C);
    let (_, unused_offset) = read_ivt_vector(&machine, 0x44);

    assert_eq!(timer_segment, 0xF000);
    assert_ne!(
        timer_offset, cascade_offset,
        "INT 08h must not share the plain EOI stub"
    );
    assert_eq!(user_hook_segment, 0xF000);
    assert_eq!(
        user_hook_offset, unused_offset,
        "INT 1Ch defaults to the shared IRET stub"
    );
}
