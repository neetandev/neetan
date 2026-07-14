use common::{Bus, CpuMode, MachineModel, NoTrace};
use machine_98::Pc9801Bus;

const VRAM_B_BASE: u32 = 0xA8000;
const VRAM_R_BASE: u32 = 0xB0000;
const VRAM_G_BASE: u32 = 0xB8000;
const VRAM_E_BASE: u32 = 0xE0000;

fn setup_grcg_bus() -> Pc9801Bus<NoTrace> {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    bus.io_write_byte(0x6A, 0x01); // analog mode for E-plane access
    bus
}

fn enable_grcg_tdw(bus: &mut Pc9801Bus<NoTrace>, tiles: [u8; 4]) {
    bus.io_write_byte(0x7C, 0x80); // TDW mode, all planes enabled
    for tile in tiles {
        bus.io_write_byte(0x7E, tile);
    }
}

fn enable_grcg_rmw(bus: &mut Pc9801Bus<NoTrace>, tiles: [u8; 4]) {
    bus.io_write_byte(0x7C, 0xC0); // RMW mode, all planes enabled
    for tile in tiles {
        bus.io_write_byte(0x7E, tile);
    }
}

fn disable_grcg_mode(bus: &mut Pc9801Bus<NoTrace>) {
    bus.io_write_byte(0x7C, 0x00);
}

fn read_plane_byte(bus: &Pc9801Bus<NoTrace>, plane_base: u32, offset: u32) -> u8 {
    bus.read_byte_direct(plane_base + offset)
}

fn read_all_planes_byte(bus: &Pc9801Bus<NoTrace>, offset: u32) -> [u8; 4] {
    [
        bus.read_byte_direct(VRAM_B_BASE + offset),
        bus.read_byte_direct(VRAM_R_BASE + offset),
        bus.read_byte_direct(VRAM_G_BASE + offset),
        bus.read_byte_direct(VRAM_E_BASE + offset),
    ]
}

fn prefill_all_planes_byte(bus: &mut Pc9801Bus<NoTrace>, offset: u32, values: [u8; 4]) {
    let bases = [VRAM_B_BASE, VRAM_R_BASE, VRAM_G_BASE, VRAM_E_BASE];
    for (i, &base) in bases.iter().enumerate() {
        bus.write_byte(base + offset, values[i]);
    }
}

#[test]
fn grcg_tdw_byte_write_all_planes() {
    let mut bus = setup_grcg_bus();
    enable_grcg_tdw(&mut bus, [0xAA, 0x55, 0xF0, 0x0F]);

    // Write any byte - CPU data is ignored in TDW, all planes get tile values.
    bus.write_byte(VRAM_B_BASE, 0xFF);

    disable_grcg_mode(&mut bus);

    assert_eq!(read_all_planes_byte(&bus, 0), [0xAA, 0x55, 0xF0, 0x0F]);
}

#[test]
fn grcg_tdw_word_write_all_planes() {
    let mut bus = setup_grcg_bus();
    enable_grcg_tdw(&mut bus, [0x33, 0xCC, 0x55, 0xAA]);

    bus.write_word(VRAM_B_BASE, 0xFFFF);

    disable_grcg_mode(&mut bus);

    // Both bytes should get tile value.
    let bases = [VRAM_B_BASE, VRAM_R_BASE, VRAM_G_BASE, VRAM_E_BASE];
    let tiles = [0x33, 0xCC, 0x55, 0xAA];
    for (i, &base) in bases.iter().enumerate() {
        assert_eq!(bus.read_byte_direct(base), tiles[i], "plane {i} low byte");
        assert_eq!(
            bus.read_byte_direct(base + 1),
            tiles[i],
            "plane {i} high byte"
        );
    }
}

#[test]
fn grcg_tcr_byte_read_all_match() {
    let mut bus = setup_grcg_bus();

    // Pre-fill VRAM to match tiles.
    prefill_all_planes_byte(&mut bus, 0, [0xAA, 0x55, 0xF0, 0x0F]);

    // Enable GRCG TCR mode (0x80 = TDW/TCR, reads are TCR).
    enable_grcg_tdw(&mut bus, [0xAA, 0x55, 0xF0, 0x0F]);

    let result = bus.read_byte(VRAM_B_BASE);
    assert_eq!(result, 0xFF, "all bits should match");
}

#[test]
fn grcg_tcr_byte_read_no_match() {
    let mut bus = setup_grcg_bus();

    // VRAM is opposite of tiles.
    prefill_all_planes_byte(&mut bus, 0, [0x55, 0xAA, 0x0F, 0xF0]);

    enable_grcg_tdw(&mut bus, [0xAA, 0x55, 0xF0, 0x0F]);

    let result = bus.read_byte(VRAM_B_BASE);
    assert_eq!(result, 0x00, "no bits should match");
}

#[test]
fn grcg_tcr_byte_read_partial_match() {
    let mut bus = setup_grcg_bus();

    // VRAM: B=0xAA matches tile, R=0xFF doesn't fully match tile 0x55.
    prefill_all_planes_byte(&mut bus, 0, [0xAA, 0xFF, 0xF0, 0x0F]);

    enable_grcg_tdw(&mut bus, [0xAA, 0x55, 0xF0, 0x0F]);

    let result = bus.read_byte(VRAM_B_BASE);
    // R-plane mismatch: !(0xFF ^ 0x55) = !(0xAA) = 0x55. AND with 0xFF from others = 0x55.
    assert_eq!(result, 0x55, "partial match from R-plane mismatch");
}

#[test]
fn grcg_tcr_word_read_all_match() {
    let mut bus = setup_grcg_bus();

    // Pre-fill 2 bytes per plane.
    let bases = [VRAM_B_BASE, VRAM_R_BASE, VRAM_G_BASE, VRAM_E_BASE];
    let tiles = [0xAA_u8, 0x55, 0xF0, 0x0F];
    for (i, &base) in bases.iter().enumerate() {
        bus.write_byte(base, tiles[i]);
        bus.write_byte(base + 1, tiles[i]);
    }

    enable_grcg_tdw(&mut bus, tiles);

    let result = bus.read_word(VRAM_B_BASE);
    assert_eq!(result, 0xFFFF, "both bytes fully match");
}

#[test]
fn grcg_rmw_byte_write() {
    let mut bus = setup_grcg_bus();

    // Pre-fill with 0x55.
    prefill_all_planes_byte(&mut bus, 0, [0x55; 4]);

    enable_grcg_rmw(&mut bus, [0xFF, 0x00, 0xAA, 0x55]);

    // CPU writes 0xF0: new = (0xF0 & tile) | (~0xF0 & current)
    //   B: (0xF0 & 0xFF) | (0x0F & 0x55) = 0xF0 | 0x05 = 0xF5
    //   R: (0xF0 & 0x00) | (0x0F & 0x55) = 0x00 | 0x05 = 0x05
    //   G: (0xF0 & 0xAA) | (0x0F & 0x55) = 0xA0 | 0x05 = 0xA5
    //   E: (0xF0 & 0x55) | (0x0F & 0x55) = 0x50 | 0x05 = 0x55
    bus.write_byte(VRAM_B_BASE, 0xF0);

    disable_grcg_mode(&mut bus);

    assert_eq!(read_all_planes_byte(&bus, 0), [0xF5, 0x05, 0xA5, 0x55]);
}

#[test]
fn grcg_rmw_word_write() {
    let mut bus = setup_grcg_bus();

    let bases = [VRAM_B_BASE, VRAM_R_BASE, VRAM_G_BASE, VRAM_E_BASE];
    for &base in &bases {
        bus.write_byte(base, 0xFF);
        bus.write_byte(base + 1, 0xFF);
    }

    enable_grcg_rmw(&mut bus, [0xFF, 0x00, 0xAA, 0x55]);

    // CPU writes 0xAAAA: low=0xAA, high=0xAA
    // For each byte: new = (cpu & tile) | (~cpu & current)
    //   B: (0xAA & 0xFF) | (0x55 & 0xFF) = 0xAA | 0x55 = 0xFF
    //   R: (0xAA & 0x00) | (0x55 & 0xFF) = 0x00 | 0x55 = 0x55
    //   G: (0xAA & 0xAA) | (0x55 & 0xFF) = 0xAA | 0x55 = 0xFF
    //   E: (0xAA & 0x55) | (0x55 & 0xFF) = 0x00 | 0x55 = 0x55
    bus.write_word(VRAM_B_BASE, 0xAAAA);

    disable_grcg_mode(&mut bus);

    for (i, &base) in bases.iter().enumerate() {
        let expected = [0xFF, 0x55, 0xFF, 0x55];
        assert_eq!(
            bus.read_byte_direct(base),
            expected[i],
            "plane {i} low byte"
        );
        assert_eq!(
            bus.read_byte_direct(base + 1),
            expected[i],
            "plane {i} high byte"
        );
    }
}

#[test]
fn grcg_rmw_read_returns_raw_vram() {
    let mut bus = setup_grcg_bus();

    // Pre-fill with distinctive data.
    prefill_all_planes_byte(&mut bus, 0, [0xAB, 0xCD, 0xEF, 0x12]);

    enable_grcg_rmw(&mut bus, [0xFF, 0xFF, 0xFF, 0xFF]);

    // In RMW mode, reads should bypass GRCG and return normal VRAM data.
    let result = bus.read_byte(VRAM_B_BASE);
    assert_eq!(result, 0xAB, "RMW read should return B-plane data");

    let result = bus.read_byte(VRAM_R_BASE);
    assert_eq!(result, 0xCD, "RMW read should return R-plane data");

    let result = bus.read_byte(VRAM_G_BASE);
    assert_eq!(result, 0xEF, "RMW read should return G-plane data");

    let result = bus.read_byte(VRAM_E_BASE);
    assert_eq!(result, 0x12, "RMW read should return E-plane data");
}

#[test]
fn grcg_rmw_read_word_returns_raw_vram() {
    let mut bus = setup_grcg_bus();

    let bases = [VRAM_B_BASE, VRAM_R_BASE, VRAM_G_BASE, VRAM_E_BASE];
    let values: [u8; 4] = [0x12, 0x34, 0x56, 0x78];
    for (i, &base) in bases.iter().enumerate() {
        bus.write_byte(base, values[i]);
        bus.write_byte(base + 1, values[i]);
    }

    enable_grcg_rmw(&mut bus, [0xFF; 4]);

    let result = bus.read_word(VRAM_B_BASE);
    assert_eq!(result, 0x1212, "RMW word read B-plane");

    let result = bus.read_word(VRAM_R_BASE);
    assert_eq!(result, 0x3434, "RMW word read R-plane");
}

#[test]
fn grcg_tdw_selective_plane_enable() {
    let mut bus = setup_grcg_bus();

    // Pre-fill all planes with 0xFF.
    prefill_all_planes_byte(&mut bus, 0, [0xFF; 4]);

    // TDW with planes 1 (R) and 3 (E) disabled: mode = 0x8A (bits 1,3 set).
    bus.io_write_byte(0x7C, 0x8A);
    bus.io_write_byte(0x7E, 0x00); // B tile
    bus.io_write_byte(0x7E, 0x00); // R tile (disabled)
    bus.io_write_byte(0x7E, 0x00); // G tile
    bus.io_write_byte(0x7E, 0x00); // E tile (disabled)

    bus.write_byte(VRAM_B_BASE, 0xFF);

    disable_grcg_mode(&mut bus);

    // B and G should get tile (0x00), R and E unchanged (0xFF).
    assert_eq!(read_plane_byte(&bus, VRAM_B_BASE, 0), 0x00, "B written");
    assert_eq!(read_plane_byte(&bus, VRAM_R_BASE, 0), 0xFF, "R unchanged");
    assert_eq!(read_plane_byte(&bus, VRAM_G_BASE, 0), 0x00, "G written");
    assert_eq!(read_plane_byte(&bus, VRAM_E_BASE, 0), 0xFF, "E unchanged");
}

#[test]
fn grcg_rmw_selective_plane_enable() {
    let mut bus = setup_grcg_bus();

    // Pre-fill all planes with 0xAA.
    prefill_all_planes_byte(&mut bus, 0, [0xAA; 4]);

    // RMW with planes 0 (B) and 2 (G) disabled: mode = 0xC5 (bits 0,2 set).
    bus.io_write_byte(0x7C, 0xC5);
    bus.io_write_byte(0x7E, 0xFF); // B tile (disabled)
    bus.io_write_byte(0x7E, 0xFF); // R tile
    bus.io_write_byte(0x7E, 0xFF); // G tile (disabled)
    bus.io_write_byte(0x7E, 0xFF); // E tile

    // CPU writes 0xFF: new = (0xFF & tile) | (0x00 & current) = tile
    bus.write_byte(VRAM_B_BASE, 0xFF);

    disable_grcg_mode(&mut bus);

    // B and G unchanged (0xAA), R and E get tile (0xFF).
    assert_eq!(read_plane_byte(&bus, VRAM_B_BASE, 0), 0xAA, "B unchanged");
    assert_eq!(read_plane_byte(&bus, VRAM_R_BASE, 0), 0xFF, "R written");
    assert_eq!(read_plane_byte(&bus, VRAM_G_BASE, 0), 0xAA, "G unchanged");
    assert_eq!(read_plane_byte(&bus, VRAM_E_BASE, 0), 0xFF, "E written");
}

#[test]
fn grcg_mode_switch_tdw_to_rmw() {
    let mut bus = setup_grcg_bus();

    // Start in TDW mode.
    enable_grcg_tdw(&mut bus, [0xAA, 0x55, 0xF0, 0x0F]);
    bus.write_byte(VRAM_B_BASE, 0xFF);

    // Verify TDW result.
    disable_grcg_mode(&mut bus);
    assert_eq!(read_all_planes_byte(&bus, 0), [0xAA, 0x55, 0xF0, 0x0F]);

    // Switch to RMW mode with different tiles.
    enable_grcg_rmw(&mut bus, [0xFF, 0xFF, 0xFF, 0xFF]);

    // CPU writes 0xF0: new = (0xF0 & 0xFF) | (0x0F & current)
    //   B: 0xF0 | (0x0F & 0xAA) = 0xF0 | 0x0A = 0xFA
    //   R: 0xF0 | (0x0F & 0x55) = 0xF0 | 0x05 = 0xF5
    //   G: 0xF0 | (0x0F & 0xF0) = 0xF0 | 0x00 = 0xF0
    //   E: 0xF0 | (0x0F & 0x0F) = 0xF0 | 0x0F = 0xFF
    bus.write_byte(VRAM_B_BASE, 0xF0);

    disable_grcg_mode(&mut bus);
    assert_eq!(read_all_planes_byte(&bus, 0), [0xFA, 0xF5, 0xF0, 0xFF]);
}

#[test]
fn grcg_tcr_selective_plane_compare() {
    let mut bus = setup_grcg_bus();

    // Set VRAM: B=0xAA, R=0x00 (mismatch), G=0xF0, E=0x0F.
    prefill_all_planes_byte(&mut bus, 0, [0xAA, 0x00, 0xF0, 0x0F]);

    // TCR with R-plane disabled (bit 1 set): mode=0x82.
    bus.io_write_byte(0x7C, 0x82);
    bus.io_write_byte(0x7E, 0xAA); // B tile
    bus.io_write_byte(0x7E, 0xFF); // R tile (disabled - won't be compared)
    bus.io_write_byte(0x7E, 0xF0); // G tile
    bus.io_write_byte(0x7E, 0x0F); // E tile

    // TCR should skip R-plane comparison.
    // B: !(0xAA ^ 0xAA) = 0xFF
    // G: !(0xF0 ^ 0xF0) = 0xFF
    // E: !(0x0F ^ 0x0F) = 0xFF
    // Result = 0xFF & 0xFF & 0xFF = 0xFF (R skipped).
    let result = bus.read_byte(VRAM_B_BASE);
    assert_eq!(
        result, 0xFF,
        "TCR with R-plane disabled should be all match"
    );
}

#[test]
fn grcg_tdw_sequential_writes() {
    let mut bus = setup_grcg_bus();
    enable_grcg_tdw(&mut bus, [0x11, 0x22, 0x33, 0x44]);

    // Write to multiple consecutive offsets.
    bus.write_byte(VRAM_B_BASE, 0xFF);
    bus.write_byte(VRAM_B_BASE + 1, 0xFF);
    bus.write_byte(VRAM_B_BASE + 2, 0xFF);

    disable_grcg_mode(&mut bus);

    // All 3 bytes at each plane should get tile values.
    for offset in 0..3 {
        let planes = read_all_planes_byte(&bus, offset);
        assert_eq!(planes, [0x11, 0x22, 0x33, 0x44], "offset {offset}");
    }
}

#[test]
fn grcg_tdw_via_e_plane_address() {
    let mut bus = setup_grcg_bus();
    enable_grcg_tdw(&mut bus, [0xDE, 0xAD, 0xBE, 0xEF]);

    // Write via E-plane address - GRCG should still write all 4 planes at the offset.
    bus.write_byte(VRAM_E_BASE, 0xFF);

    disable_grcg_mode(&mut bus);

    assert_eq!(read_all_planes_byte(&bus, 0), [0xDE, 0xAD, 0xBE, 0xEF]);
}

#[test]
fn grcg_tcr_via_e_plane_address() {
    let mut bus = setup_grcg_bus();

    prefill_all_planes_byte(&mut bus, 0, [0xAA, 0x55, 0xF0, 0x0F]);

    enable_grcg_tdw(&mut bus, [0xAA, 0x55, 0xF0, 0x0F]);

    // TCR read via E-plane address - should compare all 4 planes at offset 0.
    let result = bus.read_byte(VRAM_E_BASE);
    assert_eq!(result, 0xFF, "TCR via E-plane address, all match");
}

#[test]
fn grcg_rmw_all_zero_preserves_vram() {
    let mut bus = setup_grcg_bus();

    prefill_all_planes_byte(&mut bus, 0, [0xAB, 0xCD, 0xEF, 0x12]);

    enable_grcg_rmw(&mut bus, [0x55, 0xAA, 0x33, 0xCC]);

    // RMW write value=0x00: new = (0x00 & tile) | (0xFF & current) = current.
    bus.write_byte(VRAM_B_BASE, 0x00);

    disable_grcg_mode(&mut bus);

    assert_eq!(
        read_all_planes_byte(&bus, 0),
        [0xAB, 0xCD, 0xEF, 0x12],
        "VRAM unchanged with RMW write=0x00"
    );
}

#[test]
fn grcg_rmw_all_one_writes_tile() {
    let mut bus = setup_grcg_bus();

    prefill_all_planes_byte(&mut bus, 0, [0xAB, 0xCD, 0xEF, 0x12]);

    enable_grcg_rmw(&mut bus, [0x55, 0xAA, 0x33, 0xCC]);

    // RMW write value=0xFF: new = (0xFF & tile) | (0x00 & current) = tile.
    bus.write_byte(VRAM_B_BASE, 0xFF);

    disable_grcg_mode(&mut bus);

    assert_eq!(
        read_all_planes_byte(&bus, 0),
        [0x55, 0xAA, 0x33, 0xCC],
        "VRAM should be tile values with RMW write=0xFF"
    );
}

#[test]
fn grcg_tcr_partial_plane_combinations() {
    let mut bus = setup_grcg_bus();

    // Set VRAM: B=0xAA, R=0x55, G=0xF0, E=0x0F.
    prefill_all_planes_byte(&mut bus, 0, [0xAA, 0x55, 0xF0, 0x0F]);

    // TCR with only B+G enabled (R,E disabled): mode=0x8A (bits 1,3 set).
    bus.io_write_byte(0x7C, 0x8A);
    bus.io_write_byte(0x7E, 0xAA); // B tile (matches)
    bus.io_write_byte(0x7E, 0xFF); // R tile (disabled)
    bus.io_write_byte(0x7E, 0xF0); // G tile (matches)
    bus.io_write_byte(0x7E, 0xFF); // E tile (disabled)

    let result = bus.read_byte(VRAM_B_BASE);
    assert_eq!(result, 0xFF, "B+G match, R+E disabled -> all match");

    disable_grcg_mode(&mut bus);

    // TCR with only R+E enabled (B,G disabled): mode=0x85 (bits 0,2 set).
    bus.io_write_byte(0x7C, 0x85);
    bus.io_write_byte(0x7E, 0xFF); // B tile (disabled)
    bus.io_write_byte(0x7E, 0x55); // R tile (matches)
    bus.io_write_byte(0x7E, 0xFF); // G tile (disabled)
    bus.io_write_byte(0x7E, 0x0F); // E tile (matches)

    let result = bus.read_byte(VRAM_B_BASE);
    assert_eq!(result, 0xFF, "R+E match, B+G disabled -> all match");

    disable_grcg_mode(&mut bus);

    // Single plane (B only): mode=0x8E (bits 1,2,3 set).
    bus.io_write_byte(0x7C, 0x8E);
    bus.io_write_byte(0x7E, 0xAA); // B tile (matches)
    bus.io_write_byte(0x7E, 0x00); // R tile (disabled)
    bus.io_write_byte(0x7E, 0x00); // G tile (disabled)
    bus.io_write_byte(0x7E, 0x00); // E tile (disabled)

    let result = bus.read_byte(VRAM_B_BASE);
    assert_eq!(result, 0xFF, "B only match -> all match");
}

#[test]
fn grcg_mode_switch_resets_tile_index() {
    let mut bus = setup_grcg_bus();

    // Write 2 tiles in TDW mode.
    bus.io_write_byte(0x7C, 0x80);
    bus.io_write_byte(0x7E, 0x11); // tile[0]
    bus.io_write_byte(0x7E, 0x22); // tile[1]

    // Switch mode (write to 0x7C resets tile_index).
    bus.io_write_byte(0x7C, 0x80);
    bus.io_write_byte(0x7E, 0xAA); // tile[0] overwritten

    // Write remaining tiles.
    bus.io_write_byte(0x7E, 0xBB); // tile[1]
    bus.io_write_byte(0x7E, 0xCC); // tile[2]
    bus.io_write_byte(0x7E, 0xDD); // tile[3]

    bus.write_byte(VRAM_B_BASE, 0xFF);

    disable_grcg_mode(&mut bus);

    assert_eq!(
        read_all_planes_byte(&bus, 0),
        [0xAA, 0xBB, 0xCC, 0xDD],
        "tile[0] should be overwritten after mode switch"
    );
}

#[test]
fn grcg_tile_cycling_wraps_after_four() {
    let mut bus = setup_grcg_bus();

    bus.io_write_byte(0x7C, 0x80);
    // Write 5 tile values - 5th should wrap and overwrite tile[0].
    bus.io_write_byte(0x7E, 0x11); // tile[0]
    bus.io_write_byte(0x7E, 0x22); // tile[1]
    bus.io_write_byte(0x7E, 0x33); // tile[2]
    bus.io_write_byte(0x7E, 0x44); // tile[3]
    bus.io_write_byte(0x7E, 0xFF); // tile[0] overwritten

    bus.write_byte(VRAM_B_BASE, 0xFF);

    disable_grcg_mode(&mut bus);

    assert_eq!(
        read_all_planes_byte(&bus, 0),
        [0xFF, 0x22, 0x33, 0x44],
        "5th write should wrap to tile[0]"
    );
}

#[test]
fn grcg_tdw_word_various_offsets() {
    let mut bus = setup_grcg_bus();
    enable_grcg_tdw(&mut bus, [0x11, 0x22, 0x33, 0x44]);

    // Write at offset 0.
    bus.write_word(VRAM_B_BASE, 0xFFFF);
    // Write at offset 80 (next line).
    bus.write_word(VRAM_B_BASE + 80, 0xFFFF);
    // Write at offset 160 (2 lines down).
    bus.write_word(VRAM_B_BASE + 160, 0xFFFF);

    disable_grcg_mode(&mut bus);

    let bases = [VRAM_B_BASE, VRAM_R_BASE, VRAM_G_BASE, VRAM_E_BASE];
    let tiles = [0x11u8, 0x22, 0x33, 0x44];
    for offset in [0u32, 80, 160] {
        for (i, &base) in bases.iter().enumerate() {
            assert_eq!(
                bus.read_byte_direct(base + offset),
                tiles[i],
                "plane {i} offset {offset} lo"
            );
            assert_eq!(
                bus.read_byte_direct(base + offset + 1),
                tiles[i],
                "plane {i} offset {offset} hi"
            );
        }
    }
}

#[test]
fn grcg_e_plane_disabled_skips_write() {
    let mut bus = setup_grcg_bus();

    // Pre-fill E-plane with 0xAA while extension is enabled.
    prefill_all_planes_byte(&mut bus, 0, [0x00, 0x00, 0x00, 0xAA]);

    // Disable the graphics extension board.
    bus.set_graphics_extension_enabled(false);

    // TDW: all planes enabled, but graphics_extension is off -> E-plane skipped.
    enable_grcg_tdw(&mut bus, [0xFF, 0xFF, 0xFF, 0xFF]);
    bus.write_byte(VRAM_B_BASE, 0xFF);

    disable_grcg_mode(&mut bus);

    assert_eq!(read_plane_byte(&bus, VRAM_B_BASE, 0), 0xFF, "B written");
    assert_eq!(read_plane_byte(&bus, VRAM_R_BASE, 0), 0xFF, "R written");
    assert_eq!(read_plane_byte(&bus, VRAM_G_BASE, 0), 0xFF, "G written");

    // Re-enable extension to verify E-plane is unchanged.
    bus.set_graphics_extension_enabled(true);
    assert_eq!(
        read_plane_byte(&bus, VRAM_E_BASE, 0),
        0xAA,
        "E unchanged (extension was disabled during TDW)"
    );

    // RMW test: prefill, disable extension, write, verify.
    prefill_all_planes_byte(&mut bus, 0, [0x00, 0x00, 0x00, 0xBB]);
    bus.set_graphics_extension_enabled(false);

    enable_grcg_rmw(&mut bus, [0xFF, 0xFF, 0xFF, 0xFF]);
    bus.write_byte(VRAM_B_BASE, 0xFF);

    disable_grcg_mode(&mut bus);

    assert_eq!(read_plane_byte(&bus, VRAM_B_BASE, 0), 0xFF, "RMW B");
    assert_eq!(read_plane_byte(&bus, VRAM_R_BASE, 0), 0xFF, "RMW R");
    assert_eq!(read_plane_byte(&bus, VRAM_G_BASE, 0), 0xFF, "RMW G");

    bus.set_graphics_extension_enabled(true);
    assert_eq!(
        read_plane_byte(&bus, VRAM_E_BASE, 0),
        0xBB,
        "E unchanged (extension was disabled during RMW)"
    );
}
