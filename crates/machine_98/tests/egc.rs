use common::{Bus, CpuMode, MachineModel};
use machine_98::{NoTracing, Pc9801Bus};

const VRAM_B: u32 = 0xA8000;
const VRAM_R: u32 = 0xB0000;
const VRAM_G: u32 = 0xB8000;
const VRAM_E: u32 = 0xE0000;

fn setup_egc(bus: &mut Pc9801Bus<NoTracing>) {
    bus.io_write_byte(0x6A, 0x01); // analog mode (mode2 bit 0)
    bus.io_write_byte(0x6A, 0x07); // EGC permission (mode2 bit 3)
    bus.io_write_byte(0x6A, 0x05); // EGC request (mode2 bit 2)
    bus.io_write_byte(0x7C, 0x80); // GRCG enable
}

fn disable_grcg(bus: &mut Pc9801Bus<NoTracing>) {
    bus.io_write_byte(0x7C, 0x00);
}

fn write_egc_register(bus: &mut Pc9801Bus<NoTracing>, reg_offset: u16, value: u16) {
    let port = 0x04A0 + reg_offset;
    bus.io_write_byte(port, value as u8);
    bus.io_write_byte(port + 1, (value >> 8) as u8);
}

#[test]
fn egc_not_active_without_mode_bits() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);

    bus.io_write_byte(0x6A, 0x01); // analog mode for E-plane access

    bus.write_byte(VRAM_B, 0x11);
    bus.write_byte(VRAM_R, 0x22);
    bus.write_byte(VRAM_G, 0x33);
    bus.write_byte(VRAM_E, 0x44);

    assert_eq!(bus.read_byte(VRAM_B), 0x11);
    assert_eq!(bus.read_byte(VRAM_R), 0x22);
    assert_eq!(bus.read_byte(VRAM_G), 0x33);
    assert_eq!(bus.read_byte(VRAM_E), 0x44);
}

#[test]
fn egc_register_write_blocked_when_not_effective() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);

    // Write EGC registers without enabling EGC - should be ignored.
    write_egc_register(&mut bus, 0x00, 0x1234);
    write_egc_register(&mut bus, 0x04, 0x5678);

    // Now enable EGC and verify registers are still at defaults.
    setup_egc(&mut bus);

    // ope register default is 0.
    // If the earlier writes took effect, ope would be 0x5678.
    // Access default is 0xFFF0. If earlier write took effect, it would be 0x1234.
    // We verify by writing via EGC: with default ope=0 (CPU broadcast), writes broadcast.
    bus.write_word(VRAM_B, 0xBEEF);

    disable_grcg(&mut bus);

    // If access was 0x1234 (planes 2,0 disabled), plane 0 would be untouched.
    // With default access=0xFFF0 (all enabled), all planes get the value.
    assert_eq!(bus.read_byte_direct(VRAM_B), 0xEF);
    assert_eq!(bus.read_byte_direct(VRAM_B + 1), 0xBE);
    assert_eq!(bus.read_byte_direct(VRAM_R), 0xEF);
    assert_eq!(bus.read_byte_direct(VRAM_R + 1), 0xBE);
    assert_eq!(bus.read_byte_direct(VRAM_G), 0xEF);
    assert_eq!(bus.read_byte_direct(VRAM_G + 1), 0xBE);
    assert_eq!(bus.read_byte_direct(VRAM_E), 0xEF);
    assert_eq!(bus.read_byte_direct(VRAM_E + 1), 0xBE);
}

#[test]
fn egc_cpu_broadcast_write_byte() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    setup_egc(&mut bus);

    // ope=0 (default) -> CPU broadcast mode.
    bus.write_byte(VRAM_B, 0x5A);

    disable_grcg(&mut bus);

    assert_eq!(bus.read_byte_direct(VRAM_B), 0x5A);
    assert_eq!(bus.read_byte_direct(VRAM_R), 0x5A);
    assert_eq!(bus.read_byte_direct(VRAM_G), 0x5A);
    assert_eq!(bus.read_byte_direct(VRAM_E), 0x5A);
}

#[test]
fn egc_cpu_broadcast_write_word() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    setup_egc(&mut bus);

    bus.write_word(VRAM_B, 0xBEEF);

    disable_grcg(&mut bus);

    assert_eq!(bus.read_byte_direct(VRAM_B), 0xEF);
    assert_eq!(bus.read_byte_direct(VRAM_B + 1), 0xBE);
    assert_eq!(bus.read_byte_direct(VRAM_R), 0xEF);
    assert_eq!(bus.read_byte_direct(VRAM_R + 1), 0xBE);
    assert_eq!(bus.read_byte_direct(VRAM_G), 0xEF);
    assert_eq!(bus.read_byte_direct(VRAM_G + 1), 0xBE);
    assert_eq!(bus.read_byte_direct(VRAM_E), 0xEF);
    assert_eq!(bus.read_byte_direct(VRAM_E + 1), 0xBE);
}

#[test]
fn egc_foreground_color_fill() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    setup_egc(&mut bus);

    write_egc_register(&mut bus, 0x06, 5); // fg=5 -> planes 0,2 = 0xFFFF
    write_egc_register(&mut bus, 0x02, 0x4000); // fgbg: FGC source
    write_egc_register(&mut bus, 0x04, 0x1000); // ope: pattern source

    bus.write_word(VRAM_B, 0x0000);

    disable_grcg(&mut bus);

    // fg=5 = 0b0101 -> B-plane(0)=1, R-plane(1)=0, G-plane(2)=1, E-plane(3)=0
    assert_eq!(bus.read_byte_direct(VRAM_B), 0xFF);
    assert_eq!(bus.read_byte_direct(VRAM_B + 1), 0xFF);
    assert_eq!(bus.read_byte_direct(VRAM_R), 0x00);
    assert_eq!(bus.read_byte_direct(VRAM_R + 1), 0x00);
    assert_eq!(bus.read_byte_direct(VRAM_G), 0xFF);
    assert_eq!(bus.read_byte_direct(VRAM_G + 1), 0xFF);
    assert_eq!(bus.read_byte_direct(VRAM_E), 0x00);
    assert_eq!(bus.read_byte_direct(VRAM_E + 1), 0x00);
}

#[test]
fn egc_write_with_mask() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);

    // Pre-fill all planes with 0xFF using direct writes.
    bus.io_write_byte(0x6A, 0x01); // analog mode for E-plane
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        bus.write_byte(base, 0xFF);
        bus.write_byte(base + 1, 0xFF);
    }

    setup_egc(&mut bus);

    write_egc_register(&mut bus, 0x08, 0xF0F0); // mask=0xF0F0
    // ope=0 (default): CPU broadcast.
    bus.write_word(VRAM_B, 0x0000);

    disable_grcg(&mut bus);

    // mask=0xF0F0: bits where mask=1 get new data (0x00), bits where mask=0 keep old (0xFF).
    // Low byte: mask=0xF0 -> result = (0xFF & ~0xF0) | (0x00 & 0xF0) = 0x0F
    // High byte: mask=0xF0 -> result = 0x0F
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(
            bus.read_byte_direct(base),
            0x0F,
            "plane 0x{base:05X} byte 0"
        );
        assert_eq!(
            bus.read_byte_direct(base + 1),
            0x0F,
            "plane 0x{base:05X} byte 1"
        );
    }
}

#[test]
fn egc_plane_write_enable() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);

    // Pre-fill all planes with 0xAA.
    bus.io_write_byte(0x6A, 0x01);
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        bus.write_byte(base, 0xAA);
    }

    setup_egc(&mut bus);

    // access=0xFFFA: disable planes 1 (R) and 3 (E) - bits 1,3 set.
    write_egc_register(&mut bus, 0x00, 0xFFFA);
    // ope=0: CPU broadcast.
    bus.write_byte(VRAM_B, 0x55);

    disable_grcg(&mut bus);

    assert_eq!(
        bus.read_byte_direct(VRAM_B),
        0x55,
        "B-plane should be written"
    );
    assert_eq!(
        bus.read_byte_direct(VRAM_R),
        0xAA,
        "R-plane should be unchanged"
    );
    assert_eq!(
        bus.read_byte_direct(VRAM_G),
        0x55,
        "G-plane should be written"
    );
    assert_eq!(
        bus.read_byte_direct(VRAM_E),
        0xAA,
        "E-plane should be unchanged"
    );
}

#[test]
fn egc_supersedes_grcg() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    setup_egc(&mut bus);

    // Set GRCG tiles to distinctive values.
    bus.io_write_byte(0x7E, 0x11);
    bus.io_write_byte(0x7E, 0x22);
    bus.io_write_byte(0x7E, 0x33);
    bus.io_write_byte(0x7E, 0x44);

    // EGC CPU broadcast (ope=0 default): write 0xBB.
    bus.write_byte(VRAM_B, 0xBB);

    disable_grcg(&mut bus);

    // All planes should have EGC result (0xBB), not GRCG tile values.
    assert_eq!(bus.read_byte_direct(VRAM_B), 0xBB);
    assert_eq!(bus.read_byte_direct(VRAM_R), 0xBB);
    assert_eq!(bus.read_byte_direct(VRAM_G), 0xBB);
    assert_eq!(bus.read_byte_direct(VRAM_E), 0xBB);
}

#[test]
fn egc_e_plane_vram_access() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    setup_egc(&mut bus);

    // ope=0 (CPU broadcast): write through E-plane address.
    bus.write_byte(VRAM_E, 0xCC);

    disable_grcg(&mut bus);

    // EGC operates on all planes regardless of which address was used.
    assert_eq!(bus.read_byte_direct(VRAM_B), 0xCC);
    assert_eq!(bus.read_byte_direct(VRAM_R), 0xCC);
    assert_eq!(bus.read_byte_direct(VRAM_G), 0xCC);
    assert_eq!(bus.read_byte_direct(VRAM_E), 0xCC);
}

#[test]
fn egc_register_write_via_io_ports() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);

    // Pre-fill plane G (index 2) with 0xAA.
    bus.io_write_byte(0x6A, 0x01);
    bus.write_byte(VRAM_G, 0xAA);

    setup_egc(&mut bus);

    // Write access register to disable plane 2 (G): set bit 2.
    // access = 0xFFF4 (bit 2 set -> plane 2 disabled).
    write_egc_register(&mut bus, 0x00, 0xFFF4);

    // CPU broadcast write.
    bus.write_byte(VRAM_B, 0x55);

    disable_grcg(&mut bus);

    assert_eq!(
        bus.read_byte_direct(VRAM_B),
        0x55,
        "B-plane should be written"
    );
    assert_eq!(
        bus.read_byte_direct(VRAM_R),
        0x55,
        "R-plane should be written"
    );
    assert_eq!(
        bus.read_byte_direct(VRAM_G),
        0xAA,
        "G-plane should be unchanged (disabled)"
    );
    assert_eq!(
        bus.read_byte_direct(VRAM_E),
        0x55,
        "E-plane should be written"
    );
}

// Helper: write word to EGC register using atomic io_write_word.
fn write_egc_register_word(bus: &mut Pc9801Bus<NoTracing>, reg_offset: u16, value: u16) {
    bus.io_write_word(0x04A0 + reg_offset, value);
}

// Helper: pre-fill one word in all 4 planes via direct writes (EGC/GRCG disabled).
fn prefill_planes_word(bus: &mut Pc9801Bus<NoTracing>, offset: u32, values: [u16; 4]) {
    let bases = [VRAM_B, VRAM_R, VRAM_G, VRAM_E];
    for (i, &base) in bases.iter().enumerate() {
        bus.write_byte(base + offset, values[i] as u8);
        bus.write_byte(base + offset + 1, (values[i] >> 8) as u8);
    }
}

fn read_plane_word(bus: &Pc9801Bus<NoTracing>, plane_base: u32, offset: u32) -> u16 {
    let lo = bus.read_byte_direct(plane_base + offset) as u16;
    let hi = bus.read_byte_direct(plane_base + offset + 1) as u16;
    lo | (hi << 8)
}

#[test]
fn egc_aligned_word_block_copy_no_shift() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Write source data at offset 0.
    prefill_planes_word(&mut bus, 0, [0xAAAA, 0x5555, 0xF0F0, 0x0F0F]);
    // Clear destination at offset 2.
    prefill_planes_word(&mut bus, 2, [0; 4]);

    setup_egc(&mut bus);

    // ope = 0x08F0: write source = ROP (bits 12-11=01), read source = VRAM (bit10=0),
    // pattern load = none (bits 9-8=00), ROP = 0xF0 (source copy).
    write_egc_register_word(&mut bus, 0x04, 0x08F0);
    // sft: ascending, srcbit=0, dstbit=0.
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    // leng: 15 (16 bits).
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    // Read source at offset 0 -> feeds shift pipeline.
    let _ = bus.read_word(VRAM_B);
    // Write destination at offset 2 -> applies shifted data via ROP.
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    assert_eq!(read_plane_word(&bus, VRAM_B, 2), 0xAAAA, "B-plane copy");
    assert_eq!(read_plane_word(&bus, VRAM_R, 2), 0x5555, "R-plane copy");
    assert_eq!(read_plane_word(&bus, VRAM_G, 2), 0xF0F0, "G-plane copy");
    assert_eq!(read_plane_word(&bus, VRAM_E, 2), 0x0F0F, "E-plane copy");
}

#[test]
fn egc_descending_block_copy() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source at offset 4, destination at offset 2 (copying backwards).
    prefill_planes_word(&mut bus, 4, [0x1234, 0x5678, 0x9ABC, 0xDEF0]);
    prefill_planes_word(&mut bus, 2, [0; 4]);

    setup_egc(&mut bus);

    // ope = 0x08F0 (ROP source copy, VRAM source).
    write_egc_register_word(&mut bus, 0x04, 0x08F0);
    // sft: descending (bit12=1), srcbit=0, dstbit=0.
    write_egc_register_word(&mut bus, 0x0C, 0x1000);
    // leng: 15 (16 bits).
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    // Read source, write destination (descending).
    let _ = bus.read_word(VRAM_B + 4);
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    assert_eq!(
        read_plane_word(&bus, VRAM_B, 2),
        0x1234,
        "B-plane desc copy"
    );
    assert_eq!(
        read_plane_word(&bus, VRAM_R, 2),
        0x5678,
        "R-plane desc copy"
    );
    assert_eq!(
        read_plane_word(&bus, VRAM_G, 2),
        0x9ABC,
        "G-plane desc copy"
    );
    assert_eq!(
        read_plane_word(&bus, VRAM_E, 2),
        0xDEF0,
        "E-plane desc copy"
    );
}

#[test]
fn egc_byte_level_copy() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source byte at offset 0, destination at offset 2.
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        bus.write_byte(base, 0xAB);
        bus.write_byte(base + 2, 0x00);
    }

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x04, 0x08F0); // ROP source copy, VRAM source
    write_egc_register_word(&mut bus, 0x0C, 0x0000); // ascending, no shift
    write_egc_register_word(&mut bus, 0x0E, 0x0007); // leng=7 (8 bits)

    // Read source byte.
    let _ = bus.read_byte(VRAM_B);
    // Write destination byte.
    bus.write_byte(VRAM_B + 2, 0x00);

    disable_grcg(&mut bus);

    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(
            bus.read_byte_direct(base + 2),
            0xAB,
            "plane 0x{base:05X} byte copy"
        );
    }
}

#[test]
fn egc_misaligned_word_access() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Pre-fill destination at odd offset with zeros.
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        bus.write_byte(base + 1, 0x00);
        bus.write_byte(base + 2, 0x00);
    }

    setup_egc(&mut bus);

    // ope=0 (CPU broadcast), write word at odd address.
    bus.write_word(VRAM_B + 1, 0xBEEF);

    disable_grcg(&mut bus);

    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(
            bus.read_byte_direct(base + 1),
            0xEF,
            "plane 0x{base:05X} misaligned lo"
        );
        assert_eq!(
            bus.read_byte_direct(base + 2),
            0xBE,
            "plane 0x{base:05X} misaligned hi"
        );
    }
}

#[test]
fn egc_rop_and_c0() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::High, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source: 0xAAAA on all planes. Destination: 0xF0F0 on all planes.
    prefill_planes_word(&mut bus, 0, [0xAAAA; 4]);
    prefill_planes_word(&mut bus, 2, [0xF0F0; 4]);

    setup_egc(&mut bus);

    // ope = 0x08C0: ROP output, VRAM source, ROP=0xC0 (S AND D).
    write_egc_register_word(&mut bus, 0x04, 0x08C0);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    let _ = bus.read_word(VRAM_B);
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    // 0xAAAA & 0xF0F0 = 0xA0A0
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(
            read_plane_word(&bus, base, 2),
            0xA0A0,
            "plane 0x{base:05X} AND"
        );
    }
}

#[test]
fn egc_rop_or_not_fc() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    prefill_planes_word(&mut bus, 0, [0x00FF; 4]);
    prefill_planes_word(&mut bus, 2, [0xFF00; 4]);

    setup_egc(&mut bus);

    // ope = 0x08FC: ROP=0xFC: S | (~S & D).
    write_egc_register_word(&mut bus, 0x04, 0x08FC);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    let _ = bus.read_word(VRAM_B);
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    // S=0x00FF, D=0xFF00: S | (~S & D) = 0x00FF | (0xFF00 & 0xFF00) = 0x00FF | 0xFF00 = 0xFFFF
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(
            read_plane_word(&bus, base, 2),
            0xFFFF,
            "plane 0x{base:05X} OR+NOT"
        );
    }
}

#[test]
fn egc_rop_invert_source_0f() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    prefill_planes_word(&mut bus, 0, [0xAAAA; 4]);
    prefill_planes_word(&mut bus, 2, [0; 4]);

    setup_egc(&mut bus);

    // ope = 0x080F: ROP=0x0F (~Source).
    write_egc_register_word(&mut bus, 0x04, 0x080F);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    let _ = bus.read_word(VRAM_B);
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(
            read_plane_word(&bus, base, 2),
            0x5555,
            "plane 0x{base:05X} invert"
        );
    }
}

#[test]
fn egc_rop_with_cpu_source() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    prefill_planes_word(&mut bus, 0, [0xFF00; 4]);

    setup_egc(&mut bus);

    // ope = 0x0CF0: ROP output, CPU source (bit10=1), ROP=0xF0 (source copy).
    write_egc_register_word(&mut bus, 0x04, 0x0CF0);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    // CPU write with value 0xBEEF as shift source.
    bus.write_word(VRAM_B, 0xBEEF);

    disable_grcg(&mut bus);

    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(
            read_plane_word(&bus, base, 0),
            0xBEEF,
            "plane 0x{base:05X} CPU src"
        );
    }
}

#[test]
fn egc_compare_read_byte_full_match() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // FGC color 5 -> B=0xFF, R=0x00, G=0xFF, E=0x00
    bus.write_byte(VRAM_B, 0xFF);
    bus.write_byte(VRAM_R, 0x00);
    bus.write_byte(VRAM_G, 0xFF);
    bus.write_byte(VRAM_E, 0x00);

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x06, 5); // fg=5
    write_egc_register_word(&mut bus, 0x02, 0x4000); // fgbg: FGC compare source
    write_egc_register_word(&mut bus, 0x04, 0x2000); // ope: compare-read

    let result = bus.read_byte(VRAM_B);
    assert_eq!(result, 0xFF, "all pixels match fg color 5");
}

#[test]
fn egc_compare_read_byte_no_match() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // FGC color 5 expects B=0xFF, R=0x00, G=0xFF, E=0x00.
    // Set VRAM to opposite.
    bus.write_byte(VRAM_B, 0x00);
    bus.write_byte(VRAM_R, 0xFF);
    bus.write_byte(VRAM_G, 0x00);
    bus.write_byte(VRAM_E, 0xFF);

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x06, 5);
    write_egc_register_word(&mut bus, 0x02, 0x4000); // fgbg: FGC compare source
    write_egc_register_word(&mut bus, 0x04, 0x2000);

    let result = bus.read_byte(VRAM_B);
    assert_eq!(result, 0x00, "no pixels match fg color 5");
}

#[test]
fn egc_compare_read_byte_partial_match() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // FGC=5: B=0xFF, R=0x00, G=0xFF, E=0x00
    // VRAM: B=0xF0, R=0x00, G=0xFF, E=0x00
    // Mismatch on B plane bits 0-3 -> result should be 0xF0
    bus.write_byte(VRAM_B, 0xF0);
    bus.write_byte(VRAM_R, 0x00);
    bus.write_byte(VRAM_G, 0xFF);
    bus.write_byte(VRAM_E, 0x00);

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x06, 5);
    write_egc_register_word(&mut bus, 0x02, 0x4000); // fgbg: FGC compare source
    write_egc_register_word(&mut bus, 0x04, 0x2000);

    let result = bus.read_byte(VRAM_B);
    assert_eq!(result, 0xF0, "partial match");
}

#[test]
fn egc_compare_read_word() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // FGC=5: B=0xFFFF, R=0x0000, G=0xFFFF, E=0x0000
    prefill_planes_word(&mut bus, 0, [0xFFFF, 0x0000, 0xFFFF, 0x0000]);

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x06, 5);
    write_egc_register_word(&mut bus, 0x02, 0x4000); // fgbg: FGC compare source
    write_egc_register_word(&mut bus, 0x04, 0x2000);

    let result = bus.read_word(VRAM_B);
    assert_eq!(result, 0xFFFF, "word compare full match");
}

#[test]
fn egc_pattern_load_on_read() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source at offset 0 with distinctive data per plane.
    prefill_planes_word(&mut bus, 0, [0x1111, 0x2222, 0x3333, 0x4444]);
    // Destination at offset 2.
    prefill_planes_word(&mut bus, 2, [0; 4]);

    setup_egc(&mut bus);

    // ope = 0x1100: write source=pattern (bits 12-11=10), read source=VRAM (bit10=0),
    // pattern load=on read (bits 9-8=01), ROP=0x00 (unused for pattern source).
    write_egc_register_word(&mut bus, 0x04, 0x1100);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    // Read loads patreg from VRAM.
    let _ = bus.read_word(VRAM_B);
    // Write uses patreg as data source.
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    assert_eq!(read_plane_word(&bus, VRAM_B, 2), 0x1111, "B patreg");
    assert_eq!(read_plane_word(&bus, VRAM_R, 2), 0x2222, "R patreg");
    assert_eq!(read_plane_word(&bus, VRAM_G, 2), 0x3333, "G patreg");
    assert_eq!(read_plane_word(&bus, VRAM_E, 2), 0x4444, "E patreg");
}

#[test]
fn egc_pattern_load_on_write() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Put distinctive data at offset 0 (will be loaded as pattern on write).
    prefill_planes_word(&mut bus, 0, [0xAAAA, 0xBBBB, 0xCCCC, 0xDDDD]);

    setup_egc(&mut bus);

    // ope = 0x1200: write source=pattern (bits 12-11=10), pattern load=on write (bits 9-8=10).
    write_egc_register_word(&mut bus, 0x04, 0x1200);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    // Write at offset 0: patreg loaded from current VRAM BEFORE computing output.
    // Output = patreg (which is the current VRAM). Result = current VRAM unchanged.
    bus.write_word(VRAM_B, 0x0000);

    disable_grcg(&mut bus);

    assert_eq!(read_plane_word(&bus, VRAM_B, 0), 0xAAAA, "B unchanged");
    assert_eq!(read_plane_word(&bus, VRAM_R, 0), 0xBBBB, "R unchanged");
    assert_eq!(read_plane_word(&bus, VRAM_G, 0), 0xCCCC, "G unchanged");
    assert_eq!(read_plane_word(&bus, VRAM_E, 0), 0xDDDD, "E unchanged");
}

#[test]
fn egc_mixed_fgc_bgc_pattern() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source data (any values - ROP 0xAA ignores source/destination).
    prefill_planes_word(&mut bus, 0, [0xFFFF; 4]);
    // Destination.
    prefill_planes_word(&mut bus, 2, [0; 4]);

    setup_egc(&mut bus);

    // fg=0xF (all planes), bg=0x0 (no planes).
    write_egc_register_word(&mut bus, 0x06, 0x000F); // fg
    write_egc_register_word(&mut bus, 0x0A, 0x0000); // bg
    // fgbg=0x6000: mixed mode - planes 0-1 use FGC, planes 2-3 use BGC.
    write_egc_register_word(&mut bus, 0x02, 0x6000);
    // ope = 0x08AA: shift+ROP mode (bits 12-11=01), VRAM source (bit10=0),
    // ROP=0xAA (output = pattern). get_pattern() handles 0x6000 mixed mode.
    write_egc_register_word(&mut bus, 0x04, 0x08AA);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    // Read source to feed shift pipeline (data ignored by ROP 0xAA).
    let _ = bus.read_word(VRAM_B);
    // Write destination - ROP 0xAA outputs get_pattern().
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    // FGC=0xF -> fgc=[0xFFFF,0xFFFF,0xFFFF,0xFFFF].
    // BGC=0x0 -> bgc=[0x0000,0x0000,0x0000,0x0000].
    // Mixed (0x6000): [fgc[0], fgc[1], bgc[2], bgc[3]] = [0xFFFF, 0xFFFF, 0x0000, 0x0000].
    assert_eq!(read_plane_word(&bus, VRAM_B, 2), 0xFFFF, "B=FGC");
    assert_eq!(read_plane_word(&bus, VRAM_R, 2), 0xFFFF, "R=FGC");
    assert_eq!(read_plane_word(&bus, VRAM_G, 2), 0x0000, "G=BGC");
    assert_eq!(read_plane_word(&bus, VRAM_E, 2), 0x0000, "E=BGC");
}

#[test]
fn egc_mask_write_blocked_when_fgbg_nonzero() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Pre-fill with 0xFF.
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        bus.write_byte(base, 0xFF);
        bus.write_byte(base + 1, 0xFF);
    }

    setup_egc(&mut bus);

    // Set fgbg to 0x2000 (BGC source) -> mask writes should be blocked.
    write_egc_register_word(&mut bus, 0x02, 0x2000);
    // Attempt to set mask to 0x0000 - should be ignored because fgbg bits 13-12 != 0.
    write_egc_register_word(&mut bus, 0x08, 0x0000);

    // CPU broadcast write: if mask were 0x0000, VRAM would be unchanged.
    // But mask should still be 0xFFFF (write was blocked), so all bits get written.
    bus.write_word(VRAM_B, 0x0000);

    disable_grcg(&mut bus);

    // If mask write was correctly blocked (mask=0xFFFF), VRAM should be 0x0000.
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(
            bus.read_byte_direct(base),
            0x00,
            "plane 0x{base:05X} should be written (mask=0xFFFF)"
        );
        assert_eq!(
            bus.read_byte_direct(base + 1),
            0x00,
            "plane 0x{base:05X}+1 should be written (mask=0xFFFF)"
        );
    }
}

#[test]
fn egc_zero_mask_no_vram_write() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        bus.write_byte(base, 0xAA);
        bus.write_byte(base + 1, 0xAA);
    }

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x08, 0x0000); // mask=0
    bus.write_word(VRAM_B, 0x5555);

    disable_grcg(&mut bus);

    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(
            read_plane_word(&bus, base, 0),
            0xAAAA,
            "plane 0x{base:05X} unchanged with zero mask"
        );
    }
}

#[test]
fn egc_srcmask_partial_length() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Fill destination with 0xFF.
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        bus.write_byte(base, 0xFF);
        bus.write_byte(base + 1, 0xFF);
    }

    setup_egc(&mut bus);

    // ope=0 (CPU broadcast), mask=0xFFFF.
    // leng=7 (8 bits) - should only write one byte.
    write_egc_register_word(&mut bus, 0x0E, 0x0007);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);

    bus.write_word(VRAM_B, 0x0000);

    disable_grcg(&mut bus);

    // Low byte should be written (0x00), high byte should remain 0xFF.
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(
            bus.read_byte_direct(base),
            0x00,
            "plane 0x{base:05X} lo byte written"
        );
        assert_eq!(
            bus.read_byte_direct(base + 1),
            0xFF,
            "plane 0x{base:05X} hi byte preserved"
        );
    }
}

#[test]
fn egc_compare_read_bgc_source() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // BGC color 0xA -> B=0x0000, R=0xFFFF, G=0x0000, E=0xFFFF
    // Fill VRAM matching color 0xA.
    prefill_planes_word(&mut bus, 0, [0x0000, 0xFFFF, 0x0000, 0xFFFF]);

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x0A, 0xA); // bg=0xA
    write_egc_register_word(&mut bus, 0x02, 0x2000); // fgbg: BGC compare source
    write_egc_register_word(&mut bus, 0x04, 0x2000); // ope: compare-read

    let result = bus.read_word(VRAM_B);
    assert_eq!(result, 0xFFFF, "full match against BGC");

    // Now test no-match: fill VRAM with color 5 (opposite of 0xA).
    disable_grcg(&mut bus);
    prefill_planes_word(&mut bus, 2, [0xFFFF, 0x0000, 0xFFFF, 0x0000]);
    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x0A, 0xA);
    write_egc_register_word(&mut bus, 0x02, 0x2000);
    write_egc_register_word(&mut bus, 0x04, 0x2000);

    let result = bus.read_word(VRAM_B + 2);
    assert_eq!(result, 0x0000, "no match against BGC");
}

#[test]
fn egc_compare_read_patreg_source() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source at offset 0: distinctive data to load into patreg.
    prefill_planes_word(&mut bus, 0, [0xAAAA, 0x5555, 0xAAAA, 0x5555]);
    // Second offset matching the pattern.
    prefill_planes_word(&mut bus, 2, [0xAAAA, 0x5555, 0xAAAA, 0x5555]);

    setup_egc(&mut bus);

    // ope=0x2100: compare-read (bit13=1) + load patreg on read (bits 9-8=01).
    // fgbg=0x0000: default -> patreg compare source.
    write_egc_register_word(&mut bus, 0x04, 0x2100);

    // First read loads patreg from VRAM at offset 0.
    let _ = bus.read_word(VRAM_B);
    // Second read compares VRAM at offset 2 against loaded patreg.
    let result = bus.read_word(VRAM_B + 2);
    assert_eq!(result, 0xFFFF, "match against patreg loaded from VRAM");
}

#[test]
fn egc_compare_read_fgc_vs_bgc() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Fill VRAM matching color 5: B=0xFF, R=0x00, G=0xFF, E=0x00
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        bus.write_byte(
            base,
            if base == VRAM_B || base == VRAM_G {
                0xFF
            } else {
                0x00
            },
        );
    }

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x06, 5); // fg=5
    write_egc_register_word(&mut bus, 0x0A, 0xA); // bg=0xA (opposite)

    // With fgbg=0x4000 (FGC source): should match.
    write_egc_register_word(&mut bus, 0x02, 0x4000);
    write_egc_register_word(&mut bus, 0x04, 0x2000);
    let result = bus.read_byte(VRAM_B);
    assert_eq!(result, 0xFF, "FGC source matches VRAM color 5");

    // With fgbg=0x2000 (BGC source): should not match.
    write_egc_register_word(&mut bus, 0x02, 0x2000);
    let result = bus.read_byte(VRAM_B);
    assert_eq!(result, 0x00, "BGC source does not match VRAM color 5");
}

#[test]
fn egc_multi_word_blit_no_shift() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source: 2 words at offsets 0 and 2.
    prefill_planes_word(&mut bus, 0, [0x1234, 0x5678, 0x9ABC, 0xDEF0]);
    prefill_planes_word(&mut bus, 2, [0xFEDC, 0xBA98, 0x7654, 0x3210]);
    // Clear destination at offsets 4 and 6.
    prefill_planes_word(&mut bus, 4, [0; 4]);
    prefill_planes_word(&mut bus, 6, [0; 4]);

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x04, 0x08F0); // ROP source copy, VRAM source
    write_egc_register_word(&mut bus, 0x0C, 0x0000); // ascending, no shift
    write_egc_register_word(&mut bus, 0x0E, 0x001F); // leng=31 (32 bits)

    // Read 2 source words, write 2 dest words.
    let _ = bus.read_word(VRAM_B);
    bus.write_word(VRAM_B + 4, 0x0000);
    let _ = bus.read_word(VRAM_B + 2);
    bus.write_word(VRAM_B + 6, 0x0000);

    disable_grcg(&mut bus);

    assert_eq!(read_plane_word(&bus, VRAM_B, 4), 0x1234, "B word 0");
    assert_eq!(read_plane_word(&bus, VRAM_R, 4), 0x5678, "R word 0");
    assert_eq!(read_plane_word(&bus, VRAM_G, 4), 0x9ABC, "G word 0");
    assert_eq!(read_plane_word(&bus, VRAM_E, 4), 0xDEF0, "E word 0");
    assert_eq!(read_plane_word(&bus, VRAM_B, 6), 0xFEDC, "B word 1");
    assert_eq!(read_plane_word(&bus, VRAM_R, 6), 0xBA98, "R word 1");
    assert_eq!(read_plane_word(&bus, VRAM_G, 6), 0x7654, "G word 1");
    assert_eq!(read_plane_word(&bus, VRAM_E, 6), 0x3210, "E word 1");
}

#[test]
fn egc_shift_right_ascending() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source: 2 words at offsets 0 and 2. Dest: 2 words at offsets 4 and 6.
    // Using 2-word blit so the shift pipeline has enough data.
    prefill_planes_word(&mut bus, 0, [0xFF00; 4]);
    prefill_planes_word(&mut bus, 2, [0x00FF; 4]);
    prefill_planes_word(&mut bus, 4, [0; 4]);
    prefill_planes_word(&mut bus, 6, [0; 4]);

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x04, 0x08F0); // ROP source copy, VRAM source
    // sft: ascending, srcbit=3, dstbit=7 -> src8=3, dst8=7 -> func=2 (right shift 4).
    write_egc_register_word(&mut bus, 0x0C, 0x0073);
    write_egc_register_word(&mut bus, 0x0E, 0x001F); // 32 bits

    let _ = bus.read_word(VRAM_B);
    bus.write_word(VRAM_B + 4, 0x0000);
    let _ = bus.read_word(VRAM_B + 2);
    bus.write_word(VRAM_B + 6, 0x0000);

    disable_grcg(&mut bus);

    // Byte-by-byte shift pipeline: dstbit=7 masks the first output byte's upper bits,
    // producing a shifted version of the source data across both output words.
    let w0 = read_plane_word(&bus, VRAM_B, 4);
    let w1 = read_plane_word(&bus, VRAM_B, 6);
    assert_eq!(w0, 0x0F00, "B right shift word 0");
    assert_eq!(w1, 0xF0FF, "B right shift word 1");
}

#[test]
fn egc_shift_left_ascending() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source: 2 words. Dest: 2 words.
    prefill_planes_word(&mut bus, 0, [0x0FF0; 4]);
    prefill_planes_word(&mut bus, 2, [0x0FF0; 4]);
    prefill_planes_word(&mut bus, 4, [0; 4]);
    prefill_planes_word(&mut bus, 6, [0; 4]);

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x04, 0x08F0); // ROP source copy, VRAM source
    // sft: ascending, srcbit=7, dstbit=3 -> src8=7, dst8=3 -> func=4 (left shift 4).
    write_egc_register_word(&mut bus, 0x0C, 0x0037);
    write_egc_register_word(&mut bus, 0x0E, 0x001F); // 32 bits

    let _ = bus.read_word(VRAM_B);
    bus.write_word(VRAM_B + 4, 0x0000);
    let _ = bus.read_word(VRAM_B + 2);
    bus.write_word(VRAM_B + 6, 0x0000);

    disable_grcg(&mut bus);

    // Byte-by-byte shift pipeline: srcbit=7 consumes the first source byte's
    // single bit, leaving the first output word empty (srcmask=0).
    // Data appears in the second output word.
    let w0 = read_plane_word(&bus, VRAM_B, 4);
    let w1 = read_plane_word(&bus, VRAM_B, 6);
    assert_eq!(w0, 0x0000, "B left shift word 0");
    assert_eq!(w1, 0xFF00, "B left shift word 1");
}

#[test]
fn egc_shift_descending_no_shift() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source at offset 4, destination at offset 2.
    prefill_planes_word(&mut bus, 4, [0xBEEF, 0xCAFE, 0xDEAD, 0xF00D]);
    prefill_planes_word(&mut bus, 2, [0; 4]);

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x04, 0x08F0);
    write_egc_register_word(&mut bus, 0x0C, 0x1000); // descending, no sub-byte shift
    write_egc_register_word(&mut bus, 0x0E, 0x000F); // 16 bits

    let _ = bus.read_word(VRAM_B + 4);
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    assert_eq!(read_plane_word(&bus, VRAM_B, 2), 0xBEEF, "B desc copy");
    assert_eq!(read_plane_word(&bus, VRAM_R, 2), 0xCAFE, "R desc copy");
    assert_eq!(read_plane_word(&bus, VRAM_G, 2), 0xDEAD, "G desc copy");
    assert_eq!(read_plane_word(&bus, VRAM_E, 2), 0xF00D, "E desc copy");
}

#[test]
fn egc_rop_xor_src_dst() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    prefill_planes_word(&mut bus, 0, [0xAAAA; 4]); // source
    prefill_planes_word(&mut bus, 2, [0xFF00; 4]); // destination

    setup_egc(&mut bus);

    // ope=0x083C: ROP=0x3C = S XOR D (ope_np path, pattern-independent).
    write_egc_register_word(&mut bus, 0x04, 0x083C);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    let _ = bus.read_word(VRAM_B);
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    // 0xAAAA XOR 0xFF00 = 0x55AA
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(read_plane_word(&bus, base, 2), 0x55AA, "XOR result");
    }
}

#[test]
fn egc_rop_pattern_and_source() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source at offset 0 (will be read from VRAM).
    prefill_planes_word(&mut bus, 0, [0xFFFF, 0xFF00, 0x00FF, 0x0000]);
    // Destination at offset 2.
    prefill_planes_word(&mut bus, 2, [0xFFFF; 4]);

    setup_egc(&mut bus);

    // fg=0xF -> fgc = all 0xFFFF. Pattern source = FGC.
    write_egc_register_word(&mut bus, 0x06, 0xF);
    write_egc_register_word(&mut bus, 0x02, 0x4000); // fgbg: FGC source for pattern
    // ope=0x0880: ROP=0x80 = P & S & D (ope_xx path).
    write_egc_register_word(&mut bus, 0x04, 0x0880);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    let _ = bus.read_word(VRAM_B);
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    // P=0xFFFF for all planes, D=0xFFFF, so result = P & S & D = S.
    assert_eq!(read_plane_word(&bus, VRAM_B, 2), 0xFFFF, "P&S&D B");
    assert_eq!(read_plane_word(&bus, VRAM_R, 2), 0xFF00, "P&S&D R");
    assert_eq!(read_plane_word(&bus, VRAM_G, 2), 0x00FF, "P&S&D G");
    assert_eq!(read_plane_word(&bus, VRAM_E, 2), 0x0000, "P&S&D E");
}

#[test]
fn egc_rop_ope_nd_pattern_only() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source at offset 0.
    prefill_planes_word(&mut bus, 0, [0xF0F0; 4]);
    // Destination at offset 2.
    prefill_planes_word(&mut bus, 2, [0xAAAA; 4]);

    setup_egc(&mut bus);

    // fg=0xA -> fgc=[0x0000, 0xFFFF, 0x0000, 0xFFFF].
    write_egc_register_word(&mut bus, 0x06, 0xA);
    write_egc_register_word(&mut bus, 0x02, 0x4000);
    write_egc_register_word(&mut bus, 0x04, 0x08A0);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    let _ = bus.read_word(VRAM_B);
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    // P & S: pattern (fgc) & source.
    // fg=0xA: fgc = [0x0000, 0xFFFF, 0x0000, 0xFFFF]
    // S = [0xF0F0, 0xF0F0, 0xF0F0, 0xF0F0]
    // P & S = [0x0000, 0xF0F0, 0x0000, 0xF0F0]
    assert_eq!(read_plane_word(&bus, VRAM_B, 2), 0x0000, "P&S B");
    assert_eq!(read_plane_word(&bus, VRAM_R, 2), 0xF0F0, "P&S R");
    assert_eq!(read_plane_word(&bus, VRAM_G, 2), 0x0000, "P&S G");
    assert_eq!(read_plane_word(&bus, VRAM_E, 2), 0xF0F0, "P&S E");
}

#[test]
fn egc_rop_ope_np_no_pattern() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source at offset 0: 0xF0F0 all planes.
    prefill_planes_word(&mut bus, 0, [0xF0F0; 4]);
    // Destination at offset 2: 0xFF00 all planes.
    prefill_planes_word(&mut bus, 2, [0xFF00; 4]);

    setup_egc(&mut bus);

    write_egc_register_word(&mut bus, 0x04, 0x0830);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    let _ = bus.read_word(VRAM_B);
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    // S & ~D: S=0xF0F0, D=0xFF00, ~D=0x00FF -> 0xF0F0 & 0x00FF = 0x00F0
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(read_plane_word(&bus, base, 2), 0x00F0, "S&~D result");
    }
}

#[test]
fn egc_cpu_source_shift() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    prefill_planes_word(&mut bus, 0, [0; 4]);

    setup_egc(&mut bus);

    // ope=0x0CF0: ROP source copy, CPU source (bit10=1).
    write_egc_register_word(&mut bus, 0x04, 0x0CF0);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    bus.write_word(VRAM_B, 0x1234);

    disable_grcg(&mut bus);

    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(read_plane_word(&bus, base, 0), 0x1234, "CPU src shift");
    }
}

#[test]
fn egc_ope_word_pattern_source_patreg() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source to load as patreg at offset 0.
    prefill_planes_word(&mut bus, 0, [0x1111, 0x2222, 0x3333, 0x4444]);
    // Destination at offset 2.
    prefill_planes_word(&mut bus, 2, [0; 4]);

    setup_egc(&mut bus);

    // ope=0x1100: pattern source (bits 12-11=10), load patreg on read (bits 9-8=01).
    // fgbg=0x0000 -> patreg used as pattern.
    write_egc_register_word(&mut bus, 0x04, 0x1100);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);
    write_egc_register_word(&mut bus, 0x0E, 0x000F);

    // Read loads patreg from VRAM.
    let _ = bus.read_word(VRAM_B);
    // Write outputs patreg.
    bus.write_word(VRAM_B + 2, 0x0000);

    disable_grcg(&mut bus);

    assert_eq!(read_plane_word(&bus, VRAM_B, 2), 0x1111, "patreg B");
    assert_eq!(read_plane_word(&bus, VRAM_R, 2), 0x2222, "patreg R");
    assert_eq!(read_plane_word(&bus, VRAM_G, 2), 0x3333, "patreg G");
    assert_eq!(read_plane_word(&bus, VRAM_E, 2), 0x4444, "patreg E");
}

#[test]
fn egc_aligned_word_partial_mask() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Pre-fill with 0xAAAA.
    prefill_planes_word(&mut bus, 0, [0xAAAA; 4]);

    setup_egc(&mut bus);

    // mask=0xFF00: only high byte should be modified.
    write_egc_register_word(&mut bus, 0x08, 0xFF00);
    write_egc_register_word(&mut bus, 0x0E, 0x000F); // 16 bits
    write_egc_register_word(&mut bus, 0x0C, 0x0000);

    // ope=0 (CPU broadcast): write 0x0000.
    bus.write_word(VRAM_B, 0x0000);

    disable_grcg(&mut bus);

    // High byte -> 0x00 (from write), low byte -> 0xAA (preserved by mask).
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        assert_eq!(read_plane_word(&bus, base, 0), 0x00AA, "partial mask");
    }
}

#[test]
fn egc_sub_byte_leng() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VX, CpuMode::Low, 48000);
    bus.io_write_byte(0x6A, 0x01);

    // Source at offset 0, dest at offset 2.
    for &base in &[VRAM_B, VRAM_R, VRAM_G, VRAM_E] {
        bus.write_byte(base, 0x00); // source = 0x00
        bus.write_byte(base + 2, 0xFF); // dest = 0xFF
    }

    setup_egc(&mut bus);

    // ope=0x08F0: shift+ROP, VRAM source, ROP=0xF0 (source copy).
    // leng=3 (4 bits): srcmask gates to 4 bits.
    write_egc_register_word(&mut bus, 0x04, 0x08F0);
    write_egc_register_word(&mut bus, 0x0E, 0x0003);
    write_egc_register_word(&mut bus, 0x0C, 0x0000);

    // Read source byte.
    let _ = bus.read_byte(VRAM_B);
    // Write dest byte - srcmask should limit to 4 bits.
    bus.write_byte(VRAM_B + 2, 0x00);

    disable_grcg(&mut bus);

    // srcmask for leng=3 (4 bits), dstbit=0, ascending:
    // BYTEMASK_U1[3] = 0xF0 -> top 4 bits from source (0x00), bottom 4 preserved (0x0F).
    assert_eq!(
        bus.read_byte_direct(VRAM_B + 2),
        0x0F,
        "sub-byte leng: only 4 bits written"
    );
}
