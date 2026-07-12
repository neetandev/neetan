use device::cs4031::{
    CS4031_REG_ROMCS, CS4031_REG_SHADOW_AB, CS4031_REG_SHADOW_READ, CS4031_REG_SHADOW_WRITE,
    CS4031_REG_SOFT_RESET_GATEA20, Cs4031, RegionReadSource, RegionWriteTarget,
};

#[test]
fn config_index_is_one_shot() {
    let mut chipset = Cs4031::new();

    // Selecting a valid index lets one data access read it back.
    chipset.write_config_address(CS4031_REG_ROMCS);
    assert_eq!(chipset.read_config_data(), 0x60);

    // The selection is cleared after the read: a second read returns 0xFF.
    assert_eq!(chipset.read_config_data(), 0xFF);

    // An out-of-range index is never valid.
    chipset.write_config_address(0x40);
    assert_eq!(chipset.read_config_data(), 0xFF);
}

#[test]
fn a_and_b_shadow_regions_select_internal_dram() {
    let mut chipset = Cs4031::new();
    assert!(!chipset.ab_region_internal(0));
    assert!(!chipset.ab_region_internal(1));

    chipset.write_config_address(CS4031_REG_SHADOW_AB);
    let effects = chipset.write_config_data(0xFF);
    assert!(effects.shadow_map_changed);
    assert!(chipset.ab_region_internal(0));
    assert!(chipset.ab_region_internal(1));
    assert_eq!(chipset.registers[CS4031_REG_SHADOW_AB as usize], 0xF3);
}

#[test]
fn romcs_reset_default_maps_e_and_f_as_rom() {
    let chipset = Cs4031::new();

    // ROMCS reset value 0x60 -> regions 5 (E0000) and 6 (F0000) enabled.
    assert_eq!(chipset.region_read_source(5), RegionReadSource::Rom);
    assert_eq!(chipset.region_read_source(6), RegionReadSource::Rom);

    // The C0000-DFFFF regions are undecoded at power-on.
    for region in 0..5 {
        assert_eq!(
            chipset.region_read_source(region),
            RegionReadSource::OpenBus
        );
    }
}

#[test]
fn shadow_read_overrides_romcs() {
    let mut chipset = Cs4031::new();

    // Enable shadow read for region 6 while ROMCS is still set.
    chipset.write_config_address(CS4031_REG_SHADOW_READ);
    let effects = chipset.write_config_data(1 << 6);
    assert!(effects.shadow_map_changed);

    // Shadow read wins: the region reads from RAM, not ROM.
    assert_eq!(chipset.region_read_source(6), RegionReadSource::Ram);
}

#[test]
fn shadow_copy_sequence() {
    let mut chipset = Cs4031::new();

    // AMI shadow-copy step 1: ROMCS on (region 6 already set), shadow write on
    // -> reads ROM, writes RAM.
    chipset.write_config_address(CS4031_REG_SHADOW_WRITE);
    chipset.write_config_data(1 << 6);
    assert_eq!(chipset.region_read_source(6), RegionReadSource::Rom);
    assert_eq!(chipset.region_write_target(6), RegionWriteTarget::Ram);

    // Step 2: flip shadow read on -> reads RAM.
    chipset.write_config_address(CS4031_REG_SHADOW_READ);
    chipset.write_config_data(1 << 6);
    assert_eq!(chipset.region_read_source(6), RegionReadSource::Ram);

    // Step 3: write-protect (shadow write off) -> writes blocked.
    chipset.write_config_address(CS4031_REG_SHADOW_WRITE);
    chipset.write_config_data(0);
    assert_eq!(chipset.region_write_target(6), RegionWriteTarget::Blocked);
}

#[test]
fn fast_reset_pulses_once_on_rising_edge() {
    let mut chipset = Cs4031::new();

    // Bit 0 rising edge (0 -> 1) pulses the CPU reset.
    let effects = chipset.write_sysctrl(0x01);
    assert!(effects.cpu_reset_pulse);

    // Holding bit 0 high does not pulse again.
    let effects = chipset.write_sysctrl(0x01);
    assert!(!effects.cpu_reset_pulse);

    // Releasing then re-asserting pulses again.
    chipset.write_sysctrl(0x00);
    let effects = chipset.write_sysctrl(0x01);
    assert!(effects.cpu_reset_pulse);
}

#[test]
fn fast_gate_a20_forces_line_high() {
    let mut chipset = Cs4031::new();
    assert!(!chipset.a20_enabled());

    // Fast Gate A20 (port 0x92 bit 1) forces A20 on regardless of the KBC gate.
    chipset.write_sysctrl(0x02);
    assert!(chipset.a20_enabled());
    assert_eq!(chipset.read_sysctrl() & 0x02, 0x02);
}

#[test]
fn a20_selects_emulated_or_external_gate() {
    let mut chipset = Cs4031::new();

    // With emulation off (reg 0x1C bit 5 clear), the external gate selects A20.
    chipset.set_ext_gate_a20(true);
    assert!(chipset.a20_enabled());
    chipset.set_ext_gate_a20(false);
    assert!(!chipset.a20_enabled());

    // Enable Gate A20 emulation (reg 0x1C bit 5) and drive the emulated gate
    // high via a blocked D1 + data sequence (reg 0x1C bit 7 also set).
    chipset.write_config_address(CS4031_REG_SOFT_RESET_GATEA20);
    chipset.write_config_data(0xA0); // bit 7 blocking + bit 5 emulation
    chipset.filter_keyboard_command(0xD1);
    chipset.filter_keyboard_data(0x02); // bit 1 -> emulated Gate A20 high
    assert!(chipset.a20_enabled());

    // The external gate is now ignored.
    chipset.set_ext_gate_a20(false);
    assert!(chipset.a20_enabled());
}

#[test]
fn d1_data_consumed_when_blocking() {
    let mut chipset = Cs4031::new();

    // Enable command blocking (reg 0x1C bit 7).
    chipset.write_config_address(CS4031_REG_SOFT_RESET_GATEA20);
    chipset.write_config_data(0x80);

    // D1 is not forwarded while blocking.
    assert!(!chipset.filter_keyboard_command(0xD1).forward);

    // The following data byte is consumed by the chipset, not forwarded.
    assert!(!chipset.filter_keyboard_data(0x02).forward);
}

#[test]
fn self_test_command_always_forwards() {
    let mut chipset = Cs4031::new();

    // Even with command blocking enabled, the self-test 0xAA is forwarded.
    chipset.write_config_address(CS4031_REG_SOFT_RESET_GATEA20);
    chipset.write_config_data(0x80);
    assert!(chipset.filter_keyboard_command(0xAA).forward);
}

#[test]
fn emulated_keyboard_reset_requests_cpu_reset() {
    let mut chipset = Cs4031::new();
    chipset.write_config_address(CS4031_REG_SOFT_RESET_GATEA20);
    chipset.write_config_data(0x90);

    let pulse = chipset.filter_keyboard_command(0xFE);
    assert!(!pulse.forward);
    assert!(pulse.cpu_reset_pulse);

    chipset.filter_keyboard_command(0xD1);
    let output_port = chipset.filter_keyboard_data(0x00);
    assert!(!output_port.forward);
    assert!(output_port.cpu_reset_pulse);
}

#[test]
fn port_b_writable_bits_and_effects() {
    let mut chipset = Cs4031::new();

    // Bits 0 (timer2 gate) and 1 (speaker data) set.
    let effects = chipset.write_port_b(0x03);
    assert!(effects.timer2_gate);
    assert!(effects.speaker_data);

    // The upper bits are read-only; the refresh (bit 4) and timer2-out (bit 5)
    // bits come from the live arguments.
    let value = chipset.read_port_b(true, true);
    assert_eq!(value & 0x30, 0x30);
    assert_eq!(value & 0x03, 0x03);
}

#[test]
fn rtc_nmi_write_returns_address_and_sets_mask() {
    let mut chipset = Cs4031::new();

    // Bit 7 set masks the NMI; bits 6:0 are the RTC address.
    let address = chipset.write_rtc_nmi(0x8A);
    assert_eq!(address, 0x0A);
    assert!(!chipset.nmi_enabled);

    // Bit 7 clear enables the NMI again.
    let address = chipset.write_rtc_nmi(0x0B);
    assert_eq!(address, 0x0B);
    assert!(chipset.nmi_enabled);
}
