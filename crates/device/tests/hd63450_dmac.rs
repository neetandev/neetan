//! Integration tests for the HD63450 DMA controller.

use device::hd63450_dmac::{
    DmacBusFault, DmacBusPort, ERROR_BASE_ADDRESS, ERROR_BASE_BUS, ERROR_BASE_COUNT,
    ERROR_CONFIGURATION, ERROR_DEVICE_BUS, ERROR_MEMORY_ADDRESS, ERROR_MEMORY_BUS,
    ERROR_MEMORY_COUNT, ERROR_SOFTWARE_ABORT, ERROR_TIMING, Hd63450Dmac,
};

/// Size of the linear test bus; accesses beyond it fault.
const BUS_SIZE: usize = 0x40000;

/// A linear big-endian test bus; out-of-range accesses fault.
struct TestBus {
    data: Vec<u8>,
}

impl TestBus {
    fn new() -> Self {
        Self {
            data: vec![0; BUS_SIZE],
        }
    }
}

impl DmacBusPort for TestBus {
    fn read_byte(&mut self, address: u32) -> Result<u8, DmacBusFault> {
        self.data.get(address as usize).copied().ok_or(DmacBusFault)
    }

    fn read_word(&mut self, address: u32) -> Result<u16, DmacBusFault> {
        let high = self.read_byte(address)?;
        let low = self.read_byte(address + 1)?;
        Ok((u16::from(high) << 8) | u16::from(low))
    }

    fn write_byte(&mut self, address: u32, value: u8) -> Result<(), DmacBusFault> {
        let slot = self.data.get_mut(address as usize).ok_or(DmacBusFault)?;
        *slot = value;
        Ok(())
    }

    fn write_word(&mut self, address: u32, value: u16) -> Result<(), DmacBusFault> {
        self.write_byte(address, (value >> 8) as u8)?;
        self.write_byte(address + 1, value as u8)
    }
}

/// Computes the register offset of `register` on `channel`.
fn offset(channel: u8, register: u8) -> u8 {
    (channel << 6) | register
}

fn write(dmac: &mut Hd63450Dmac, bus: &mut TestBus, channel: u8, register: u8, value: u8) {
    dmac.write_register(offset(channel, register), value, bus, 0);
}

fn write_u16(dmac: &mut Hd63450Dmac, bus: &mut TestBus, channel: u8, register: u8, value: u16) {
    write(dmac, bus, channel, register, (value >> 8) as u8);
    write(dmac, bus, channel, register + 1, value as u8);
}

fn write_u32(dmac: &mut Hd63450Dmac, bus: &mut TestBus, channel: u8, register: u8, value: u32) {
    for byte_index in 0..4 {
        let shift = 8 * (3 - byte_index);
        write(
            dmac,
            bus,
            channel,
            register + byte_index,
            (value >> shift) as u8,
        );
    }
}

fn read_u32(dmac: &mut Hd63450Dmac, channel: u8, register: u8) -> u32 {
    let mut value = 0u32;
    for byte_index in 0..4 {
        value =
            (value << 8) | u32::from(dmac.read_register(offset(channel, register + byte_index)));
    }
    value
}

/// Programs DCR/OCR/SCR/MTC/MAR/DAR for a transfer on `channel`.
#[allow(clippy::too_many_arguments)]
fn program(
    dmac: &mut Hd63450Dmac,
    bus: &mut TestBus,
    channel: u8,
    dcr: u8,
    ocr: u8,
    scr: u8,
    count: u16,
    memory_address: u32,
    device_address: u32,
) {
    write(dmac, bus, channel, 0x04, dcr);
    write(dmac, bus, channel, 0x05, ocr);
    write(dmac, bus, channel, 0x06, scr);
    write_u16(dmac, bus, channel, 0x0A, count);
    write_u32(dmac, bus, channel, 0x0C, memory_address);
    write_u32(dmac, bus, channel, 0x14, device_address);
}

/// Starts `channel` with interrupts enabled at clock 0.
fn start(dmac: &mut Hd63450Dmac, bus: &mut TestBus, channel: u8) {
    write(dmac, bus, channel, 0x07, 0x88);
}

/// DCR: 68000 dual address, 16-bit device port.
const DCR_16_BIT: u8 = 0x08;

/// DCR: 68000 dual address, 8-bit device port.
const DCR_8_BIT: u8 = 0x00;

/// SCR: memory increments, device increments.
const SCR_BOTH_UP: u8 = 0x05;

/// SCR: memory increments, device static.
const SCR_MEMORY_UP: u8 = 0x04;

#[test]
fn niv_and_eiv_reset_to_0x0f() {
    let mut dmac = Hd63450Dmac::new();
    for channel in 0..4u8 {
        assert_eq!(dmac.read_register(offset(channel, 0x25)), 0x0F);
        assert_eq!(dmac.read_register(offset(channel, 0x27)), 0x0F);
    }
    assert!(!dmac.irq_asserted());
    assert_eq!(dmac.next_work_clock(), None);
}

#[test]
fn word_transfers_copy_memory_to_device_at_max_rate() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    for (index, byte) in bus.data[0x1000..0x1010].iter_mut().enumerate() {
        *byte = index as u8;
    }
    // OCR: mem->dev, word size, no chain, auto max rate.
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        8,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);

    assert_eq!(
        &bus.data[0x2000..0x2010],
        &bus.data[0x1000..0x1010].to_vec()[..]
    );
    assert!(!dmac.channel_active(0));
    assert_eq!(dmac.read_register(offset(0, 0x00)) & 0x80, 0x80, "COC set");
    assert!(dmac.irq_asserted());
    // 8 operands x (read 5 + write 5) clocks.
    assert_eq!(dmac.take_consumed_clocks(), 80);
    assert_eq!(dmac.take_channel_completions(), 0x01);
    assert_eq!(read_u32(&mut dmac, 0, 0x0C), 0x1010, "MAR stepped by 16");
    assert_eq!(read_u32(&mut dmac, 0, 0x14), 0x2010, "DAR stepped by 16");
}

#[test]
fn word_transfers_copy_device_to_memory() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x2000] = 0xAB;
    bus.data[0x2001] = 0xCD;
    // OCR: dev->mem, word size, auto max.
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x91,
        SCR_BOTH_UP,
        1,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(&bus.data[0x1000..0x1002], &[0xAB, 0xCD]);
}

#[test]
fn long_transfers_move_four_bytes_per_operand() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x1000..0x1008].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    // OCR: mem->dev, long size, auto max.
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x21,
        SCR_BOTH_UP,
        2,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(&bus.data[0x2000..0x2008], &[1, 2, 3, 4, 5, 6, 7, 8]);
    // 2 operands x (2 reads + 2 writes) x 5 clocks.
    assert_eq!(dmac.take_consumed_clocks(), 40);
}

#[test]
fn packed_bytes_use_word_cycles_and_carries() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x1000..0x1004].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
    // OCR: mem->dev, byte size, auto max; both sides increment; 16-bit port.
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x01,
        SCR_BOTH_UP,
        4,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(&bus.data[0x2000..0x2004], &[0x11, 0x22, 0x33, 0x44]);
    // Packing turns 4 byte operands into 2 word reads + 2 word writes plus
    // the fixed 4-clock byte-mode correction per operand.
    assert_eq!(dmac.take_consumed_clocks(), 2 * 5 + 2 * 5 + 4 * 4);
}

#[test]
fn unpacked_bytes_move_one_byte_per_operand() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x1000..0x1002].copy_from_slice(&[0x5A, 0xA5]);
    // OCR: mem->dev, unpacked 8-bit, auto max; 8-bit port required.
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_8_BIT,
        0x31,
        SCR_BOTH_UP,
        2,
        0x1000,
        0x2001,
    );
    start(&mut dmac, &mut bus, 0);
    // The 8-bit port doubles the device step: bytes land at 0x2001, 0x2003.
    assert_eq!(bus.data[0x2001], 0x5A);
    assert_eq!(bus.data[0x2003], 0xA5);
    assert_eq!(dmac.take_consumed_clocks(), 2 * 10);
}

#[test]
fn eight_bit_port_word_writes_split_across_alternate_addresses() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x1000..0x1002].copy_from_slice(&[0xBE, 0xEF]);
    // OCR: mem->dev, word size, auto max; 8-bit port, device static.
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_8_BIT,
        0x11,
        SCR_MEMORY_UP,
        1,
        0x1000,
        0x2001,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(bus.data[0x2001], 0xBE, "high byte at DAR");
    assert_eq!(bus.data[0x2003], 0xEF, "low byte at DAR + 2");
    // Read word (5) + two byte writes (10) + word mem->dev correction (4).
    assert_eq!(dmac.take_consumed_clocks(), 19);
}

#[test]
fn decrement_and_static_stepping() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x1000..0x1006].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
    // OCR: mem->dev, word, auto max; memory increments, device decrements.
    program(
        &mut dmac, &mut bus, 0, DCR_16_BIT, 0x11, 0x06, 3, 0x1000, 0x2004,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(&bus.data[0x2004..0x2006], &[1, 2]);
    assert_eq!(&bus.data[0x2002..0x2004], &[3, 4]);
    assert_eq!(&bus.data[0x2000..0x2002], &[5, 6]);
    assert_eq!(read_u32(&mut dmac, 0, 0x14), 0x1FFE);
}

#[test]
fn array_chain_reads_entries_and_sets_block_complete() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x1000] = 0xAA;
    bus.data[0x1100] = 0xBB;
    // Array table at 0x3000: two entries of (address.l, count.w).
    bus.data[0x3000..0x3006].copy_from_slice(&[0x00, 0x00, 0x10, 0x00, 0x00, 0x01]);
    bus.data[0x3006..0x300C].copy_from_slice(&[0x00, 0x00, 0x11, 0x00, 0x00, 0x01]);

    // OCR: mem->dev, unpacked, array chain, auto max; 8-bit port.
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_8_BIT,
        0x39,
        SCR_MEMORY_UP,
        0,
        0,
        0x2001,
    );
    write_u16(&mut dmac, &mut bus, 0, 0x1A, 2);
    write_u32(&mut dmac, &mut bus, 0, 0x1C, 0x3000);
    start(&mut dmac, &mut bus, 0);

    assert_eq!(bus.data[0x2001], 0xBB, "both blocks landed on the device");
    let csr = dmac.read_register(offset(0, 0x00));
    assert_eq!(csr & 0xC0, 0xC0, "COC and BLC set after array chain");
    assert_eq!(dmac.read_register(offset(0, 0x1B)), 0, "BTC exhausted");
}

#[test]
fn linked_array_chain_terminates_on_zero_link() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x1000] = 0x77;
    bus.data[0x1100] = 0x88;
    // Linked entries: (address.l, count.w, next.l).
    bus.data[0x3000..0x300A]
        .copy_from_slice(&[0x00, 0x00, 0x10, 0x00, 0x00, 0x01, 0x00, 0x00, 0x30, 0x10]);
    bus.data[0x3010..0x301A]
        .copy_from_slice(&[0x00, 0x00, 0x11, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);

    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_8_BIT,
        0x3D,
        SCR_MEMORY_UP,
        0,
        0,
        0x2001,
    );
    write_u32(&mut dmac, &mut bus, 0, 0x1C, 0x3000);
    start(&mut dmac, &mut bus, 0);

    assert_eq!(bus.data[0x2001], 0x88);
    let csr = dmac.read_register(offset(0, 0x00));
    assert_eq!(csr & 0xC0, 0xC0, "COC and BLC set after linked chain");
}

#[test]
fn continue_mode_reloads_and_interrupts() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x1000..0x1004].copy_from_slice(&[1, 2, 3, 4]);
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        1,
        0x1000,
        0x2000,
    );
    write_u16(&mut dmac, &mut bus, 0, 0x1A, 1);
    write_u32(&mut dmac, &mut bus, 0, 0x1C, 0x1002);
    // CCR: STR | CNT | ITE.
    write(&mut dmac, &mut bus, 0, 0x07, 0xC8);

    assert_eq!(&bus.data[0x2000..0x2002], &[1, 2], "first block");
    assert_eq!(&bus.data[0x2002..0x2004], &[3, 4], "continued block");
    // The final completion clears the intermediate block-complete flag.
    let csr = dmac.read_register(offset(0, 0x00));
    assert_eq!(csr & 0xC0, 0x80, "COC set, BLC cleared at operation end");
    // Block completion and operation completion both queued interrupts.
    assert!(dmac.irq_asserted());
    assert_eq!(dmac.acknowledge_interrupt(), Some(0x0F));
}

#[test]
fn halt_suspends_and_resume_finishes() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    for (index, byte) in bus.data[0x1000..0x1020].iter_mut().enumerate() {
        *byte = index as u8;
    }
    // Auto limited rate so the operation yields between bursts.
    write(&mut dmac, &mut bus, 0, 0xFF, 0x00);
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x10,
        SCR_BOTH_UP,
        16,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert!(dmac.channel_active(0));
    let resume_clock = dmac.next_work_clock().expect("burst scheduled");

    // Halt: scheduled work is withdrawn.
    write(&mut dmac, &mut bus, 0, 0x07, 0x20);
    assert_eq!(dmac.next_work_clock(), None);
    assert_eq!(dmac.read_register(offset(0, 0x07)) & 0x20, 0x20);

    // Resume: work is rescheduled and completes.
    dmac.write_register(offset(0, 0x07), 0x08, &mut bus, resume_clock);
    let mut clock = dmac.next_work_clock().expect("rescheduled");
    while dmac.channel_active(0) {
        dmac.run_due(&mut bus, clock);
        clock = dmac.next_work_clock().unwrap_or(clock + 1);
    }
    assert_eq!(
        &bus.data[0x2000..0x2020],
        &bus.data[0x1000..0x1020].to_vec()[..]
    );
}

#[test]
fn gcr_paces_limited_rate_bursts() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    // BT=0 (span 16 clocks), BR=0 (interval 32 clocks).
    write(&mut dmac, &mut bus, 0, 0xFF, 0x00);
    // Word operands cost 10 clocks: two fit into one 16-clock span.
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x10,
        SCR_BOTH_UP,
        6,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);

    assert_eq!(dmac.take_consumed_clocks(), 20, "two operands in burst one");
    assert_eq!(
        dmac.next_work_clock(),
        Some(32),
        "next burst at the interval"
    );

    dmac.run_due(&mut bus, 32);
    assert_eq!(dmac.take_consumed_clocks(), 20);
    assert_eq!(dmac.next_work_clock(), Some(64));

    dmac.run_due(&mut bus, 64);
    assert!(
        !dmac.channel_active(0),
        "third burst finishes the operation"
    );
    assert_eq!(dmac.next_work_clock(), None);

    // BT=3 (span 128) fits all remaining operands into the first burst.
    write(&mut dmac, &mut bus, 0, 0x00, 0xFF);
    write(&mut dmac, &mut bus, 0, 0xFF, 0x0C);
    dmac.take_consumed_clocks();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x10,
        SCR_BOTH_UP,
        6,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert!(!dmac.channel_active(0));
    assert_eq!(dmac.take_consumed_clocks(), 60);
}

#[test]
fn external_request_transfers_one_operand_per_edge() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x2001] = 0xAA;
    bus.data[0x2003] = 0xBB;
    // OCR: dev->mem, unpacked, external request; 8-bit port; device up.
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_8_BIT,
        0xB2,
        SCR_BOTH_UP,
        2,
        0x1000,
        0x2001,
    );
    start(&mut dmac, &mut bus, 0);
    assert!(dmac.channel_active(0));
    assert_eq!(bus.data[0x1000], 0, "no transfer before the first edge");

    dmac.assert_request(0, &mut bus, 0);
    assert_eq!(bus.data[0x1000], 0xAA);
    assert!(dmac.channel_active(0));

    dmac.assert_request(0, &mut bus, 0);
    assert_eq!(bus.data[0x1001], 0xBB);
    assert!(!dmac.channel_active(0));
    assert_eq!(dmac.take_channel_completions(), 0x01);
}

#[test]
fn dual_request_runs_first_operand_automatically() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    bus.data[0x2001] = 0x11;
    bus.data[0x2003] = 0x22;
    // OCR: dev->mem, unpacked, dual request.
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_8_BIT,
        0xB3,
        SCR_BOTH_UP,
        2,
        0x1000,
        0x2001,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(bus.data[0x1000], 0x11, "first operand at start");
    dmac.assert_request(0, &mut bus, 0);
    assert_eq!(bus.data[0x1001], 0x22);
}

#[test]
fn configuration_errors_set_cer() {
    // Undefined XRM encoding.
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        0x40 | DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        1,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_CONFIGURATION);

    // Unpacked size on a 16-bit port.
    let mut dmac = Hd63450Dmac::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x31,
        SCR_BOTH_UP,
        1,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_CONFIGURATION);

    // Byte size on a 16-bit port with external requests.
    let mut dmac = Hd63450Dmac::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x02,
        SCR_BOTH_UP,
        1,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_CONFIGURATION);
}

#[test]
fn timing_count_and_address_errors_set_cer() {
    // STR with an uncleared CSR.
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        1,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_TIMING);

    // MTC of zero.
    let mut dmac = Hd63450Dmac::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        0,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_MEMORY_COUNT);

    // Odd memory address for a word transfer.
    let mut dmac = Hd63450Dmac::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        1,
        0x1001,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_MEMORY_ADDRESS);

    // Array chain with BTC of zero, then with an odd base address.
    let mut dmac = Hd63450Dmac::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x19,
        SCR_BOTH_UP,
        0,
        0,
        0x2000,
    );
    write_u32(&mut dmac, &mut bus, 0, 0x1C, 0x3000);
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_BASE_COUNT);

    let mut dmac = Hd63450Dmac::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x19,
        SCR_BOTH_UP,
        0,
        0,
        0x2000,
    );
    write_u16(&mut dmac, &mut bus, 0, 0x1A, 1);
    write_u32(&mut dmac, &mut bus, 0, 0x1C, 0x3001);
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_BASE_ADDRESS);
}

#[test]
fn bus_faults_set_memory_device_and_base_error_codes() {
    // Memory side fault (reading beyond the bus).
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        1,
        0x40000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_MEMORY_BUS);
    assert_eq!(dmac.read_register(offset(0, 0x00)) & 0x10, 0x10, "ERR set");

    // Device side fault.
    let mut dmac = Hd63450Dmac::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        1,
        0x1000,
        0x40000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_DEVICE_BUS);

    // Base table fault during an array chain fetch.
    let mut dmac = Hd63450Dmac::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x19,
        SCR_BOTH_UP,
        0,
        0,
        0x2000,
    );
    write_u16(&mut dmac, &mut bus, 0, 0x1A, 1);
    write_u32(&mut dmac, &mut bus, 0, 0x1C, 0x40000);
    start(&mut dmac, &mut bus, 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_BASE_BUS);
}

#[test]
fn software_abort_sets_cer_0x11() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    write(&mut dmac, &mut bus, 0, 0xFF, 0x00);
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x10,
        SCR_BOTH_UP,
        16,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert!(dmac.channel_active(0));

    write(&mut dmac, &mut bus, 0, 0x07, 0x10);
    assert!(!dmac.channel_active(0));
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_SOFTWARE_ABORT);
}

#[test]
fn iack_selects_error_vector_and_respects_priority() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();

    // Channel 1 completes normally with NIV 0x6A and priority 1.
    write(&mut dmac, &mut bus, 1, 0x25, 0x6A);
    write(&mut dmac, &mut bus, 1, 0x2D, 0x01);
    program(
        &mut dmac,
        &mut bus,
        1,
        DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        1,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 1);

    // Channel 2 errors with EIV 0x6B and priority 0 (higher).
    write(&mut dmac, &mut bus, 2, 0x27, 0x6B);
    write(&mut dmac, &mut bus, 2, 0x2D, 0x00);
    program(
        &mut dmac,
        &mut bus,
        2,
        DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        1,
        0x1001,
        0x2000,
    );
    start(&mut dmac, &mut bus, 2);

    assert!(dmac.irq_asserted());
    assert_eq!(
        dmac.acknowledge_interrupt(),
        Some(0x6B),
        "error channel first"
    );
    assert_eq!(dmac.acknowledge_interrupt(), Some(0x6A));
    assert_eq!(dmac.acknowledge_interrupt(), None);
    assert!(!dmac.irq_asserted());
}

#[test]
fn csr_write_clears_status_and_cer() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x11,
        SCR_BOTH_UP,
        1,
        0x1001,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert_ne!(
        dmac.read_register(offset(0, 0x00)) & 0x90,
        0,
        "COC and ERR set"
    );

    // Clearing COC alone keeps ERR and CER.
    write(&mut dmac, &mut bus, 0, 0x00, 0x80);
    assert_eq!(dmac.read_register(offset(0, 0x00)) & 0x10, 0x10);
    assert_ne!(dmac.read_register(offset(0, 0x01)), 0);

    // Clearing ERR also clears CER and drops the interrupt.
    write(&mut dmac, &mut bus, 0, 0x00, 0x10);
    assert_eq!(dmac.read_register(offset(0, 0x00)), 0);
    assert_eq!(dmac.read_register(offset(0, 0x01)), 0);
    assert!(!dmac.irq_asserted());
}

#[test]
fn register_writes_while_active_abort_with_timing_error() {
    let mut dmac = Hd63450Dmac::new();
    let mut bus = TestBus::new();
    write(&mut dmac, &mut bus, 0, 0xFF, 0x00);
    program(
        &mut dmac,
        &mut bus,
        0,
        DCR_16_BIT,
        0x10,
        SCR_BOTH_UP,
        16,
        0x1000,
        0x2000,
    );
    start(&mut dmac, &mut bus, 0);
    assert!(dmac.channel_active(0));

    write_u32(&mut dmac, &mut bus, 0, 0x0C, 0x1234);
    assert!(!dmac.channel_active(0));
    assert_eq!(dmac.read_register(offset(0, 0x01)), ERROR_TIMING);
}

#[test]
fn pcl_falling_edge_latches_transition() {
    let mut dmac = Hd63450Dmac::new();
    dmac.set_peripheral_control_line(0, true);
    assert_eq!(dmac.read_register(offset(0, 0x00)) & 0x03, 0x01, "PCS high");
    dmac.set_peripheral_control_line(0, false);
    assert_eq!(
        dmac.read_register(offset(0, 0x00)) & 0x03,
        0x02,
        "PCT latched"
    );
    let mut bus = TestBus::new();
    write(&mut dmac, &mut bus, 0, 0x00, 0x02);
    assert_eq!(dmac.read_register(offset(0, 0x00)) & 0x03, 0x00);
}
