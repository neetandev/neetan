use device::i8042_kbc::I8042Kbc;

/// Delivers pending output bytes and collects them with their IRQ1 flags.
fn drain(kbc: &mut I8042Kbc) -> Vec<(u8, bool)> {
    let mut out = Vec::new();
    while let Some(irq1) = kbc.deliver_next() {
        let (byte, _) = kbc.read_data();
        out.push((byte, irq1));
    }
    out
}

#[test]
fn self_test_reports_pass_and_sets_system_flag() {
    let mut kbc = I8042Kbc::new();
    let effects = kbc.write_command(0xAA);
    assert!(effects.schedule_delivery);

    assert_eq!(kbc.deliver_next(), Some(false)); // controller response, no IRQ1
    assert_eq!(kbc.read_status() & 0x01, 0x01); // OBF set
    let (byte, _) = kbc.read_data();
    assert_eq!(byte, 0x55);
    assert_eq!(kbc.read_status() & 0x04, 0x04); // SYS set
}

#[test]
fn interface_test_reports_no_error() {
    let mut kbc = I8042Kbc::new();
    kbc.write_command(0xAB);
    kbc.deliver_next();
    let (byte, _) = kbc.read_data();
    assert_eq!(byte, 0x00);
}

#[test]
fn command_byte_round_trip() {
    let mut kbc = I8042Kbc::new();
    // Write command byte (0x60) then the data byte.
    kbc.write_command(0x60);
    kbc.write_data(0x45);
    assert_eq!(kbc.command_byte, 0x45);

    // Read command byte (0x20).
    kbc.write_command(0x20);
    kbc.deliver_next();
    let (byte, _) = kbc.read_data();
    assert_eq!(byte, 0x45);
}

#[test]
fn input_port_has_at_jumper_defaults() {
    let mut kbc = I8042Kbc::new();
    kbc.write_command(0xC0);
    kbc.deliver_next();
    assert_eq!(kbc.read_data().0, 0xBF);
}

#[test]
fn output_port_write_toggles_a20_and_reset() {
    let mut kbc = I8042Kbc::new();
    assert!(kbc.a20_enabled());

    // Write output port (0xD1) with A20 bit clear -> A20 disabled, no reset.
    kbc.write_command(0xD1);
    let effects = kbc.write_data(0x01); // reset high, A20 low
    assert!(effects.output_port_changed);
    assert!(!effects.reset_pulse);
    assert!(!kbc.a20_enabled());

    // Now clear the reset line (bit 0) -> reset pulse.
    kbc.write_command(0xD1);
    let effects = kbc.write_data(0x02); // reset low, A20 high
    assert!(effects.reset_pulse);
    assert!(kbc.a20_enabled());
}

#[test]
fn pulse_command_fe_requests_reset() {
    let mut kbc = I8042Kbc::new();
    let effects = kbc.write_command(0xFE); // pulse bit 0 (reset) low
    assert!(effects.reset_pulse);

    // 0xFF pulses nothing.
    let effects = kbc.write_command(0xFF);
    assert!(!effects.reset_pulse);
}

#[test]
fn write_output_buffer_injects_keyboard_byte() {
    let mut kbc = I8042Kbc::new();
    // Enable IRQ1 so injected keyboard bytes raise it.
    kbc.write_command(0x60);
    kbc.write_data(0x01);

    kbc.write_command(0xD2);
    kbc.write_data(0x5A);
    assert_eq!(kbc.deliver_next(), Some(true)); // IRQ1 raised
    let (byte, _) = kbc.read_data();
    assert_eq!(byte, 0x5A);
}

#[test]
fn keyboard_reset_acks_then_passes_bat() {
    let mut kbc = I8042Kbc::new();
    kbc.write_data(0xFF); // keyboard reset command
    let delivered = drain(&mut kbc);
    assert_eq!(
        delivered.iter().map(|(b, _)| *b).collect::<Vec<_>>(),
        vec![0xFA, 0xAA]
    );
}

#[test]
fn irq1_gated_by_command_byte_bit0() {
    let mut kbc = I8042Kbc::new();
    // IRQ1 disabled (default command byte 0): keyboard scancodes do not raise it.
    kbc.keyboard.push_scancode(0x1C);
    assert_eq!(kbc.deliver_next(), Some(false));
    kbc.read_data();

    // Enable IRQ1.
    kbc.write_command(0x60);
    kbc.write_data(0x01);
    kbc.keyboard.push_scancode(0x1C);
    assert_eq!(kbc.deliver_next(), Some(true));
}

#[test]
fn translation_folds_break_prefix() {
    let mut kbc = I8042Kbc::new();
    // Enable translation (bit 6) and IRQ1 (bit 0).
    kbc.write_command(0x60);
    kbc.write_data(0x41);

    // Make code for 'A' (set 2 0x1C) -> set 1 0x1E.
    kbc.keyboard.push_scancode(0x1C);
    kbc.deliver_next();
    assert_eq!(kbc.read_data().0, 0x1E);

    // Break sequence 0xF0 0x1C -> set 1 0x1E | 0x80 = 0x9E.
    kbc.keyboard.push_scancode(0xF0);
    kbc.keyboard.push_scancode(0x1C);
    kbc.deliver_next();
    assert_eq!(kbc.read_data().0, 0x9E);
}

#[test]
fn translation_off_passes_raw_bytes() {
    let mut kbc = I8042Kbc::new();
    // Command byte 0 -> translation off.
    kbc.keyboard.push_scancode(0x1C);
    kbc.deliver_next();
    assert_eq!(kbc.read_data().0, 0x1C);
}

#[test]
fn disabled_keyboard_holds_scancodes() {
    let mut kbc = I8042Kbc::new();
    // Disable the keyboard interface (0xAD).
    kbc.write_command(0xAD);
    kbc.keyboard.push_scancode(0x1C);
    assert_eq!(kbc.deliver_next(), None); // nothing delivered while disabled

    // Re-enable (0xAE) and the byte flows.
    kbc.write_command(0xAE);
    assert!(kbc.deliver_next().is_some());
}

#[test]
fn delivery_waits_while_buffer_full() {
    let mut kbc = I8042Kbc::new();
    kbc.keyboard.push_scancode(0x1C);
    kbc.keyboard.push_scancode(0x23);
    assert!(kbc.deliver_next().is_some());
    // Buffer full: a second delivery is blocked until the byte is read.
    assert_eq!(kbc.deliver_next(), None);
    kbc.read_data();
    assert!(kbc.deliver_next().is_some());
}
