//! Super Graphic Processor (SGP) blitter integration tests.
//!
//! Each test enables single-plane (GMSP) mode, builds a command list and pixel
//! data in main RAM (which the SGP addresses at `0x000000`), triggers the
//! engine through its I/O ports, advances the scheduler until the completion
//! event fires, and verifies the destination memory. BITBLT, PATBLT, CLS and
//! LINE are checked against hand-computed results; SCAN is a best-effort,
//! hardware-unverified command and is checked against its documented behavior.

use common::Bus;
use machine88va::Pc88VaMachine;

#[path = "common/harness.rs"]
mod harness;
use harness::*;

const CMD_ADDR: u32 = 0x1000;

/// Selects single-plane (GMSP) mode with the given system-memory bank.
fn select_sysm(machine: &mut Pc88VaMachine, bank: u8) {
    machine.bus.io_write_byte(0x153, 0x10 | (bank & 0x0F));
}

fn put_words(machine: &mut Pc88VaMachine, address: u32, words: &[u16]) {
    for (index, &word) in words.iter().enumerate() {
        let target = address + (index as u32) * 2;
        machine.bus.write_byte(target, (word & 0xFF) as u8);
        machine.bus.write_byte(target + 1, (word >> 8) as u8);
    }
}

fn put_bytes(machine: &mut Pc88VaMachine, address: u32, bytes: &[u8]) {
    for (index, &byte) in bytes.iter().enumerate() {
        machine.bus.write_byte(address + index as u32, byte);
    }
}

fn read_bytes(machine: &mut Pc88VaMachine, address: u32, count: u32) -> Vec<u8> {
    (0..count)
        .map(|i| machine.bus.read_byte(address + i))
        .collect()
}

/// A 6-word SGP block descriptor.
fn block(scrnmode: u16, dot: u16, width: u16, height: u16, fbw: u16, address: u32) -> [u16; 6] {
    [
        scrnmode | (dot << 4),
        width,
        height,
        fbw,
        (address & 0xFFFF) as u16,
        (address >> 16) as u16,
    ]
}

fn set_initialpc(machine: &mut Pc88VaMachine, address: u32) {
    for byte in 0..4 {
        machine
            .bus
            .io_write_byte(0x500 + byte, (address >> (byte * 8)) as u8);
    }
}

fn trigger(machine: &mut Pc88VaMachine) {
    machine.bus.io_write_byte(0x506, 0x01);
}

fn busy(machine: &mut Pc88VaMachine) -> bool {
    machine.bus.io_read_byte(0x506) & 0x01 != 0
}

/// Advances the scheduler one event at a time until the SGP is idle.
fn run_to_idle(machine: &mut Pc88VaMachine) {
    for _ in 0..4000 {
        if !busy(machine) {
            return;
        }
        let next = machine
            .bus
            .next_event_cycle()
            .expect("an event is always scheduled");
        machine.bus.set_current_cycle(next);
    }
    panic!("SGP did not complete");
}

/// Loads a command list at `CMD_ADDR`, triggers a run, and waits for completion.
fn run_program(machine: &mut Pc88VaMachine, program: &[u16]) {
    put_words(machine, CMD_ADDR, program);
    set_initialpc(machine, CMD_ADDR);
    trigger(machine);
    assert!(busy(machine), "SGP reports busy right after the trigger");
    run_to_idle(machine);
}

fn bitblt_program(src: &[u16], dst: &[u16], bltmode: u16) -> Vec<u16> {
    let mut program = vec![0x0004];
    program.extend_from_slice(src);
    program.push(0x0005);
    program.extend_from_slice(dst);
    program.extend_from_slice(&[0x0007, bltmode, 0x0001]);
    program
}

#[test]
fn bitblt_copies_an_8bpp_rectangle() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let source = 0x2000;
    let destination = 0x3000;
    let pixels = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    put_bytes(&mut machine, source, &pixels);

    let program = bitblt_program(
        &block(2, 0, 4, 2, 4, source),
        &block(2, 0, 4, 2, 4, destination),
        0x0005,
    );
    run_program(&mut machine, &program);

    assert_eq!(read_bytes(&mut machine, destination, 8), pixels);
}

#[test]
fn bitblt_applies_the_xor_raster_op() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let source = 0x2000;
    let destination = 0x3000;
    put_bytes(&mut machine, source, &[0xFF, 0xFF]);
    put_bytes(&mut machine, destination, &[0x0F, 0xF0]);

    let program = bitblt_program(
        &block(2, 0, 2, 1, 2, source),
        &block(2, 0, 2, 1, 2, destination),
        0x0006,
    );
    run_program(&mut machine, &program);

    assert_eq!(read_bytes(&mut machine, destination, 2), vec![0xF0, 0x0F]);
}

#[test]
fn bitblt_source_transparency_skips_zero_pixels() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let source = 0x2000;
    let destination = 0x3000;
    put_bytes(&mut machine, source, &[0x00, 0x42]);
    put_bytes(&mut machine, destination, &[0x99, 0x88]);

    let program = bitblt_program(
        &block(2, 0, 2, 1, 2, source),
        &block(2, 0, 2, 1, 2, destination),
        0x0105,
    );
    run_program(&mut machine, &program);

    // Pixel 0 is transparent (source 0), so the destination keeps 0x99.
    assert_eq!(read_bytes(&mut machine, destination, 2), vec![0x99, 0x42]);
}

#[test]
fn patblt_wraps_a_small_pattern_over_the_destination() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let source = 0x2000;
    let destination = 0x3000;
    put_bytes(&mut machine, source, &[0xAB, 0xCD]);

    let mut program = vec![0x0004];
    program.extend_from_slice(&block(2, 0, 2, 1, 2, source));
    program.push(0x0005);
    program.extend_from_slice(&block(2, 0, 4, 2, 4, destination));
    program.extend_from_slice(&[0x0008, 0x0005, 0x0001]);
    run_program(&mut machine, &program);

    let expected = vec![0xAB, 0xCD, 0xAB, 0xCD, 0xAB, 0xCD, 0xAB, 0xCD];
    assert_eq!(read_bytes(&mut machine, destination, 8), expected);
}

#[test]
fn cls_fills_words_with_the_set_color() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let destination = 0x3000;
    let program = vec![
        0x0006,
        0x1234,
        0x000A,
        (destination & 0xFFFF) as u16,
        (destination >> 16) as u16,
        2,
        0,
        0x0001,
    ];
    run_program(&mut machine, &program);

    assert_eq!(
        read_bytes(&mut machine, destination, 4),
        vec![0x34, 0x12, 0x34, 0x12]
    );
}

#[test]
fn line_draws_a_horizontal_run() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let destination = 0x3000;
    let mut program = vec![0x0006, 0xAAAA, 0x0009, 0x0005];
    program.extend_from_slice(&block(2, 0, 4, 1, 4, destination));
    program.push(0x0001);
    run_program(&mut machine, &program);

    assert_eq!(
        read_bytes(&mut machine, destination, 4),
        vec![0xAA, 0xAA, 0xAA, 0xAA]
    );
}

#[test]
fn line_draws_a_vertical_run() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let destination = 0x3000;
    let mut program = vec![0x0006, 0xAAAA, 0x0009, 0x0005];
    program.extend_from_slice(&block(2, 0, 1, 4, 4, destination));
    program.push(0x0001);
    run_program(&mut machine, &program);

    let pixels = read_bytes(&mut machine, destination, 16);
    assert_eq!(pixels[0], 0xAA);
    assert_eq!(pixels[4], 0xAA);
    assert_eq!(pixels[8], 0xAA);
    assert_eq!(pixels[12], 0xAA);
}

#[test]
fn bitblt_writes_into_graphics_vram() {
    let mut machine = machine();
    // GMSP on with the graphics bank selected so the CPU window reads GVRAM.
    select_sysm(&mut machine, 4);

    let source = 0x2000;
    let pixels = [0x11, 0x22, 0x33, 0x44];
    put_bytes(&mut machine, source, &pixels);

    let program = bitblt_program(
        &block(2, 0, 4, 1, 4, source),
        &block(2, 0, 4, 1, 4, 0x20_0000),
        0x0005,
    );
    run_program(&mut machine, &program);

    // The SGP wrote graphics VRAM offset 0; read it back through the CPU window.
    assert_eq!(read_bytes(&mut machine, 0xA_0000, 4), pixels);
}

#[test]
fn scan_right_finds_the_color_and_sets_the_width() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let destination = 0x3000;
    let pattern = 0x2000;
    // Destination row: the target color 0x42 sits at pixel index 3.
    put_bytes(
        &mut machine,
        destination,
        &[0x00, 0x00, 0x00, 0x42, 0x00, 0x00, 0x00, 0x00],
    );
    put_bytes(&mut machine, pattern, &[0x99]);

    // SET COLOR 0x42, SET DESTINATION (8 wide), SCAN RIGHT (sets width to 3),
    // SET SOURCE pattern, PATBLT into the (now 3-wide) destination.
    let mut program = vec![0x0006, 0x0042, 0x0005];
    program.extend_from_slice(&block(2, 0, 8, 1, 16, destination));
    program.push(0x000B);
    program.push(0x0004);
    program.extend_from_slice(&block(2, 0, 1, 1, 2, pattern));
    program.extend_from_slice(&[0x0008, 0x0005, 0x0001]);
    run_program(&mut machine, &program);

    // SCAN found the target at index 3, so PATBLT filled pixels 0..2 only.
    let pixels = read_bytes(&mut machine, destination, 8);
    assert_eq!(&pixels[0..4], &[0x99, 0x99, 0x99, 0x42]);
}

#[test]
fn scan_left_finds_the_color_and_fills_the_interior_span() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let row = 0x3000;
    let pattern = 0x2000;
    // The boundary color 0x42 sits at byte index 2; the start pixel is byte index 6.
    // Scanning left from index 6 finds 0x42 at index 2 after 4 interior pixels
    // (indices 3..6), so the fill span is indices 3..6.
    put_bytes(
        &mut machine,
        row,
        &[0x00, 0x00, 0x42, 0x00, 0x00, 0x00, 0x00, 0x00],
    );
    put_bytes(&mut machine, pattern, &[0x99]);

    // SET COLOR 0x42, SET DESTINATION starting at byte index 6 (8 wide), SCAN LEFT
    // (width -> 4, start repositioned to index 3), SET SOURCE pattern, PATBLT.
    let mut program = vec![0x0006, 0x0042, 0x0005];
    program.extend_from_slice(&block(2, 0, 8, 1, 16, row + 6));
    program.push(0x000C);
    program.push(0x0004);
    program.extend_from_slice(&block(2, 0, 1, 1, 2, pattern));
    program.extend_from_slice(&[0x0008, 0x0005, 0x0001]);
    run_program(&mut machine, &program);

    // PATBLT filled the interior span (indices 3..6); the boundary at index 2 and
    // the pixels outside the span are untouched.
    let pixels = read_bytes(&mut machine, row, 8);
    assert_eq!(
        &pixels[0..8],
        &[0x00, 0x00, 0x42, 0x99, 0x99, 0x99, 0x99, 0x00]
    );
}

#[test]
fn busy_stays_set_until_the_scheduled_completion() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let destination = 0x3000;
    let program = vec![
        0x0006,
        0x1234,
        0x000A,
        (destination & 0xFFFF) as u16,
        (destination >> 16) as u16,
        100,
        0,
        0x0001,
    ];
    put_words(&mut machine, CMD_ADDR, &program);
    set_initialpc(&mut machine, CMD_ADDR);
    trigger(&mut machine);

    assert!(busy(&mut machine), "busy right after the trigger");
    // The completion is scheduled well into the future for a 100-word clear.
    machine.bus.set_current_cycle(1);
    assert!(busy(&mut machine), "still busy before completion");
    machine.bus.set_current_cycle(10_000_000);
    assert!(!busy(&mut machine), "idle after completion");
}

#[test]
fn completion_raises_the_sgp_interrupt_when_enabled() {
    let mut machine = machine();
    program_pic_cascade(&mut machine);
    select_sysm(&mut machine, 0);

    // Enable the SGP completion interrupt.
    machine.bus.io_write_byte(0x504, 0x04);

    let destination = 0x3000;
    let program = vec![
        0x0006,
        0x1234,
        0x000A,
        (destination & 0xFFFF) as u16,
        (destination >> 16) as u16,
        4,
        0,
        0x0001,
    ];
    run_program(&mut machine, &program);

    assert!(machine.bus.has_irq(), "SGP raised its interrupt");
    assert_eq!(machine.bus.acknowledge_irq(), 0x70, "slave IRQ8 vector");
}

#[test]
fn completion_raises_no_interrupt_when_disabled() {
    let mut machine = machine();
    program_pic_cascade(&mut machine);
    select_sysm(&mut machine, 0);

    let destination = 0x3000;
    let program = vec![
        0x0006,
        0x1234,
        0x000A,
        (destination & 0xFFFF) as u16,
        (destination >> 16) as u16,
        4,
        0,
        0x0001,
    ];
    run_program(&mut machine, &program);

    assert!(!machine.bus.has_irq(), "no SGP interrupt without INTF");
}

#[test]
fn abort_clears_busy() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let destination = 0x3000;
    let program = vec![
        0x0006,
        0x1234,
        0x000A,
        (destination & 0xFFFF) as u16,
        (destination >> 16) as u16,
        100,
        0,
        0x0001,
    ];
    put_words(&mut machine, CMD_ADDR, &program);
    set_initialpc(&mut machine, CMD_ADDR);
    trigger(&mut machine);
    assert!(busy(&mut machine));

    // Abort request: busy clears and the completion event is cancelled.
    machine.bus.io_write_byte(0x504, 0x02);
    assert!(!busy(&mut machine), "abort cleared busy");
}

#[test]
fn setting_gmsp_resets_a_busy_sgp() {
    let mut machine = machine();
    select_sysm(&mut machine, 0);

    let destination = 0x3000;
    let program = vec![
        0x0006,
        0x1234,
        0x000A,
        (destination & 0xFFFF) as u16,
        (destination >> 16) as u16,
        100,
        0,
        0x0001,
    ];
    put_words(&mut machine, CMD_ADDR, &program);
    set_initialpc(&mut machine, CMD_ADDR);
    trigger(&mut machine);
    assert!(busy(&mut machine));

    // Clear GMSP, then set it again: setting GMSP resets the SGP.
    machine.bus.io_write_byte(0x153, 0x00);
    machine.bus.io_write_byte(0x153, 0x10);
    assert!(!busy(&mut machine), "setting GMSP reset the busy SGP");
}

/// Programs the master/slave 8259 cascade so a slave IRQ (IRQ8) surfaces, with
/// the cascade line (IR2) and IRQ8 unmasked.
fn program_pic_cascade(machine: &mut Pc88VaMachine) {
    // Master init: ICW1, ICW2 (base 0x08), ICW3 (slave on IR2), ICW4.
    machine.bus.io_write_byte(0x188, 0x11);
    machine.bus.io_write_byte(0x18A, 0x08);
    machine.bus.io_write_byte(0x18A, 0x04);
    machine.bus.io_write_byte(0x18A, 0x01);
    // Slave init: ICW1, ICW2 (base 0x70), ICW3 (cascade id 2), ICW4.
    machine.bus.io_write_byte(0x184, 0x11);
    machine.bus.io_write_byte(0x186, 0x70);
    machine.bus.io_write_byte(0x186, 0x02);
    machine.bus.io_write_byte(0x186, 0x01);
    // Unmask the cascade line on the master and IRQ8 on the slave.
    machine.bus.io_write_byte(0x18A, 0xFB);
    machine.bus.io_write_byte(0x186, 0xFE);
}
