use super::{create_machine_pc9821as_empty_hdd, create_machine_pc9821as_hdd, read_ram_u16};

const BDA_DISK_EQUIP: usize = 0x055C;
const BDA_F2HD_MODE: usize = 0x0493;
const BDA_BOOT_DEVICE: usize = 0x0584;

#[test]
fn ide_initialization_pc9821() {
    let mut machine = create_machine_pc9821as_hdd();
    let _cycles = boot_to_halt_hdd!(machine);
    let state = machine.save_state();

    let disk_equip = read_ram_u16(&state.memory.ram, BDA_DISK_EQUIP);
    assert_eq!(
        disk_equip, 0x0103,
        "DISK_EQUIP should have IDE drive 0 + 2 FDD drives (got {disk_equip:#06X})"
    );
    assert_eq!(
        state.memory.ram[BDA_BOOT_DEVICE], 0x80,
        "BOOT_DEVICE should be 0x80 for IDE-0"
    );
    assert_eq!(state.memory.ram[BDA_F2HD_MODE], 0xFF, "F2HD_MODE");
    assert_eq!(state.memory.ram[0x1FC00], 0xFA, "Boot sector byte 0 (CLI)");
    assert_eq!(state.memory.ram[0x1FC01], 0xF4, "Boot sector byte 1 (HLT)");
}

#[test]
fn ide_empty_boot_sector_falls_through_to_fdd() {
    let mut machine = create_machine_pc9821as_empty_hdd();
    let _cycles = boot_to_halt!(machine);
    let state = machine.save_state();
    assert_eq!(
        state.memory.ram[BDA_BOOT_DEVICE], 0x90,
        "BOOT_DEVICE should be 0x90 for FDD-0 (got {:#04X})",
        state.memory.ram[BDA_BOOT_DEVICE]
    );
    assert_eq!(state.memory.ram[0x1FC00], 0xFA, "Boot sector byte 0 (CLI)");
    assert_eq!(state.memory.ram[0x1FC01], 0xF4, "Boot sector byte 1 (HLT)");
}
