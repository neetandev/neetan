use device::i8255_mouse_ppi::I8255MousePpi;

#[test]
fn no_mouse_returns_0x70() {
    let mut ppi = I8255MousePpi::new();
    ppi.state.mouse_connected = false;
    assert_eq!(ppi.read_port_a(0), 0x70);
}

#[test]
fn default_buttons_no_press() {
    let mut ppi = I8255MousePpi::new();
    // Default: all buttons released (active-low high).
    // buttons = 0xE0, ret = (0xE0 & 0xF0) | 0x40 = 0xE0.
    let result = ppi.read_port_a(0);
    assert_eq!(result & 0xF0, 0xE0);
}

#[test]
fn left_button_press() {
    let mut ppi = I8255MousePpi::new();
    ppi.set_buttons(true, false, false);
    let result = ppi.read_port_a(0);
    assert_eq!(result & 0x80, 0x00, "bit 7 clear = LEFT pressed");
    assert_eq!(result & 0x40, 0x40, "bit 6 set = MIDDLE not pressed");
    assert_eq!(result & 0x20, 0x20, "bit 5 set = RIGHT not pressed");
}

#[test]
fn right_button_press() {
    let mut ppi = I8255MousePpi::new();
    ppi.set_buttons(false, true, false);
    let result = ppi.read_port_a(0);
    assert_eq!(result & 0x80, 0x80, "bit 7 set = LEFT not pressed");
    assert_eq!(result & 0x40, 0x40, "bit 6 set = MIDDLE not pressed");
    assert_eq!(result & 0x20, 0x00, "bit 5 clear = RIGHT pressed");
}

#[test]
fn hc_latch_captures_and_resets() {
    let mut ppi = I8255MousePpi::new();
    ppi.set_cpu_clock(8_000_000);

    // Directly set accumulators for deterministic testing.
    ppi.state.counter_x = 10;
    ppi.state.counter_y = -5;
    ppi.state.pending_x = 0;
    ppi.state.pending_y = 0;

    ppi.latch(100_000);

    assert_eq!(ppi.state.latch_x, 10);
    assert_eq!(ppi.state.latch_y, -5);
    assert_eq!(ppi.state.counter_x, 0);
    assert_eq!(ppi.state.counter_y, 0);
}

#[test]
fn latch_clamps_to_signed_byte_range() {
    let mut ppi = I8255MousePpi::new();
    ppi.set_cpu_clock(8_000_000);
    ppi.state.counter_x = 200;
    ppi.state.counter_y = -200;

    ppi.latch(0);
    assert_eq!(ppi.state.latch_x, 127);
    assert_eq!(ppi.state.latch_y, -128);
}

#[test]
fn nibble_read_protocol() {
    let mut ppi = I8255MousePpi::new();
    ppi.set_cpu_clock(8_000_000);

    // Set latched values directly for testing nibble readout.
    ppi.state.latch_x = 0x3A; // Low nibble = 0xA, high nibble = 0x3
    ppi.state.latch_y = -8; // -8 as i16 = 0xFFF8; low = 0x8, high >> 4 = 0xF

    // HC=1 (read latched), SXY=0 (X), SHL=0 (low nibble)
    ppi.state.port_c = 0x80;
    assert_eq!(ppi.read_port_a(0) & 0x0F, 0x0A);

    // HC=1, SXY=0, SHL=1 (high nibble)
    ppi.state.port_c = 0xA0;
    assert_eq!(ppi.read_port_a(0) & 0x0F, 0x03);

    // HC=1, SXY=1 (Y), SHL=0 (low nibble)
    ppi.state.port_c = 0xC0;
    assert_eq!(ppi.read_port_a(0) & 0x0F, 0x08);

    // HC=1, SXY=1, SHL=1 (high nibble)
    ppi.state.port_c = 0xE0;
    assert_eq!(ppi.read_port_a(0) & 0x0F, 0x0F);
}

#[test]
fn bsr_set_hc_triggers_rising_edge() {
    let mut ppi = I8255MousePpi::new();
    ppi.state.port_c = 0x00; // HC = 0

    // BSR command to set bit 7: value = (7 << 1) | 1 = 0x0F
    let rising = ppi.write_control(0x0F, 0);
    assert!(rising, "HC should have risen");
    assert_ne!(ppi.state.port_c & 0x80, 0);
}

#[test]
fn bsr_reset_hc_no_rising_edge() {
    let mut ppi = I8255MousePpi::new();
    ppi.state.port_c = 0x80; // HC = 1

    // BSR command to reset bit 7: value = (7 << 1) | 0 = 0x0E
    let rising = ppi.write_control(0x0E, 0);
    assert!(!rising, "HC should not have risen (it fell)");
    assert_eq!(ppi.state.port_c & 0x80, 0);
}

#[test]
fn mode_set_resets_port_c() {
    let mut ppi = I8255MousePpi::new();
    ppi.state.port_c = 0x80; // HC = 1

    let rising = ppi.write_control(0x93, 0); // Mode set
    assert!(!rising, "Mode set should not produce rising edge");
    assert_eq!(ppi.state.port_c, 0x00);
}

#[test]
fn port_c_write_detects_hc_rising_edge() {
    let mut ppi = I8255MousePpi::new();
    ppi.state.port_c = 0x00; // HC = 0

    let rising = ppi.write_port_c(0x80, 0); // HC -> 1
    assert!(rising);

    // Writing again with HC still 1 should not trigger.
    let rising = ppi.write_port_c(0x80, 0);
    assert!(!rising);
}
