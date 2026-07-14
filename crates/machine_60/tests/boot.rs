//! Boot and render tests. Each test runs a small hand-assembled Z80 program
//! from the reset vector of a synthetic ROM: it selects a video mode through
//! the I/O ports and writes a glyph cell into the active video RAM window. The
//! rendered frame is asserted to differ from one produced by a program that
//! only selects the mode, confirming the CPU's OUT and memory writes reach the
//! display.

use machine_60::{LoadedRoms, Pc6000Model};

mod harness;
use harness::{build_machine_with_synthetic_roms, run_frames};

/// Glyph fully lit in the character generator, so a referenced cell renders as a
/// solid block that differs from the blank background.
fn light_glyph(cg: &mut [u8], tile: usize) {
    for line in 0..12 {
        cg[tile * 0x10 + line] = 0xFF;
    }
}

/// Installs a boot program for `model` at its reset vector. When `draw` is set
/// the program also writes a glyph cell into the video RAM window; otherwise it
/// only selects the video mode, giving a content-free baseline.
fn install_boot_program(roms: &mut LoadedRoms, model: Pc6000Model, draw: bool) {
    match model {
        Pc6000Model::Pc6001 => {
            let mut program = vec![
                0xF3, // DI
                0x3E, 0x00, // LD A, 0x00
                0xD3, 0xB0, // OUT (0xB0), A     ; select 0xC000 base, text mode
            ];
            if draw {
                program.extend_from_slice(&[
                    0x3E, 0x41, // LD A, 0x41
                    0x32, 0x00, 0xC2, // LD (0xC200), A   ; tile map cell 0 -> glyph 0x41
                ]);
            }
            program.extend_from_slice(&[0x18, 0xFE]); // JR $
            let basic = roms.basic.as_mut().unwrap();
            basic[..program.len()].copy_from_slice(&program);
            light_glyph(roms.cg_base.as_mut().unwrap(), 0x41);
        }
        Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6601 => {
            let mut program = vec![
                0xF3, // DI
                0x3E, 0x00, // LD A, 0x00
                0xD3, 0xB0, // OUT (0xB0), A     ; latch the 0x8000 legacy text window
            ];
            if draw {
                program.extend_from_slice(&[
                    0x3E, 0x41, // LD A, 0x41
                    0x32, 0x00, 0x82, // LD (0x8200), A   ; tile map cell 0 -> glyph 0x41
                ]);
            }
            program.extend_from_slice(&[0x18, 0xFE]); // JR $
            let basic = roms.basic.as_mut().unwrap();
            basic[..program.len()].copy_from_slice(&program);
            light_glyph(roms.cg_base.as_mut().unwrap(), 0x41);
        }
        Pc6000Model::Pc6001Mk2Sr | Pc6000Model::Pc6601Sr => {
            let mut program = vec![
                0xF3, // DI
                0x3E, 0x0C, // LD A, 0x0C
                0xD3, 0xC8, // OUT (0xC8), A     ; native SR text, 20-row geometry
            ];
            if draw {
                program.extend_from_slice(&[
                    0x3E, 0x01, // LD A, 0x01
                    0x32, 0x00, 0xE0, // LD (0xE000), A   ; text cell 0 -> tile 1
                    0x3E, 0x0F, // LD A, 0x0F
                    0x32, 0x01, 0xE0, // LD (0xE001), A   ; foreground pen 0x0F
                ]);
            }
            program.extend_from_slice(&[0x18, 0xFE]); // JR $
            // The SR reset maps system ROM half 1 at 0x8000 into the 0x0000 window.
            let system_rom1 = roms.system_rom1.as_mut().unwrap();
            system_rom1[0x8000..0x8000 + program.len()].copy_from_slice(&program);
            roms.cg_sr.as_mut().unwrap()[0x10] = 0x80;
        }
    }
}

/// Boots `model` from a synthetic ROM running the boot program, then returns the
/// rendered frame.
fn render_boot(model: Pc6000Model, draw: bool) -> Vec<u8> {
    let mut machine =
        build_machine_with_synthetic_roms(model, |roms| install_boot_program(roms, model, draw));
    run_frames(&mut machine, 4);
    machine.bus.display_framebuffer().to_vec()
}

/// Asserts that the CPU-written glyph changes the rendered frame for `model`.
fn assert_boot_program_renders(model: Pc6000Model) {
    assert_ne!(
        render_boot(model, false),
        render_boot(model, true),
        "the CPU-written glyph did not change the frame for {model:?}"
    );
}

#[test]
fn pc6001_boot_program_renders() {
    assert_boot_program_renders(Pc6000Model::Pc6001);
}

#[test]
fn pc6001mk2_boot_program_renders() {
    assert_boot_program_renders(Pc6000Model::Pc6001Mk2);
}

#[test]
fn pc6601_boot_program_renders() {
    assert_boot_program_renders(Pc6000Model::Pc6601);
}

#[test]
fn pc6001mk2sr_boot_program_renders() {
    assert_boot_program_renders(Pc6000Model::Pc6001Mk2Sr);
}

#[test]
fn pc6601sr_boot_program_renders() {
    assert_boot_program_renders(Pc6000Model::Pc6601Sr);
}
