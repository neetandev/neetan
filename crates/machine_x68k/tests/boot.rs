//! Full-machine boot tests: the real CPU executes a scripted IPL, renders
//! text output, honors the reset vectors, and takes hardware-probe bus
//! errors on every model.

#[path = "common/harness.rs"]
mod harness;

use common::{Bus, CpuM68000, M68000AccessSize};
use harness::{
    STOP_MASKED, byte_write_script, pixel, read_byte, scripted_machine, word_write_script,
    write_byte,
};
use machine_x68k::X68kModel;

/// White pixel produced by a full-intensity text palette entry.
const WHITE: [u8; 4] = [255, 255, 255, 255];
/// Black pixel of an unlit display position.
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// Scripted IPL: program a 16x1 text display and light the leftmost pixel.
fn text_boot_program() -> Vec<u16> {
    let mut program = word_write_script(&[
        // CRTC R00-R09: an 11-column, 4-raster frame with a 16x1 display.
        (0xE80000, 10),
        (0xE80002, 0),
        (0xE80004, 0),
        (0xE80006, 2),
        (0xE80008, 3),
        (0xE8000A, 0),
        (0xE8000C, 0),
        (0xE8000E, 1),
        (0xE80010, 0),
        (0xE80012, 3),
        // Text palette entry 1: white.
        (0xE82202, 0xFFFF),
        // Video controller R2: enable the text layer.
        (0xE82600, 0x0020),
    ]);
    program.extend(byte_write_script(&[
        // Full contrast.
        (0xE8E001, 15),
        // Text VRAM plane 0: the leftmost pixel of the first row.
        (0xE00000, 0x80),
    ]));
    program.extend(STOP_MASKED);
    program
}

#[test]
fn every_model_boots_and_renders_text_through_the_cpu() {
    for model in [
        X68kModel::X68000,
        X68kModel::X68000Super,
        X68kModel::X68000Xvi,
    ] {
        let mut machine = scripted_machine(model, &text_boot_program());
        machine.run_for(50_000);
        assert_eq!(
            machine.bus.display_dimensions(),
            (16, 1),
            "{model}: the programmed geometry must publish"
        );
        assert_eq!(
            pixel(&machine, 0, 0),
            WHITE,
            "{model}: the lit text pixel must render"
        );
        assert_eq!(
            pixel(&machine, 8, 0),
            BLACK,
            "{model}: unlit positions stay black"
        );
    }
}

#[test]
fn reset_vectors_come_from_the_ipl_and_address_zero_is_ram() {
    let mut machine = scripted_machine(X68kModel::X68000, &STOP_MASKED);
    machine.run_for(100);
    // The reset vectors were fetched from the last IPL half: the CPU ran the
    // scripted STOP right behind the reset entry point.
    assert!((0x00FE_0008..=0x00FE_0010).contains(&machine.cpu.pc()));

    // Address zero reads and writes ordinary RAM immediately after reset:
    // no ROM overlay remains.
    assert_eq!(machine.bus.ram_byte(0), Some(0));
    write_byte(&mut machine, 0x000000, 0x12);
    assert_eq!(read_byte(&mut machine, 0x000000), 0x12);
    assert_eq!(machine.bus.ram_byte(0), Some(0x12));
}

#[test]
fn shutdown_register_sequence_requests_shutdown() {
    let mut program = byte_write_script(&[(0xE8E00F, 0x00), (0xE8E00F, 0x0F), (0xE8E00F, 0x0F)]);
    program.extend(STOP_MASKED);
    let mut machine = scripted_machine(X68kModel::X68000, &program);
    assert!(!machine.bus.shutdown_requested());
    machine.run_for(2_000);
    assert!(machine.bus.shutdown_requested());
}

#[test]
fn absent_hardware_probe_takes_the_bus_error_exception() {
    // Install the bus-error handler, probe an unmapped I/O address, and mark
    // RAM from the handler.
    // The handler starts after the ten words of the main program.
    let handler_address = 0x00FE_0008u32 + 10 * 2;
    let program = [
        // move.l #handler, (0x0008).l -- the bus-error vector.
        0x23FC,
        (handler_address >> 16) as u16,
        handler_address as u16,
        0x0000,
        0x0008,
        // tst.b (0xED4000).l -- probing unmapped I/O space faults.
        0x4A39,
        0x00ED,
        0x4000,
        // Fallthrough if no fault arrived: stop without the marker.
        0x4E72,
        0x2700,
        // handler: move.b #0x5A, (0x2000).l then halt.
        0x13FC,
        0x005A,
        0x0000,
        0x2000,
        0x4E72,
        0x2700,
    ];
    let mut machine = scripted_machine(X68kModel::X68000, &program);
    machine.run_for(5_000);
    assert_eq!(
        machine.bus.ram_byte(0x2000),
        Some(0x5A),
        "the bus-error handler must have run"
    );
}

#[test]
fn supervisor_area_register_protects_low_ram_from_user_mode() {
    let mut machine = scripted_machine(X68kModel::X68000, &STOP_MASKED);
    machine.run_for(100);
    let user_read = |machine: &mut machine_x68k::X68kMachine, address: u32| {
        machine.bus.m68000_read(common::M68000BusAccess {
            address,
            size: M68000AccessSize::Byte,
            function_code: common::M68000FunctionCode::UserData,
            cycle_kind: common::M68000CycleKind::Normal,
        })
    };
    // The reset value 0 already protects the first 8 KiB.
    assert!(user_read(&mut machine, 0x1000).is_err());
    assert!(user_read(&mut machine, 0x3000).is_ok());
    // Raising the register widens the protected window.
    write_byte(&mut machine, 0xE86001, 3);
    assert!(user_read(&mut machine, 0x3000).is_err());
    assert!(user_read(&mut machine, 0x8000).is_ok());
    // The register itself is write-only even for the supervisor.
    assert!(
        machine
            .bus
            .m68000_read(common::M68000BusAccess {
                address: 0xE86001,
                size: M68000AccessSize::Byte,
                function_code: common::M68000FunctionCode::SupervisorData,
                cycle_kind: common::M68000CycleKind::Normal,
            })
            .is_err()
    );
}
