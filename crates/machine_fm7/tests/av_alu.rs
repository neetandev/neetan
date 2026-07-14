//! FM-77AV MB61VH010 ALU: per-plane logical operations, write mask, bank
//! disable, multipage access mask, compare unit, tile paint, and the hardware
//! line generator with its busy lifecycle.

mod harness;

use common::Bus;
use harness::{build_av_bus_with_synthetic_roms, run_bus_cycles};
use machine_fm7::{BootMode, Fm7Bus, SubBusView};

/// Byte offset of the blue plane within a VRAM page.
const PLANE_BLUE: u16 = 0x0000;
/// Byte offset of the red plane within a VRAM page.
const PLANE_RED: u16 = 0x4000;
/// Byte offset of the green plane within a VRAM page.
const PLANE_GREEN: u16 = 0x8000;

/// `0xD410` command register.
const REG_COMMAND: u8 = 0x10;
/// `0xD411` logical colour register.
const REG_COLOR: u8 = 0x11;
/// `0xD412` write mask register.
const REG_MASK: u8 = 0x12;
/// `0xD413` compare status (read) / first compare colour bank (write).
const REG_COMPARE: u8 = 0x13;
/// `0xD41B` bank-disable register.
const REG_BANK_DISABLE: u8 = 0x1B;
/// `0xD41C` tile-paint pattern for the blue plane.
const REG_TILE_BLUE: u8 = 0x1C;
/// `0xD41D` tile-paint pattern for the red plane.
const REG_TILE_RED: u8 = 0x1D;
/// `0xD41E` tile-paint pattern for the green plane.
const REG_TILE_GREEN: u8 = 0x1E;
/// `0xD422` line pattern, high byte.
const REG_PATTERN_HIGH: u8 = 0x22;
/// `0xD423` line pattern, low byte.
const REG_PATTERN_LOW: u8 = 0x23;
/// `0xD424` line start X, high bits.
const REG_X_BEGIN_HIGH: u8 = 0x24;
/// `0xD425` line start X, low byte.
const REG_X_BEGIN_LOW: u8 = 0x25;
/// `0xD426` line start Y, high bit.
const REG_Y_BEGIN_HIGH: u8 = 0x26;
/// `0xD427` line start Y, low byte.
const REG_Y_BEGIN_LOW: u8 = 0x27;
/// `0xD428` line end X, high bits.
const REG_X_END_HIGH: u8 = 0x28;
/// `0xD429` line end X, low byte.
const REG_X_END_LOW: u8 = 0x29;
/// `0xD42A` line end Y, high bit.
const REG_Y_END_HIGH: u8 = 0x2A;
/// `0xD42B` line end Y, low byte; writing it triggers the line draw.
const REG_Y_END_LOW: u8 = 0x2B;
/// `0xD430` sub misc register (ALU busy readback in bit 4).
const REG_SUB_MISC: u16 = 0xD430;

/// Command bit 7: enable the ALU.
const ENABLE: u8 = 0x80;
/// Command bit 6: read-modify-write through the compare unit.
const CALC: u8 = 0x40;
/// Command bit 5: protect the compare-matched pixels instead of the rest.
const MASK_SELECT: u8 = 0x20;
/// Operation selector: set each plane to the logical colour.
const OP_PSET: u8 = 0;
/// Operation selector: blank the unmasked pixels.
const OP_BLANK: u8 = 1;
/// Operation selector: OR the logical colour in.
const OP_OR: u8 = 2;
/// Operation selector: AND the logical colour in.
const OP_AND: u8 = 3;
/// Operation selector: XOR the logical colour in.
const OP_XOR: u8 = 4;
/// Operation selector: invert the planes.
const OP_NOT: u8 = 5;
/// Operation selector: stamp the tile pattern.
const OP_TILEPAINT: u8 = 6;
/// Operation selector: compare only.
const OP_COMPARE: u8 = 7;
/// `0xD430` read bit 4: the ALU is idle (line draw complete).
const ALU_IDLE_BIT: u8 = 0x10;

/// Builds an FM-77AV bus with synthetic ROMs.
fn build_av_bus() -> Fm7Bus {
    build_av_bus_with_synthetic_roms(BootMode::Basic, |_| {})
}

/// Writes an ALU register by its low-byte port number.
fn alu_write(bus: &mut Fm7Bus, register: u8, value: u8) {
    let mut view = SubBusView { bus };
    view.write_byte(0xD400 | u32::from(register), value);
}

/// Reads a byte from the sub address space through a sub bus view.
fn sub_read(bus: &mut Fm7Bus, address: u16) -> u8 {
    let mut view = SubBusView { bus };
    view.read_byte(u32::from(address))
}

/// Triggers an ALU operation by writing the VRAM byte at `offset`; the data
/// value is ignored because the operation is register-driven.
fn trigger(bus: &mut Fm7Bus, offset: u16) {
    let mut view = SubBusView { bus };
    view.write_byte(u32::from(offset), 0);
}

/// Seeds the same in-plane byte of all three planes.
fn seed_planes(bus: &mut Fm7Bus, offset: u16, blue: u8, red: u8, green: u8) {
    bus.sub_poke_byte(PLANE_BLUE + offset, blue);
    bus.sub_poke_byte(PLANE_RED + offset, red);
    bus.sub_poke_byte(PLANE_GREEN + offset, green);
}

/// The stored bytes of all three planes at `offset`.
fn read_planes(bus: &Fm7Bus, offset: u16) -> (u8, u8, u8) {
    (
        bus.sub_peek_byte(PLANE_BLUE + offset),
        bus.sub_peek_byte(PLANE_RED + offset),
        bus.sub_peek_byte(PLANE_GREEN + offset),
    )
}

/// Programs the ALU for a single-byte operation and triggers it at `offset`.
fn run_operation(bus: &mut Fm7Bus, command: u8, color: u8, mask: u8, offset: u16) {
    alu_write(bus, REG_COLOR, color);
    alu_write(bus, REG_MASK, mask);
    alu_write(bus, REG_COMMAND, ENABLE | command);
    trigger(bus, offset);
}

#[test]
fn pset_writes_the_solid_logical_color() {
    let mut bus = build_av_bus();
    run_operation(&mut bus, OP_PSET, 0x07, 0x00, 0);
    assert_eq!(read_planes(&bus, 0), (0xFF, 0xFF, 0xFF));

    let mut bus = build_av_bus();
    run_operation(&mut bus, OP_PSET, 0x02, 0x00, 0); // red only
    assert_eq!(read_planes(&bus, 0), (0x00, 0xFF, 0x00));
}

#[test]
fn write_mask_preserves_masked_pixels() {
    let mut bus = build_av_bus();
    // Blue set solid, but the high nibble is masked so it keeps the seeded zero.
    run_operation(&mut bus, OP_PSET, 0x01, 0xF0, 0);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE), 0x0F);
}

#[test]
fn bank_disable_leaves_the_plane_untouched() {
    let mut bus = build_av_bus();
    alu_write(&mut bus, REG_BANK_DISABLE, 0x02); // disable the red plane
    run_operation(&mut bus, OP_PSET, 0x07, 0x00, 0);
    assert_eq!(read_planes(&bus, 0), (0xFF, 0x00, 0xFF));
}

#[test]
fn multipage_access_mask_blocks_a_plane() {
    let mut bus = build_av_bus();
    bus.write_byte(0xFD37, 0x04); // block the green plane from the CPU/ALU
    run_operation(&mut bus, OP_PSET, 0x07, 0x00, 0);
    assert_eq!(read_planes(&bus, 0), (0xFF, 0xFF, 0x00));
}

#[test]
fn logical_operations_combine_with_the_source() {
    // OR sets planes whose colour bit is set, and leaves the others as-is.
    let mut bus = build_av_bus();
    seed_planes(&mut bus, 0, 0x0F, 0x0F, 0x0F);
    run_operation(&mut bus, OP_OR, 0x01, 0x00, 0);
    assert_eq!(read_planes(&bus, 0), (0xFF, 0x0F, 0x0F));

    // AND keeps planes whose colour bit is set, and clears the others.
    let mut bus = build_av_bus();
    seed_planes(&mut bus, 0, 0x0F, 0x0F, 0x0F);
    run_operation(&mut bus, OP_AND, 0x01, 0x00, 0);
    assert_eq!(read_planes(&bus, 0), (0x0F, 0x00, 0x00));

    // XOR inverts the planes whose colour bit is set.
    let mut bus = build_av_bus();
    seed_planes(&mut bus, 0, 0x0F, 0x0F, 0x0F);
    run_operation(&mut bus, OP_XOR, 0x01, 0x00, 0);
    assert_eq!(read_planes(&bus, 0), (0xF0, 0x0F, 0x0F));
}

#[test]
fn not_inverts_every_enabled_plane() {
    let mut bus = build_av_bus();
    seed_planes(&mut bus, 0, 0x0F, 0x33, 0x55);
    run_operation(&mut bus, OP_NOT, 0x00, 0x00, 0);
    assert_eq!(read_planes(&bus, 0), (0xF0, 0xCC, 0xAA));
}

#[test]
fn blank_clears_the_unmasked_pixels() {
    let mut bus = build_av_bus();
    seed_planes(&mut bus, 0, 0xFF, 0xFF, 0xFF);
    run_operation(&mut bus, OP_BLANK, 0x00, 0x0F, 0);
    assert_eq!(read_planes(&bus, 0), (0x0F, 0x0F, 0x0F));
}

#[test]
fn tile_paint_stamps_the_per_plane_pattern() {
    let mut bus = build_av_bus();
    alu_write(&mut bus, REG_TILE_BLUE, 0xAA);
    alu_write(&mut bus, REG_TILE_RED, 0xBB);
    alu_write(&mut bus, REG_TILE_GREEN, 0xCC);
    run_operation(&mut bus, OP_TILEPAINT, 0x00, 0x00, 0);
    assert_eq!(read_planes(&bus, 0), (0xAA, 0xBB, 0xCC));
}

#[test]
fn compare_status_marks_matching_pixels() {
    let mut bus = build_av_bus();
    // Only the most significant pixel is blue.
    seed_planes(&mut bus, 0, 0x80, 0x00, 0x00);
    alu_write(&mut bus, REG_COMPARE, 0x01); // compare bank 0 = blue, enabled
    run_operation(&mut bus, OP_COMPARE, 0x00, 0x00, 0);
    assert_eq!(sub_read(&mut bus, 0xD413), 0x80);
}

#[test]
fn compare_mode_protects_matching_pixels() {
    // With mask-select, pixels matching a compare bank survive a clearing PSET.
    let mut bus = build_av_bus();
    seed_planes(&mut bus, 0, 0xFF, 0x00, 0x00); // every pixel blue
    alu_write(&mut bus, REG_COMPARE, 0x01); // match blue
    run_operation(&mut bus, CALC | MASK_SELECT | OP_PSET, 0x00, 0x00, 0);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE), 0xFF);

    // A non-matching pixel colour is left unprotected and gets cleared.
    let mut bus = build_av_bus();
    seed_planes(&mut bus, 0, 0xFF, 0xFF, 0x00); // every pixel blue+red
    alu_write(&mut bus, REG_COMPARE, 0x01); // match blue only
    run_operation(&mut bus, CALC | MASK_SELECT | OP_PSET, 0x00, 0x00, 0);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE), 0x00);
}

/// Programs the line coordinates and pattern, then triggers the draw.
#[allow(clippy::too_many_arguments)]
fn draw_line(
    bus: &mut Fm7Bus,
    command: u8,
    color: u8,
    pattern: u16,
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
) {
    alu_write(bus, REG_COLOR, color);
    alu_write(bus, REG_MASK, 0x00);
    alu_write(bus, REG_COMMAND, ENABLE | command);
    alu_write(bus, REG_PATTERN_HIGH, (pattern >> 8) as u8);
    alu_write(bus, REG_PATTERN_LOW, pattern as u8);
    alu_write(bus, REG_X_BEGIN_HIGH, (x0 >> 8) as u8);
    alu_write(bus, REG_X_BEGIN_LOW, x0 as u8);
    alu_write(bus, REG_Y_BEGIN_HIGH, (y0 >> 8) as u8);
    alu_write(bus, REG_Y_BEGIN_LOW, y0 as u8);
    alu_write(bus, REG_X_END_HIGH, (x1 >> 8) as u8);
    alu_write(bus, REG_X_END_LOW, x1 as u8);
    alu_write(bus, REG_Y_END_HIGH, (y1 >> 8) as u8);
    alu_write(bus, REG_Y_END_LOW, y1 as u8);
}

#[test]
fn hardware_line_draws_a_horizontal_run() {
    let mut bus = build_av_bus();
    // A 16-pixel horizontal blue line at y=0 fills the first two blue bytes.
    draw_line(&mut bus, OP_PSET, 0x01, 0xFFFF, 0, 0, 15, 0);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE), 0xFF);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE + 1), 0xFF);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE + 2), 0x00);
    // The other planes stay clear.
    assert_eq!(bus.sub_peek_byte(PLANE_RED), 0x00);
}

#[test]
fn hardware_line_draws_a_vertical_run() {
    let mut bus = build_av_bus();
    // A vertical blue line at x=0 sets the top pixel of each row's byte.
    draw_line(&mut bus, OP_PSET, 0x01, 0xFFFF, 0, 0, 0, 3);
    for row in 0..4u16 {
        assert_eq!(bus.sub_peek_byte(PLANE_BLUE + row * 80), 0x80);
    }
}

#[test]
fn hardware_line_honors_the_pattern() {
    let mut bus = build_av_bus();
    // Alternating pattern draws every other pixel of the first byte.
    draw_line(&mut bus, OP_PSET, 0x01, 0xAAAA, 0, 0, 7, 0);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE), 0xAA);
}

#[test]
fn disabled_hardware_line_does_not_touch_vram() {
    let mut bus = build_av_bus();
    alu_write(&mut bus, REG_COLOR, 0x01);
    alu_write(&mut bus, REG_COMMAND, OP_PSET);
    alu_write(&mut bus, REG_PATTERN_HIGH, 0xFF);
    alu_write(&mut bus, REG_PATTERN_LOW, 0xFF);
    alu_write(&mut bus, REG_X_BEGIN_HIGH, 0);
    alu_write(&mut bus, REG_X_BEGIN_LOW, 0);
    alu_write(&mut bus, REG_Y_BEGIN_HIGH, 0);
    alu_write(&mut bus, REG_Y_BEGIN_LOW, 0);
    alu_write(&mut bus, REG_X_END_HIGH, 0);
    alu_write(&mut bus, REG_X_END_LOW, 15);
    alu_write(&mut bus, REG_Y_END_HIGH, 0);
    alu_write(&mut bus, REG_Y_END_LOW, 0);

    assert_eq!(bus.sub_peek_byte(PLANE_BLUE), 0x00);
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE + 1), 0x00);
}

#[test]
fn hardware_line_busy_flag_clears_after_a_long_line() {
    let mut bus = build_av_bus();
    // A 256-pixel line touches enough bytes to model a busy period.
    draw_line(&mut bus, OP_PSET, 0x01, 0xFFFF, 0, 0, 255, 0);
    assert_eq!(sub_read(&mut bus, REG_SUB_MISC) & ALU_IDLE_BIT, 0);

    run_bus_cycles(&mut bus, 10_000);
    assert_eq!(
        sub_read(&mut bus, REG_SUB_MISC) & ALU_IDLE_BIT,
        ALU_IDLE_BIT
    );
}

#[test]
fn hardware_line_busy_flag_stays_clear_for_a_short_line() {
    let mut bus = build_av_bus();
    // An 8-pixel line is too short to register a busy period.
    draw_line(&mut bus, OP_PSET, 0x01, 0xFFFF, 0, 0, 7, 0);
    assert_eq!(
        sub_read(&mut bus, REG_SUB_MISC) & ALU_IDLE_BIT,
        ALU_IDLE_BIT
    );
}

#[test]
fn the_fm7_never_enables_the_alu() {
    // On the FM-7 the ALU registers do not decode, so a would-be enable leaves
    // VRAM writes as plain stores.
    let mut bus = harness::build_bus_with_synthetic_roms(BootMode::Basic, |_| {});
    alu_write(&mut bus, REG_COMMAND, ENABLE | OP_PSET);
    alu_write(&mut bus, REG_COLOR, 0x07);
    {
        let mut view = SubBusView { bus: &mut bus };
        view.write_byte(u32::from(PLANE_BLUE), 0x5A);
    }
    // The plain store landed; the ALU did not paint the solid colour.
    assert_eq!(bus.sub_peek_byte(PLANE_BLUE), 0x5A);
}
