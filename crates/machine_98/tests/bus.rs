use common::{Bus, CpuMode, MachineModel};
use machine_98::{NoTracing, Pc9801Bus};

#[test]
fn ram_read_write() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    bus.write_byte(0x00100, 0x42);
    assert_eq!(bus.read_byte(0x00100), 0x42);

    bus.write_byte(0x50000, 0x42);
    assert_eq!(bus.read_byte(0x50000), 0x42);
}

#[test]
fn ram_word_access() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    bus.write_word(0x00200, 0xBEEF);
    assert_eq!(bus.read_word(0x00200), 0xBEEF);
    assert_eq!(bus.read_byte(0x00200), 0xEF);
    assert_eq!(bus.read_byte(0x00201), 0xBE);
}

#[test]
fn unmapped_regions() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    assert_eq!(bus.read_byte(0xC0000), 0xFF);
    assert_eq!(bus.read_byte(0xD0000), 0xFF);

    // Writes ignored.
    bus.write_byte(0xC0000, 0x42);
    assert_eq!(bus.read_byte(0xC0000), 0xFF);
}

#[test]
fn rom_read_only() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    // Load some ROM data.
    let rom_data = vec![0xAB; 16];
    bus.load_bios_rom(&rom_data);

    assert_eq!(bus.read_byte(0xE8000), 0xAB);
    assert_eq!(bus.read_byte(0xE800F), 0xAB);

    // Write to ROM region should be ignored.
    bus.write_byte(0xE8000, 0x00);
    assert_eq!(bus.read_byte(0xE8000), 0xAB);
}

#[test]
fn text_vram_access() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    bus.write_byte(0xA0000, 0x55);
    assert_eq!(bus.read_byte(0xA0000), 0x55);

    bus.write_byte(0xA3FFF, 0xAA);
    assert_eq!(bus.read_byte(0xA3FFF), 0xAA);
}

#[test]
fn graphics_vram_access() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    bus.write_byte(0xA8000, 0x11);
    assert_eq!(bus.read_byte(0xA8000), 0x11);

    bus.write_byte(0xBFFFF, 0x22);
    assert_eq!(bus.read_byte(0xBFFFF), 0x22);
}

#[test]
fn bus_clock_config_uses_cpu_mode_without_changing_pit_lineage() {
    let cases = [
        (MachineModel::PC9801F, 5_000_000, 8_000_000, 1_996_800),
        (MachineModel::PC9801VM, 8_000_000, 10_000_000, 2_457_600),
        (MachineModel::PC9801VX, 8_000_000, 10_000_000, 2_457_600),
        (MachineModel::PC9801RS, 16_000_000, 16_000_000, 1_996_800),
        (MachineModel::PC9801RA, 20_000_000, 20_000_000, 1_996_800),
        (MachineModel::PC9821AS, 33_000_000, 33_000_000, 1_996_800),
        (MachineModel::PC9821AP, 66_000_000, 66_000_000, 1_996_800),
    ];

    for (model, low_clock_hz, high_clock_hz, pit_clock_hz) in cases {
        let low_bus = Pc9801Bus::<NoTracing>::new(model, CpuMode::Low, 48000);
        let high_bus = Pc9801Bus::<NoTracing>::new(model, CpuMode::High, 48000);

        assert_eq!(low_bus.cpu_clock_hz(), low_clock_hz, "{model} low clock");
        assert_eq!(high_bus.cpu_clock_hz(), high_clock_hz, "{model} high clock");
        assert_eq!(low_bus.pit_clock_hz(), pit_clock_hz, "{model} low PIT");
        assert_eq!(high_bus.pit_clock_hz(), pit_clock_hz, "{model} high PIT");
    }
}

#[test]
fn kanji_ram_stub() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    assert_eq!(bus.read_byte(0xA4000), 0xFF);
    assert_eq!(bus.read_byte(0xA4FFF), 0xFF);
}

#[test]
fn plane_e_vram_stub() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    // Default VM boot state uses mode2 bit 0 = 1, so force digital mode first.
    bus.io_write_byte(0x6A, 0x00);
    assert_eq!(bus.read_byte(0xE0000), 0xFF);
    assert_eq!(bus.read_byte(0xE7FFF), 0xFF);
}

#[test]
fn plane_e_vram_mapped_in_analog_mode() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    // Start from digital mode.
    bus.io_write_byte(0x6A, 0x00);
    // Digital mode: E-plane is unmapped (open bus).
    assert_eq!(bus.read_byte(0xE0000), 0xFF);

    // Analog mode (mode2 bit 0): E-plane becomes visible.
    // Flip-flop: ADR=0 DT=1.
    bus.io_write_byte(0x6A, 0x01);
    bus.write_byte(0xE0000, 0x5A);
    bus.write_byte(0xE7FFF, 0xC3);
    assert_eq!(bus.read_byte(0xE0000), 0x5A);
    assert_eq!(bus.read_byte(0xE7FFF), 0xC3);

    // Switching back to digital mode hides E-plane again.
    // Flip-flop: ADR=0 DT=0.
    bus.io_write_byte(0x6A, 0x00);
    assert_eq!(bus.read_byte(0xE0000), 0xFF);
    assert_eq!(bus.read_byte(0xE7FFF), 0xFF);
}

#[test]
fn grcg_writes_all_four_planes_when_extension_present() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    // Enable analog mode so E-plane is mapped (mode2 bit 0).
    bus.io_write_byte(0x6A, 0x01);

    // GRCG TDW mode, all planes enabled.
    bus.io_write_byte(0x7C, 0x80);
    bus.io_write_byte(0x7E, 0x11);
    bus.io_write_byte(0x7E, 0x22);
    bus.io_write_byte(0x7E, 0x33);
    bus.io_write_byte(0x7E, 0x44);

    // CPU data is ignored in TDW mode.
    bus.write_byte(0xA8000, 0x00);

    // Disable GRCG so reads return raw VRAM bytes.
    bus.io_write_byte(0x7C, 0x00);

    // Plane B/R/G at A8000/B0000/B8000, plane E at E0000.
    assert_eq!(bus.read_byte(0xA8000), 0x11);
    assert_eq!(bus.read_byte(0xB0000), 0x22);
    assert_eq!(bus.read_byte(0xB8000), 0x33);
    assert_eq!(bus.read_byte(0xE0000), 0x44);
}

#[test]
fn graphics_page_select_affects_cpu_access_and_snapshot_display() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    // Enable analog mode so E-plane addresses are valid.
    bus.io_write_byte(0x6A, 0x01);

    // Access page 0 writes.
    bus.io_write_byte(0xA6, 0x00);
    bus.write_byte(0xA8000, 0x11);
    bus.write_byte(0xE0000, 0x55);

    // Access page 1 writes.
    bus.io_write_byte(0xA6, 0x01);
    bus.write_byte(0xA8000, 0x22);
    bus.write_byte(0xE0000, 0x66);
    assert_eq!(bus.read_byte(0xA8000), 0x22);
    assert_eq!(bus.read_byte(0xE0000), 0x66);

    // Switch back to page 0 and verify values are independent.
    bus.io_write_byte(0xA6, 0x00);
    assert_eq!(bus.read_byte(0xA8000), 0x11);
    assert_eq!(bus.read_byte(0xE0000), 0x55);
}

#[test]
fn memory_switch_writes_require_mode1_bit6() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    let memory_switch_1 = 0xA3FE2;

    // Boot state has memory-switch write protection enabled.
    assert_eq!(bus.read_byte(memory_switch_1), 0x48);
    bus.write_byte(memory_switch_1, 0xAA);
    assert_eq!(bus.read_byte(memory_switch_1), 0x48);

    // Enable mode1 bit 6 (flip-flop: ADR=6, DT=1), then write.
    bus.io_write_byte(0x68, 0x0D);
    bus.write_byte(memory_switch_1, 0xAA);
    assert_eq!(bus.read_byte(memory_switch_1), 0xAA);

    // Disable mode1 bit 6 (flip-flop: ADR=6, DT=0), writes are blocked again.
    bus.io_write_byte(0x68, 0x0C);
    bus.write_byte(memory_switch_1, 0x55);
    assert_eq!(bus.read_byte(memory_switch_1), 0xAA);
}

#[test]
fn mode1_bit5_switches_cgrom_access_bank() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    bus.io_write_byte(0xA1, 0x21);
    bus.io_write_byte(0xA3, 0x56);
    bus.io_write_byte(0xA5, 0x23);

    // Bit5 set: dot-access bank.
    bus.io_write_byte(0x68, 0x0B);
    bus.io_write_byte(0xA9, 0x11);

    // Bit5 clear: code-access bank (+0x1000).
    bus.io_write_byte(0x68, 0x0A);
    bus.io_write_byte(0xA9, 0x22);

    bus.io_write_byte(0x68, 0x0B);
    assert_eq!(bus.io_read_byte(0xA9), 0x11);

    bus.io_write_byte(0x68, 0x0A);
    assert_eq!(bus.io_read_byte(0xA9), 0x22);
}

#[test]
fn io_pic_routing() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::High, 48000);

    // Master IMR.
    bus.io_write_byte(0x02, 0xFF);
    assert_eq!(bus.io_read_byte(0x02), 0xFF);

    // Slave IMR.
    bus.io_write_byte(0x0A, 0xAA);
    assert_eq!(bus.io_read_byte(0x0A), 0xAA);
}

#[test]
fn io_pit_routing() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

    // Program PIT channel 0: word mode, mode 3.
    bus.io_write_byte(0x77, 0x36);
    // Write counter LSB + MSB.
    bus.io_write_byte(0x71, 0xE8);
    bus.io_write_byte(0x71, 0x03);

    // Latch channel 0.
    bus.io_write_byte(0x77, 0x00);
    // Read counter.
    let low = bus.io_read_byte(0x71);
    let high = bus.io_read_byte(0x71);
    let count = (high as u16) << 8 | low as u16;
    // Should be close to 1000 (just loaded, 0 cycles elapsed).
    assert!(count <= 1000, "count was {count}");
    assert!(count >= 990, "count was {count}");
}

#[test]
fn io_nmi_control() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

    // NMI disable.
    bus.io_write_byte(0x50, 0x00);
    assert!(!bus.has_nmi());

    // NMI enable.
    bus.io_write_byte(0x52, 0x00);
    // Still no NMI (no source).
    assert!(!bus.has_nmi());
}

#[test]
fn io_unmapped_returns_ff() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

    assert_eq!(bus.io_read_byte(0xFF), 0xFF);
    assert_eq!(bus.io_read_byte(0x1234), 0xFF);
}

#[test]
fn address_wraps_20bit() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

    // Address above 1MB wraps to 20-bit.
    bus.write_byte(0x100000, 0x42); // wraps to 0x00000.
    assert_eq!(bus.read_byte(0x00000), 0x42);
}

#[test]
fn pit_mirror_ports() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

    // Program PIT via mirror control port.
    bus.io_write_byte(0x3FDF, 0x36); // ch0: word, mode 3
    bus.io_write_byte(0x3FD9, 0xE8); // ch0 LSB via mirror
    bus.io_write_byte(0x3FD9, 0x03); // ch0 MSB via mirror

    // Latch via primary port.
    bus.io_write_byte(0x77, 0x00);

    // Read via primary port.
    let low = bus.io_read_byte(0x71);
    let high = bus.io_read_byte(0x71);
    let count = (high as u16) << 8 | low as u16;
    assert!((990..=1000).contains(&count), "count was {count}");

    // Channel 1 via mirror.
    bus.io_write_byte(0x3FDF, 0x74); // ch1: word, mode 2
    bus.io_write_byte(0x3FDB, 0xC8); // ch1 LSB via mirror
    bus.io_write_byte(0x3FDB, 0x00); // ch1 MSB -> 200

    // Latch ch1 and read via mirror.
    bus.io_write_byte(0x3FDF, 0x40);
    let low = bus.io_read_byte(0x3FDB);
    let high = bus.io_read_byte(0x3FDB);
    let count = (high as u16) << 8 | low as u16;
    assert!((190..=200).contains(&count), "count was {count}");
}

#[test]
fn pc9821_pit_latch_command_applies_delay() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AP, CpuMode::High, 48000);

    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0xE8);
    bus.io_write_byte(0x71, 0x03);
    bus.drain_wait_cycles();

    bus.set_current_cycle(1000);
    bus.io_write_byte(0x77, 0x00);
    assert_eq!(bus.drain_wait_cycles(), 68);

    let low = bus.io_read_byte(0x71);
    let high = bus.io_read_byte(0x71);
    let count = (high as u16) << 8 | low as u16;
    assert_eq!(count, 968);
}

#[test]
fn timer_full_cycle() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::High, 48000);

    // Set up PIC: unmask all.
    bus.io_write_byte(0x00, 0x11);
    bus.io_write_byte(0x02, 0x08);
    bus.io_write_byte(0x02, 0x80);
    bus.io_write_byte(0x02, 0x1D);
    bus.io_write_byte(0x02, 0x00);

    // Program PIT ch0: word, mode 2, reload = 100
    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0x64);
    bus.io_write_byte(0x71, 0x00);

    // No IRQ yet.
    assert!(!bus.has_irq());

    // fire_cycle = 100 * 20_000_000 / 1_996_800 = 1001
    bus.set_current_cycle(1001);
    assert!(bus.has_irq());

    // Acknowledge
    let vector = bus.acknowledge_irq();
    assert_eq!(vector, 0x08);

    // EOI
    bus.io_write_byte(0x00, 0x20);
    assert!(!bus.has_irq());

    // Mode 2 periodic: re-fires at cycle 2002.
    bus.set_current_cycle(2002);
    assert!(bus.has_irq());
    let vector = bus.acknowledge_irq();
    assert_eq!(vector, 0x08);
}

#[test]
fn timer_value_zero_65536_period() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::High, 48000);

    // Set up PIC: enable only IRQ 0 (timer). Mask IRQ 2 (VSYNC) since this
    // test advances past the VSYNC period.
    bus.io_write_byte(0x00, 0x11);
    bus.io_write_byte(0x02, 0x08);
    bus.io_write_byte(0x02, 0x80);
    bus.io_write_byte(0x02, 0x1D);
    bus.io_write_byte(0x02, 0xFE);

    // PIT ch0: word, mode 2, reload = 0 (means 65536)
    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0x00);
    bus.io_write_byte(0x71, 0x00);

    // fire_cycle = 65536 * 20_000_000 / 1_996_800 = 656_410
    bus.set_current_cycle(656_409);
    assert!(!bus.has_irq());

    bus.set_current_cycle(656_410);
    assert!(bus.has_irq());
}

#[test]
fn control_word_write_clears_irq0() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::High, 48000);

    // Set up PIC.
    bus.io_write_byte(0x00, 0x11);
    bus.io_write_byte(0x02, 0x08);
    bus.io_write_byte(0x02, 0x80);
    bus.io_write_byte(0x02, 0x1D);
    bus.io_write_byte(0x02, 0x00);

    // PIT ch0: mode 2, reload = 100
    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0x64);
    bus.io_write_byte(0x71, 0x00);

    // Fire the timer. fire_cycle = 100 * 20_000_000 / 1_996_800 = 1001
    bus.set_current_cycle(1001);
    assert!(bus.has_irq());

    // Write new control word for ch0 - clears IRQ 0.
    bus.io_write_byte(0x77, 0x34);
    assert!(!bus.has_irq());
}

#[test]
fn pit_latch_command_does_not_clear_irq0() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::High, 48000);

    init_pic(&mut bus);

    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0x64);
    bus.io_write_byte(0x71, 0x00);

    bus.set_current_cycle(1001);
    assert!(
        bus.has_irq(),
        "timer IRQ should be pending before the latch command"
    );

    bus.io_write_byte(0x77, 0x00);

    assert!(
        bus.has_irq(),
        "latching PIT channel 0 must not clear a pending IRQ0"
    );
}

fn init_pic(bus: &mut Pc9801Bus) {
    bus.io_write_byte(0x00, 0x11);
    bus.io_write_byte(0x02, 0x08);
    bus.io_write_byte(0x02, 0x80);
    bus.io_write_byte(0x02, 0x1D);
    bus.io_write_byte(0x02, 0x00);
}

#[test]
fn timer_reprogram_mid_count() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::High, 48000);
    init_pic(&mut bus);

    // PIT ch0: mode 2, reload = 100 -> fire at cycle 1001
    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0x64);
    bus.io_write_byte(0x71, 0x00);

    // Advance to mid-count (cycle 500), no fire yet.
    bus.set_current_cycle(500);
    assert!(!bus.has_irq());

    // Reprogram with new control + counter: reload = 50
    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0x32); // 50
    bus.io_write_byte(0x71, 0x00);

    // fire = 500 + 50*20000000/1996800 = 500 + 500 = 1000
    bus.set_current_cycle(999);
    assert!(!bus.has_irq());

    bus.set_current_cycle(1000);
    assert!(bus.has_irq());
    let vector = bus.acknowledge_irq();
    assert_eq!(vector, 0x08);
}

#[test]
fn timer_reprogram_larger_value_delays_fire() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::High, 48000);
    init_pic(&mut bus);

    // PIT ch0: mode 2, reload = 100 -> fire at cycle 1001
    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0x64);
    bus.io_write_byte(0x71, 0x00);

    // Advance to cycle 500, reprogram with reload = 200
    bus.set_current_cycle(500);
    assert!(!bus.has_irq());

    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0xC8); // 200
    bus.io_write_byte(0x71, 0x00);

    // New fire = 500 + 200*20000000/1996800 = 500 + 2003 = 2503
    // Old event at 1001 should be replaced.
    bus.set_current_cycle(1001);
    assert!(!bus.has_irq(), "old event should be replaced");

    bus.set_current_cycle(2502);
    assert!(!bus.has_irq());

    bus.set_current_cycle(2503);
    assert!(bus.has_irq());
    let vector = bus.acknowledge_irq();
    assert_eq!(vector, 0x08);
}

#[test]
fn pit_channels_1_and_2_no_irq() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
    init_pic(&mut bus);

    // Program PIT ch1: mode 2, reload = 100
    bus.io_write_byte(0x77, 0x74); // ch1: word, mode 2
    bus.io_write_byte(0x73, 0x64);
    bus.io_write_byte(0x73, 0x00);

    // Program PIT ch2: mode 2, reload = 100
    bus.io_write_byte(0x77, 0xB4); // ch2: word, mode 2
    bus.io_write_byte(0x75, 0x64);
    bus.io_write_byte(0x75, 0x00);

    // Advance well past any period - no IRQ should fire.
    bus.set_current_cycle(10_000);
    assert!(!bus.has_irq());
    bus.set_current_cycle(100_000);
    assert!(!bus.has_irq());
}

#[test]
fn multiple_timer_periods_accumulate() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::High, 48000);
    init_pic(&mut bus);

    // PIT ch0: mode 2, reload = 100 -> fire every 1001 cycles
    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0x64);
    bus.io_write_byte(0x71, 0x00);

    // Verify 5 consecutive periodic timer fires.
    for i in 1..=5u64 {
        let fire_cycle = i * 1001;
        bus.set_current_cycle(fire_cycle);
        assert!(
            bus.has_irq(),
            "period {i}: should have IRQ at cycle {fire_cycle}"
        );
        let vector = bus.acknowledge_irq();
        assert_eq!(vector, 0x08, "period {i}: wrong vector");
        bus.io_write_byte(0x00, 0x20); // EOI
        assert!(!bus.has_irq(), "period {i}: should be clear after EOI");
    }
}

#[test]
fn timer_count_readable_while_running() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::High, 48000);
    init_pic(&mut bus);

    // PIT ch0: mode 2, reload = 1000 -> fire at cycle ~10016
    bus.io_write_byte(0x77, 0x34);
    bus.io_write_byte(0x71, 0xE8);
    bus.io_write_byte(0x71, 0x03);

    // Advance to mid-period.
    bus.set_current_cycle(4997);
    assert!(!bus.has_irq());

    // Latch ch0 via I/O.
    bus.io_write_byte(0x77, 0x00);

    // Read counter via I/O.
    let low = bus.io_read_byte(0x71);
    let high = bus.io_read_byte(0x71);
    let count = (high as u16) << 8 | low as u16;

    // elapsed_pit at cycle 4997 = 4997*1996800/20000000 = 498
    // pos = 498 % 1000 = 498, count = 1000 - 498 = 502
    assert_eq!(count, 502);
}

#[test]
fn mouse_timer_irq15_periodic_and_masked() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

    // Unmask only the slave cascade on master and only slave IR5 (IRQ13).
    bus.io_write_byte(0x02, 0x7F);
    bus.io_write_byte(0x0A, 0xDF);

    // 120 Hz mouse timer (bits 1:0 = 0b00) and INT# enable (bit4=0).
    bus.io_write_byte(0xBFDB, 0x00);
    bus.io_write_byte(0x7FDD, 0xE0);

    // 20 MHz / 120 Hz = 166,667 cycles.
    bus.set_current_cycle(166_667);
    assert!(bus.has_irq());
    assert_eq!(bus.acknowledge_irq(), 0x15);
    bus.io_write_byte(0x08, 0x20);
    bus.io_write_byte(0x00, 0x20);

    bus.set_current_cycle(333_334);
    assert!(bus.has_irq());
    assert_eq!(bus.acknowledge_irq(), 0x15);
    bus.io_write_byte(0x08, 0x20);
    bus.io_write_byte(0x00, 0x20);

    // Mask INT# (bit4=1): timer must stop raising IRQ13.
    bus.io_write_byte(0x7FDD, 0xF0);
    bus.set_current_cycle(500_000);
    assert!(!bus.has_irq());
}

#[test]
fn mouse_timer_ignores_upper_bit_writes_to_bfdb() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

    // Port 0xBFDB is write-only on the PC-9801VM (reads return 0xFF).
    assert_eq!(bus.io_read_byte(0xBFDB), 0xFF);

    // Unmask slave cascade on master and slave IR5 (IRQ13).
    bus.io_write_byte(0x02, 0x7F);
    bus.io_write_byte(0x0A, 0xDF);

    // 120 Hz timer, INT# enable (bit4=0).
    bus.io_write_byte(0xBFDB, 0x00);
    bus.io_write_byte(0x7FDD, 0xE0);

    // Write with upper bits set (e.g. 0x27 from jastrike) - must be ignored.
    // Timer should remain at 120 Hz (20 MHz / 120 = 166,667 cycles).
    bus.io_write_byte(0xBFDB, 0x27);
    bus.set_current_cycle(166_667);
    assert!(
        bus.has_irq(),
        "timer must still fire at 120 Hz after ignored write"
    );
    bus.acknowledge_irq();
    bus.io_write_byte(0x08, 0x20);
    bus.io_write_byte(0x00, 0x20);

    // Valid write (bits 1:0 = 0b01, 60 Hz) accepted.
    // 20 MHz / 60 = 333,334 cycles per tick.
    bus.io_write_byte(0xBFDB, 0x01);
    bus.set_current_cycle(166_667 + 333_334);
    assert!(bus.has_irq(), "timer must fire at 60 Hz after valid write");
}

#[test]
fn mouse_timer_fires_regardless_of_ppi_mode_bit3() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

    // Unmask slave cascade on master and slave IR5 (IRQ13).
    bus.io_write_byte(0x02, 0x7F);
    bus.io_write_byte(0x0A, 0xDF);

    // Set PPI mode with bit 3 = 1 (port C upper = input direction).
    // This should NOT prevent the mouse timer from firing.
    bus.io_write_byte(0x7FDF, 0x9B);

    // 120 Hz timer, INT# enable (bit4=0).
    bus.io_write_byte(0xBFDB, 0x00);
    bus.io_write_byte(0x7FDD, 0xE0);

    // 20 MHz / 120 Hz = 166,667 cycles.
    bus.set_current_cycle(166_667);
    assert!(bus.has_irq());
    assert_eq!(bus.acknowledge_irq(), 0x15);
}

#[test]
fn sdip_ports_return_parity_correct_defaults_on_pc9821() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

    // Most SDIP bytes have per-byte odd parity. The BIOS checks this during
    // POST and shows "SET THE SOFTWARE DIP SWITCH" if any check fails.
    // Exception: 0x891E and 0x8A1E share combined odd parity across both bytes.
    // Ref: undoc98 `io_sdip.txt`
    let per_byte_parity_ports: [u16; 10] = [
        0x841E, 0x851E, 0x861E, 0x871E, 0x881E, 0x8B1E, 0x8C1E, 0x8D1E, 0x8E1E, 0x8F1E,
    ];

    for &port in &per_byte_parity_ports {
        let value = bus.io_read_byte(port);
        let ones = value.count_ones();
        assert!(
            ones % 2 == 1,
            "SDIP port {port:#06X} value {value:#04X} has {ones} ones (must be odd parity)"
        );
    }

    // 0x891E + 0x8A1E use combined odd parity (bit 7 of 0x8A1E is the parity
    // bit for the pair). Total 1-bits across both bytes must be odd.
    let modem_a = bus.io_read_byte(0x891E);
    let modem_b = bus.io_read_byte(0x8A1E);
    let combined_ones = modem_a.count_ones() + modem_b.count_ones();
    assert!(
        combined_ones % 2 == 1,
        "SDIP 0x891E+0x8A1E combined {modem_a:#04X}+{modem_b:#04X} has {combined_ones} ones (must be odd)"
    );

    // Verify specific critical defaults:
    // 0x841E: GRPH extended, 512B HDD sectors, RS-232C async, FDD 1/2
    assert_eq!(bus.io_read_byte(0x841E), 0xF8);
    // 0x851E: GDC 2.5 MHz, HDD connected, 25 lines, 80 cols
    assert_eq!(bus.io_read_byte(0x851E), 0xE3);
    // 0x871E bit 5: MEMSW init = 1 (do initialize memory switches)
    assert_ne!(bus.io_read_byte(0x871E) & 0x20, 0);
}

#[test]
fn sdip_read_write_roundtrip() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

    // Write a test value to SDIP port 0x841E and read it back.
    bus.io_write_byte(0x841E, 0x42);
    assert_eq!(bus.io_read_byte(0x841E), 0x42);

    // Write to a different port and verify independence.
    bus.io_write_byte(0x851E, 0x55);
    assert_eq!(bus.io_read_byte(0x851E), 0x55);
    assert_eq!(bus.io_read_byte(0x841E), 0x42);
}

#[test]
fn sdip_bank_selection_via_port_f6() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

    // Read front bank default at offset 0.
    let _front_default = bus.io_read_byte(0x841E);

    // Write a test value to front bank.
    bus.io_write_byte(0x841E, 0xAA);
    assert_eq!(bus.io_read_byte(0x841E), 0xAA);

    // Select back bank via port 0xF6 (0xE0 = back, bit 6 = 1).
    bus.io_write_byte(0xF6, 0xE0);
    let back_value = bus.io_read_byte(0x841E);
    assert_ne!(
        back_value, 0xAA,
        "back bank should be independent from front"
    );

    // Write to back bank.
    bus.io_write_byte(0x841E, 0xBB);
    assert_eq!(bus.io_read_byte(0x841E), 0xBB);

    // Select front bank via port 0xF6 (0xA0 = front, bit 6 = 0).
    bus.io_write_byte(0xF6, 0xA0);
    assert_eq!(
        bus.io_read_byte(0x841E),
        0xAA,
        "front bank should be preserved"
    );

    // Verify back bank value via port 0xF6 again.
    bus.io_write_byte(0xF6, 0xE0);
    assert_eq!(
        bus.io_read_byte(0x841E),
        0xBB,
        "back bank should be preserved"
    );

    // Verify that other 0xF6 values still control A20 (not SDIP bank).
    bus.io_write_byte(0xF6, 0xA0); // back to front
    bus.io_write_byte(0xF6, 0x02); // A20 enable - should NOT switch bank
    assert_eq!(
        bus.io_read_byte(0x841E),
        0xAA,
        "A20 write must not switch bank"
    );
}

#[test]
fn sdip_bank_selection_via_port_8f1f() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

    // Write to front bank.
    bus.io_write_byte(0x841E, 0xCC);

    // Select back bank via port 0x8F1F (0xC0 = back, bit 6 = 1).
    bus.io_write_byte(0x8F1F, 0xC0);
    assert_ne!(bus.io_read_byte(0x841E), 0xCC);

    // Select front bank via port 0x8F1F (0x80 = front, bit 6 = 0).
    bus.io_write_byte(0x8F1F, 0x80);
    assert_eq!(bus.io_read_byte(0x841E), 0xCC);
}

#[test]
fn sdip_ports_not_present_on_pc9801() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);

    // SDIP ports should return open bus (0xFF) on PC-9801 models.
    assert_eq!(bus.io_read_byte(0x841E), 0xFF);
    assert_eq!(bus.io_read_byte(0x8F1E), 0xFF);
}

#[test]
fn wab_relay_default_matches_np21w() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

    // NP21W returns 0xFC | relay_state on reads. Initial relay state = 0,
    // so reads return 0xFC (bits [7:2] high, bits [1:0] clear).
    // Ref: NP21W wab/wab.c:527 (np2wab_ifac)
    assert_eq!(bus.io_read_byte(0x0FAC), 0xFC);

    // Writing updates the relay register.
    bus.io_write_byte(0x0FAC, 0x03);
    assert_eq!(bus.io_read_byte(0x0FAC), 0x03);
}

#[test]
fn port_31_synthesizes_dip_switch_2_from_sdip_on_pc9821() {
    let mut bus = Pc9801Bus::<NoTracing>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);

    // Port 0x31 synthesizes DIP switch 2 from SDIP front bank registers:
    //   bits {7,6,5,3,2,1,0} from register 1 (0x851E)
    //   bit 4 (memsw init) from register 3 (0x871E) bit 5
    //
    // Defaults: SDIP[1]=0xE3, SDIP[3]=0x2C (bit 5 set).
    // Result: (0xE3 & 0xEF) | ((0x2C & 0x20) >> 1) = 0xE3 | 0x10 = 0xF3.
    assert_eq!(bus.io_read_byte(0x31), 0xF3);

    // Write to SDIP register 1: verify port 0x31 uses bits from reg1 except bit 4.
    bus.io_write_byte(0x851E, 0x42);
    // (0x42 & 0xEF) | ((0x2C & 0x20) >> 1) = 0x42 | 0x10 = 0x52
    assert_eq!(bus.io_read_byte(0x31), 0x52);

    // Clear register 3 bit 5 (memsw init = OFF): port 0x31 bit 4 should go low.
    bus.io_write_byte(0x871E, 0x00);
    // (0x42 & 0xEF) | ((0x00 & 0x20) >> 1) = 0x42 | 0x00 = 0x42
    assert_eq!(bus.io_read_byte(0x31), 0x42);

    // Switch to back bank via port 0x8F1F.
    bus.io_write_byte(0x8F1F, 0xC0);

    // Port 0x31 must still return the front bank synthesis, not the back bank.
    assert_eq!(bus.io_read_byte(0x31), 0x42);

    // Write to back bank register 1 via SDIP port 0x851E.
    bus.io_write_byte(0x851E, 0x99);

    // Port 0x31 must still reflect the front bank value.
    assert_eq!(bus.io_read_byte(0x31), 0x42);

    // Switch back to front bank and verify SDIP port 0x851E shows front value.
    bus.io_write_byte(0x8F1F, 0x80);
    assert_eq!(bus.io_read_byte(0x851E), 0x42);
}
