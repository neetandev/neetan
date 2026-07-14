//! Full-machine interrupt tests: vectored delivery through the real CPU and
//! motherboard-level priority arbitration across all interrupt levels.

#[path = "common/harness.rs"]
mod harness;

use common::Bus;
use device::floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage};
use harness::{
    STOP_MASKED, byte_write_script, machine, read_byte, run_pending_events, scripted_machine,
    write_byte,
};
use machine_x68k::{X68kMachine, X68kModel};

/// MFP register writes arming timer A with a 16-count, divide-by-4 delay
/// and the auto-EOI vector base 0x40.
const MFP_TIMER_A_SETUP: [(u32, u8); 5] = [
    (0xE88017, 0x40),
    (0xE88007, 0x20),
    (0xE88013, 0x20),
    (0xE8801F, 16),
    (0xE88019, 0x01),
];

/// SCC channel B writes programming the Human68k mouse setup followed by an
/// MSCTRL request edge.
const SCC_MOUSE_SETUP: [(u32, u8); 10] = [
    (0xE98001, 9),
    (0xE98001, 0x09),
    (0xE98001, 2),
    (0xE98001, 0x40),
    (0xE98001, 1),
    (0xE98001, 0x10),
    (0xE98001, 5),
    (0xE98001, 0x60),
    (0xE98001, 5),
    (0xE98001, 0x62),
];

/// One mouse byte time in CPU cycles at 10 MHz.
const MOUSE_BYTE_CYCLES: u64 = 23_232;

/// Builds a one-sector in-memory D88 disk.
fn probe_disk() -> FloppyImage {
    let sector = D88Sector {
        cylinder: 0,
        head: 0,
        record: 1,
        size_code: 3,
        sector_count: 1,
        mfm_flag: 0x00,
        deleted: 0x00,
        status: 0x00,
        reserved: [0; 5],
        data: vec![0x5A; 1024],
        source_offset: None,
    };
    let disk = D88Disk::from_tracks(
        String::from("PROBE"),
        false,
        D88MediaType::Disk2HD,
        vec![Some(vec![sector])],
    );
    FloppyImage::from_d88(disk)
}

#[test]
fn mfp_timer_a_delivers_a_vectored_level_6_interrupt_to_the_cpu() {
    // Vector 0x4D (base 0x40, MFP channel 13) lives at address 0x134.
    let mut program = Vec::new();
    let handler_words = 5 + MFP_TIMER_A_SETUP.len() as u32 * 4 + 2;
    let handler_address = 0x00FE_0008 + handler_words * 2;
    program.extend([
        0x23FC,
        (handler_address >> 16) as u16,
        handler_address as u16,
        0x0000,
        0x0134,
    ]);
    program.extend(byte_write_script(&MFP_TIMER_A_SETUP));
    // stop #0x2000: wait with all interrupt levels open.
    program.extend([0x4E72, 0x2000]);
    // handler: mark RAM and halt.
    program.extend([0x13FC, 0x00A5, 0x0000, 0x2000, 0x4E72, 0x2700]);

    let mut machine = scripted_machine(X68kModel::X68000, &program);
    machine.run_for(50_000);
    assert_eq!(
        machine.bus.ram_byte(0x2000),
        Some(0xA5),
        "the timer A handler must have run"
    );
}

#[test]
fn scc_mouse_packet_reaches_the_cpu_one_paced_byte_at_a_time() {
    // Vector 0x44 (SCC channel B receive) lives at address 0x110.
    let mut program = Vec::new();
    let handler_words = 5 + SCC_MOUSE_SETUP.len() as u32 * 4 + 8;
    let handler_address = 0x00FE_0008 + handler_words * 2;
    program.extend([
        0x23FC,
        (handler_address >> 16) as u16,
        handler_address as u16,
        0x0000,
        0x0110,
    ]);
    program.extend(byte_write_script(&SCC_MOUSE_SETUP));
    // Three interruptible waits, one per packet byte, then a masked halt.
    program.extend([0x4E72, 0x2000, 0x4E72, 0x2000, 0x4E72, 0x2000]);
    program.extend(STOP_MASKED);
    // handler: store the byte, count it, reset the IUS, return.
    program.extend([
        0x13F9, 0x00E9, 0x8003, 0x0000, 0x2000, // move.b (0xE98003).l, (0x2000).l
        0x5239, 0x0000, 0x2001, // addq.b #1, (0x2001).l
        0x13FC, 0x0038, 0x00E9, 0x8001, // move.b #0x38, (0xE98001).l
        0x4E73, // rte
    ]);

    let mut machine = scripted_machine(X68kModel::X68000, &program);
    machine.bus.push_mouse_delta(5, -3);
    machine.bus.set_mouse_buttons(true, false);
    machine.run_for(5 * MOUSE_BYTE_CYCLES);
    assert_eq!(
        machine.bus.ram_byte(0x2001),
        Some(3),
        "all three packet bytes must interrupt"
    );
    assert_eq!(
        machine.bus.ram_byte(0x2000),
        Some(0xFD),
        "the last delivered byte is the Y delta"
    );
}

/// Programs and completes an interrupt-enabled DMAC channel 0 transfer.
fn complete_dmac_transfer(machine: &mut X68kMachine) {
    for index in 0..4u32 {
        write_byte(machine, 0x2000 + index, 0xC0 | index as u8);
    }
    write_byte(machine, 0xE84025, 0x72);
    write_byte(machine, 0xE84004, 0x08);
    write_byte(machine, 0xE84005, 0x11);
    write_byte(machine, 0xE84006, 0x05);
    write_byte(machine, 0xE8400A, 0x00);
    write_byte(machine, 0xE8400B, 0x02);
    for (index, byte) in 0x0000_2000u32.to_be_bytes().into_iter().enumerate() {
        write_byte(machine, 0xE8400C + index as u32, byte);
    }
    for (index, byte) in 0x0000_3000u32.to_be_bytes().into_iter().enumerate() {
        write_byte(machine, 0xE84014 + index as u32, byte);
    }
    write_byte(machine, 0xE84007, 0x88);
}

#[test]
fn dmac_completion_interrupts_at_level_3_with_the_programmed_vector() {
    let mut machine = machine(X68kModel::X68000);
    complete_dmac_transfer(&mut machine);
    run_pending_events(&mut machine, 64);
    assert_eq!(machine.bus.m68000_interrupt_level(), 3);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(3), 0x72);
    assert_eq!(machine.bus.m68000_interrupt_level(), 0);
}

#[test]
fn ioc_fdd_change_interrupts_at_level_1_with_subvector_one() {
    let mut machine = machine(X68kModel::X68000);
    write_byte(&mut machine, 0xE9C001, 0x0F);
    write_byte(&mut machine, 0xE9C003, 0x40);
    machine.bus.insert_floppy(0, probe_disk(), None).unwrap();
    assert_eq!(machine.bus.m68000_interrupt_level(), 1);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(1), 0x41);
}

#[test]
fn simultaneous_interrupts_resolve_from_level_6_down() {
    let mut machine = machine(X68kModel::X68000);

    // Level 1: IOC FDD status change.
    write_byte(&mut machine, 0xE9C001, 0x0F);
    write_byte(&mut machine, 0xE9C003, 0x40);
    machine.bus.insert_floppy(0, probe_disk(), None).unwrap();

    // Level 5: a latched mouse packet byte.
    for (address, value) in SCC_MOUSE_SETUP {
        write_byte(&mut machine, address, value);
    }

    // Level 6: MFP timer A.
    for (address, value) in MFP_TIMER_A_SETUP {
        write_byte(&mut machine, address, value);
    }

    // Advance far enough for the timer and the first mouse byte.
    while machine.bus.current_cycle() < 2 * MOUSE_BYTE_CYCLES {
        run_pending_events(&mut machine, 1);
    }

    // Level 3: an immediate DMAC completion.
    complete_dmac_transfer(&mut machine);

    assert_eq!(machine.bus.m68000_interrupt_level(), 6);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(6), 0x4D);
    assert_eq!(machine.bus.m68000_interrupt_level(), 5);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(5), 0x44);
    assert_eq!(machine.bus.m68000_interrupt_level(), 3);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(3), 0x72);
    assert_eq!(machine.bus.m68000_interrupt_level(), 1);
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(1), 0x41);
    assert_eq!(machine.bus.m68000_interrupt_level(), 0);
}

#[test]
fn opm_timer_reaches_level_6_through_the_active_low_gpip3_edge() {
    let mut machine = machine(X68kModel::X68000);
    // MFP: vector base 0x40, GPIP3 falling-edge interrupt enabled.
    write_byte(&mut machine, 0xE88017, 0x40);
    write_byte(&mut machine, 0xE88003, 0x00);
    write_byte(&mut machine, 0xE88009, 0x08);
    write_byte(&mut machine, 0xE88015, 0x08);
    // OPM: a short timer A, loaded with its interrupt enabled.
    write_byte(&mut machine, 0xE90001, 0x10);
    write_byte(&mut machine, 0xE90003, 0xFF);
    write_byte(&mut machine, 0xE90001, 0x11);
    write_byte(&mut machine, 0xE90003, 0x03);
    write_byte(&mut machine, 0xE90001, 0x14);
    write_byte(&mut machine, 0xE90003, 0x05);

    for _ in 0..64 {
        if machine.bus.m68000_interrupt_level() == 6 {
            break;
        }
        run_pending_events(&mut machine, 1);
    }
    assert_eq!(machine.bus.m68000_interrupt_level(), 6);
    // GPIP3 is MFP channel 3: vector 0x43.
    assert_eq!(machine.bus.m68000_acknowledge_interrupt(6), 0x43);
    // The OPM status register reports the expired timer A.
    assert_ne!(read_byte(&mut machine, 0xE90003) & 0x01, 0);
}
