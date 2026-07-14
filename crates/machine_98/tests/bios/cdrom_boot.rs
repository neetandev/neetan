use super::{
    create_machine_pc9821as_cdrom, create_machine_pc9821as_non_bootable_cdrom,
    make_halt_boot_ide_hdd, read_ram_u16,
};

const BDA_DISK_EQUIP: usize = 0x055C;
const BDA_BOOT_DEVICE: usize = 0x0584;

#[test]
fn cdrom_boot_pc9821() {
    let mut machine = create_machine_pc9821as_cdrom();
    let _cycles = boot_to_halt_hdd!(machine);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[BDA_BOOT_DEVICE], 0x82,
        "BOOT_DEVICE should be 0x82 for CD-ROM (got {:#04X})",
        state.memory.ram[BDA_BOOT_DEVICE]
    );

    let disk_equip = read_ram_u16(&state.memory.ram, BDA_DISK_EQUIP);
    assert!(
        disk_equip & 0x0400 != 0,
        "DISK_EQUIP should have CD-ROM bit set (got {disk_equip:#06X})"
    );

    assert_eq!(state.memory.ram[0x1FC00], 0xFA, "Boot sector byte 0 (CLI)");
    assert_eq!(state.memory.ram[0x1FC01], 0xF4, "Boot sector byte 1 (HLT)");
}

#[test]
fn cdrom_boot_requires_signature() {
    let mut machine = create_machine_pc9821as_non_bootable_cdrom();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[BDA_BOOT_DEVICE], 0x90,
        "BOOT_DEVICE should be 0x90 for FDD-0 when CD-ROM lacks signature (got {:#04X})",
        state.memory.ram[BDA_BOOT_DEVICE]
    );
}

#[test]
fn cdrom_boot_priority_over_ide_hdd() {
    let mut machine = create_machine_pc9821as_cdrom();
    machine.bus.insert_hdd(0, make_halt_boot_ide_hdd(), None);
    let _cycles = boot_to_halt_hdd!(machine);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[BDA_BOOT_DEVICE], 0x82,
        "BOOT_DEVICE should be 0x82 for CD-ROM, not 0x80 for IDE HDD (got {:#04X})",
        state.memory.ram[BDA_BOOT_DEVICE]
    );
}

#[test]
fn ide_hdd_boots_when_cdrom_not_bootable() {
    let mut machine = create_machine_pc9821as_non_bootable_cdrom();
    machine.bus.insert_hdd(0, make_halt_boot_ide_hdd(), None);
    let _cycles = boot_to_halt_hdd!(machine);
    let state = machine.save_state();

    assert_eq!(
        state.memory.ram[BDA_BOOT_DEVICE], 0x80,
        "BOOT_DEVICE should be 0x80 for IDE HDD when CD-ROM is not bootable (got {:#04X})",
        state.memory.ram[BDA_BOOT_DEVICE]
    );
}
