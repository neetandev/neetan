//! Scripted Z8530 walk mirroring the Human68k IOCS mouse driver.

use device::z8530::{SccChannel, SccWriteEffect, Z8530};

/// Writes a register through the WR0 pointer protocol.
fn write_register(scc: &mut Z8530, channel: SccChannel, register: u8, value: u8) -> SccWriteEffect {
    assert_eq!(
        scc.write_control(channel, register),
        SccWriteEffect::None,
        "selecting a register must not produce a side effect"
    );
    scc.write_control(channel, value)
}

/// Runs one MSCTRL polling round, returning the delivered packet.
fn poll_mouse(scc: &mut Z8530, tick: &mut u64, packet: [u8; 3]) -> [u8; 3] {
    // MSCTRL pulse: RTS low, then high.
    write_register(scc, SccChannel::B, 5, 0x60);
    let effect = write_register(scc, SccChannel::B, 5, 0x62);
    assert_eq!(effect, SccWriteEffect::MouseRequestEdge);
    scc.load_mouse_packet(packet, *tick);

    let mut received = [0u8; 3];
    for byte in &mut received {
        assert!(!scc.irq_asserted(), "the byte is still on the wire");
        *tick += scc.mouse_byte_duration_ticks();
        scc.advance_to(*tick);
        assert!(scc.irq_asserted(), "each packet byte raises an interrupt");
        let vector = scc.acknowledge_interrupt().expect("a vector is pending");
        assert_eq!(vector, 0x40 | (2 << 1));
        *byte = scc.read_data(SccChannel::B);
        assert_eq!(scc.write_control(SccChannel::B, 0x38), SccWriteEffect::None);
    }
    assert!(!scc.irq_asserted(), "the chain ends after three bytes");
    received
}

#[test]
fn human68k_mouse_initialization_and_read_loop() {
    let mut scc = Z8530::new();

    // IOCS _MS_INIT style setup on channel B: hardware reset, vector 0x40
    // with VIS status-low, receive interrupts on, 4800 bps, Rx/Tx enabled.
    write_register(&mut scc, SccChannel::B, 9, 0xC0);
    write_register(&mut scc, SccChannel::B, 4, 0x4C);
    write_register(&mut scc, SccChannel::B, 2, 0x40);
    write_register(&mut scc, SccChannel::B, 3, 0xC0);
    write_register(&mut scc, SccChannel::B, 5, 0x60);
    write_register(&mut scc, SccChannel::B, 12, 31);
    write_register(&mut scc, SccChannel::B, 13, 0);
    write_register(&mut scc, SccChannel::B, 14, 0x03);
    write_register(&mut scc, SccChannel::B, 3, 0xC1);
    write_register(&mut scc, SccChannel::B, 1, 0x10);
    write_register(&mut scc, SccChannel::B, 9, 0x09);

    assert!(!scc.irq_asserted());

    // A motion packet, an idle packet, and a button-press packet.
    let mut tick = 0;
    assert_eq!(
        poll_mouse(&mut scc, &mut tick, [0x00, 0x05, 0xFB]),
        [0x00, 0x05, 0xFB]
    );
    assert_eq!(
        poll_mouse(&mut scc, &mut tick, [0x00, 0x00, 0x00]),
        [0x00, 0x00, 0x00]
    );
    assert_eq!(
        poll_mouse(&mut scc, &mut tick, [0x03, 0x00, 0x00]),
        [0x03, 0x00, 0x00]
    );
}
