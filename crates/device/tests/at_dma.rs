use device::at_dma::AtDma;

/// Programs a byte-channel (0-3) address and count through the standard ports.
fn program_byte_channel(dma: &mut AtDma, channel: usize, address: u16, count: u16) {
    let base = (channel * 2) as u16;
    dma.io_write(0x0C, 0); // clear flip-flop
    dma.io_write(base, address as u8);
    dma.io_write(base, (address >> 8) as u8);
    dma.io_write(base + 1, count as u8);
    dma.io_write(base + 1, (count >> 8) as u8);
}

/// Programs a word-channel (5-7) address and count through the 0xC0-0xDF ports.
fn program_word_channel(dma: &mut AtDma, channel: usize, address: u16, count: u16) {
    let local = (channel - 4) as u16;
    let base = 0xC0 + local * 4;
    dma.io_write(0xD8, 0); // clear flip-flop
    dma.io_write(base, address as u8);
    dma.io_write(base, (address >> 8) as u8);
    dma.io_write(base + 2, count as u8);
    dma.io_write(base + 2, (count >> 8) as u8);
}

#[test]
fn page_file_reads_back_all_sixteen_bytes() {
    let mut dma = AtDma::new();
    for port in 0x80..=0x8F {
        dma.io_write(port, (port & 0xFF) as u8);
    }
    for port in 0x80..=0x8F {
        assert_eq!(dma.io_read(port), Some((port & 0xFF) as u8));
    }
}

#[test]
fn controller1_decodes_only_even_ports() {
    let mut dma = AtDma::new();
    // Odd ports in the 0xC0-0xDF window read open bus and ignore writes.
    assert_eq!(dma.io_read(0xC1), Some(0xFF));
    // A write to an odd port must not disturb an even register.
    dma.io_write(0xD8, 0); // clear flip-flop
    dma.io_write(0xC0, 0x11);
    dma.io_write(0xC1, 0x99); // odd: ignored
    dma.io_write(0xC0, 0x22);
    dma.io_write(0xD8, 0);
    assert_eq!(dma.io_read(0xC0), Some(0x11));
    assert_eq!(dma.io_read(0xC0), Some(0x22));
}

#[test]
fn address_and_count_registers_round_trip() {
    let mut dma = AtDma::new();
    program_byte_channel(&mut dma, 2, 0x1234, 0x00FF);
    dma.io_write(0x0C, 0); // clear flip-flop for reading
    let base = 4; // channel 2 address port
    assert_eq!(dma.io_read(base), Some(0x34));
    assert_eq!(dma.io_read(base), Some(0x12));
    assert_eq!(dma.io_read(base + 1), Some(0xFF));
    assert_eq!(dma.io_read(base + 1), Some(0x00));
}

#[test]
fn byte_channel_address_composition() {
    let mut dma = AtDma::new();
    dma.io_write(0x80 + 7, 0x34); // ch0 page register
    program_byte_channel(&mut dma, 0, 0x1000, 0x000F);
    // Unmask channel 0 and the cascade (controller 1 channel 0).
    dma.io_write(0x0A, 0x00); // controller 0 single-mask clear ch0
    dma.io_write(0xD4, 0x00); // controller 1 single-mask clear ch0 (cascade)
    dma.io_write(0x0B, 0x04); // channel 0 device-to-memory transfer

    assert!(dma.channel_unmasked(0));
    let result = dma.transfer_write_to_memory(0, &[0xAA, 0xBB]);
    assert_eq!(result.writes[0], (0x0034_1000, 0xAA));
    assert_eq!(result.writes[1], (0x0034_1001, 0xBB));
}

#[test]
fn word_channel_address_composition_and_word_count() {
    let mut dma = AtDma::new();
    dma.io_write(0x80 + 0xB, 0x12); // ch5 page register
    program_word_channel(&mut dma, 5, 0x0100, 0x0001); // 2 words before TC
    // Mode for local channel 1 (system ch5): single, increment.
    dma.io_write(0xD6, 0x05);

    let result = dma.transfer_write_to_memory(5, &[0x11, 0x22, 0x33, 0x44]);
    // First word at (0x12 << 16) | (0x0100 << 1) = 0x120200.
    assert_eq!(result.writes[0], (0x0012_0200, 0x11));
    assert_eq!(result.writes[1], (0x0012_0201, 0x22));
    // Second word: address advanced by one word to 0x0101 -> 0x120202.
    assert_eq!(result.writes[2], (0x0012_0202, 0x33));
    assert_eq!(result.writes[3], (0x0012_0203, 0x44));
    assert!(result.terminal_count);
}

#[test]
fn terminal_count_stops_byte_transfer() {
    let mut dma = AtDma::new();
    dma.io_write(0x80 + 7, 0x00);
    program_byte_channel(&mut dma, 0, 0x0000, 0x0002); // count 2 -> 3 transfers
    dma.io_write(0x0B, 0x04); // mode ch0: device-to-memory, increment

    let result = dma.transfer_write_to_memory(0, &[1, 2, 3, 4, 5]);
    assert_eq!(result.writes.len(), 3);
    assert!(result.terminal_count);
    // Status TC bit for channel 0 is set (and cleared on read).
    assert_eq!(dma.io_read(0x08), Some(0x01));
    assert_eq!(dma.io_read(0x08), Some(0x00));
}

#[test]
fn auto_init_reloads_word_channel() {
    let mut dma = AtDma::new();
    dma.io_write(0x80 + 0xB, 0x00);
    program_word_channel(&mut dma, 5, 0x0010, 0x0000); // 1 word then TC
    dma.io_write(0xD6, 0x05 | 0x10); // mode ch5: write, auto-init on, increment

    let result = dma.transfer_write_to_memory(5, &[0xEE, 0xFF]);
    assert!(result.terminal_count);
    // Auto-init reloaded the address to its programmed start (0x0010).
    let ch = &dma.controllers[1].state.channels[1];
    assert_eq!(ch.address, 0x0010);
    assert_eq!(ch.count, 0x0000);
}

#[test]
fn master_clear_resets_controller() {
    let mut dma = AtDma::new();
    dma.io_write(0x0B, 0x00);
    program_byte_channel(&mut dma, 1, 0xABCD, 0x0010);
    dma.io_write(0x0D, 0); // master clear on controller 0
    // After master clear all channels are masked again.
    assert!(!dma.controllers[0].channel_unmasked(1));
}

#[test]
fn cascade_masking_blocks_byte_channels() {
    let mut dma = AtDma::new();
    dma.io_write(0x0A, 0x00); // unmask controller 0 channel 0
    // Cascade channel (controller 1 channel 0) is masked at power-on.
    assert!(!dma.channel_unmasked(0));
    // Unmask the cascade -> byte channel becomes usable.
    dma.io_write(0xD4, 0x00);
    assert!(dma.channel_unmasked(0));
}

#[test]
fn verify_transfer_advances_without_writing_memory() {
    let mut dma = AtDma::new();
    dma.io_write(0x87, 0x04);
    program_byte_channel(&mut dma, 0, 0x2741, 0x0001);
    dma.io_write(0x0B, 0x00);

    let result = dma.transfer_write_to_memory(0, &[0xAA, 0xBB]);

    assert!(result.writes.is_empty());
    assert!(result.terminal_count);
    let channel = &dma.controllers[0].state.channels[0];
    assert_eq!(channel.address, 0x2743);
    assert_eq!(channel.count, 0xFFFF);
}
