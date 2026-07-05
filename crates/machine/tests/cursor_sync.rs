//! HLE DOS cursor reconciliation against the master GDC hardware cursor.

use common::{BUILTIN_FONT_ROM, Bus, CpuMode, MachineModel};

const INJECT_CODE_SEGMENT: u16 = 0x2000;
const INJECT_CODE_BASE: u32 = (INJECT_CODE_SEGMENT as u32) << 4;
const IOSYS_BASE: u32 = 0x0600;
const IOSYS_OFF_CURSOR_Y: u32 = 0x0110;
const IOSYS_OFF_CURSOR_X: u32 = 0x011C;
const TEXT_VRAM_COLUMNS: usize = 80;
const TEXT_VRAM_ROWS: usize = 25;
const BOOT_MAX_CYCLES: u64 = 500_000_000;

fn create_hle_machine() -> machine::Pc9801Ra {
    let mut machine = machine::Pc9801Ra::new(
        cpu::I386::new(),
        machine::Pc9801Bus::new(MachineModel::PC9801RA, CpuMode::High, 48000),
    );
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine.bus.set_xms_32_enabled(true);
    machine
}

fn prompt_visible(machine: &machine::Pc9801Ra) -> bool {
    machine
        .bus
        .text_vram()
        .chunks_exact(2)
        .take(TEXT_VRAM_COLUMNS * TEXT_VRAM_ROWS)
        .any(|cell| u16::from_le_bytes([cell[0], cell[1]]) == 0x003E)
}

fn boot_hle() -> machine::Pc9801Ra {
    let mut machine = create_hle_machine();
    let mut total_cycles = 0u64;
    loop {
        total_cycles += machine.run_for(1_000_000);
        if prompt_visible(&machine) {
            break;
        }
        assert!(
            total_cycles < BOOT_MAX_CYCLES,
            "HLE DOS did not show prompt within {BOOT_MAX_CYCLES} cycles"
        );
    }
    machine
}

fn inject_and_run(machine: &mut machine::Pc9801Ra, code: &[u8]) {
    for (index, &byte) in code.iter().enumerate() {
        machine
            .bus
            .write_byte(INJECT_CODE_BASE + index as u32, byte);
    }
    let mut state = cpu::I386State {
        ip: 0x0000,
        ..Default::default()
    };
    state.set_cs(INJECT_CODE_SEGMENT);
    state.set_ss(INJECT_CODE_SEGMENT);
    state.set_ds(INJECT_CODE_SEGMENT);
    state.set_es(INJECT_CODE_SEGMENT);
    state.set_esp(0xFFFE);
    state.set_eflags(state.eflags() | 0x0200);
    machine.cpu.load_state(&state);
    machine.run_for(50_000_000);
}

#[test]
fn direct_iosys_cursor_write_survives_dos_dispatch() {
    let mut machine = boot_hle();

    // Move only the IOSYS cursor to a blank cell (row 20, col 40), leaving the
    // GDC hardware cursor at the prompt (stale). The dispatch reconciliation must
    // let this IOSYS write win.
    machine.bus.write_byte(IOSYS_BASE + IOSYS_OFF_CURSOR_Y, 20);
    machine.bus.write_byte(IOSYS_BASE + IOSYS_OFF_CURSOR_X, 40);

    // INT 21h AH=02h prints 'A' at the DOS console cursor (reads 0060:011C).
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB4, 0x02,     // MOV AH, 02h
        0xB2, 0x41,     // MOV DL, 'A'
        0xCD, 0x21,     // INT 21h
        0xFA, 0xF4,     // CLI; HLT
    ];
    inject_and_run(&mut machine, code);

    let cell_index = 20 * TEXT_VRAM_COLUMNS + 40;
    let vram = machine.bus.text_vram();
    let code_at_cell = u16::from_le_bytes([vram[cell_index * 2], vram[cell_index * 2 + 1]]);
    assert_eq!(
        code_at_cell, 0x0041,
        "'A' must be drawn at the IOSYS cursor (row 20, col 40), got {code_at_cell:#06X}"
    );

    let state = machine.save_state();
    assert_eq!(
        state.gdc_master.ead,
        (20 * TEXT_VRAM_COLUMNS + 41) as u32,
        "GDC cursor must advance to col 41 after the output",
    );
}
