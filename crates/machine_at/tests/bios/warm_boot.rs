//! Warm versus cold reboot through the BDA reset flag at 40:72.
//!
//! Probed on the real AMI BIOS (see tempt_real_bios_probes.rs): the POST
//! memory test zeroes conventional memory on every boot path, and the reset
//! flag ends as 0x1200 after a warm boot (40:72 was 0x1234) or 0 after a
//! cold one.

use common::{Cpu, Machine};

use super::{
    IDLE_LOOP_CODE, create_machine_dx50, inject_and_run, make_halt_boot_floppy, read_ram_u8,
    read_ram_u16, write_bytes,
};

/// Conventional memory markers planted before a reboot.
const MARKERS: &[(u32, u8)] = &[(0x600, 0x5A), (0x50000, 0xC3), (0x9F000, 0xA5)];

/// Fast reset through port 0x92 bit 0.
#[rustfmt::skip]
const PORT_92_RESET_CODE: &[u8] = &[
    0xB0, 0x01,             // MOV AL, 0x01
    0xE6, 0x92,             // OUT 0x92, AL
    0xF4,                   // HLT (never reached)
];

fn boot_and_plant_markers() -> machine_at::AtMachine<common::NoTrace> {
    let mut machine = create_machine_dx50();
    machine
        .bus
        .insert_floppy(0, make_halt_boot_floppy(), None)
        .expect("insert boot floppy");
    boot_to_halt!(machine);
    for &(address, value) in MARKERS {
        write_bytes(&mut machine, address, &[value]);
    }
    machine
}

fn assert_rebooted_state(machine: &machine_at::AtMachine<common::NoTrace>, reset_flag: u16) {
    assert!(machine.cpu.halted(), "re-halted at the boot sector");
    for &(address, value) in MARKERS {
        assert_eq!(
            read_ram_u8(machine, address),
            0,
            "POST wiped the marker {value:#04X} at {address:#07X}"
        );
    }
    assert_eq!(read_ram_u16(machine, 0x472), reset_flag, "BDA 40:72");
    assert_eq!(read_ram_u16(machine, 0x413), 640, "BDA rebuilt");
}

#[test]
fn ctrl_alt_del_reboots_with_the_warm_residue() {
    let mut machine = boot_and_plant_markers();
    machine.push_keyboard_scancode(0x1D);
    machine.push_keyboard_scancode(0x38);
    machine.push_keyboard_scancode(0x53);
    inject_and_run(&mut machine, IDLE_LOOP_CODE, &[], 100_000_000);
    assert_rebooted_state(&machine, 0x1200);
}

#[test]
fn port_92_reset_reboots_cold() {
    let mut machine = boot_and_plant_markers();
    inject_and_run(&mut machine, PORT_92_RESET_CODE, &[], 100_000_000);
    assert_rebooted_state(&machine, 0x0000);
}

#[test]
fn warm_flag_survives_a_port_92_reset() {
    let mut machine = boot_and_plant_markers();
    write_bytes(&mut machine, 0x472, &[0x34, 0x12]);
    inject_and_run(&mut machine, PORT_92_RESET_CODE, &[], 100_000_000);
    assert_rebooted_state(&machine, 0x1200);
}
