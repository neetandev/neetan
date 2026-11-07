//! INT 18h display functions under a V86 monitor. The BIOS programs the display
//! hardware with guest OUT instructions, so a monitor that traps the display
//! ports sees every write of the BIOS, as a protected-mode display driver of a
//! multitasking system does.

use common::Bus;
use machine_98::{Pc98InspectionState, Pc98Machine};

use super::{
    create_machine_pc9821as, create_machine_ra, read_ivt_vector,
    v86_monitor::{
        Register, TrappedAccess, load_register, make_v86_call_code, run_real_mode,
        run_under_v86_monitor_trapping,
    },
    write_bytes,
};

/// Ports of the display hardware that the INT 18h functions write.
const DISPLAY_PORTS: [u16; 17] = [
    0x0037, 0x0060, 0x0062, 0x0068, 0x006A, 0x0070, 0x0072, 0x0074, 0x0076, 0x00A0, 0x00A2, 0x00A4,
    0x00A8, 0x00AA, 0x00AC, 0x00AE, 0x09A8,
];

/// Master IMR during a monitor run: every interrupt line masked.
const ALL_IRQS_MASKED: u8 = 0xFF;

/// Linear address of the UCW that AH=43h reads with DS=0000h.
const UCW_ADDRESS: u16 = 0x2100;

/// BIOS work area byte with the graphics status flags (PRXCRT).
const PRXCRT: usize = 0x054C;
/// PRXCRT bit 7: the graphics screen is shown.
const PRXCRT_GRAPHICS_DISPLAY: u8 = 0x80;

/// Text GDC command port.
const TEXT_GDC_COMMAND: u16 = 0x0062;
/// Graphics GDC parameter port.
const GRAPHICS_GDC_PARAMETER: u16 = 0x00A0;
/// Graphics GDC command port.
const GRAPHICS_GDC_COMMAND: u16 = 0x00A2;
/// Mode F/F 2 port.
const MODE_FLIP_FLOP_2: u16 = 0x006A;
/// Horizontal scan frequency port.
const SCAN_FREQUENCY: u16 = 0x09A8;

/// Machine code for `MOV AL, value` and `OUT port, AL`.
fn out_imm8(port: u8, value: u8) -> [u8; 4] {
    [0xB0, value, 0xE6, port]
}

/// Returns a trapped `OUT port, value`.
fn out(port: u16, value: u8) -> TrappedAccess {
    TrappedAccess::Out { port, value }
}

/// Returns the trapped writes of a GDC command and its parameters.
fn gdc_command(command_port: u16, command: u8, parameters: &[u8]) -> Vec<TrappedAccess> {
    let parameter_port = command_port - 2;
    let mut writes = vec![out(command_port, command)];
    writes.extend(
        parameters
            .iter()
            .map(|&parameter| out(parameter_port, parameter)),
    );
    writes
}

/// Boots `machine` and returns it with the INT 18h handler of its IVT.
fn boot_for_int18h<const CPU_MODEL: u8>(
    mut machine: Pc98Machine<cpu::I386<CPU_MODEL>>,
) -> (Pc98Machine<cpu::I386<CPU_MODEL>>, (u16, u16)) {
    boot_to_halt!(machine);
    let handler = read_ivt_vector(&machine.inspection_state().memory.ram, 0x18);
    (machine, handler)
}

/// Runs `setup` and an INT 18h call in V86 mode under a monitor that traps the
/// display ports and returns the trapped accesses.
fn int18h_under_v86_monitor<const CPU_MODEL: u8>(
    machine: &mut Pc98Machine<cpu::I386<CPU_MODEL>>,
    handler: (u16, u16),
    setup: &[u8],
) -> Vec<TrappedAccess> {
    let code = make_v86_call_code(setup, handler);
    run_under_v86_monitor_trapping(machine, ALL_IRQS_MASKED, &code, &DISPLAY_PORTS)
}

/// Runs `setup` and an INT 18h instruction in real mode.
fn int18h_in_real_mode<const CPU_MODEL: u8>(
    machine: &mut Pc98Machine<cpu::I386<CPU_MODEL>>,
    setup: &[u8],
) {
    let mut code = vec![0xFA];
    code.extend_from_slice(setup);
    code.extend_from_slice(&[0xCD, 0x18, 0xF4]);
    run_real_mode(machine, &code);
}

/// Reads a little-endian word.
fn read_word(bus: &mut impl Bus, address: u32) -> u16 {
    u16::from(bus.read_byte(address)) | (u16::from(bus.read_byte(address + 1)) << 8)
}

/// Machine code that loads AX.
fn ax(value: u16) -> Vec<u8> {
    load_register(Register::Ax, value).to_vec()
}

/// Machine code that loads AX and BX.
fn ax_bx(ax: u16, bx: u16) -> Vec<u8> {
    let mut code = load_register(Register::Ax, ax).to_vec();
    code.extend_from_slice(&load_register(Register::Bx, bx));
    code
}

/// Machine code that loads AX and CX.
fn ax_cx(ax: u16, cx: u16) -> Vec<u8> {
    let mut code = load_register(Register::Ax, ax).to_vec();
    code.extend_from_slice(&load_register(Register::Cx, cx));
    code
}

/// Machine code that loads AX and DX.
fn ax_dx(ax: u16, dx: u16) -> Vec<u8> {
    let mut code = load_register(Register::Ax, ax).to_vec();
    code.extend_from_slice(&load_register(Register::Dx, dx));
    code
}

/// Machine code that loads AX, BX, CX and DX.
fn ax_bx_cx_dx(ax: u16, bx: u16, cx: u16, dx: u16) -> Vec<u8> {
    let mut code = ax_bx(ax, bx);
    code.extend_from_slice(&load_register(Register::Cx, cx));
    code.extend_from_slice(&load_register(Register::Dx, dx));
    code
}

/// Returns true when `writes` holds `sequence` as consecutive writes.
fn contains_sequence(writes: &[TrappedAccess], sequence: &[TrappedAccess]) -> bool {
    writes
        .windows(sequence.len())
        .any(|window| window == sequence)
}

/// Asserts that two machines hold the same display registers.
fn assert_same_display_registers(v86: &Pc98InspectionState, real_mode: &Pc98InspectionState) {
    for (name, v86_gdc, real_mode_gdc) in [
        ("text GDC", &v86.gdc_master, &real_mode.gdc_master),
        ("graphics GDC", &v86.gdc_slave, &real_mode.gdc_slave),
    ] {
        assert_eq!(
            v86_gdc.display_enabled, real_mode_gdc.display_enabled,
            "{name}"
        );
        assert_eq!(v86_gdc.display_mode, real_mode_gdc.display_mode, "{name}");
        assert_eq!(v86_gdc.pitch, real_mode_gdc.pitch, "{name}");
        assert_eq!(v86_gdc.aw, real_mode_gdc.aw, "{name}");
        assert_eq!(v86_gdc.al, real_mode_gdc.al, "{name}");
        assert_eq!(v86_gdc.scroll, real_mode_gdc.scroll, "{name}");
        assert_eq!(v86_gdc.lines_per_row, real_mode_gdc.lines_per_row, "{name}");
        assert_eq!(v86_gdc.dot_clock_hz, real_mode_gdc.dot_clock_hz, "{name}");
    }
    assert_eq!(
        v86.display_control.video_mode,
        real_mode.display_control.video_mode
    );
    assert_eq!(v86.display_control.mode2, real_mode.display_control.mode2);
    assert_eq!(
        v86.display_control.display_page,
        real_mode.display_control.display_page
    );
    assert_eq!(
        v86.display_control.crt_31khz_enabled,
        real_mode.display_control.crt_31khz_enabled
    );
    assert_eq!(v86.crtc, real_mode.crtc);
}

#[test]
fn int18h_graphics_display_start_writes_start_to_the_graphics_gdc_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x4000));

    assert_eq!(accesses, [out(GRAPHICS_GDC_COMMAND, 0x0D)]);
    let state = machine.inspection_state();
    assert!(state.gdc_slave.display_enabled);
    assert_ne!(state.memory.ram[PRXCRT] & PRXCRT_GRAPHICS_DISPLAY, 0);
}

#[test]
fn int18h_graphics_display_stop_writes_stop_to_the_graphics_gdc_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x4100));

    assert_eq!(accesses, [out(GRAPHICS_GDC_COMMAND, 0x0C)]);
    let state = machine.inspection_state();
    assert!(!state.gdc_slave.display_enabled);
    assert_eq!(state.memory.ram[PRXCRT] & PRXCRT_GRAPHICS_DISPLAY, 0);
}

#[test]
fn int18h_graphics_display_start_follows_a_pitch_write_of_the_caller_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_pc9821as());
    let mut setup = Vec::new();
    setup.extend_from_slice(&out_imm8(0xA2, 0x47));
    setup.extend_from_slice(&out_imm8(0xA0, 0x80));
    setup.extend_from_slice(&ax(0x4000));

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &setup);

    assert_eq!(
        accesses,
        [
            out(GRAPHICS_GDC_COMMAND, 0x47),
            out(GRAPHICS_GDC_PARAMETER, 0x80),
            out(GRAPHICS_GDC_COMMAND, 0x0D),
        ]
    );
    assert_eq!(machine.inspection_state().gdc_slave.pitch, 0x80);
}

#[test]
fn int18h_text_display_start_and_stop_write_to_the_text_gdc_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x0D00));
    assert_eq!(accesses, [out(TEXT_GDC_COMMAND, 0x0C)]);
    assert!(!machine.inspection_state().gdc_master.display_enabled);

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x0C00));
    assert_eq!(accesses, [out(TEXT_GDC_COMMAND, 0x0D)]);
    assert!(machine.inspection_state().gdc_master.display_enabled);
}

#[test]
fn int18h_cursor_position_writes_csrw_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax_dx(0x1300, 0x0100));

    assert_eq!(accesses, gdc_command(TEXT_GDC_COMMAND, 0x49, &[0x80, 0x00]));
    assert_eq!(machine.inspection_state().gdc_master.ead, 0x80);
}

#[test]
fn int18h_cursor_display_writes_the_first_csrform_parameter_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x1100));
    assert_eq!(accesses, gdc_command(TEXT_GDC_COMMAND, 0x4B, &[0x8F]));
    assert!(machine.inspection_state().gdc_master.cursor_display);

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x1200));
    assert_eq!(accesses, gdc_command(TEXT_GDC_COMMAND, 0x4B, &[0x0F]));
    assert!(!machine.inspection_state().gdc_master.cursor_display);
}

#[test]
fn int18h_cursor_form_writes_csrform_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x1001));

    assert_eq!(
        accesses,
        gdc_command(TEXT_GDC_COMMAND, 0x4B, &[0x0F, 0x20, 0x7B])
    );
    let state = machine.inspection_state();
    assert_eq!(state.gdc_master.lines_per_row, 16);
    assert_eq!(state.gdc_master.cursor_bottom, 15);
    assert_eq!(state.gdc_master.cursor_blink_rate, 12);
    assert!(state.gdc_master.cursor_blink);
}

#[test]
fn int18h_single_display_area_writes_pram_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax_dx(0x0E00, 0x0200));

    assert_eq!(
        accesses,
        gdc_command(TEXT_GDC_COMMAND, 0x70, &[0x00, 0x01, 0x00, 0x19])
    );
    let area = machine.inspection_state().gdc_master.scroll[0];
    assert_eq!(area.start_address, 0x0100);
    assert_eq!(area.line_count, 400);
}

/// Linear address of the display area list that AH=0Fh reads with BX=0000h.
const DISPLAY_AREA_LIST_ADDRESS: u16 = 0x2200;

/// Writes `count` AH=0Fh list entries. Entry `index` shows text VRAM offset `index * 100h`
/// for `index + 1` rows.
fn write_display_area_list(bus: &mut impl Bus, count: u16) {
    for index in 0..count {
        let entry = u32::from(DISPLAY_AREA_LIST_ADDRESS) + u32::from(index) * 4;
        let vram_offset = index * 0x0100;
        let rows = index + 1;
        write_bytes(bus, entry, &vram_offset.to_le_bytes());
        write_bytes(bus, entry + 2, &rows.to_le_bytes());
    }
}

/// Returns the trapped PRAM writes of list entry `index` for the display area `area`. The
/// line count keeps its low 10 bits.
fn display_area_list_entry_writes(area: u8, index: u16) -> Vec<TrappedAccess> {
    let [address_low, address_high] = (index * 0x0080).to_le_bytes();
    let lines = ((index + 1) * 16) & 0x03FF;
    gdc_command(
        TEXT_GDC_COMMAND,
        0x70 | (area * 4),
        &[
            address_low,
            address_high,
            ((lines & 0x0F) << 4) as u8,
            (lines >> 4) as u8,
        ],
    )
}

#[test]
fn int18h_display_area_list_writes_pram_at_the_first_area_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());
    write_display_area_list(&mut machine.bus, 1);

    let setup = ax_bx_cx_dx(0x0F00, 0x0000, DISPLAY_AREA_LIST_ADDRESS, 0x0201);
    let accesses = int18h_under_v86_monitor(&mut machine, handler, &setup);

    assert_eq!(accesses, display_area_list_entry_writes(2, 0));
    let area = machine.inspection_state().gdc_master.scroll[2];
    assert_eq!(area.start_address, 0x0000);
    assert_eq!(area.line_count, 16);
}

#[test]
fn int18h_display_area_list_writes_the_last_entry_of_each_area_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());
    write_display_area_list(&mut machine.bus, 6);

    let setup = ax_bx_cx_dx(0x0F00, 0x0000, DISPLAY_AREA_LIST_ADDRESS, 0x0006);
    let accesses = int18h_under_v86_monitor(&mut machine, handler, &setup);

    let mut expected = display_area_list_entry_writes(2, 2);
    expected.extend(display_area_list_entry_writes(3, 3));
    expected.extend(display_area_list_entry_writes(0, 4));
    expected.extend(display_area_list_entry_writes(1, 5));
    assert_eq!(accesses, expected);
    let state = machine.inspection_state();
    assert_eq!(state.gdc_master.scroll[0].start_address, 0x0200);
    assert_eq!(state.gdc_master.scroll[1].start_address, 0x0280);
    assert_eq!(state.gdc_master.scroll[2].start_address, 0x0100);
    assert_eq!(state.gdc_master.scroll[3].start_address, 0x0180);
}

#[test]
fn int18h_display_area_list_with_255_entries_fits_the_port_write_frame_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());
    write_display_area_list(&mut machine.bus, 255);

    let setup = ax_bx_cx_dx(0x0F00, 0x0000, DISPLAY_AREA_LIST_ADDRESS, 0x00FF);
    let accesses = int18h_under_v86_monitor(&mut machine, handler, &setup);

    let mut expected = display_area_list_entry_writes(3, 251);
    expected.extend(display_area_list_entry_writes(0, 252));
    expected.extend(display_area_list_entry_writes(1, 253));
    expected.extend(display_area_list_entry_writes(2, 254));
    assert_eq!(accesses, expected);
}

#[test]
fn int18h_buzzer_writes_the_system_ppi_control_port_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x1700));
    assert_eq!(accesses, [out(0x0037, 0x06)]);

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x1800));
    assert_eq!(accesses, [out(0x0037, 0x07)]);
}

#[test]
fn int18h_kcg_access_mode_writes_mode_flip_flop_1_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x1B01));

    assert_eq!(accesses, [out(0x0068, 0x0B)]);
}

#[test]
fn int18h_crt_mode_writes_mode_flip_flops_line_counter_and_csrform_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x0A00));

    let mut expected = vec![
        out(0x0068, 0x00),
        out(0x0068, 0x04),
        out(0x0068, 0x07),
        out(0x0068, 0x0A),
        out(0x0070, 0x00),
        out(0x0072, 0x0F),
        out(0x0074, 0x10),
        out(0x0076, 0x00),
    ];
    expected.extend(gdc_command(TEXT_GDC_COMMAND, 0x4B, &[0x0F, 0x00, 0x7B]));
    assert_eq!(accesses, expected);
}

#[test]
fn int18h_palette_writes_the_digital_palette_ports_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());
    write_bytes(
        &mut machine.bus,
        u32::from(UCW_ADDRESS) + 4,
        &[0x12, 0x34, 0x56, 0x78],
    );

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax_bx(0x4300, UCW_ADDRESS));

    assert_eq!(
        accesses,
        [
            out(0x00A8, 0x62),
            out(0x00AA, 0x84),
            out(0x00AC, 0x51),
            out(0x00AE, 0x73),
        ]
    );
    assert_eq!(
        machine.inspection_state().palette.digital,
        [0x62, 0x84, 0x51, 0x73]
    );
}

#[test]
fn int18h_draw_mode_writes_sync_and_pitch_to_the_graphics_gdc_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax_cx(0x4A00, 0x1200));

    let mut expected = gdc_command(
        GRAPHICS_GDC_COMMAND,
        0x0E,
        &[0x12, 0x26, 0x03, 0x11, 0x03, 0x07, 0x90, 0x65],
    );
    expected.extend(gdc_command(GRAPHICS_GDC_COMMAND, 0x47, &[40]));
    assert_eq!(accesses, expected);
    let state = machine.inspection_state();
    assert!(state.gdc_slave.draw_on_retrace);
    assert_eq!(state.gdc_slave.pitch, 40);
    assert_eq!(state.gdc_slave.aw, 40);
    assert_eq!(state.gdc_slave.al, 400);
}

#[test]
fn int18h_extended_graphics_select_writes_the_protected_mode_flip_flop_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_pc9821as());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax_cx(0x4D00, 0x0100));

    assert_eq!(
        accesses,
        [
            out(MODE_FLIP_FLOP_2, 0x07),
            out(MODE_FLIP_FLOP_2, 0x21),
            out(MODE_FLIP_FLOP_2, 0x06),
        ]
    );
    let state = machine.inspection_state();
    assert_ne!(state.memory.ram[0x054D] & 0x80, 0);
    assert_eq!(state.display_control.mode2 & 0x08, 0);
}

#[test]
fn int18h_standard_graphics_select_writes_the_protected_mode_flip_flop_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_pc9821as());
    int18h_under_v86_monitor(&mut machine, handler, &ax_cx(0x4D00, 0x0100));

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax_cx(0x4D00, 0x0000));

    assert_eq!(
        accesses,
        [
            out(MODE_FLIP_FLOP_2, 0x07),
            out(MODE_FLIP_FLOP_2, 0x20),
            out(MODE_FLIP_FLOP_2, 0x06),
        ]
    );
    assert_eq!(machine.inspection_state().memory.ram[0x054D] & 0x80, 0);
}

/// Returns an RA whose DIP SW 2-8 selects the 5 MHz graphics GDC clock.
fn create_machine_ra_5mhz() -> machine_98::Pc9801Ra {
    let mut machine = create_machine_ra();
    machine.bus.set_gdc_clock_5mhz();
    machine
}

/// Returns a PC-9821AS whose DIP SW 2-8 selects the 5 MHz graphics GDC clock.
fn create_machine_pc9821as_5mhz() -> machine_98::Pc9821As {
    let mut machine = create_machine_pc9821as();
    machine.bus.set_gdc_clock_5mhz();
    machine
}

/// Calls AH=42h CH=C0h (640x400, color, bank 0) under the monitor and in real mode on two
/// machines from `create`, asserts the trapped writes and the same display registers.
fn assert_display_area_set_writes<const CPU_MODEL: u8>(
    create: fn() -> Pc98Machine<cpu::I386<CPU_MODEL>>,
    expected: &[TrappedAccess],
) {
    let setup = ax_cx(0x4200, 0xC000);
    let (mut v86_machine, handler) = boot_for_int18h(create());
    let accesses = int18h_under_v86_monitor(&mut v86_machine, handler, &setup);
    let (mut real_mode_machine, _) = boot_for_int18h(create());
    int18h_in_real_mode(&mut real_mode_machine, &setup);

    assert_eq!(accesses, expected);
    assert_same_display_registers(
        &v86_machine.inspection_state(),
        &real_mode_machine.inspection_state(),
    );
}

/// Trapped writes of AH=42h CH=C0h after the mode F/F 1 writes: CSRFORM with one line per
/// row, the full screen display area and display bank 0.
fn display_area_set_tail() -> Vec<TrappedAccess> {
    let mut writes = gdc_command(GRAPHICS_GDC_COMMAND, 0x4B, &[0x00]);
    writes.extend(gdc_command(
        GRAPHICS_GDC_COMMAND,
        0x70,
        &[0x00, 0x00, 0x00, 0x00],
    ));
    writes.push(out(0x00A4, 0x00));
    writes
}

#[test]
fn int18h_display_area_set_writes_match_real_mode_state_ra() {
    let mut expected = vec![out(0x0068, 0x02), out(0x0068, 0x08)];
    expected.extend(display_area_set_tail());
    assert_display_area_set_writes(create_machine_ra, &expected);
}

#[test]
fn int18h_display_area_set_switches_to_5mhz_with_sync_pitch_and_clock_writes_ra() {
    let mut expected = vec![out(0x0068, 0x02), out(0x0068, 0x08)];
    expected.extend(gdc_command(
        GRAPHICS_GDC_COMMAND,
        0x0E,
        &[0x02, 0x4E, 0x07, 0x25, 0x07, 0x07, 0x90, 0x65],
    ));
    expected.extend(gdc_command(GRAPHICS_GDC_COMMAND, 0x47, &[80]));
    expected.extend([out(MODE_FLIP_FLOP_2, 0x83), out(MODE_FLIP_FLOP_2, 0x85)]);
    expected.extend(display_area_set_tail());
    assert_display_area_set_writes(create_machine_ra_5mhz, &expected);
}

/// Calls AH=30h AL=08h BH=21h (24.83 kHz, 640x400, 25 rows) under the monitor and in real
/// mode on two machines from `create`. Asserts the leading scan frequency and text GDC writes,
/// the closing graphics GDC pitch `graphics_pitch` and the same display registers.
fn assert_extended_screen_mode_writes<const CPU_MODEL: u8>(
    create: fn() -> Pc98Machine<cpu::I386<CPU_MODEL>>,
    graphics_pitch: u8,
) {
    let setup = ax_bx(0x3008, 0x2100);
    let (mut v86_machine, handler) = boot_for_int18h(create());
    let accesses = int18h_under_v86_monitor(&mut v86_machine, handler, &setup);
    let (mut real_mode_machine, _) = boot_for_int18h(create());
    int18h_in_real_mode(&mut real_mode_machine, &setup);

    let mut leading = vec![out(SCAN_FREQUENCY, 0x00)];
    leading.extend(gdc_command(
        TEXT_GDC_COMMAND,
        0x0E,
        &[0x10, 0x4E, 0x07, 0x25, 0x07, 0x07, 0x90, 0x65],
    ));
    leading.extend(gdc_command(TEXT_GDC_COMMAND, 0x47, &[80]));
    assert_eq!(accesses[..leading.len()], leading[..]);
    assert_eq!(
        accesses[accesses.len() - 2..],
        gdc_command(GRAPHICS_GDC_COMMAND, 0x47, &[graphics_pitch])[..]
    );
    assert!(
        accesses
            .iter()
            .all(|access| matches!(access, TrappedAccess::Out { .. }))
    );
    assert_eq!(v86_machine.bus.read_byte(0x0597) & 0x03, 0x02);
    assert_same_display_registers(
        &v86_machine.inspection_state(),
        &real_mode_machine.inspection_state(),
    );
    let state = v86_machine.inspection_state();
    assert!(!state.gdc_master.display_enabled);
    assert_eq!(state.gdc_slave.pitch, u16::from(graphics_pitch));
}

#[test]
fn int18h_extended_screen_mode_writes_match_real_mode_state_pc9821as() {
    assert_extended_screen_mode_writes(create_machine_pc9821as, 40);
}

#[test]
fn int18h_extended_screen_mode_writes_match_real_mode_state_5mhz_pc9821as() {
    assert_extended_screen_mode_writes(create_machine_pc9821as_5mhz, 80);
}

#[test]
fn int18h_extended_screen_mode_640x480_writes_match_real_mode_state_pc9821as() {
    let setup = ax_bx(0x300C, 0x3200);
    let (mut v86_machine, handler) = boot_for_int18h(create_machine_pc9821as());
    let accesses = int18h_under_v86_monitor(&mut v86_machine, handler, &setup);
    let (mut real_mode_machine, _) = boot_for_int18h(create_machine_pc9821as());
    int18h_in_real_mode(&mut real_mode_machine, &setup);

    assert_eq!(accesses[0], out(SCAN_FREQUENCY, 0x01));
    let extended_graphics = [
        out(MODE_FLIP_FLOP_2, 0x07),
        out(MODE_FLIP_FLOP_2, 0x21),
        out(MODE_FLIP_FLOP_2, 0x06),
    ];
    let vram_800_lines = [
        out(MODE_FLIP_FLOP_2, 0x07),
        out(MODE_FLIP_FLOP_2, 0x69),
        out(MODE_FLIP_FLOP_2, 0x06),
    ];
    assert!(contains_sequence(&accesses, &extended_graphics));
    assert!(contains_sequence(&accesses, &vram_800_lines));
    assert!(contains_sequence(
        &accesses,
        &gdc_command(TEXT_GDC_COMMAND, 0x70, &[0x00, 0x00, 0x00, 0x1E])
    ));
    assert!(contains_sequence(
        &accesses,
        &gdc_command(GRAPHICS_GDC_COMMAND, 0x70, &[0x00, 0x00, 0x00, 0x00])
    ));
    assert_same_display_registers(
        &v86_machine.inspection_state(),
        &real_mode_machine.inspection_state(),
    );
    let state = v86_machine.inspection_state();
    assert!(state.display_control.crt_31khz_enabled);
    assert_eq!(state.gdc_master.scroll[0].line_count, 480);
}

#[test]
fn int18h_key_sense_writes_no_display_port_under_v86() {
    let (mut machine, handler) = boot_for_int18h(create_machine_ra());

    let accesses = int18h_under_v86_monitor(&mut machine, handler, &ax(0x0100));

    assert_eq!(accesses, []);
}

#[test]
fn int18h_returns_the_registers_of_the_function_after_the_port_writes() {
    let (mut machine, handler) = boot_for_int18h(create_machine_pc9821as());
    let mut setup = ax_bx(0x3008, 0x2134);
    setup.extend_from_slice(&load_register(Register::Cx, 0x5678));
    setup.extend_from_slice(&load_register(Register::Dx, 0x9ABC));
    #[rustfmt::skip]
    let store_registers = [
        0xA3, 0x30, 0x06,                           // MOV [0x0630], AX
        0x89, 0x1E, 0x32, 0x06,                     // MOV [0x0632], BX
        0x89, 0x0E, 0x34, 0x06,                     // MOV [0x0634], CX
        0x89, 0x16, 0x36, 0x06,                     // MOV [0x0636], DX
        0x89, 0x26, 0x38, 0x06,                     // MOV [0x0638], SP
    ];
    let mut code = make_v86_call_code(&setup, handler);
    let halt = code.len() - 1;
    code.splice(halt..halt, store_registers);

    let accesses =
        run_under_v86_monitor_trapping(&mut machine, ALL_IRQS_MASKED, &code, &DISPLAY_PORTS);

    assert_eq!(accesses[0], out(SCAN_FREQUENCY, 0x00));
    let registers: Vec<u16> = (0..5)
        .map(|index| read_word(&mut machine.bus, 0x0630 + 2 * index))
        .collect();
    assert_eq!(registers, [0x3000, 0x0034, 0x5678, 0x9ABC, 0x5000]);
}
